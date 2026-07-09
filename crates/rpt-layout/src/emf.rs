//! # EMF (Enhanced Metafile) interpreter
//!
//! Parses a user-inserted EMF picture's record stream and projects its core drawing records onto the
//! [`rpt_pages`] Page IR, mapped into the picture object's destination box. EMF records map ~1:1
//! onto line/polygon/ellipse/rect/text. A Metafile picture is a *vector command
//! stream*, not raster bytes, so replaying its records is the faithful render — the alternative is
//! the placeholder box a raster-only backend draws.
//!
//! First cut: the common shape/pen/brush/text records only. Unknown records are skipped (advance by
//! their `nSize`), never fatal; a truncated or malformed stream returns `None` so the caller keeps
//! the placeholder. WMF and OLE-embedded presentations remain placeholders (separate follow-ups).
//!
//! Coordinate model: EMF drawing records use device/logical units bounded by the header's
//! `rclBounds`. We build one affine map `rclBounds → dest` (both axis-aligned) and transform every
//! record coordinate through it, so the whole metafile scales to fit the destination twip box.

use rpt_model::{Color, Rect, Twips};
use rpt_pages::{
    DrawOp, EllipseOp, Fill, FontSpec, LineOp, LineStyle, ObjectRef, Point, PolygonOp, RectOp,
    Stroke, TextAlign, TextRun,
};

// EMF record types we handle (MS-EMF spec section 2.1.1). All other types are skipped by `nSize`.
const EMR_HEADER: u32 = 1;
const EMR_POLYGON: u32 = 3;
const EMR_POLYLINE: u32 = 4;
const EMR_EOF: u32 = 14;
const EMR_SETTEXTCOLOR: u32 = 24;
const EMR_MOVETOEX: u32 = 27;
const EMR_SELECTOBJECT: u32 = 37;
const EMR_CREATEPEN: u32 = 38;
const EMR_CREATEBRUSHINDIRECT: u32 = 39;
const EMR_ELLIPSE: u32 = 42;
const EMR_RECTANGLE: u32 = 43;
const EMR_LINETO: u32 = 54;
const EMR_EXTTEXTOUTA: u32 = 83;
const EMR_EXTTEXTOUTW: u32 = 84;
const EMR_POLYLINE16: u32 = 85;
const EMR_POLYGON16: u32 = 86;
const EMR_EXTCREATEPEN: u32 = 95;

/// The ` EMF` signature at header offset 40 (little-endian `0x464D4520`).
const EMF_SIGNATURE: &[u8; 4] = b" EMF";

/// A hairline stroke width in twips (~1px at 96 dpi) — the floor for any drawn pen so a zero-width
/// (cosmetic) or sub-pixel-scaled pen still paints a visible line.
const HAIRLINE: i32 = 15;

/// Interpret an EMF byte stream into Page-IR draw-ops mapped into `dest` (a twip box). Returns `None`
/// on a bad signature or a truncated/garbage stream so the caller can keep its placeholder. `source`
/// tags every emitted op with the originating report object.
pub(crate) fn interpret_emf(
    bytes: &[u8],
    dest: Rect,
    source: Option<ObjectRef>,
) -> Option<Vec<DrawOp>> {
    let header = Header::parse(bytes)?;
    let transform = Transform::new(header.bounds, dest)?;

    let mut state = State {
        ops: Vec::new(),
        transform,
        source,
        objects: Vec::new(),
        cur_pen: Some(Stroke {
            color: BLACK,
            width: Twips(HAIRLINE),
            style: LineStyle::Single,
        }),
        cur_brush: None,
        text_color: BLACK,
        cur_point: (0, 0),
    };

    // Records follow the header; iterate from the byte after it.
    let mut pos = header.size;
    loop {
        // A record is `iType u32`, `nSize u32`, then `nSize - 8` payload bytes.
        let size = rd_u32(bytes, pos + 4)? as usize;
        if size < 8 {
            return None; // malformed: a record cannot be smaller than its own header
        }
        let end = pos.checked_add(size)?;
        let rec = bytes.get(pos..end)?; // bounds-checked: truncated stream ⇒ None

        if rd_u32(rec, 0)? == EMR_EOF {
            break;
        }
        // A field-read failure inside a well-sized record is non-fatal: skip that record.
        decode_record(rec, &mut state);
        pos = end;
    }

    Some(state.ops)
}

