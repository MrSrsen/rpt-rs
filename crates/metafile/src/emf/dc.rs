//! The EMF device context: the coordinate transform machinery and the GDI object/graphics-state
//! stack that turn record coordinates into device-space [`Point`]s.

use crate::{Brush, Color, Font, Pen, Point};

/// A 2-D affine transform in GDI's `XFORM` convention: a point `(x, y)` maps to
/// `(m11·x + m21·y + dx, m12·x + m22·y + dy)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Matrix {
    pub m11: f64,
    pub m12: f64,
    pub m21: f64,
    pub m22: f64,
    pub dx: f64,
    pub dy: f64,
}

impl Matrix {
    /// The identity transform.
    pub(crate) const IDENTITY: Matrix = Matrix {
        m11: 1.0,
        m12: 0.0,
        m21: 0.0,
        m22: 1.0,
        dx: 0.0,
        dy: 0.0,
    };

    /// A pure scale-and-translate transform: `(x, y) → (x·sx + tx, y·sy + ty)`.
    pub(crate) fn scale_translate(sx: f64, sy: f64, tx: f64, ty: f64) -> Matrix {
        Matrix {
            m11: sx,
            m12: 0.0,
            m21: 0.0,
            m22: sy,
            dx: tx,
            dy: ty,
        }
    }

    /// Apply this transform to a point.
    pub(crate) fn apply(&self, x: f64, y: f64) -> Point {
        Point {
            x: self.m11 * x + self.m21 * y + self.dx,
            y: self.m12 * x + self.m22 * y + self.dy,
        }
    }

    /// The transform that applies `self` first, then `other` — i.e. `other ∘ self`.
    pub(crate) fn then(&self, other: &Matrix) -> Matrix {
        Matrix {
            m11: self.m11 * other.m11 + self.m12 * other.m21,
            m12: self.m11 * other.m12 + self.m12 * other.m22,
            m21: self.m21 * other.m11 + self.m22 * other.m21,
            m22: self.m21 * other.m12 + self.m22 * other.m22,
            dx: self.dx * other.m11 + self.dy * other.m21 + other.dx,
            dy: self.dx * other.m12 + self.dy * other.m22 + other.dy,
        }
    }

    /// The geometric mean of the absolute axis scales — used to map a scalar logical length (a pen
    /// width) into device space.
    pub(crate) fn mean_scale(&self) -> f64 {
        let sx = (self.m11 * self.m11 + self.m12 * self.m12).sqrt();
        let sy = (self.m21 * self.m21 + self.m22 * self.m22).sqrt();
        (sx * sy).sqrt()
    }
}

/// A GDI object stored in the metafile's handle table, created by a `CREATE*` record and made
/// current by `SELECTOBJECT`.
#[derive(Clone, Debug)]
pub(crate) enum GdiObject {
    /// A pen: its stroke, or `None` for a `PS_NULL` (invisible) pen.
    Pen(Option<Pen>),
    /// A brush: its fill, or `None` for a `BS_NULL`/hollow brush.
    Brush(Option<Brush>),
    /// A font.
    Font(Font),
}

/// The mutable graphics state saved and restored by `SAVEDC`/`RESTOREDC`: everything that affects how
/// the next primitive is drawn or transformed.
#[derive(Clone, Debug)]
pub(crate) struct GraphicsFrame {
    pub pen: Option<Pen>,
    pub brush: Option<Brush>,
    pub font: Font,
    pub text_color: Color,
    /// The world transform (`SETWORLDTRANSFORM`/`MODIFYWORLDTRANSFORM`).
    pub world: Matrix,
    /// The page (map-mode + window→viewport) transform, applied after `world`. Derived from the
    /// window/viewport origin and extent below; recomputed whenever they change.
    pub page: Matrix,
    /// Window/viewport origin and extent, tracked so the page transform can be recomputed when any
    /// part changes. Extents default to 1 (an `MM_TEXT` 1:1 mapping).
    win_org: (f64, f64),
    win_ext: (f64, f64),
    vp_org: (f64, f64),
    vp_ext: (f64, f64),
    /// Current position (logical units), set by `MOVETOEX` and consumed by `LINETO`.
    pub point: (f64, f64),
}

