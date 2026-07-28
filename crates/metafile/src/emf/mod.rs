//! EMF (Enhanced Metafile) parsing.
//!
//! [`parse_emf`] replays an EMF record stream against a [`MetafileSink`],
//! applying the device context (world transform, window/viewport mapping, object selection,
//! `SAVEDC`/`RESTOREDC`) so the sink receives device-space geometry and a resolved graphics state.
//! Unknown records are skipped by their length; a truncated or malformed stream returns an
//! [`Error`] without panicking.

mod dc;

use crate::reader::{f32_at, i16_at, i32_at, u32_at};
use crate::{
    Bitmap, BitmapFormat, Brush, Color, Error, Feature, Font, GraphicsState, MetafileHeader,
    MetafileSink, Pen, PenStyle, Point, Rect,
};
use dc::{Dc, GdiObject, Matrix};

// EMF record types (MS-EMF §2.1.1). All others are skipped by `nSize`.
const EMR_HEADER: u32 = 1;
const EMR_POLYBEZIER: u32 = 2;
const EMR_POLYGON: u32 = 3;
const EMR_POLYLINE: u32 = 4;
const EMR_POLYBEZIERTO: u32 = 5;
const EMR_POLYLINETO: u32 = 6;
const EMR_POLYPOLYLINE: u32 = 7;
const EMR_POLYPOLYGON: u32 = 8;
const EMR_SETWINDOWEXTEX: u32 = 9;
const EMR_SETWINDOWORGEX: u32 = 10;
const EMR_SETVIEWPORTEXTEX: u32 = 11;
const EMR_SETVIEWPORTORGEX: u32 = 12;
const EMR_EOF: u32 = 14;
const EMR_COMMENT: u32 = 70;
const EMR_BITBLT: u32 = 76;
const EMR_STRETCHBLT: u32 = 77;
const EMR_SETDIBITSTODEVICE: u32 = 80;
const EMR_STRETCHDIBITS: u32 = 81;
const EMR_ALPHABLEND: u32 = 114;
const EMR_TRANSPARENTBLT: u32 = 116;
const EMR_SETTEXTCOLOR: u32 = 24;
const EMR_MOVETOEX: u32 = 27;
const EMR_SAVEDC: u32 = 33;
const EMR_RESTOREDC: u32 = 34;
const EMR_SETWORLDTRANSFORM: u32 = 35;
const EMR_MODIFYWORLDTRANSFORM: u32 = 36;
const EMR_SELECTOBJECT: u32 = 37;
const EMR_CREATEPEN: u32 = 38;
const EMR_CREATEBRUSHINDIRECT: u32 = 39;
const EMR_DELETEOBJECT: u32 = 40;
const EMR_ELLIPSE: u32 = 42;
const EMR_RECTANGLE: u32 = 43;
const EMR_ROUNDRECT: u32 = 44;
const EMR_LINETO: u32 = 54;
const EMR_EXTCREATEFONTINDIRECTW: u32 = 82;
const EMR_EXTTEXTOUTA: u32 = 83;
const EMR_EXTTEXTOUTW: u32 = 84;
const EMR_POLYBEZIER16: u32 = 85;
const EMR_POLYGON16: u32 = 86;
const EMR_POLYLINE16: u32 = 87;
const EMR_POLYBEZIERTO16: u32 = 88;
const EMR_POLYLINETO16: u32 = 89;
const EMR_POLYPOLYLINE16: u32 = 90;
const EMR_POLYPOLYGON16: u32 = 91;
const EMR_EXTCREATEPEN: u32 = 95;

/// The ` EMF` signature at header offset 40 (little-endian `0x464D4520`).
const EMF_SIGNATURE: &[u8; 4] = b" EMF";

/// The `MODIFYWORLDTRANSFORM` mode that replaces the world transform outright (`MWT_SET`).
const MWT_SET: u32 = 4;
/// `MODIFYWORLDTRANSFORM` mode that pre-multiplies (`MWT_LEFTMULTIPLY`).
const MWT_LEFTMULTIPLY: u32 = 2;
/// `MODIFYWORLDTRANSFORM` mode that post-multiplies (`MWT_RIGHTMULTIPLY`).
const MWT_RIGHTMULTIPLY: u32 = 3;

/// Segments a cubic Bézier is flattened into.
const BEZIER_STEPS: usize = 24;

