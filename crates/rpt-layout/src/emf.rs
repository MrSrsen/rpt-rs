//! EMF picture → Page IR.
//!
//! Bridges the standalone [`metafile`] crate to the [`rpt_pages`] Page IR: a user-inserted EMF
//! picture is a *vector command stream*, so replaying its records is the faithful render (the
//! alternative is a placeholder box). The [`metafile`] parser decodes
//! the stream into device-independent primitives; [`PageSink`] maps each into a [`DrawOp`], scaling
//! the metafile's device-space bounds onto the picture object's destination twip box.

use metafile::{
    Bitmap, Brush, Feature, GraphicsState, MetafileHeader, MetafileSink, Pen, PenStyle,
    Point as MfPoint,
};
use rpt_model::{Color, Rect, Twips};
use rpt_pages::{
    DrawOp, EllipseOp, Fill, FontSpec, ImageAsset, ImageFit, ImageOp, LineOp, LineStyle, ObjectRef,
    Point, PolygonOp, RectOp, Stroke, TextAlign, TextRun,
};

/// A hairline stroke width in twips (~1px at 96 dpi) — the floor for any drawn pen so a zero-width
/// (cosmetic) or sub-pixel-scaled pen still paints a visible line.
const HAIRLINE: i32 = 15;

/// The result of interpreting an EMF picture: the draw-ops to place, plus the out-of-band image
/// assets any embedded bitmaps produced (keyed by the `image_id` their [`DrawOp::Image`] references),
/// and whether the stream carried EMF+ content we could not render.
pub(crate) struct InterpretedEmf {
    /// The draw-ops, in painter's order.
    pub ops: Vec<DrawOp>,
    /// Image assets referenced by an emitted [`DrawOp::Image`], for the caller to register.
    pub assets: Vec<(String, ImageAsset)>,
    /// `true` if the picture embedded EMF+ (GDI+) content that was detected but not rendered.
    pub has_emf_plus: bool,
}

/// Interpret an EMF byte stream into Page-IR draw-ops mapped into `dest` (a twip box). Returns `None`
/// on a stream the parser rejects (bad signature, truncated/garbage) so the caller can keep its
/// placeholder. `source` tags every emitted op with the originating report object; `id_base` seeds
/// the `image_id` of any embedded-bitmap asset (made unique per bitmap).
pub(crate) fn interpret_emf(
    bytes: &[u8],
    dest: Rect,
    source: Option<ObjectRef>,
    id_base: &str,
) -> Result<InterpretedEmf, metafile::Error> {
    let mut sink = PageSink {
        ops: Vec::new(),
        assets: Vec::new(),
        has_emf_plus: false,
        dest,
        source,
        id_base: id_base.to_string(),
        map: None,
    };
    metafile::parse_emf(bytes, &mut sink)?;
    Ok(InterpretedEmf {
        ops: sink.ops,
        assets: sink.assets,
        has_emf_plus: sink.has_emf_plus,
    })
}

/// An axis-aligned affine map from the metafile's device space (`rclBounds`) into destination twips.
struct BoundsMap {
    ox: f64,
    oy: f64,
    bx: f64,
    by: f64,
    sx: f64,
    sy: f64,
}

impl BoundsMap {
    fn point(&self, p: MfPoint) -> Point {
        Point {
            x: Twips((self.ox + (p.x - self.bx) * self.sx).round() as i32),
            y: Twips((self.oy + (p.y - self.by) * self.sy).round() as i32),
        }
    }

    /// Map a scalar device length (a pen width) into twips via the mean per-axis scale.
    fn scalar(&self, len: f64) -> i32 {
        (len * (self.sx.abs() + self.sy.abs()) / 2.0).round() as i32
    }
}

/// A [`MetafileSink`] that projects each decoded primitive into the Page IR.
struct PageSink {
    ops: Vec<DrawOp>,
    assets: Vec<(String, ImageAsset)>,
    has_emf_plus: bool,
    dest: Rect,
    source: Option<ObjectRef>,
    id_base: String,
    map: Option<BoundsMap>,
}

impl PageSink {
    /// The stroke for a resolved pen, or `None` for no outline. Width is scaled to twips and floored
    /// at a hairline so a cosmetic pen still paints.
    fn stroke(&self, pen: Option<Pen>, map: &BoundsMap) -> Option<Stroke> {
        let pen = pen?;
        let style = match pen.style {
            PenStyle::Solid => LineStyle::Single,
            PenStyle::Dash => LineStyle::Dashed,
            PenStyle::Dot | PenStyle::DashDot | PenStyle::DashDotDot => LineStyle::Dotted,
        };
        Some(Stroke {
            color: color(pen.color),
            width: Twips(map.scalar(pen.width).max(HAIRLINE)),
            style,
        })
    }