impl Default for GraphicsFrame {
    fn default() -> GraphicsFrame {
        GraphicsFrame {
            // A freshly created DC selects a black solid pen and a white brush.
            pen: Some(Pen {
                color: Color::BLACK,
                width: 0.0,
                style: crate::PenStyle::Solid,
            }),
            brush: Some(Brush {
                color: Color::WHITE,
            }),
            font: Font::default(),
            text_color: Color::BLACK,
            world: Matrix::IDENTITY,
            page: Matrix::IDENTITY,
            win_org: (0.0, 0.0),
            win_ext: (1.0, 1.0),
            vp_org: (0.0, 0.0),
            vp_ext: (1.0, 1.0),
            point: (0.0, 0.0),
        }
    }
}

/// The device context: the current graphics state, the `SAVEDC` stack, and the handle table.
pub(crate) struct Dc {
    pub cur: GraphicsFrame,
    saved: Vec<GraphicsFrame>,
    objects: Vec<Option<GdiObject>>,
}

impl Dc {
    pub(crate) fn new() -> Dc {
        Dc {
            cur: GraphicsFrame::default(),
            saved: Vec::new(),
            objects: Vec::new(),
        }
    }

    /// The current transform: world first, then the page (window→viewport) transform.
    pub(crate) fn ctm(&self) -> Matrix {
        self.cur.world.then(&self.cur.page)
    }

    /// Map a logical point through the current transform into device space.
    pub(crate) fn map(&self, x: f64, y: f64) -> Point {
        self.ctm().apply(x, y)
    }

    /// Push the current graphics state (`SAVEDC`).
    pub(crate) fn save(&mut self) {
        self.saved.push(self.cur.clone());
    }

    /// Pop back to a saved graphics state (`RESTOREDC`). `depth` is the record's signed argument: a
    /// negative `-n` restores the n-th state from the top of the stack (the common case); a positive
    /// absolute index is treated the same way relative to the stack top. A depth with no matching
    /// saved state is ignored.
    pub(crate) fn restore(&mut self, depth: i32) {
        let n = if depth < 0 {
            depth.unsigned_abs() as usize
        } else if depth >= 1 {
            // A positive absolute index counts from the bottom; convert to a from-top count.
            self.saved.len().saturating_sub(depth as usize - 1)
        } else {
            return;
        };
        if n == 0 || n > self.saved.len() {
            return;
        }
        // Drop the intervening states and restore the target.
        self.saved.truncate(self.saved.len() - n + 1);
        if let Some(frame) = self.saved.pop() {
            self.cur = frame;
        }
    }

    /// Store a GDI object in the handle table at `index`, growing the table as needed.
    pub(crate) fn set_object(&mut self, index: u32, obj: GdiObject) {
        let index = index as usize;
        if index >= self.objects.len() {
            self.objects.resize_with(index + 1, || None);
        }
        self.objects[index] = Some(obj);
    }

    /// The object at `index`, if any.
    pub(crate) fn object(&self, index: u32) -> Option<&GdiObject> {
        self.objects.get(index as usize).and_then(|o| o.as_ref())
    }

    /// Release the handle-table slot at `index` (`DELETEOBJECT`).
    pub(crate) fn clear_object(&mut self, index: u32) {
        if let Some(slot) = self.objects.get_mut(index as usize) {
            *slot = None;
        }
    }

    // --- Page (window/viewport) transform ---------------------------------------------------------

    pub(crate) fn set_window_org(&mut self, x: f64, y: f64) {
        self.cur.win_org = (x, y);
        self.recompute_page();
    }

    pub(crate) fn set_window_ext(&mut self, x: f64, y: f64) {
        if x != 0.0 && y != 0.0 {
            self.cur.win_ext = (x, y);
            self.recompute_page();
        }
    }

    pub(crate) fn set_viewport_org(&mut self, x: f64, y: f64) {
        self.cur.vp_org = (x, y);
        self.recompute_page();
    }

    pub(crate) fn set_viewport_ext(&mut self, x: f64, y: f64) {
        if x != 0.0 && y != 0.0 {
            self.cur.vp_ext = (x, y);
            self.recompute_page();
        }
    }

    /// Rebuild the page transform from the window/viewport origin and extent:
    /// `device = (logical − win_org) · (vp_ext / win_ext) + vp_org`.
    fn recompute_page(&mut self) {
        let f = &mut self.cur;
        let sx = f.vp_ext.0 / f.win_ext.0;
        let sy = f.vp_ext.1 / f.win_ext.1;
        f.page = Matrix::scale_translate(
            sx,
            sy,
            f.vp_org.0 - f.win_org.0 * sx,
            f.vp_org.1 - f.win_org.1 * sy,
        );
    }
}