/// Parse an EMF byte stream, driving `sink` with its decoded primitives. Returns the parsed
/// [`MetafileHeader`] on success.
///
/// # Errors
/// Returns [`Error::NotAMetafile`] if the stream is not an EMF (bad type or signature),
/// [`Error::UnexpectedEof`] if a record runs past the end of the buffer, and [`Error::Malformed`]
/// for a structurally invalid stream (a record smaller than its own header, or degenerate bounds).
pub fn parse_emf<S: MetafileSink>(bytes: &[u8], sink: &mut S) -> Result<MetafileHeader, Error> {
    let header = parse_header(bytes)?;
    sink.header(&header);

    let mut dc = Dc::new();

    // Records follow the header; the header record's own `nSize` is where the stream begins.
    let mut pos = header_size(bytes).ok_or(Error::malformed("header size", 4))?;
    loop {
        let size = u32_at(bytes, pos + 4).ok_or(Error::eof(pos + 4))? as usize;
        if size < 8 {
            return Err(Error::malformed("record smaller than its header", pos));
        }
        let end = pos
            .checked_add(size)
            .ok_or(Error::malformed("record size overflow", pos))?;
        let rec = bytes.get(pos..end).ok_or(Error::eof(pos))?;

        let itype = u32_at(rec, 0).ok_or(Error::eof(pos))?;
        if itype == EMR_EOF {
            break;
        }
        // A field-read failure inside a well-sized record is non-fatal: skip that record and keep
        // going, since the outer loop still advances by the intact `nSize`.
        decode_record(itype, rec, &mut dc, sink);
        pos = end;
    }
    Ok(header)
}

/// The header record's `nSize` (offset 4), if the buffer is long enough.
fn header_size(bytes: &[u8]) -> Option<usize> {
    u32_at(bytes, 4).map(|s| s as usize)
}

/// Parse the `ENHMETAHEADER` into the public [`MetafileHeader`].
fn parse_header(bytes: &[u8]) -> Result<MetafileHeader, Error> {
    if u32_at(bytes, 0) != Some(EMR_HEADER) {
        return Err(Error::NotAMetafile);
    }
    if bytes.get(40..44) != Some(EMF_SIGNATURE.as_slice()) {
        return Err(Error::NotAMetafile);
    }
    let size = header_size(bytes).ok_or(Error::eof(4))?;
    if size < 88 {
        return Err(Error::malformed("header shorter than ENHMETAHEADER", 0));
    }
    let bounds = read_rectl_raw(bytes, 8).ok_or(Error::eof(8))?;
    if bounds.width() < 0.0 || bounds.height() < 0.0 {
        return Err(Error::malformed("degenerate rclBounds", 8));
    }
    // rclFrame (offset 24, 4×i32) is in units of 0.01 mm.
    let frame = read_rectl_raw(bytes, 24);
    Ok(MetafileHeader { bounds, frame })
}

/// Read a `RECTL` (left, top, right, bottom i32) at `off` as a normalised [`Rect`] with no transform
/// applied (device units).
fn read_rectl_raw(b: &[u8], off: usize) -> Option<Rect> {
    let l = i32_at(b, off)? as f64;
    let t = i32_at(b, off + 4)? as f64;
    let r = i32_at(b, off + 8)? as f64;
    let bot = i32_at(b, off + 12)? as f64;
    Some(Rect {
        left: l.min(r),
        top: t.min(bot),
        right: l.max(r),
        bottom: t.max(bot),
    })
}

/// Build the [`GraphicsState`] handed to a sink callback: the current brush, text colour, and font
/// verbatim, and the pen with its width resolved from logical units into device units through the
/// current transform (a cosmetic width of `0` stays `0`).
fn graphics_state(dc: &Dc) -> GraphicsState<'_> {
    let scale = dc.ctm().mean_scale();
    let pen = dc.cur.pen.map(|p| Pen {
        width: if p.width == 0.0 { 0.0 } else { p.width * scale },
        ..p
    });
    GraphicsState {
        pen,
        brush: dc.cur.brush,
        font: &dc.cur.font,
        text_color: dc.cur.text_color,
    }
}

