//! The backend-agnostic output surface: the [`MetafileSink`] visitor, the [`GraphicsState`] handed to
//! its callbacks, and a ready-made [`Recording`] collector for consumers that just want a flat list
//! of [`Primitive`]s.

use crate::{Bitmap, Brush, Color, Font, Pen, Point, Rect};

/// The header facts a parser reports once, before any drawing callback.
#[derive(Clone, Debug, PartialEq)]
pub struct MetafileHeader {
    /// Inclusive device-unit bounds every drawing coordinate lives within (`rclBounds`). Map this
    /// rectangle onto your own output box to place the metafile.
    pub bounds: Rect,
    /// The picture's physical dimensions in units of 0.01 mm (`rclFrame`), if the format records
    /// them; use it to preserve the metafile's real-world size and aspect ratio.
    pub frame: Option<Rect>,
}

/// The fully resolved drawing state at the moment a primitive is emitted: the current pen, brush,
/// font, and text color, with the metafile's object selection and graphics-state stack already
/// applied. A `None` pen or brush means the corresponding attribute is disabled (a `PS_NULL` pen or
/// hollow brush), so a shape with `pen: None` has no outline and one with `brush: None` no fill.
#[derive(Clone, Copy, Debug)]
pub struct GraphicsState<'a> {
    /// The current pen, or `None` when no outline should be drawn.
    pub pen: Option<Pen>,
    /// The current brush, or `None` when the interior should not be filled.
    pub brush: Option<Brush>,
    /// The current font (for text runs).
    pub font: &'a Font,
    /// The current text color.
    pub text_color: Color,
}

/// A consumer of the vector primitives decoded from a metafile.
///
/// Implement the callbacks you care about; every method has a no-op default, so a sink that only
/// needs shapes can ignore text, and vice versa. Each drawing callback receives geometry already
/// transformed into the metafile's device space (see [`MetafileHeader::bounds`]) together with the
/// resolved [`GraphicsState`]. Callbacks arrive in the metafile's record order (painter's order).
pub trait MetafileSink {
    /// Called once, before any drawing callback, with the metafile's header facts.
    fn header(&mut self, header: &MetafileHeader) {
        let _ = header;
    }

    /// An open sequence of connected line segments (a polyline). Only [`GraphicsState::pen`] applies.
    fn polyline(&mut self, points: &[Point], state: &GraphicsState<'_>) {
        let _ = (points, state);
    }

    /// A closed, optionally filled polygon. [`GraphicsState::brush`] fills it, [`GraphicsState::pen`]
    /// outlines it.
    fn polygon(&mut self, points: &[Point], state: &GraphicsState<'_>) {
        let _ = (points, state);
    }

    /// An axis-aligned rectangle, filled by [`GraphicsState::brush`] and outlined by
    /// [`GraphicsState::pen`].
    fn rectangle(&mut self, bounds: Rect, state: &GraphicsState<'_>) {
        let _ = (bounds, state);
    }

    /// An ellipse inscribed in `bounds`, filled by [`GraphicsState::brush`] and outlined by
    /// [`GraphicsState::pen`].
    fn ellipse(&mut self, bounds: Rect, state: &GraphicsState<'_>) {
        let _ = (bounds, state);
    }

    /// A text run whose reference point is `position`. The typeface is [`GraphicsState::font`] and
    /// the color [`GraphicsState::text_color`].
    fn text(&mut self, text: &str, position: Point, state: &GraphicsState<'_>) {
        let _ = (text, position, state);
    }

    /// A raster image blitted into `bounds` (device space). A blit carries no pen or brush, so no
    /// [`GraphicsState`] is passed; [`Bitmap`] holds a complete, self-contained image file.
    fn image(&mut self, bounds: Rect, bitmap: &Bitmap) {
        let _ = (bounds, bitmap);
    }

    /// A record the parser recognizes but does not render — currently only embedded EMF+/GDI+
    /// content (see [`Feature`]). The default is a no-op; override to surface that a picture was only
    /// partially rendered. Never called for records the parser simply skips by length.
    fn unsupported(&mut self, feature: Feature) {
        let _ = feature;
    }
}

