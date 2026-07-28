//! SVG output backend for the [`rpt_pages`] Page IR.
//!
//! Coordinate model: 1 SVG user unit = 1 twip via the `viewBox` — see [`rpt_render_util`] for the
//! cross-backend coordinate reference.
//!
//! The simplest backend: a pure, dependency-light emit that maps each
//! [`DrawOp`] to one SVG element. Vector output diffs exactly (no rasterization tolerance needed),
//! which makes it the easiest backend to verify.
//!
//! Coordinates are twips throughout: the `<svg>` `viewBox` is the page's twip extent, so 1 SVG
//! user unit = 1 twip and every op keeps its native coordinates (a consumer scales via CSS or the
//! `width`/`height` attributes). Font point sizes convert to twips (1pt = 20 twips) so text scales
//! with the same coordinate system.
//!
//! # Images
//! Each distinct image is embedded once per page as `<image>` in `<defs>`, keyed by a content hash of
//! its bytes and referenced by `<use>` at every placement — so the repeated header logo and the
//! shared product thumbnail each contribute a single `<defs>` entry. Bytes are inlined as a `data:`
//! URI (`image/png`, `image/jpeg`, `image/gif`, and Crystal's embedded `image/bmp`, which browsers
//! render), so the SVG stays self-contained. An image op with no matching asset draws a placeholder.

use rpt_model::{Color, Rect};
use rpt_pages::{
    DrawOp, EllipseOp, Fill, HatchPattern, ImageAsset, ImageFit, ImageOp, LineOp, Page, PolygonOp,
    RectOp, Stroke, TextAlign, TextRun,
};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

/// 1 typographic point = 20 twips (exact in `f32`).
const TWIPS_PER_POINT: f32 = rpt_render_util::TWIPS_PER_POINT as f32;
/// Symbol fallback family appended to every `font-family` so a viewer resolves glyphs the named
/// family lacks (⚠ etc.). Matches the face `rpt-text` bundles for the PDF/raster backends (DejaVu
/// Sans); SVG names families and lets the renderer resolve them, so naming it is enough.
const SYMBOL_FALLBACK_FAMILY: &str = "DejaVu Sans";

/// Render one [`Page`] to a standalone SVG document string. Image ops are drawn as placeholders (no
/// bytes are available); use [`render_page_with_assets`] to embed images.
pub fn render_page(page: &Page) -> String {
    render_page_with_assets(page, &BTreeMap::new())
}

/// Like [`render_page`], but embeds each image op whose `image_id` resolves in `assets`. Each distinct
/// image is inlined once into `<defs>` (keyed by a content hash of its bytes) and referenced by
/// `<use>` at every placement, so identical bytes never embed more than once. An image op with no
/// matching asset draws a placeholder box.
pub fn render_page_with_assets(page: &Page, assets: &BTreeMap<String, ImageAsset>) -> String {
    let mut svg = String::new();
    let (w, h) = (page.size.width.0, page.size.height.0);
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" \
         width=\"{w}\" height=\"{h}\">"
    );
    // Distinct images, embedded once in <defs>; the resolver maps each op's id to its shared def id.
    let images = ImageDefs::collect(&page.ops, assets);
    images.emit_defs(&mut svg);
    // Draw-op coordinates are printable-relative (0-based); on the physical page, shift them by the
    // page origin (the report's top-left margin) so content sits inside the margins.
    let (ox, oy) = (page.origin.x.0, page.origin.y.0);
    let close_g = if ox != 0 || oy != 0 {
        let _ = writeln!(svg, "<g transform=\"translate({ox} {oy})\">");
        true
    } else {
        false
    };
    let mut ids: u32 = 0;
    for op in &page.ops {
        emit_op(&mut svg, &mut ids, &images, op);
    }
    if close_g {
        svg.push_str("</g>\n");
    }
    svg.push_str("</svg>\n");
    svg
}