/// Decode one record (sliced to `[iType..iType + nSize]`) into `dc`, emitting to `sink`. Field reads
/// are bounds-checked; a short/garbage record produces no output (the outer loop still advances by
/// the intact `nSize`), so a malformed record is skipped rather than fatal.
fn decode_record<S: MetafileSink>(itype: u32, rec: &[u8], dc: &mut Dc, sink: &mut S) -> Option<()> {
    match itype {
        EMR_MOVETOEX => {
            dc.cur.point = (i32_at(rec, 8)? as f64, i32_at(rec, 12)? as f64);
        }
        EMR_LINETO => {
            let to = (i32_at(rec, 8)? as f64, i32_at(rec, 12)? as f64);
            let state = graphics_state(dc);
            let pts = [dc.map(dc.cur.point.0, dc.cur.point.1), dc.map(to.0, to.1)];
            sink.polyline(&pts, &state);
            dc.cur.point = to;
        }
        EMR_RECTANGLE => {
            let bounds = map_rect(dc, read_rectl_raw(rec, 8)?);
            sink.rectangle(bounds, &graphics_state(dc));
        }
        EMR_ROUNDRECT => {
            // Rounded corners are approximated by a plain rectangle (the neutral sink has no
            // rounded-rect primitive); the corner ellipse size at offset 24 is ignored.
            let bounds = map_rect(dc, read_rectl_raw(rec, 8)?);
            sink.rectangle(bounds, &graphics_state(dc));
        }
        EMR_ELLIPSE => {
            let bounds = map_rect(dc, read_rectl_raw(rec, 8)?);
            sink.ellipse(bounds, &graphics_state(dc));
        }
        EMR_POLYGON => poly(rec, dc, sink, Shape::Polygon, false),
        EMR_POLYLINE => poly(rec, dc, sink, Shape::Polyline, false),
        EMR_POLYGON16 => poly(rec, dc, sink, Shape::Polygon, true),
        EMR_POLYLINE16 => poly(rec, dc, sink, Shape::Polyline, true),
        EMR_POLYLINETO => poly(rec, dc, sink, Shape::PolylineTo, false),
        EMR_POLYLINETO16 => poly(rec, dc, sink, Shape::PolylineTo, true),
        EMR_POLYBEZIER => poly(rec, dc, sink, Shape::Bezier, false),
        EMR_POLYBEZIER16 => poly(rec, dc, sink, Shape::Bezier, true),
        EMR_POLYBEZIERTO => poly(rec, dc, sink, Shape::BezierTo, false),
        EMR_POLYBEZIERTO16 => poly(rec, dc, sink, Shape::BezierTo, true),
        EMR_POLYPOLYLINE => poly_poly(rec, dc, sink, false, false),
        EMR_POLYPOLYGON => poly_poly(rec, dc, sink, true, false),
        EMR_POLYPOLYLINE16 => poly_poly(rec, dc, sink, false, true),
        EMR_POLYPOLYGON16 => poly_poly(rec, dc, sink, true, true),
        EMR_EXTTEXTOUTW => text(rec, dc, sink, true)?,
        EMR_EXTTEXTOUTA => text(rec, dc, sink, false)?,
        // Raster blits. The dest box and source-DIB field offsets differ per record; `STRETCHDIBITS`
        // and `SETDIBITSTODEVICE` carry the DIB at 48.., the `*BLT` family (which prefix a source
        // XFORM) at 84.. — see [MS-EMF] §2.3.1.
        EMR_STRETCHDIBITS => blit(rec, dc, sink, &BlitLayout::STRETCHDIBITS)?,
        EMR_SETDIBITSTODEVICE => blit(rec, dc, sink, &BlitLayout::SETDIBITSTODEVICE)?,
        EMR_BITBLT | EMR_STRETCHBLT | EMR_ALPHABLEND | EMR_TRANSPARENTBLT => {
            blit(rec, dc, sink, &BlitLayout::BLT)?
        }
        EMR_COMMENT => {
            // An EMR_COMMENT whose data begins with the "EMF+" signature carries GDI+ records we do
            // not interpret. Detect it so a consumer can flag partial rendering; cbData is at 8.
            let cb = u32_at(rec, 8)? as usize;
            let data = rec.get(12..12usize.checked_add(cb)?)?;
            if data.starts_with(b"EMF+") {
                sink.unsupported(Feature::EmfPlus);
            }
        }
        EMR_SETTEXTCOLOR => dc.cur.text_color = Color::from_colorref(u32_at(rec, 8)?),
        EMR_SETWORLDTRANSFORM => {
            dc.cur.world = read_xform(rec, 8)?;
        }
        EMR_MODIFYWORLDTRANSFORM => {
            let m = read_xform(rec, 8)?;
            let mode = u32_at(rec, 44)?;
            dc.cur.world = match mode {
                MWT_SET => m,
                MWT_LEFTMULTIPLY => m.then(&dc.cur.world),
                MWT_RIGHTMULTIPLY => dc.cur.world.then(&m),
                _ => dc.cur.world, // MWT_IDENTITY / unknown: leave unchanged
            };
        }
        EMR_SETWINDOWORGEX => dc.set_window_org(i32_at(rec, 8)? as f64, i32_at(rec, 12)? as f64),
        EMR_SETWINDOWEXTEX => dc.set_window_ext(i32_at(rec, 8)? as f64, i32_at(rec, 12)? as f64),
        EMR_SETVIEWPORTORGEX => {
            dc.set_viewport_org(i32_at(rec, 8)? as f64, i32_at(rec, 12)? as f64)
        }
        EMR_SETVIEWPORTEXTEX => {
            dc.set_viewport_ext(i32_at(rec, 8)? as f64, i32_at(rec, 12)? as f64)
        }
        EMR_SAVEDC => dc.save(),
        EMR_RESTOREDC => dc.restore(i32_at(rec, 8)?),
        EMR_CREATEPEN => {
            let index = u32_at(rec, 8)?;
            // LOGPEN: lopnStyle u32 (12), lopnWidth POINTL.x i32 (16), lopnColor COLORREF (24).
            let pen = pen_from(u32_at(rec, 12)?, i32_at(rec, 16)? as f64, u32_at(rec, 24)?);
            dc.set_object(index, GdiObject::Pen(pen));
        }
        EMR_EXTCREATEPEN => {
            let index = u32_at(rec, 8)?;
            // EXTLOGPEN: elpPenStyle u32 (28), elpWidth u32 (32), elpColor COLORREF (40).
            let pen = pen_from(u32_at(rec, 28)?, i32_at(rec, 32)? as f64, u32_at(rec, 40)?);
            dc.set_object(index, GdiObject::Pen(pen));
        }
        EMR_CREATEBRUSHINDIRECT => {
            let index = u32_at(rec, 8)?;
            // LOGBRUSH: lbStyle u32 (12), lbColor COLORREF (16). Style 1 = BS_NULL (hollow).
            let style = u32_at(rec, 12)?;
            let brush = if style == BS_NULL {
                None
            } else {
                Some(Brush {
                    color: Color::from_colorref(u32_at(rec, 16)?),
                })
            };
            dc.set_object(index, GdiObject::Brush(brush));
        }
        EMR_EXTCREATEFONTINDIRECTW => {
            let index = u32_at(rec, 8)?;
            dc.set_object(index, GdiObject::Font(read_logfont(rec, 12)?));
        }
        EMR_SELECTOBJECT => select_object(dc, u32_at(rec, 8)?),
        EMR_DELETEOBJECT => dc.clear_object(u32_at(rec, 8)?),
        _ => {} // unknown/unhandled: skip (the outer loop advances by nSize)
    }
    Some(())
}