/// The subset of `ENHMETAHEADER` we need: the record's byte length and the device-unit bounds the
/// drawing coordinates live in.
struct Header {
    /// Total header record size (`nSize`) — where the record stream begins.
    size: usize,
    /// `rclBounds`: inclusive device-unit rectangle (left, top, right, bottom).
    bounds: (i32, i32, i32, i32),
}

impl Header {
    fn parse(bytes: &[u8]) -> Option<Header> {
        if rd_u32(bytes, 0)? != EMR_HEADER {
            return None;
        }
        // Signature ` EMF` at offset 40 validates this is an EMF (not a bare DIB / other stream).
        if bytes.get(40..44)? != EMF_SIGNATURE {
            return None;
        }
        let size = rd_u32(bytes, 4)? as usize;
        if size < 88 {
            return None; // an ENHMETAHEADER carries at least through szlMillimeters
        }
        let bounds = (
            rd_i32(bytes, 8)?,
            rd_i32(bytes, 12)?,
            rd_i32(bytes, 16)?,
            rd_i32(bytes, 20)?,
        );
        Some(Header { size, bounds })
    }
}

/// An axis-aligned affine map from EMF device/logical units into destination twips.
struct Transform {
    /// Destination origin (twips) the bounds' top-left maps to.
    ox: f64,
    oy: f64,
    /// Bounds' top-left (logical), subtracted before scaling.
    bx: f64,
    by: f64,
    /// Per-axis scale (twips per logical unit).
    sx: f64,
    sy: f64,
}

impl Transform {
    fn new(bounds: (i32, i32, i32, i32), dest: Rect) -> Option<Transform> {
        let (l, t, r, b) = bounds;
        // rclBounds is inclusive-inclusive, so its span is (right - left + 1) device units.
        let bw = (r - l + 1) as f64;
        let bh = (b - t + 1) as f64;
        if bw <= 0.0 || bh <= 0.0 {
            return None; // degenerate bounds — nothing sensible to map
        }
        Some(Transform {
            ox: dest.left.0 as f64,
            oy: dest.top.0 as f64,
            bx: l as f64,
            by: t as f64,
            sx: dest.width.0 as f64 / bw,
            sy: dest.height.0 as f64 / bh,
        })
    }

    fn point(&self, x: i32, y: i32) -> Point {
        Point {
            x: Twips((self.ox + (x as f64 - self.bx) * self.sx).round() as i32),
            y: Twips((self.oy + (y as f64 - self.by) * self.sy).round() as i32),
        }
    }

    /// The average absolute per-axis scale, for converting a scalar logical width (pen width).
    fn scalar(&self) -> f64 {
        (self.sx.abs() + self.sy.abs()) / 2.0
    }
}

/// A GDI object stored in the metafile's handle table (populated by the `CREATE*` records, referenced
/// by `SELECTOBJECT`).
enum GdiObject {
    /// A pen: its stroke, or `None` for a `PS_NULL` (invisible) pen.
    Pen(Option<Stroke>),
    /// A brush: its solid fill colour, or `None` for a `BS_NULL`/hollow brush.
    Brush(Option<Color>),
}

/// Running interpreter state: the emitted ops, the coordinate map, and the current GDI selection.
struct State {
    ops: Vec<DrawOp>,
    transform: Transform,
    source: Option<ObjectRef>,
    /// Handle table indexed by `ihObject`. Entry `None` = an unset/released slot.
    objects: Vec<Option<GdiObject>>,
    cur_pen: Option<Stroke>,
    cur_brush: Option<Color>,
    text_color: Color,
    /// Current position (logical units) set by `MOVETOEX`, consumed by `LINETO`.
    cur_point: (i32, i32),
}

impl State {
    fn set_object(&mut self, index: u32, obj: GdiObject) {
        let index = index as usize;
        if index >= self.objects.len() {
            self.objects.resize_with(index + 1, || None);
        }
        self.objects[index] = Some(obj);
    }
}