/// Render a slice of [`DrawOp`]s (in absolute twip coordinates) to a bare `<svg>` fragment whose
/// `viewBox` is `viewbox` (twips). Used by the HTML backend to embed non-axis-aligned chart geometry
/// (which its positioned-div model can't express) as an inline SVG island; the caller positions the
/// island at `viewbox` and the ops keep their absolute coordinates (the viewBox maps them into it).
pub fn render_fragment(ops: &[DrawOp], viewbox: Rect) -> String {
    let (x, y, w, h) = (
        viewbox.left.0,
        viewbox.top.0,
        viewbox.width.0.max(1),
        viewbox.height.0.max(1),
    );
    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{x} {y} {w} {h}\" \
         width=\"100%\" height=\"100%\" preserveAspectRatio=\"none\">"
    );
    // A fragment (chart island) carries no out-of-band assets; any image op draws a placeholder.
    let images = ImageDefs::default();
    let mut ids: u32 = 0;
    for op in ops {
        emit_op(&mut svg, &mut ids, &images, op);
    }
    svg.push_str("</svg>");
    svg
}

fn emit_op(svg: &mut String, ids: &mut u32, images: &ImageDefs, op: &DrawOp) {
    match op {
        DrawOp::Rect(r) => emit_rect(svg, ids, r),
        DrawOp::Ellipse(e) => emit_ellipse(svg, ids, e),
        DrawOp::Line(l) => emit_line(svg, l),
        DrawOp::Polygon(p) => emit_polygon(svg, ids, p),
        DrawOp::Text(t) => emit_text(svg, t),
        DrawOp::Image(i) => emit_image(svg, images, i),
    }
}

fn emit_rect(svg: &mut String, ids: &mut u32, r: &RectOp) {
    let fill = fill_paint(svg, ids, r.fill.as_ref());
    let (stroke, stroke_w, dash) = stroke_attrs(r.stroke.as_ref());
    let radius = if r.corner_radius.0 > 0 {
        format!(" rx=\"{}\"", r.corner_radius.0)
    } else {
        String::new()
    };
    let _ = writeln!(
        svg,
        "  <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{radius} \
         fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{stroke_w}\"{dash}{}/>",
        r.bounds.left.0,
        r.bounds.top.0,
        r.bounds.width.0,
        r.bounds.height.0,
        source_attr(r.source.as_ref()),
    );
}

fn emit_polygon(svg: &mut String, ids: &mut u32, p: &PolygonOp) {
    if p.points.is_empty() {
        return;
    }
    let fill = fill_paint(svg, ids, p.fill.as_ref());
    let (stroke, stroke_w, dash) = stroke_attrs(p.stroke.as_ref());
    let mut points = String::new();
    for (i, pt) in p.points.iter().enumerate() {
        if i > 0 {
            points.push(' ');
        }
        let _ = write!(points, "{},{}", pt.x.0, pt.y.0);
    }
    // A closed region is `<polygon>` (auto-joins last→first); an open path is `<polyline>`.
    let tag = if p.closed { "polygon" } else { "polyline" };
    let _ = writeln!(
        svg,
        "  <{tag} points=\"{points}\" fill=\"{fill}\" stroke=\"{stroke}\" \
         stroke-width=\"{stroke_w}\"{dash}{}/>",
        source_attr(p.source.as_ref()),
    );
}

fn emit_ellipse(svg: &mut String, ids: &mut u32, e: &EllipseOp) {
    if e.bounds.width.0 <= 0 || e.bounds.height.0 <= 0 {
        return;
    }
    let fill = fill_paint(svg, ids, e.fill.as_ref());
    let (stroke, stroke_w, dash) = stroke_attrs(e.stroke.as_ref());
    let cx = e.bounds.left.0 + e.bounds.width.0 / 2;
    let cy = e.bounds.top.0 + e.bounds.height.0 / 2;
    let rx = e.bounds.width.0 / 2;
    let ry = e.bounds.height.0 / 2;
    let _ = writeln!(
        svg,
        "  <ellipse cx=\"{cx}\" cy=\"{cy}\" rx=\"{rx}\" ry=\"{ry}\" fill=\"{fill}\" \
         stroke=\"{stroke}\" stroke-width=\"{stroke_w}\"{dash}{}/>",
        source_attr(e.source.as_ref()),
    );
}