/// Map a device-unit rectangle through the current transform, re-normalising the corners.
fn map_rect(dc: &Dc, r: Rect) -> Rect {
    Rect::from_corners(dc.map(r.left, r.top), dc.map(r.right, r.bottom))
}

/// The variety of point-array record [`poly`] handles.
#[derive(Clone, Copy)]
enum Shape {
    /// Closed, fillable polygon.
    Polygon,
    /// Open polyline.
    Polyline,
    /// Open polyline continuing from the current position.
    PolylineTo,
    /// Cubic Bézier spline (start + triplets), flattened to a polyline.
    Bezier,
    /// Cubic Bézier continuing from the current position (triplets), flattened.
    BezierTo,
}

/// Decode a single point-array record and emit it. `p16` selects `POINT16` (2×i16) vs `POINTL`
/// (2×i32). Layout: `rclBounds` (8..24), count `u32` (24..28), then the point array at 28.
fn poly<S: MetafileSink>(rec: &[u8], dc: &mut Dc, sink: &mut S, shape: Shape, p16: bool) {
    let Some(count) = u32_at(rec, 24) else { return };
    let Some(raw) = read_points(rec, 28, count as usize, p16) else {
        return; // truncated point array: skip
    };
    emit_shape(dc, sink, shape, &raw);
}