/// Decode one record (already sliced to `[iType..iType + nSize]`) into state. Field reads are
/// bounds-checked; a short/garbage record simply produces no op (the outer loop still advances by the
/// intact `nSize`), so a malformed record is skipped rather than fatal.
fn decode_record(rec: &[u8], state: &mut State) -> Option<()> {
    match rd_u32(rec, 0)? {
        EMR_MOVETOEX => {
            state.cur_point = (rd_i32(rec, 8)?, rd_i32(rec, 12)?);
        }
        EMR_LINETO => {
            let to = (rd_i32(rec, 8)?, rd_i32(rec, 12)?);
            if let Some(stroke) = state.cur_pen {
                let from = state.transform.point(state.cur_point.0, state.cur_point.1);
                let to_pt = state.transform.point(to.0, to.1);
                state.ops.push(DrawOp::Line(LineOp {
                    from,
                    to: to_pt,
                    stroke,
                    source: state.source.clone(),
                }));
            }
            state.cur_point = to;
        }
        EMR_RECTANGLE => {
            let bounds = read_rectl(rec, 8, &state.transform)?;
            state.ops.push(DrawOp::Rect(RectOp {
                bounds,
                fill: state.cur_brush.map(Fill::Solid),
                stroke: state.cur_pen,
                corner_radius: Twips(0),
                source: state.source.clone(),
            }));
        }
        EMR_ELLIPSE => {
            let bounds = read_rectl(rec, 8, &state.transform)?;
            state.ops.push(DrawOp::Ellipse(EllipseOp {
                bounds,
                fill: state.cur_brush.map(Fill::Solid),
                stroke: state.cur_pen,
                source: state.source.clone(),
            }));
        }
        EMR_POLYGON => push_polygon(rec, state, true, false)?,
        EMR_POLYLINE => push_polygon(rec, state, false, false)?,
        EMR_POLYGON16 => push_polygon(rec, state, true, true)?,
        EMR_POLYLINE16 => push_polygon(rec, state, false, true)?,
        EMR_EXTTEXTOUTW => push_text(rec, state, true)?,
        EMR_EXTTEXTOUTA => push_text(rec, state, false)?,
        EMR_SETTEXTCOLOR => {
            state.text_color = colorref(rd_u32(rec, 8)?);
        }
        EMR_CREATEPEN => {
            let index = rd_u32(rec, 8)?;
            // LOGPEN: lopnStyle u32 (12), lopnWidth POINTL (16 = x, 20 = y), lopnColor (24).
            let style = rd_u32(rec, 12)?;
            let width = rd_i32(rec, 16)?;
            let color = colorref(rd_u32(rec, 24)?);
            state.set_object(
                index,
                GdiObject::Pen(pen_stroke(style, width, color, &state.transform)),
            );
        }
        EMR_EXTCREATEPEN => {
            let index = rd_u32(rec, 8)?;
            // EXTLOGPEN begins at offset 28: elpPenStyle u32 (28), elpWidth u32 (32), elpBrushStyle
            // u32 (36), elpColor COLORREF (40).
            let style = rd_u32(rec, 28)?;
            let width = rd_i32(rec, 32)?;
            let color = colorref(rd_u32(rec, 40)?);
            state.set_object(
                index,
                GdiObject::Pen(pen_stroke(style, width, color, &state.transform)),
            );
        }
        EMR_CREATEBRUSHINDIRECT => {
            let index = rd_u32(rec, 8)?;
            // LOGBRUSH: lbStyle u32 (12), lbColor COLORREF (16). Style 1 = BS_NULL (hollow).
            let style = rd_u32(rec, 12)?;
            let fill = if style == BS_NULL {
                None
            } else {
                Some(colorref(rd_u32(rec, 16)?))
            };
            state.set_object(index, GdiObject::Brush(fill));
        }
        EMR_SELECTOBJECT => {
            let handle = rd_u32(rec, 8)?;
            select_object(state, handle);
        }
        _ => {} // unknown/unhandled record: skip (outer loop advances by nSize)
    }
    Some(())
}