/// Resolve a fill to an SVG paint value, appending any `<defs>` (gradient/pattern) it needs to `svg`
/// with a document-unique id from `ids`. A solid fill returns `#rrggbb` and appends nothing, so
/// solid output is byte-identical to the pre-widening backend; `None` returns `"none"`.
fn fill_paint(svg: &mut String, ids: &mut u32, fill: Option<&Fill>) -> String {
    match fill {
        None => "none".to_string(),
        Some(Fill::Solid(c)) => css_color(*c),
        Some(Fill::LinearGradient { stops, angle_deg }) => {
            let id = format!("g{}", *ids);
            *ids += 1;
            // Map the angle (CCW, y-down) to a gradient vector across the object bounding box.
            let rad = (*angle_deg).to_radians();
            let (dx, dy) = (rad.cos(), -rad.sin());
            let (x1, y1) = (0.5 - dx * 0.5, 0.5 - dy * 0.5);
            let (x2, y2) = (0.5 + dx * 0.5, 0.5 + dy * 0.5);
            let mut defs = String::new();
            let _ = write!(
                defs,
                "  <defs><linearGradient id=\"{id}\" x1=\"{x1:.4}\" y1=\"{y1:.4}\" \
                 x2=\"{x2:.4}\" y2=\"{y2:.4}\">"
            );
            for (offset, color) in stops {
                let _ = write!(
                    defs,
                    "<stop offset=\"{o:.4}\" stop-color=\"{c}\"/>",
                    o = offset.clamp(0.0, 1.0),
                    c = css_color(*color),
                );
            }
            defs.push_str("</linearGradient></defs>\n");
            svg.push_str(&defs);
            format!("url(#{id})")
        }
        Some(Fill::Hatch { fg, bg, pattern }) => {
            let id = format!("h{}", *ids);
            *ids += 1;
            // A 120-twip (~8px) tile: a background rect plus the pattern's foreground lines.
            const T: i32 = 120;
            let (fg, bg) = (css_color(*fg), css_color(*bg));
            let mut defs = String::new();
            let _ = write!(
                defs,
                "  <defs><pattern id=\"{id}\" width=\"{T}\" height=\"{T}\" \
                 patternUnits=\"userSpaceOnUse\"><rect width=\"{T}\" height=\"{T}\" fill=\"{bg}\"/>"
            );
            for line in hatch_lines(*pattern, T) {
                let _ = write!(
                    defs,
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{fg}\" stroke-width=\"12\"/>",
                    line.0, line.1, line.2, line.3,
                );
            }
            defs.push_str("</pattern></defs>\n");
            svg.push_str(&defs);
            format!("url(#{id})")
        }
    }
}

/// The `(x1,y1,x2,y2)` foreground segments of a hatch tile of side `t`.
fn hatch_lines(pattern: HatchPattern, t: i32) -> Vec<(i32, i32, i32, i32)> {
    let h = t / 2;
    match pattern {
        HatchPattern::Horizontal => vec![(0, h, t, h)],
        HatchPattern::Vertical => vec![(h, 0, h, t)],
        HatchPattern::ForwardDiagonal => vec![(0, t, t, 0)],
        HatchPattern::BackwardDiagonal => vec![(0, 0, t, t)],
        HatchPattern::Cross => vec![(0, h, t, h), (h, 0, h, t)],
        HatchPattern::DiagonalCross => vec![(0, t, t, 0), (0, 0, t, t)],
    }
}

fn emit_line(svg: &mut String, l: &LineOp) {
    let (stroke, stroke_w, dash) = stroke_attrs(Some(&l.stroke));
    let _ = writeln!(
        svg,
        "  <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" \
         stroke=\"{stroke}\" stroke-width=\"{stroke_w}\"{dash}{}/>",
        l.from.x.0,
        l.from.y.0,
        l.to.x.0,
        l.to.y.0,
        source_attr(l.source.as_ref()),
    );
}