/// Decode a `POLYPOLYGON`/`POLYPOLYLINE` record (one or more sub-figures). Layout: `rclBounds`
/// (8..24), polygon count `u32` (24..28), total point count `u32` (28..32), the per-polygon counts
/// (`nPolys × u32` from 32), then the point array.
fn poly_poly<S: MetafileSink>(rec: &[u8], dc: &mut Dc, sink: &mut S, closed: bool, p16: bool) {
    let Some(n_polys) = u32_at(rec, 24) else {
        return;
    };
    let n_polys = n_polys as usize;
    let counts_at = 32usize;
    let Some(points_at) = counts_at.checked_add(n_polys.saturating_mul(4)) else {
        return;
    };
    let mut offset = points_at;
    for i in 0..n_polys {
        let Some(c) = u32_at(rec, counts_at + i * 4) else {
            return;
        };
        let c = c as usize;
        let Some(raw) = read_points(rec, offset, c, p16) else {
            return;
        };
        let stride = if p16 { 4 } else { 8 };
        offset += c * stride;
        emit_shape(
            dc,
            sink,
            if closed {
                Shape::Polygon
            } else {
                Shape::Polyline
            },
            &raw,
        );
    }
}

/// Transform, flatten (for Béziers), and dispatch a decoded point list to the right sink callback,
/// updating the current position for the `*To` variants.
fn emit_shape<S: MetafileSink>(dc: &mut Dc, sink: &mut S, shape: Shape, raw: &[(f64, f64)]) {
    if raw.is_empty() {
        return;
    }
    let state = graphics_state(dc);
    match shape {
        Shape::Polygon => {
            let pts: Vec<Point> = raw.iter().map(|&(x, y)| dc.map(x, y)).collect();
            sink.polygon(&pts, &state);
        }
        Shape::Polyline => {
            let pts: Vec<Point> = raw.iter().map(|&(x, y)| dc.map(x, y)).collect();
            sink.polyline(&pts, &state);
            dc.cur.point = *raw.last().unwrap();
        }
        Shape::PolylineTo => {
            let mut pts = Vec::with_capacity(raw.len() + 1);
            pts.push(dc.map(dc.cur.point.0, dc.cur.point.1));
            pts.extend(raw.iter().map(|&(x, y)| dc.map(x, y)));
            sink.polyline(&pts, &state);
            dc.cur.point = *raw.last().unwrap();
        }
        Shape::Bezier => {
            let flat = flatten_bezier(dc, raw.first().copied(), raw);
            if flat.len() >= 2 {
                sink.polyline(&flat, &state);
            }
            dc.cur.point = *raw.last().unwrap();
        }
        Shape::BezierTo => {
            let flat = flatten_bezier(dc, Some(dc.cur.point), raw);
            if flat.len() >= 2 {
                sink.polyline(&flat, &state);
            }
            dc.cur.point = *raw.last().unwrap();
        }
    }
}

/// Flatten a cubic-Bézier point list into device-space polyline vertices. `start` is the spline's
/// first point (the current position for `*To` variants); `pts` supplies the remaining control/anchor
/// triplets. For a plain `POLYBEZIER`, `start` is `pts[0]` and the triplets follow.
fn flatten_bezier(dc: &Dc, start: Option<(f64, f64)>, pts: &[(f64, f64)]) -> Vec<Point> {
    let Some(start) = start else {
        return Vec::new();
    };
    // For POLYBEZIER the first array element is the anchor; skip it so the remainder is whole triplets.
    let triplets = if pts.first() == Some(&start) {
        &pts[1..]
    } else {
        pts
    };
    let mut out = vec![dc.map(start.0, start.1)];
    let mut p0 = start;
    for seg in triplets.chunks(3) {
        if seg.len() < 3 {
            break;
        }
        let (c1, c2, p3) = (seg[0], seg[1], seg[2]);
        for step in 1..=BEZIER_STEPS {
            let t = step as f64 / BEZIER_STEPS as f64;
            let mt = 1.0 - t;
            let x = mt * mt * mt * p0.0
                + 3.0 * mt * mt * t * c1.0
                + 3.0 * mt * t * t * c2.0
                + t * t * t * p3.0;
            let y = mt * mt * mt * p0.1
                + 3.0 * mt * mt * t * c1.1
                + 3.0 * mt * t * t * c2.1
                + t * t * t * p3.1;
            out.push(dc.map(x, y));
        }
        p0 = p3;
    }
    out
}