/// A recognized-but-unrendered metafile feature, reported to [`MetafileSink::unsupported`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Feature {
    /// The stream embeds **EMF+** (GDI+) records inside `EMR_COMMENT` blocks. The enclosing EMF is
    /// still parsed, but the EMF+ drawing is not interpreted, so such a picture may render partially.
    EmfPlus,
}

/// One decoded drawing primitive, as collected by [`Recording`]. A neutral, self-contained value for
/// consumers that prefer a data structure over implementing [`MetafileSink`].
#[derive(Clone, Debug, PartialEq)]
pub enum Primitive {
    /// An open polyline; drawn with `pen` if present.
    Polyline {
        /// The vertices, in order.
        points: Vec<Point>,
        /// The stroke, or `None` for no outline.
        pen: Option<Pen>,
    },
    /// A closed polygon; filled with `brush` and outlined with `pen` when present.
    Polygon {
        /// The vertices, in order (implicitly closed).
        points: Vec<Point>,
        /// The stroke, or `None` for no outline.
        pen: Option<Pen>,
        /// The fill, or `None` for no fill.
        brush: Option<Brush>,
    },
    /// An axis-aligned rectangle.
    Rectangle {
        /// The rectangle's extent.
        bounds: Rect,
        /// The stroke, or `None` for no outline.
        pen: Option<Pen>,
        /// The fill, or `None` for no fill.
        brush: Option<Brush>,
    },
    /// An ellipse inscribed in `bounds`.
    Ellipse {
        /// The bounding box the ellipse is inscribed in.
        bounds: Rect,
        /// The stroke, or `None` for no outline.
        pen: Option<Pen>,
        /// The fill, or `None` for no fill.
        brush: Option<Brush>,
    },
    /// A text run.
    Text {
        /// The string.
        text: String,
        /// The reference point.
        position: Point,
        /// The typeface.
        font: Font,
        /// The text color.
        color: Color,
    },
    /// A raster image blitted into `bounds`.
    Image {
        /// The destination box in device space.
        bounds: Rect,
        /// The image, as a complete, self-contained file.
        bitmap: Bitmap,
    },
}

/// A [`MetafileSink`] that records every callback into a flat [`Primitive`] list — the convenience
/// sink for consumers that want the decoded drawing as data rather than a stream of calls.
#[derive(Clone, Debug, Default)]
pub struct Recording {
    /// The header, once the parser has reported it.
    pub header: Option<MetafileHeader>,
    /// The primitives, in painter's order.
    pub primitives: Vec<Primitive>,
    /// Recognized-but-unrendered features encountered (e.g. embedded EMF+ content); non-empty means
    /// the recording may be incomplete. Reported in encounter order, with duplicates.
    pub unsupported: Vec<Feature>,
}

impl MetafileSink for Recording {
    fn header(&mut self, header: &MetafileHeader) {
        self.header = Some(header.clone());
    }

    fn polyline(&mut self, points: &[Point], state: &GraphicsState<'_>) {
        self.primitives.push(Primitive::Polyline {
            points: points.to_vec(),
            pen: state.pen,
        });
    }

    fn polygon(&mut self, points: &[Point], state: &GraphicsState<'_>) {
        self.primitives.push(Primitive::Polygon {
            points: points.to_vec(),
            pen: state.pen,
            brush: state.brush,
        });
    }

    fn rectangle(&mut self, bounds: Rect, state: &GraphicsState<'_>) {
        self.primitives.push(Primitive::Rectangle {
            bounds,
            pen: state.pen,
            brush: state.brush,
        });
    }

    fn ellipse(&mut self, bounds: Rect, state: &GraphicsState<'_>) {
        self.primitives.push(Primitive::Ellipse {
            bounds,
            pen: state.pen,
            brush: state.brush,
        });
    }

    fn text(&mut self, text: &str, position: Point, state: &GraphicsState<'_>) {
        self.primitives.push(Primitive::Text {
            text: text.to_string(),
            position,
            font: state.font.clone(),
            color: state.text_color,
        });
    }

    fn image(&mut self, bounds: Rect, bitmap: &Bitmap) {
        self.primitives.push(Primitive::Image {
            bounds,
            bitmap: bitmap.clone(),
        });
    }

    fn unsupported(&mut self, feature: Feature) {
        self.unsupported.push(feature);
    }
}