fn emit_text(svg: &mut String, t: &TextRun) {
    let size_twips = (t.font.size_pt * TWIPS_PER_POINT).round() as i32;
    // SVG text anchors on the baseline = top + ascent. Use the run's resolved ascent when the layout
    // engine measured it; else fall back to the shared em ratio. (Horizontal alignment stays on
    // `text-anchor`, which SVG measures exactly, so the stored advance is not needed here.)
    let baseline = match &t.metrics {
        Some(m) => t.bounds.top.0 + m.ascent.0,
        None => t.bounds.top.0 + (size_twips as f64 * rpt_render_util::ASCENT_FALLBACK_EM) as i32,
    };
    let (anchor, x) = match t.align {
        TextAlign::Left | TextAlign::Justified => ("start", t.bounds.left.0),
        TextAlign::Center => ("middle", t.bounds.left.0 + t.bounds.width.0 / 2),
        TextAlign::Right => ("end", t.bounds.left.0 + t.bounds.width.0),
    };
    // Justified: stretch the run to fill its box width by widening the spacing (SVG measures the shaped
    // width itself, so `textLength` + `lengthAdjust="spacing"` flushes both edges). Only when the line
    // has an inter-word gap and its shaped width is under the box (the layout marks a paragraph's last
    // line `Left`, so this only stretches the interior wrapped lines).
    let justify = if matches!(t.align, TextAlign::Justified)
        && rpt_render_util::word_gap_count(&t.text) > 0
        && t.metrics.is_none_or(|m| m.advance.0 < t.bounds.width.0)
    {
        format!(
            " textLength=\"{}\" lengthAdjust=\"spacing\"",
            t.bounds.width.0
        )
    } else {
        String::new()
    };
    let weight = if t.font.bold {
        " font-weight=\"bold\""
    } else {
        ""
    };
    let style = if t.font.italic {
        " font-style=\"italic\""
    } else {
        ""
    };
    let decoration = match (t.font.underline, t.font.strikethrough) {
        (true, true) => " text-decoration=\"underline line-through\"",
        (true, false) => " text-decoration=\"underline\"",
        (false, true) => " text-decoration=\"line-through\"",
        (false, false) => "",
    };
    // Rotation is CCW degrees about the run's origin (top-left of bounds); SVG `rotate` is CW-positive,
    // so negate. `0.0` emits no transform, keeping upright output byte-identical.
    let rotate = if t.rotation != 0.0 {
        format!(
            " transform=\"rotate({:.4} {} {})\"",
            -t.rotation, t.bounds.left.0, t.bounds.top.0,
        )
    } else {
        String::new()
    };
    let _ = writeln!(
        svg,
        "  <text x=\"{x}\" y=\"{baseline}\" font-family=\"{}, {SYMBOL_FALLBACK_FAMILY}\" font-size=\"{size_twips}\" \
         text-anchor=\"{anchor}\" fill=\"{}\"{weight}{style}{decoration}{justify}{rotate}{}>{}</text>",
        escape_attr(&t.font.family),
        css_color(t.color),
        source_attr(t.source.as_ref()),
        escape_text(&t.text),
    );
}

fn emit_image(svg: &mut String, images: &ImageDefs, i: &ImageOp) {
    // A resolved image references its shared <defs> entry via <use>; an op with no asset (chart, or
    // undecoded picture) draws a placeholder rect carrying the id so the layout is still diffable.
    match images.def_id(&i.image_id) {
        Some(def_id) => {
            let _ = writeln!(
                svg,
                "  <use href=\"#{def_id}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{}/>",
                i.bounds.left.0,
                i.bounds.top.0,
                i.bounds.width.0,
                i.bounds.height.0,
                source_attr(i.source.as_ref()),
            );
        }
        None => {
            let _ = writeln!(
                svg,
                "  <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"none\" \
                 stroke=\"#888\" stroke-dasharray=\"40 40\" data-image-id=\"{}\"{}/>",
                i.bounds.left.0,
                i.bounds.top.0,
                i.bounds.width.0,
                i.bounds.height.0,
                escape_attr(&i.image_id),
                source_attr(i.source.as_ref()),
            );
        }
    }
}

/// The page's distinct images, each embedded once as `<image>` in `<defs>` and keyed by a content
/// hash of its bytes, so identical placements (a per-page logo, a shared thumbnail) share one entry.
#[derive(Default)]
struct ImageDefs {
    /// Distinct images in first-appearance order: `(def id, media type, bytes, fit)`.
    defs: Vec<(String, String, Vec<u8>, ImageFit)>,
    /// `<defs>` id for a (hash, fit) pair, so a later op with the same bytes and fit reuses the entry.
    by_hash: HashMap<(u64, ImageFit), String>,
    /// Maps an op's `image_id` to its `<defs>` id (only for resolved assets).
    by_image_id: HashMap<String, String>,
}

impl ImageDefs {
    /// Resolve every image op against `assets`, interning distinct byte strings by content hash and
    /// fit mode (a shared entry carries the `preserveAspectRatio` its fit implies).
    fn collect(ops: &[DrawOp], assets: &BTreeMap<String, ImageAsset>) -> ImageDefs {
        let mut d = ImageDefs::default();
        for op in ops {
            let DrawOp::Image(i) = op else { continue };
            if d.by_image_id.contains_key(&i.image_id) {
                continue;
            }
            let Some(asset) = assets.get(&i.image_id) else {
                continue;
            };
            let key = (rpt_render_util::content_hash(&asset.bytes), i.fit);
            let def_id = match d.by_hash.get(&key) {
                Some(def_id) => def_id.clone(),
                None => {
                    let suffix = if i.fit == ImageFit::Contain { "c" } else { "" };
                    let def_id = format!("im-{:016x}{suffix}", key.0);
                    d.by_hash.insert(key, def_id.clone());
                    d.defs.push((
                        def_id.clone(),
                        asset.media_type.clone(),
                        asset.bytes.clone(),
                        i.fit,
                    ));
                    def_id
                }
            };
            d.by_image_id.insert(i.image_id.clone(), def_id);
        }
        d
    }