/// Read `count` raw (untransformed) points at `off`. `POINT16` (2×i16, 4 bytes) when `p16`, else
/// `POINTL` (2×i32, 8 bytes). Returns `None` if the array would overrun the record.
fn read_points(rec: &[u8], off: usize, count: usize, p16: bool) -> Option<Vec<(f64, f64)>> {
    let stride = if p16 { 4 } else { 8 };
    let need = off.checked_add(count.checked_mul(stride)?)?;
    if need > rec.len() {
        return None;
    }
    let mut points = Vec::with_capacity(count);
    for i in 0..count {
        let o = off + i * stride;
        let (x, y) = if p16 {
            (i16_at(rec, o)? as f64, i16_at(rec, o + 2)? as f64)
        } else {
            (i32_at(rec, o)? as f64, i32_at(rec, o + 4)? as f64)
        };
        points.push((x, y));
    }
    Some(points)
}

/// An `EMR_EXTTEXTOUT*` record → a text callback at the (transformed) reference point. `wide` selects
/// UTF-16LE (`W`) vs. ANSI/Latin-1 (`A`) decoding.
fn text<S: MetafileSink>(rec: &[u8], dc: &Dc, sink: &mut S, wide: bool) -> Option<()> {
    // EMRTEXT begins at offset 36: ptlReference POINTL (36..44), nChars u32 (44), offString u32 (48).
    let ref_x = i32_at(rec, 36)? as f64;
    let ref_y = i32_at(rec, 40)? as f64;
    let n_chars = u32_at(rec, 44)? as usize;
    let off_string = u32_at(rec, 48)? as usize;

    let s = if wide {
        let bytes = rec.get(off_string..off_string.checked_add(n_chars.checked_mul(2)?)?)?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        let bytes = rec.get(off_string..off_string.checked_add(n_chars)?)?;
        bytes.iter().map(|&b| b as char).collect()
    };
    if s.is_empty() {
        return Some(());
    }
    let position = dc.map(ref_x, ref_y);
    sink.text(&s, position, &graphics_state(dc));
    Some(())
}

/// The record-relative field offsets a raster-blit record shares: the destination box (`x`/`y` origin
/// and `cx`/`cy` extent) and the source DIB (`BITMAPINFOHEADER`+palette block and pixel block).
struct BlitLayout {
    dst_x: usize,
    dst_y: usize,
    dst_cx: usize,
    dst_cy: usize,
    off_bmi: usize,
    cb_bmi: usize,
    off_bits: usize,
    cb_bits: usize,
}

impl BlitLayout {
    /// `EMR_STRETCHDIBITS`: dest origin at 24, dest extent (`cxDest`/`cyDest`) at 72, DIB at 48.
    const STRETCHDIBITS: BlitLayout = BlitLayout {
        dst_x: 24,
        dst_y: 28,
        dst_cx: 72,
        dst_cy: 76,
        off_bmi: 48,
        cb_bmi: 52,
        off_bits: 56,
        cb_bits: 60,
    };
    /// `EMR_SETDIBITSTODEVICE`: no dest extent — the blit is 1:1, so the source extent
    /// (`cxSrc`/`cySrc` at 40) is the destination size.
    const SETDIBITSTODEVICE: BlitLayout = BlitLayout {
        dst_x: 24,
        dst_y: 28,
        dst_cx: 40,
        dst_cy: 44,
        off_bmi: 48,
        cb_bmi: 52,
        off_bits: 56,
        cb_bits: 60,
    };
    /// `EMR_BITBLT`/`EMR_STRETCHBLT`/`EMR_ALPHABLEND`/`EMR_TRANSPARENTBLT`: dest box at 24/32, DIB at
    /// 84 (a source `XFORM` occupies 52..76 first). The extra `*BLT` fields (ROP, source extent,
    /// blend/transparent-colour) don't affect placement or the source bitmap, so one layout serves.
    const BLT: BlitLayout = BlitLayout {
        dst_x: 24,
        dst_y: 28,
        dst_cx: 32,
        dst_cy: 36,
        off_bmi: 84,
        cb_bmi: 88,
        off_bits: 92,
        cb_bits: 96,
    };
}