/// A polygon/polyline record: `rclBounds` (8..24), point count `u32` (24..28), then the point array
/// at offset 28 — `POINT16` (2×i16, 4 bytes) when `p16`, else `POINTL` (2×i32, 8 bytes).
fn push_polygon(rec: &[u8], state: &mut State, closed: bool, p16: bool) -> Option<()> {
    let count = rd_u32(rec, 24)? as usize;
    // A sane bound: refuse a count that cannot fit in the record.
    let stride = if p16 { 4 } else { 8 };
    let need = 28usize.checked_add(count.checked_mul(stride)?)?;
    if need > rec.len() {
        return Some(()); // truncated point array: skip this record, don't abort
    }
    let mut points = Vec::with_capacity(count);
    for i in 0..count {
        let off = 28 + i * stride;
        let (x, y) = if p16 {
            (rd_i16(rec, off)? as i32, rd_i16(rec, off + 2)? as i32)
        } else {
            (rd_i32(rec, off)?, rd_i32(rec, off + 4)?)
        };
        points.push(state.transform.point(x, y));
    }
    if points.is_empty() {
        return Some(());
    }
    state.ops.push(DrawOp::Polygon(PolygonOp {
        points,
        closed,
        fill: if closed {
            state.cur_brush.map(Fill::Solid)
        } else {
            None
        },
        stroke: state.cur_pen,
        source: state.source.clone(),
    }));
    Some(())
}

/// An `EMR_EXTTEXTOUT*` record → a [`TextRun`] at the reference point. Best-effort: the font is the
/// default (no `LOGFONT` selection tracked in this first cut) and the box is sized from the string
/// length. `wide` selects UTF-16LE (`W`) vs. ANSI/Latin-1 (`A`) decoding.
fn push_text(rec: &[u8], state: &mut State, wide: bool) -> Option<()> {
    // EMRTEXT begins at offset 36: ptlReference POINTL (36..44), nChars u32 (44), offString u32 (48).
    let ref_x = rd_i32(rec, 36)?;
    let ref_y = rd_i32(rec, 40)?;
    let n_chars = rd_u32(rec, 44)? as usize;
    let off_string = rd_u32(rec, 48)? as usize;

    let text = if wide {
        let bytes = rec.get(off_string..off_string.checked_add(n_chars.checked_mul(2)?)?)?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        let bytes = rec.get(off_string..off_string.checked_add(n_chars)?)?;
        // ANSI text: decode as Latin-1 (each byte a code point) — good enough for a first cut.
        bytes.iter().map(|&b| b as char).collect()
    };
    if text.is_empty() {
        return Some(());
    }

    let origin = state.transform.point(ref_x, ref_y);
    let font = FontSpec::default();
    // Approximate the run box from the default font: ~0.5em per char wide, one em tall.
    let em = (font.size_pt * 20.0).round() as i32; // pt → twips
    let width = (n_chars as i32 * em / 2).max(em);
    state.ops.push(DrawOp::Text(TextRun {
        bounds: Rect {
            left: origin.x,
            top: origin.y,
            width: Twips(width),
            height: Twips(em),
        },
        text,
        font,
        color: state.text_color,
        align: TextAlign::Left,
        rotation: 0.0,
        metrics: None,
        source: state.source.clone(),
    }));
    Some(())
}

/// Apply an `EMR_SELECTOBJECT`: set the current pen or brush from the handle table, or from a stock
/// object when the high bit (`0x80000000`) is set.
fn select_object(state: &mut State, handle: u32) {
    if handle & STOCK_OBJECT != 0 {
        match handle {
            WHITE_BRUSH => state.cur_brush = Some(WHITE),
            LTGRAY_BRUSH => state.cur_brush = Some(gray(192)),
            GRAY_BRUSH => state.cur_brush = Some(gray(128)),
            DKGRAY_BRUSH => state.cur_brush = Some(gray(64)),
            BLACK_BRUSH => state.cur_brush = Some(BLACK),
            NULL_BRUSH => state.cur_brush = None,
            WHITE_PEN => state.cur_pen = Some(stock_pen(WHITE)),
            BLACK_PEN => state.cur_pen = Some(stock_pen(BLACK)),
            NULL_PEN => state.cur_pen = None,
            _ => {} // other stock objects (fonts/palettes) don't affect pen/brush
        }
        return;
    }
    match state.objects.get(handle as usize).and_then(|o| o.as_ref()) {
        Some(GdiObject::Pen(p)) => state.cur_pen = *p,
        Some(GdiObject::Brush(b)) => state.cur_brush = *b,
        None => {}
    }
}