    /// The `<defs>` id an op's `image_id` resolves to, or `None` for an unembeddable/missing asset.
    fn def_id(&self, image_id: &str) -> Option<&str> {
        self.by_image_id.get(image_id).map(String::as_str)
    }

    /// Emit the `<defs>` block: one `<image>` per distinct asset, bytes inlined as a `data:` URI.
    /// The image's `preserveAspectRatio` follows its fit: `none` stretches to each `<use>` box
    /// (`Fill`), `xMidYMid meet` letterboxes uniformly and centers within it (`Contain`).
    fn emit_defs(&self, svg: &mut String) {
        if self.defs.is_empty() {
            return;
        }
        svg.push_str("<defs>\n");
        for (def_id, media_type, bytes, fit) in &self.defs {
            let par = match fit {
                ImageFit::Fill => "none",
                ImageFit::Contain => "xMidYMid meet",
            };
            let _ = writeln!(
                svg,
                "  <image id=\"{def_id}\" preserveAspectRatio=\"{par}\" \
                 href=\"data:{media};base64,{data}\"/>",
                media = escape_attr(media_type),
                data = rpt_render_util::base64_encode(bytes),
            );
        }
        svg.push_str("</defs>\n");
    }
}

/// `(stroke_color, stroke_width, dash_attr)` for an optional stroke.
fn stroke_attrs(stroke: Option<&Stroke>) -> (String, i32, String) {
    match stroke {
        None => ("none".to_string(), 0, String::new()),
        Some(s) => {
            let w = s.width.0.max(1);
            let dash = match rpt_render_util::dash_pattern(s.style, w) {
                None => String::new(),
                Some([on, off]) => format!(" stroke-dasharray=\"{on} {off}\""),
            };
            (css_color(s.color), w, dash)
        }
    }
}

/// A `data-src` attribute encoding the op's originating object (section/name/kind) — lets a viewer
/// or the parity tool key an element back to its report object without re-deriving it.
fn source_attr(source: Option<&rpt_pages::ObjectRef>) -> String {
    match source {
        None => String::new(),
        Some(o) => {
            let name = o.object_name.as_deref().unwrap_or("");
            format!(
                " data-section=\"{}\" data-object=\"{}\" data-kind=\"{:?}\"",
                escape_attr(&o.section),
                escape_attr(name),
                o.kind,
            )
        }
    }
}

/// `#rrggbb` (alpha dropped — SVG carries it via `fill-opacity`, unused by the current ops).
fn css_color(c: Color) -> String {
    c.to_hex()
}

use rpt_render_util::{escape_xml_attr as escape_attr, escape_xml_text as escape_text};

/// The SVG backend as a [`PageBackend`](rpt_pages::PageBackend): one standalone SVG string per page. `render_fragment` and
/// the per-page [`render_page`] stay available as free functions for callers that want them.
#[derive(Debug, Default, Clone, Copy)]
pub struct SvgBackend;

impl rpt_pages::PageBackend for SvgBackend {
    type Output = Vec<String>;
    type Options = ();