/// Decode a raster-blit record: read its destination box and source DIB, repackage the DIB into a
/// self-contained image ([`dib_to_bitmap`]), map the box through the current transform, and emit it.
/// A blit with no source bitmap (`cbBmiSrc == 0`, i.e. a ROP-only pattern fill) draws nothing.
fn blit<S: MetafileSink>(rec: &[u8], dc: &Dc, sink: &mut S, l: &BlitLayout) -> Option<()> {
    let cb_bmi = u32_at(rec, l.cb_bmi)? as usize;
    if cb_bmi == 0 {
        return Some(());
    }
    let dx = i32_at(rec, l.dst_x)? as f64;
    let dy = i32_at(rec, l.dst_y)? as f64;
    let cx = i32_at(rec, l.dst_cx)? as f64;
    let cy = i32_at(rec, l.dst_cy)? as f64;
    let off_bmi = u32_at(rec, l.off_bmi)? as usize;
    let off_bits = u32_at(rec, l.off_bits)? as usize;
    let cb_bits = u32_at(rec, l.cb_bits)? as usize;
    let bmi = rec.get(off_bmi..off_bmi.checked_add(cb_bmi)?)?;
    let bits = rec.get(off_bits..off_bits.checked_add(cb_bits)?)?;
    let bitmap = dib_to_bitmap(bmi, bits)?;
    let bounds = Rect::from_corners(dc.map(dx, dy), dc.map(dx + cx, dy + cy));
    sink.image(bounds, &bitmap);
    Some(())
}

/// Repackage a device-independent bitmap (its `BITMAPINFO` block `bmi` — header + colour table — and
/// pixel block `bits`) into a complete image file. A `BI_JPEG`/`BI_PNG` DIB already *is* a JPEG/PNG
/// file, so its bits are returned verbatim; any other DIB is wrapped in a 14-byte `BITMAPFILEHEADER`
/// to form a `.bmp`. No pixel decoding happens here.
fn dib_to_bitmap(bmi: &[u8], bits: &[u8]) -> Option<Bitmap> {
    // biCompression lives at offset 16 of a BITMAPINFOHEADER (biSize >= 40); older/smaller headers
    // (BITMAPCOREHEADER) have no compression field and are always uncompressed.
    let bi_size = u32_at(bmi, 0)? as usize;
    let compression = if bi_size >= 40 { u32_at(bmi, 16)? } else { 0 };
    match compression {
        BI_JPEG => Some(Bitmap {
            format: BitmapFormat::Jpeg,
            bytes: bits.to_vec(),
        }),
        BI_PNG => Some(Bitmap {
            format: BitmapFormat::Png,
            bytes: bits.to_vec(),
        }),
        _ => {
            // BITMAPFILEHEADER: "BM", u32 file size, 2×u16 reserved, u32 offset to the pixel bits.
            let off_bits = 14usize.checked_add(bmi.len())?;
            let total = off_bits.checked_add(bits.len())?;
            let mut out = Vec::with_capacity(total);
            out.extend_from_slice(b"BM");
            out.extend_from_slice(&(total as u32).to_le_bytes());
            out.extend_from_slice(&[0, 0, 0, 0]);
            out.extend_from_slice(&(off_bits as u32).to_le_bytes());
            out.extend_from_slice(bmi);
            out.extend_from_slice(bits);
            Some(Bitmap {
                format: BitmapFormat::Bmp,
                bytes: out,
            })
        }
    }
}

/// Apply an `EMR_SELECTOBJECT`: set the current pen, brush, or font from the handle table, or from a
/// stock object when the high bit (`0x80000000`) is set.
fn select_object(dc: &mut Dc, handle: u32) {
    if handle & STOCK_OBJECT != 0 {
        match handle {
            WHITE_BRUSH => dc.cur.brush = Some(Brush { color: gray(255) }),
            LTGRAY_BRUSH => dc.cur.brush = Some(Brush { color: gray(192) }),
            GRAY_BRUSH => dc.cur.brush = Some(Brush { color: gray(128) }),
            DKGRAY_BRUSH => dc.cur.brush = Some(Brush { color: gray(64) }),
            BLACK_BRUSH => {
                dc.cur.brush = Some(Brush {
                    color: Color::BLACK,
                })
            }
            NULL_BRUSH => dc.cur.brush = None,
            WHITE_PEN => dc.cur.pen = Some(stock_pen(Color::WHITE)),
            BLACK_PEN => dc.cur.pen = Some(stock_pen(Color::BLACK)),
            NULL_PEN => dc.cur.pen = None,
            _ => {} // other stock objects (fonts/palettes) don't affect pen/brush
        }
        return;
    }
    match dc.object(handle) {
        Some(GdiObject::Pen(p)) => dc.cur.pen = *p,
        Some(GdiObject::Brush(b)) => dc.cur.brush = *b,
        Some(GdiObject::Font(f)) => dc.cur.font = f.clone(),
        None => {}
    }
}