/// Build a pen stroke from a LOGPEN/EXTLOGPEN style, logical width, and colour. `PS_NULL` (5) yields
/// `None` (an invisible pen). Width is scaled to twips and floored at a hairline.
fn pen_stroke(style: u32, width_logical: i32, color: Color, t: &Transform) -> Option<Stroke> {
    let line_style = match style & PS_STYLE_MASK {
        PS_NULL => return None,
        PS_DASH => LineStyle::Dashed,
        PS_DOT | PS_DASHDOT | PS_DASHDOTDOT => LineStyle::Dotted,
        _ => LineStyle::Single, // PS_SOLID / PS_INSIDEFRAME / unknown
    };
    let scaled = (width_logical.max(0) as f64 * t.scalar()).round() as i32;
    Some(Stroke {
        color,
        width: Twips(scaled.max(HAIRLINE)),
        style: line_style,
    })
}

fn stock_pen(color: Color) -> Stroke {
    Stroke {
        color,
        width: Twips(HAIRLINE),
        style: LineStyle::Single,
    }
}

/// Read a `RECTL` (left, top, right, bottom i32) at `off` and map it to a destination twip [`Rect`].
fn read_rectl(rec: &[u8], off: usize, t: &Transform) -> Option<Rect> {
    let l = rd_i32(rec, off)?;
    let top = rd_i32(rec, off + 4)?;
    let r = rd_i32(rec, off + 8)?;
    let b = rd_i32(rec, off + 12)?;
    let p0 = t.point(l, top);
    let p1 = t.point(r, b);
    Some(Rect {
        left: Twips(p0.x.0.min(p1.x.0)),
        top: Twips(p0.y.0.min(p1.y.0)),
        width: Twips((p1.x.0 - p0.x.0).abs()),
        height: Twips((p1.y.0 - p0.y.0).abs()),
    })
}

/// COLORREF `0x00BBGGRR` → an opaque [`Color`].
fn colorref(v: u32) -> Color {
    Color {
        a: 255,
        r: (v & 0xff) as u8,
        g: ((v >> 8) & 0xff) as u8,
        b: ((v >> 16) & 0xff) as u8,
    }
}

fn gray(level: u8) -> Color {
    Color {
        a: 255,
        r: level,
        g: level,
        b: level,
    }
}

const BLACK: Color = Color {
    a: 255,
    r: 0,
    g: 0,
    b: 0,
};
const WHITE: Color = Color {
    a: 255,
    r: 255,
    g: 255,
    b: 255,
};

// Pen styles (MS-EMF `PenStyle`), low nibble.
const PS_STYLE_MASK: u32 = 0x0000_000F;
const PS_DASH: u32 = 1;
const PS_DOT: u32 = 2;
const PS_DASHDOT: u32 = 3;
const PS_DASHDOTDOT: u32 = 4;
const PS_NULL: u32 = 5;

// Brush style (MS-WMF `BrushStyle`): BS_NULL / BS_HOLLOW.
const BS_NULL: u32 = 1;

// Stock objects (MS-EMF spec section 2.1.31): high bit set marks a stock-object handle.
const STOCK_OBJECT: u32 = 0x8000_0000;
const WHITE_BRUSH: u32 = 0x8000_0000;
const LTGRAY_BRUSH: u32 = 0x8000_0001;
const GRAY_BRUSH: u32 = 0x8000_0002;
const DKGRAY_BRUSH: u32 = 0x8000_0003;
const BLACK_BRUSH: u32 = 0x8000_0004;
const NULL_BRUSH: u32 = 0x8000_0005;
const WHITE_PEN: u32 = 0x8000_0006;
const BLACK_PEN: u32 = 0x8000_0007;
const NULL_PEN: u32 = 0x8000_0008;