    fn fill(&self, brush: Option<Brush>) -> Option<Fill> {
        brush.map(|b| Fill::Solid(color(b.color)))
    }

    fn rect_from(&self, bounds: metafile::Rect, map: &BoundsMap) -> Rect {
        let p0 = map.point(MfPoint::new(bounds.left, bounds.top));
        let p1 = map.point(MfPoint::new(bounds.right, bounds.bottom));
        Rect {
            left: Twips(p0.x.0.min(p1.x.0)),
            top: Twips(p0.y.0.min(p1.y.0)),
            width: Twips((p1.x.0 - p0.x.0).abs()),
            height: Twips((p1.y.0 - p0.y.0).abs()),
        }
    }
}

impl MetafileSink for PageSink {
    fn header(&mut self, header: &MetafileHeader) {
        let b = header.bounds;
        // rclBounds is inclusive-inclusive, so its span is (right - left + 1) device units.
        let bw = (b.width() + 1.0).max(1.0);
        let bh = (b.height() + 1.0).max(1.0);
        self.map = Some(BoundsMap {
            ox: self.dest.left.0 as f64,
            oy: self.dest.top.0 as f64,
            bx: b.left,
            by: b.top,
            sx: self.dest.width.0 as f64 / bw,
            sy: self.dest.height.0 as f64 / bh,
        });
    }

    fn rectangle(&mut self, bounds: metafile::Rect, state: &GraphicsState<'_>) {
        let Some(map) = &self.map else { return };
        let op = RectOp {
            bounds: self.rect_from(bounds, map),
            fill: self.fill(state.brush),
            stroke: self.stroke(state.pen, map),
            corner_radius: Twips(0),
            source: self.source.clone(),
        };
        self.ops.push(DrawOp::Rect(op));
    }

    fn ellipse(&mut self, bounds: metafile::Rect, state: &GraphicsState<'_>) {
        let Some(map) = &self.map else { return };
        let op = EllipseOp {
            bounds: self.rect_from(bounds, map),
            fill: self.fill(state.brush),
            stroke: self.stroke(state.pen, map),
            source: self.source.clone(),
        };
        self.ops.push(DrawOp::Ellipse(op));
    }

    fn polygon(&mut self, points: &[MfPoint], state: &GraphicsState<'_>) {
        let Some(map) = &self.map else { return };
        if points.is_empty() {
            return;
        }
        let op = PolygonOp {
            points: points.iter().map(|&p| map.point(p)).collect(),
            closed: true,
            fill: self.fill(state.brush),
            stroke: self.stroke(state.pen, map),
            source: self.source.clone(),
        };
        self.ops.push(DrawOp::Polygon(op));
    }

    fn polyline(&mut self, points: &[MfPoint], state: &GraphicsState<'_>) {
        let Some(map) = &self.map else { return };
        let Some(stroke) = self.stroke(state.pen, map) else {
            return; // no pen ⇒ nothing to draw for an open path
        };
        match points {
            [] | [_] => {}
            // A single segment is a Line; a longer run is an open polygon.
            [a, b] => self.ops.push(DrawOp::Line(LineOp {
                from: map.point(*a),
                to: map.point(*b),
                stroke,
                source: self.source.clone(),
            })),
            _ => self.ops.push(DrawOp::Polygon(PolygonOp {
                points: points.iter().map(|&p| map.point(p)).collect(),
                closed: false,
                fill: None,
                stroke: Some(stroke),
                source: self.source.clone(),
            })),
        }
    }