/// Build a pen from a LOGPEN/EXTLOGPEN style word, logical width, and `COLORREF`. `PS_NULL` yields
/// `None` (an invisible pen). The width stays in logical units; the caller resolves it.
fn pen_from(style: u32, width: f64, color: u32) -> Option<Pen> {
    let pen_style = match style & PS_STYLE_MASK {
        PS_NULL => return None,
        PS_DASH => PenStyle::Dash,
        PS_DOT => PenStyle::Dot,
        PS_DASHDOT => PenStyle::DashDot,
        PS_DASHDOTDOT => PenStyle::DashDotDot,
        _ => PenStyle::Solid, // PS_SOLID / PS_INSIDEFRAME / unknown
    };
    Some(Pen {
        color: Color::from_colorref(color),
        width: width.max(0.0),
        style: pen_style,
    })
}

fn stock_pen(color: Color) -> Pen {
    Pen {
        color,
        width: 0.0,
        style: PenStyle::Solid,
    }
}

/// Read an `XFORM` (6× `f32`: eM11, eM12, eM21, eM22, eDx, eDy) at `off`.
fn read_xform(rec: &[u8], off: usize) -> Option<Matrix> {
    Some(Matrix {
        m11: f32_at(rec, off)? as f64,
        m12: f32_at(rec, off + 4)? as f64,
        m21: f32_at(rec, off + 8)? as f64,
        m22: f32_at(rec, off + 12)? as f64,
        dx: f32_at(rec, off + 16)? as f64,
        dy: f32_at(rec, off + 20)? as f64,
    })
}

/// Read the leading fields of a `LOGFONT` (as embedded in `EMR_EXTCREATEFONTINDIRECTW`) at `off`:
/// `lfHeight` i32 (0), `lfWeight` i32 (16), `lfItalic` u8 (20), `lfUnderline` u8 (22), `lfEscapement`
/// i32 (4), and the 32-`u16` `lfFaceName` at offset 28.
fn read_logfont(rec: &[u8], off: usize) -> Option<Font> {
    let height = i32_at(rec, off)? as f64;
    let escapement = i32_at(rec, off + 4)?;
    let weight = i32_at(rec, off + 16)?.clamp(0, 1000) as u16;
    let italic = rec.get(off + 20).copied().unwrap_or(0) != 0;
    let underline = rec.get(off + 22).copied().unwrap_or(0) != 0;
    // lfFaceName: up to 32 UTF-16LE code units, NUL-terminated.
    let face_at = off + 28;
    let mut units = Vec::new();
    for i in 0..32 {
        let u = crate::reader::u16_at(rec, face_at + i * 2)?;
        if u == 0 {
            break;
        }
        units.push(u);
    }
    Some(Font {
        face: String::from_utf16_lossy(&units),
        height,
        weight,
        italic,
        underline,
        escapement,
    })
}

fn gray(level: u8) -> Color {
    Color::rgb(level, level, level)
}

// Pen styles (MS-EMF `PenStyle`), low nibble.
const PS_STYLE_MASK: u32 = 0x0000_000F;
const PS_DASH: u32 = 1;
const PS_DOT: u32 = 2;
const PS_DASHDOT: u32 = 3;
const PS_DASHDOTDOT: u32 = 4;
const PS_NULL: u32 = 5;

// Brush style (MS-WMF `BrushStyle`): BS_NULL / BS_HOLLOW.
const BS_NULL: u32 = 1;

// DIB compression (MS-WMF `Compression`) values that carry an embedded image file rather than raw
// pixels: the DIB bits are a whole JPEG or PNG.
const BI_JPEG: u32 = 4;
const BI_PNG: u32 = 5;

// Stock objects (MS-EMF §2.1.31): the high bit marks a stock-object handle.
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