// --- Bounds-checked little-endian scalar reads --------------------------------------------------

fn rd_u32(b: &[u8], off: usize) -> Option<u32> {
    let s = b.get(off..off.checked_add(4)?)?;
    Some(u32::from_le_bytes(s.try_into().ok()?))
}

fn rd_i32(b: &[u8], off: usize) -> Option<i32> {
    let s = b.get(off..off.checked_add(4)?)?;
    Some(i32::from_le_bytes(s.try_into().ok()?))
}

fn rd_i16(b: &[u8], off: usize) -> Option<i16> {
    let s = b.get(off..off.checked_add(2)?)?;
    Some(i16::from_le_bytes(s.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal 88-byte `ENHMETAHEADER` with `rclBounds = (0,0,99,99)` (a 100×100 logical grid) and
    /// the ` EMF` signature. `bad_sig` corrupts the signature to exercise the reject path.
    fn header(bad_sig: bool) -> Vec<u8> {
        let mut h = vec![0u8; 88];
        h[0..4].copy_from_slice(&EMR_HEADER.to_le_bytes());
        h[4..8].copy_from_slice(&88u32.to_le_bytes());
        // rclBounds: 0,0,99,99
        h[16..20].copy_from_slice(&99i32.to_le_bytes());
        h[20..24].copy_from_slice(&99i32.to_le_bytes());
        let sig = if bad_sig { b"XXXX" } else { EMF_SIGNATURE };
        h[40..44].copy_from_slice(sig);
        h
    }

    /// A generic record: `iType`, `nSize` (= 8 + payload), then the payload bytes.
    fn record(itype: u32, payload: &[u8]) -> Vec<u8> {
        let size = 8 + payload.len();
        let mut r = Vec::with_capacity(size);
        r.extend_from_slice(&itype.to_le_bytes());
        r.extend_from_slice(&(size as u32).to_le_bytes());
        r.extend_from_slice(payload);
        r
    }

    fn eof() -> Vec<u8> {
        record(EMR_EOF, &[0u8; 12])
    }

    fn rectl(l: i32, t: i32, r: i32, b: i32) -> Vec<u8> {
        let mut v = Vec::new();
        for x in [l, t, r, b] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v
    }

    /// The unit destination used across the tests: a 1440×1440-twip (1") box at (1000, 2000). With
    /// the 100-unit bounds span this makes the scale exactly 14.4 twips per logical unit.
    fn dest() -> Rect {
        Rect {
            left: Twips(1000),
            top: Twips(2000),
            width: Twips(1440),
            height: Twips(1440),
        }
    }

    #[test]
    fn transform_maps_logical_point_to_twips() {
        let t = Transform::new((0, 0, 99, 99), dest()).unwrap();
        // 14.4 twips/unit, offset by the destination origin.
        assert_eq!(t.point(0, 0), Point::new(1000, 2000));
        assert_eq!(t.point(10, 10), Point::new(1144, 2144)); // 10 * 14.4 = 144
        assert_eq!(t.point(50, 50), Point::new(1720, 2720)); // 50 * 14.4 = 720
    }

    #[test]
    fn degenerate_bounds_bail() {
        assert!(Transform::new((0, 0, -1, -1), dest()).is_none());
    }

    #[test]
    fn rectangle_maps_into_dest() {
        let mut emf = header(false);
        emf.extend(record(EMR_RECTANGLE, &rectl(10, 10, 50, 50)));
        emf.extend(eof());

        let ops = interpret_emf(&emf, dest(), None).unwrap();
        assert_eq!(ops.len(), 1);
        let DrawOp::Rect(r) = &ops[0] else {
            panic!("expected a Rect, got {:?}", ops[0]);
        };
        assert_eq!(r.bounds.left, Twips(1144));
        assert_eq!(r.bounds.top, Twips(2144));
        assert_eq!(r.bounds.width, Twips(576)); // (720 - 144)
        assert_eq!(r.bounds.height, Twips(576));
    }

    #[test]
    fn brush_then_polygon16_is_closed_and_red() {
        let red = colorref(0x0000_00FF); // COLORREF 0x00BBGGRR: r=255

        let mut brush = 1u32.to_le_bytes().to_vec(); // ihBrush = 1
        brush.extend_from_slice(&0u32.to_le_bytes()); // lbStyle = BS_SOLID
        brush.extend_from_slice(&0x0000_00FFu32.to_le_bytes()); // lbColor = red
        brush.extend_from_slice(&0u32.to_le_bytes()); // lbHatch

        let mut poly = rectl(0, 0, 99, 99); // rclBounds (unused by us)
        poly.extend_from_slice(&3u32.to_le_bytes()); // 3 points
        for (x, y) in [(10i16, 10i16), (90, 10), (50, 90)] {
            poly.extend_from_slice(&x.to_le_bytes());
            poly.extend_from_slice(&y.to_le_bytes());
        }

        let mut emf = header(false);
        emf.extend(record(EMR_CREATEBRUSHINDIRECT, &brush));
        emf.extend(record(EMR_SELECTOBJECT, &1u32.to_le_bytes())); // select brush 1
        emf.extend(record(EMR_POLYGON16, &poly));
        emf.extend(eof());

        let ops = interpret_emf(&emf, dest(), None).unwrap();
        assert_eq!(ops.len(), 1);
        let DrawOp::Polygon(p) = &ops[0] else {
            panic!("expected a Polygon, got {:?}", ops[0]);
        };
        assert!(p.closed);
        assert_eq!(p.points.len(), 3);
        assert_eq!(p.fill, Some(Fill::Solid(red)));
        assert_eq!(p.points[0], Point::new(1144, 2144)); // (10,10) → 14.4×
    }

    #[test]
    fn bad_signature_returns_none() {
        let mut emf = header(true);
        emf.extend(record(EMR_RECTANGLE, &rectl(10, 10, 50, 50)));
        emf.extend(eof());
        assert!(interpret_emf(&emf, dest(), None).is_none());
    }

    #[test]
    fn unknown_record_is_skipped_not_fatal() {
        let mut emf = header(false);
        emf.extend(record(EMR_RECTANGLE, &rectl(10, 10, 50, 50)));
        emf.extend(record(9999, &[0u8; 24])); // unknown record between known ones
        emf.extend(record(EMR_ELLIPSE, &rectl(0, 0, 20, 20)));
        emf.extend(eof());

        let ops = interpret_emf(&emf, dest(), None).unwrap();
        // The unknown record is advanced over; both shapes still emit.
        assert_eq!(ops.len(), 2);
        assert!(matches!(ops[0], DrawOp::Rect(_)));
        assert!(matches!(ops[1], DrawOp::Ellipse(_)));
    }

    #[test]
    fn truncated_stream_returns_none() {
        let mut emf = header(false);
        // A rectangle record that claims 24 bytes but supplies only 10 — the record overruns the buffer.
        let mut rec = EMR_RECTANGLE.to_le_bytes().to_vec();
        rec.extend_from_slice(&24u32.to_le_bytes());
        rec.extend_from_slice(&[0u8; 2]);
        emf.extend(rec);
        assert!(interpret_emf(&emf, dest(), None).is_none());
    }

    #[test]
    fn lineto_uses_current_position_and_pen() {
        let mut emf = header(false);
        let mut moveto = 20i32.to_le_bytes().to_vec();
        moveto.extend_from_slice(&20i32.to_le_bytes());
        emf.extend(record(EMR_MOVETOEX, &moveto));
        let mut lineto = 80i32.to_le_bytes().to_vec();
        lineto.extend_from_slice(&80i32.to_le_bytes());
        emf.extend(record(EMR_LINETO, &lineto));
        emf.extend(eof());

        let ops = interpret_emf(&emf, dest(), None).unwrap();
        assert_eq!(ops.len(), 1);
        let DrawOp::Line(l) = &ops[0] else {
            panic!("expected a Line, got {:?}", ops[0]);
        };
        assert_eq!(l.from, Point::new(1288, 2288)); // (20,20) → 20×14.4 = 288
        assert_eq!(l.to, Point::new(2152, 3152)); // (80,80) → 80×14.4 = 1152
    }
}