    fn text(&mut self, text: &str, position: MfPoint, state: &GraphicsState<'_>) {
        let Some(map) = &self.map else { return };
        let origin = map.point(position);
        let font = FontSpec::default();
        // Approximate the run box from the default font: ~0.5em per char wide, one em tall.
        let em = (font.size_pt as f64 * crate::TWIPS_PER_PT).round() as i32;
        let n_chars = text.chars().count() as i32;
        let width = (n_chars * em / 2).max(em);
        self.ops.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: origin.x,
                top: origin.y,
                width: Twips(width),
                height: Twips(em),
            },
            text: text.to_string(),
            font,
            color: color(state.text_color),
            align: TextAlign::Left,
            rotation: 0.0,
            metrics: None,
            character_spacing: Twips(0),
            source: self.source.clone(),
        }));
    }

    fn image(&mut self, bounds: metafile::Rect, bitmap: &Bitmap) {
        let Some(map) = &self.map else { return };
        // The metafile decoded a self-contained image file; register it as a page asset (id unique
        // per bitmap within this picture) and place it in the mapped destination box.
        let image_id = format!("{}#emf{}", self.id_base, self.assets.len());
        self.assets.push((
            image_id.clone(),
            ImageAsset {
                media_type: bitmap.format.media_type().to_string(),
                bytes: bitmap.bytes.clone(),
            },
        ));
        self.ops.push(DrawOp::Image(ImageOp {
            bounds: self.rect_from(bounds, map),
            image_id,
            fit: ImageFit::Fill,
            source: self.source.clone(),
        }));
    }

    fn unsupported(&mut self, feature: Feature) {
        match feature {
            Feature::EmfPlus => self.has_emf_plus = true,
        }
    }
}