    fn render(&self, doc: &rpt_pages::PagedDocument, _opts: &()) -> Vec<String> {
        doc.pages
            .iter()
            .map(|p| render_page_with_assets(p, &doc.assets))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_model::{Color, Twips};
    use rpt_pages::{FontSpec, LineStyle, ObjectKind, ObjectRef, PageSize, Point};

    fn page() -> Page {
        let mut p = Page::new(
            1,
            PageSize {
                width: Twips(12240),
                height: Twips(15840),
            },
        );
        p.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(100),
                top: Twips(100),
                width: Twips(2000),
                height: Twips(400),
            },
            fill: None,
            stroke: Some(Stroke {
                color: Color {
                    a: 255,
                    r: 0,
                    g: 0,
                    b: 0,
                },
                width: Twips(15),
                style: LineStyle::Single,
            }),
            corner_radius: Twips(0),
            source: Some(ObjectRef::new("Details", ObjectKind::Box).named("Box1")),
        }));
        p.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(150),
                top: Twips(150),
                width: Twips(1900),
                height: Twips(300),
            },
            text: "A & B <ok>".to_string(),
            font: FontSpec {
                bold: true,
                ..FontSpec::default()
            },
            color: Color {
                a: 255,
                r: 16,
                g: 32,
                b: 48,
            },
            align: TextAlign::Center,
            rotation: 0.0,
            metrics: None,
            source: Some(ObjectRef::new("Details", ObjectKind::Field).named("name")),
        }));
        p.push(DrawOp::Line(LineOp {
            from: Point::new(100, 600),
            to: Point::new(2100, 600),
            stroke: Stroke {
                color: Color {
                    a: 255,
                    r: 0,
                    g: 0,
                    b: 0,
                },
                width: Twips(15),
                style: LineStyle::Dashed,
            },
            source: None,
        }));
        p
    }

    #[test]
    fn golden_svg_page_snapshot() {
        rpt_test_support::assert_golden(
            env!("CARGO_MANIFEST_DIR"),
            "page.svg",
            &render_page(&page()),
        );
    }

    #[test]
    fn emits_polygon_and_polyline_for_closed_and_open() {
        use rpt_pages::PolygonOp;
        let pts = vec![
            Point::new(100, 100),
            Point::new(500, 100),
            Point::new(300, 400),
        ];
        let mut p = Page::new(
            1,
            PageSize {
                width: Twips(1000),
                height: Twips(1000),
            },
        );
        p.push(DrawOp::Polygon(PolygonOp {
            points: pts.clone(),
            closed: true,
            fill: Some(
                Color {
                    a: 255,
                    r: 10,
                    g: 20,
                    b: 30,
                }
                .into(),
            ),
            stroke: None,
            source: None,
        }));
        p.push(DrawOp::Polygon(PolygonOp {
            points: pts,
            closed: false,
            fill: None,
            stroke: Some(Stroke {
                color: Color {
                    a: 255,
                    r: 0,
                    g: 0,
                    b: 0,
                },
                width: Twips(15),
                style: LineStyle::Single,
            }),
            source: None,
        }));
        let svg = render_page(&p);
        // Closed → <polygon>, open → <polyline>, both with the point list.
        assert!(
            svg.contains("<polygon points=\"100,100 500,100 300,400\""),
            "{svg}"
        );
        assert!(
            svg.contains("<polyline points=\"100,100 500,100 300,400\""),
            "{svg}"
        );
        assert!(svg.contains("fill=\"#0a141e\""));
    }

    #[test]
    fn renders_well_formed_svg() {
        let svg = render_page(&page());
        assert!(svg.starts_with("<svg"));
        assert!(svg.trim_end().ends_with("</svg>"));
        assert!(svg.contains("viewBox=\"0 0 12240 15840\""));
        // one rect, one text, one line
        assert_eq!(svg.matches("<rect").count(), 1);
        assert_eq!(svg.matches("<text").count(), 1);
        assert_eq!(svg.matches("<line").count(), 1);
    }

    #[test]
    fn escapes_text_and_carries_identity() {
        let svg = render_page(&page());
        assert!(svg.contains("A &amp; B &lt;ok&gt;"));
        assert!(svg.contains("data-object=\"name\""));
        assert!(svg.contains("data-kind=\"Field\""));
        // Centered text anchors at the horizontal midpoint (150 + 1900/2 = 1100).
        assert!(svg.contains("text-anchor=\"middle\""));
        assert!(svg.contains("x=\"1100\""));
    }

    #[test]
    fn bold_and_dash_attributes() {
        let svg = render_page(&page());
        assert!(svg.contains("font-weight=\"bold\""));
        // 10pt → 200 twips.
        assert!(svg.contains("font-size=\"200\""));
        assert!(svg.contains("stroke-dasharray"));
    }

    #[test]
    fn ellipse_op_emits_ellipse_element() {
        use rpt_pages::EllipseOp;
        let mut p = Page::new(
            1,
            PageSize {
                width: Twips(1000),
                height: Twips(1000),
            },
        );
        p.push(DrawOp::Ellipse(EllipseOp {
            bounds: Rect {
                left: Twips(100),
                top: Twips(200),
                width: Twips(600),
                height: Twips(400),
            },
            fill: Some(
                Color {
                    a: 255,
                    r: 10,
                    g: 20,
                    b: 30,
                }
                .into(),
            ),
            stroke: None,
            source: None,
        }));
        let svg = render_page(&p);
        // Inscribed in bounds: cx=400, cy=400, rx=300, ry=200.
        assert!(
            svg.contains("<ellipse cx=\"400\" cy=\"400\" rx=\"300\" ry=\"200\""),
            "{svg}"
        );
        assert!(svg.contains("fill=\"#0a141e\""));
    }

    #[test]
    fn rotated_text_emits_rotate_transform_and_zero_is_noop() {
        use rpt_pages::FontSpec;
        let make = |rotation: f32| {
            let mut p = Page::new(
                1,
                PageSize {
                    width: Twips(1000),
                    height: Twips(1000),
                },
            );
            p.push(DrawOp::Text(TextRun {
                bounds: Rect {
                    left: Twips(120),
                    top: Twips(240),
                    width: Twips(600),
                    height: Twips(300),
                },
                text: "R".into(),
                font: FontSpec::default(),
                color: Color {
                    a: 255,
                    r: 0,
                    g: 0,
                    b: 0,
                },
                align: TextAlign::Left,
                rotation,
                metrics: None,
                source: None,
            }));
            render_page(&p)
        };
        // CCW 90° → SVG rotate(-90) about the origin (120,240).
        assert!(make(90.0).contains("transform=\"rotate(-90.0000 120 240)\""));
        // 0.0 is a no-op — no transform attribute.
        assert!(!make(0.0).contains("transform="));
    }

    #[test]
    fn justified_run_stretches_with_textlength() {
        use rpt_pages::{FontSpec, TextMetrics};
        let run = |align, advance| {
            let mut p = Page::new(
                1,
                PageSize {
                    width: Twips(2000),
                    height: Twips(1000),
                },
            );
            p.push(DrawOp::Text(TextRun {
                bounds: Rect {
                    left: Twips(0),
                    top: Twips(0),
                    width: Twips(1000),
                    height: Twips(240),
                },
                text: "two words".into(),
                font: FontSpec::default(),
                color: Color {
                    a: 255,
                    r: 0,
                    g: 0,
                    b: 0,
                },
                align,
                rotation: 0.0,
                metrics: Some(TextMetrics {
                    advance: Twips(advance),
                    ascent: Twips(160),
                    line_height: Twips(240),
                }),
                source: None,
            }));
            render_page(&p)
        };
        // A justified run under its box width stretches to the full width via textLength.
        assert!(
            run(TextAlign::Justified, 600).contains("textLength=\"1000\" lengthAdjust=\"spacing\"")
        );
        // A left run never stretches; a justified run already at/over width does not compress.
        assert!(!run(TextAlign::Left, 600).contains("textLength"));
        assert!(!run(TextAlign::Justified, 1000).contains("textLength"));
    }

    #[test]
    fn linear_gradient_emits_gradient_def() {
        let mut p = Page::new(
            1,
            PageSize {
                width: Twips(1000),
                height: Twips(1000),
            },
        );
        p.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(0),
                top: Twips(0),
                width: Twips(500),
                height: Twips(500),
            },
            fill: Some(Fill::LinearGradient {
                stops: vec![
                    (
                        0.0,
                        Color {
                            a: 255,
                            r: 255,
                            g: 0,
                            b: 0,
                        },
                    ),
                    (
                        1.0,
                        Color {
                            a: 255,
                            r: 0,
                            g: 0,
                            b: 255,
                        },
                    ),
                ],
                angle_deg: 0.0,
            }),
            stroke: None,
            corner_radius: Twips(0),
            source: None,
        }));
        let svg = render_page(&p);
        assert!(svg.contains("<linearGradient id=\"g0\""), "{svg}");
        assert!(svg.contains("<stop offset=\"0.0000\" stop-color=\"#ff0000\"/>"));
        assert!(svg.contains("<stop offset=\"1.0000\" stop-color=\"#0000ff\"/>"));
        assert!(svg.contains("fill=\"url(#g0)\""));
    }

    #[test]
    fn solid_fill_is_unchanged_by_widening() {
        // A solid fill must still emit the bare `#rrggbb` paint with no <defs>.
        let mut p = Page::new(
            1,
            PageSize {
                width: Twips(1000),
                height: Twips(1000),
            },
        );
        p.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(0),
                top: Twips(0),
                width: Twips(500),
                height: Twips(500),
            },
            fill: Some(
                Color {
                    a: 255,
                    r: 10,
                    g: 20,
                    b: 30,
                }
                .into(),
            ),
            stroke: None,
            corner_radius: Twips(0),
            source: None,
        }));
        let svg = render_page(&p);
        assert!(svg.contains("fill=\"#0a141e\""));
        assert!(!svg.contains("<defs>"));
    }

    fn image_op(id: &str, left: i32, top: i32) -> DrawOp {
        image_op_fit(id, left, top, ImageFit::Fill)
    }

    fn image_op_fit(id: &str, left: i32, top: i32, fit: ImageFit) -> DrawOp {
        DrawOp::Image(ImageOp {
            bounds: Rect {
                left: Twips(left),
                top: Twips(top),
                width: Twips(720),
                height: Twips(480),
            },
            image_id: id.to_string(),
            fit,
            source: Some(ObjectRef::new("Details", ObjectKind::Image).named(id)),
        })
    }

    fn image_page(ops: Vec<DrawOp>) -> Page {
        let mut p = Page::new(
            1,
            PageSize {
                width: Twips(4000),
                height: Twips(4000),
            },
        );
        for op in ops {
            p.push(op);
        }
        p
    }

    #[test]
    fn image_without_asset_renders_placeholder_not_use() {
        let svg = render_page(&image_page(vec![image_op("Picture1", 100, 100)]));
        assert!(!svg.contains("<defs>"));
        assert!(!svg.contains("<use "));
        assert!(svg.contains("data-image-id=\"Picture1\""));
    }

    #[test]
    fn contain_image_uses_meet_aspect_ratio_in_defs() {
        let png: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        let mut assets = BTreeMap::new();
        assets.insert(
            "Picture1".to_string(),
            ImageAsset {
                media_type: "image/png".to_string(),
                bytes: png.to_vec(),
            },
        );
        // A Fill op stretches (`preserveAspectRatio="none"`); a Contain op letterboxes.
        let fill = render_page_with_assets(&image_page(vec![image_op("Picture1", 0, 0)]), &assets);
        assert!(fill.contains("preserveAspectRatio=\"none\""));
        let contain = render_page_with_assets(
            &image_page(vec![image_op_fit("Picture1", 0, 0, ImageFit::Contain)]),
            &assets,
        );
        assert!(contain.contains("preserveAspectRatio=\"xMidYMid meet\""));
        assert!(!contain.contains("preserveAspectRatio=\"none\""));
    }

    #[test]
    fn image_with_asset_embeds_in_defs_and_references_via_use() {
        let png: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        let mut assets = BTreeMap::new();
        assets.insert(
            "Picture1".to_string(),
            ImageAsset {
                media_type: "image/png".to_string(),
                bytes: png.to_vec(),
            },
        );
        let svg =
            render_page_with_assets(&image_page(vec![image_op("Picture1", 100, 100)]), &assets);
        assert!(svg.contains("<defs>"));
        assert!(svg.contains("<image id=\"im-"));
        assert!(svg.contains("href=\"data:image/png;base64,"));
        assert!(svg.contains(&rpt_render_util::base64_encode(png)));
        // Referenced by <use> at the placement box, not re-inlined.
        assert!(svg.contains("<use href=\"#im-"));
        assert!(svg.contains("width=\"720\" height=\"480\""));
    }

    #[test]
    fn identical_bytes_embed_once_referenced_many() {
        // The same image bytes under two ids (a per-page logo, a shared thumbnail) → one <defs>
        // <image>, N <use> references.
        let bytes: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        let mut assets = BTreeMap::new();
        for id in ["Logo", "Thumb"] {
            assets.insert(
                id.to_string(),
                ImageAsset {
                    media_type: "image/png".to_string(),
                    bytes: bytes.to_vec(),
                },
            );
        }
        let page = image_page(vec![
            image_op("Logo", 0, 0),
            image_op("Thumb", 0, 1000),
            image_op("Thumb", 0, 2000),
        ]);
        let svg = render_page_with_assets(&page, &assets);
        assert_eq!(svg.matches("<image ").count(), 1, "one shared <defs> entry");
        assert_eq!(
            svg.matches("<use ").count(),
            3,
            "three placements reference it"
        );
    }
}