/// Convert a [`metafile`] color into an [`rpt_model`] color.
fn color(c: metafile::Color) -> Color {
    Color {
        a: c.a,
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An 88-byte `ENHMETAHEADER` with `rclBounds = (0,0,99,99)` and the ` EMF` signature.
    fn header() -> Vec<u8> {
        let mut h = vec![0u8; 88];
        h[0..4].copy_from_slice(&1u32.to_le_bytes()); // EMR_HEADER
        h[4..8].copy_from_slice(&88u32.to_le_bytes());
        h[16..20].copy_from_slice(&99i32.to_le_bytes());
        h[20..24].copy_from_slice(&99i32.to_le_bytes());
        h[40..44].copy_from_slice(b" EMF");
        h
    }

    fn record(itype: u32, payload: &[u8]) -> Vec<u8> {
        let size = 8 + payload.len();
        let mut r = Vec::new();
        r.extend_from_slice(&itype.to_le_bytes());
        r.extend_from_slice(&(size as u32).to_le_bytes());
        r.extend_from_slice(payload);
        r
    }

    fn rectl(l: i32, t: i32, r: i32, b: i32) -> Vec<u8> {
        let mut v = Vec::new();
        for x in [l, t, r, b] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v
    }

    /// A 1440×1440-twip (1") box at (1000, 2000): with the 100-unit bounds span the scale is exactly
    /// 14.4 twips per logical unit.
    fn dest() -> Rect {
        Rect {
            left: Twips(1000),
            top: Twips(2000),
            width: Twips(1440),
            height: Twips(1440),
        }
    }

    #[test]
    fn rectangle_maps_into_dest() {
        let mut emf = header();
        emf.extend(record(43, &rectl(10, 10, 50, 50))); // EMR_RECTANGLE
        emf.extend(record(14, &[0u8; 12])); // EMR_EOF

        let ops = interpret_emf(&emf, dest(), None, "pic").unwrap().ops;
        assert_eq!(ops.len(), 1);
        let DrawOp::Rect(r) = &ops[0] else {
            panic!("expected a Rect, got {:?}", ops[0]);
        };
        assert_eq!(r.bounds.left, Twips(1144)); // 1000 + 10*14.4
        assert_eq!(r.bounds.top, Twips(2144));
        assert_eq!(r.bounds.width, Twips(576)); // (50-10)*14.4
        assert_eq!(r.bounds.height, Twips(576));
    }

    #[test]
    fn lineto_maps_into_dest() {
        let mut emf = header();
        emf.extend(record(27, &{
            let mut p = 20i32.to_le_bytes().to_vec();
            p.extend_from_slice(&20i32.to_le_bytes());
            p
        })); // EMR_MOVETOEX
        emf.extend(record(54, &{
            let mut p = 80i32.to_le_bytes().to_vec();
            p.extend_from_slice(&80i32.to_le_bytes());
            p
        })); // EMR_LINETO
        emf.extend(record(14, &[0u8; 12]));

        let ops = interpret_emf(&emf, dest(), None, "pic").unwrap().ops;
        assert_eq!(ops.len(), 1);
        let DrawOp::Line(l) = &ops[0] else {
            panic!("expected a Line, got {:?}", ops[0]);
        };
        assert_eq!(l.from, Point::new(1288, 2288)); // 20*14.4 = 288
        assert_eq!(l.to, Point::new(2152, 3152)); // 80*14.4 = 1152
    }

    /// A bad stream returns the parser's reason, not a bare `None` — the caller renders a placeholder
    /// either way, but only with the reason can it say *why*.
    #[test]
    fn a_bad_stream_returns_the_reason_it_failed() {
        let mut emf = header();
        emf[40..44].copy_from_slice(b"XXXX"); // corrupt signature
        emf.extend(record(14, &[0u8; 12]));
        let Err(err) = interpret_emf(&emf, dest(), None, "pic") else {
            panic!("a corrupt signature must fail");
        };
        assert_eq!(err, metafile::Error::NotAMetafile);
    }

    /// A truncated stream is a different diagnosis from a bad signature, and it locates itself.
    #[test]
    fn a_truncated_stream_reports_where_it_ran_out() {
        let mut emf = header();
        // A record claiming far more bytes than follow it.
        emf.extend(14u32.to_le_bytes());
        emf.extend(400u32.to_le_bytes());
        let Err(err) = interpret_emf(&emf, dest(), None, "pic") else {
            panic!("a truncated record must fail");
        };
        assert!(
            matches!(err, metafile::Error::UnexpectedEof { .. }),
            "{err:?}"
        );
        assert!(err.offset().is_some(), "the offset is the diagnosis: {err}");
    }

    #[test]
    fn stretchdibits_becomes_image_asset() {
        // A minimal 1×1 24bpp uncompressed DIB: 40-byte BITMAPINFOHEADER + 4 bytes of BGR0 pixel.
        let mut bmi = vec![0u8; 40];
        bmi[0..4].copy_from_slice(&40u32.to_le_bytes()); // biSize
        bmi[4..8].copy_from_slice(&1i32.to_le_bytes()); // biWidth
        bmi[8..12].copy_from_slice(&1i32.to_le_bytes()); // biHeight
        bmi[12..14].copy_from_slice(&1u16.to_le_bytes()); // biPlanes
        bmi[14..16].copy_from_slice(&24u16.to_le_bytes()); // biBitCount
                                                           // biCompression = BI_RGB (0) already zeroed.
        let bits = vec![0x10u8, 0x20, 0x30, 0x00]; // one padded BGR pixel

        // EMR_STRETCHDIBITS payload begins at record offset 8; build it with the fields the parser
        // reads (dest box at 24/72, DIB at 48). Offsets below are record-relative minus the 8-byte
        // record header, since `record()` prepends that header.
        let mut p = vec![0u8; 72];
        // rclBounds (8..24) left zeroed. xDest(24)/yDest(28). off* fields are record-relative, so the
        // DIB appended at payload offset 72 sits at record offset 80.
        p[16..20].copy_from_slice(&5i32.to_le_bytes()); // xDest = 5   (rec offset 24)
        p[20..24].copy_from_slice(&5i32.to_le_bytes()); // yDest = 5   (rec offset 28)
        p[40..44].copy_from_slice(&(80u32).to_le_bytes()); // offBmiSrc (rec offset 48)
        p[44..48].copy_from_slice(&(bmi.len() as u32).to_le_bytes()); // cbBmiSrc (52)
        p[48..52].copy_from_slice(&((80 + bmi.len()) as u32).to_le_bytes()); // offBitsSrc (56)
        p[52..56].copy_from_slice(&(bits.len() as u32).to_le_bytes()); // cbBitsSrc (60)
        p[64..68].copy_from_slice(&10i32.to_le_bytes()); // cxDest = 10 (rec offset 72)
        p[68..72].copy_from_slice(&10i32.to_le_bytes()); // cyDest = 10 (rec offset 76)
        p.extend_from_slice(&bmi);
        p.extend_from_slice(&bits);

        let mut emf = header();
        emf.extend(record(81, &p)); // EMR_STRETCHDIBITS
        emf.extend(record(14, &[0u8; 12]));

        let out = interpret_emf(&emf, dest(), None, "pic").unwrap();
        assert_eq!(out.ops.len(), 1);
        let DrawOp::Image(img) = &out.ops[0] else {
            panic!("expected an Image, got {:?}", out.ops[0]);
        };
        assert_eq!(img.bounds.left, Twips(1072)); // 1000 + 5*14.4
        assert_eq!(img.bounds.width, Twips(144)); // 10*14.4
        assert_eq!(out.assets.len(), 1);
        let (id, asset) = &out.assets[0];
        assert_eq!(id, &img.image_id);
        assert_eq!(asset.media_type, "image/bmp");
        assert!(asset.bytes.starts_with(b"BM")); // reconstructed BITMAPFILEHEADER
    }
}
