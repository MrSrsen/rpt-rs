//! Raster (PNG) output backend for the [`rpt_pages`] Page IR.
//!
//! Where the SVG/PDF/HTML backends emit vector output, this one **rasterizes** a [`Page`] to a
//! pixel bitmap (`tiny_skia::Pixmap`) and encodes it as PNG — the pixel-preview backend (it also
//! aligns with a possible future interactive editor canvas, which would draw the same pixels).
//!
//! Coordinate model: `px = (twip + origin) * dpi / 1440`, default 96 dpi — see [`rpt_render_util`]
//! for the cross-backend coordinate reference.
//!
//! # Coordinate model
//! The Page IR is in **twips** (1/1440 inch) and draw-op coordinates are **printable-relative**
//! (0,0 = top-left of the printable area, margin removed). A physical backend adds [`Page::origin`]
//! (the report's top-left margin, in twips) to place content on the paper — the raster analogue of
//! the SVG backend's `<g transform="translate(origin)">`. We then scale twips → pixels
//! by the chosen DPI: `px = (twip + origin) * DPI / 1440`. The default is 96 DPI, matching the HTML
//! backend's `TWIPS_PER_PX = 15` (1440/96 = 15).
//!
//! # Text
//! tiny-skia rasterizes *paths*, not text. We resolve each run's family (with bold/italic) to a
//! system face via [`fontdb`], parse it with [`fontdue`], rasterize each glyph to a coverage bitmap,
//! and alpha-composite it onto the pixmap in the run's colour — real glyphs, not boxes. A run whose
//! family cannot be resolved (no matching system font, no sans fallback) is skipped.
//!
//! # Images
//! Each [`ImageOp`] references its bytes by `image_id` in the document's out-of-band `assets` map.
//! Distinct images are decoded once (cached by a content hash of their bytes, so a picture placed N
//! times decodes once) to an RGBA bitmap and composited into the canvas scaled to the op's box. PNG
//! goes through tiny-skia; BMP (Crystal's embedded OLE bitmap format) through the in-crate decoder;
//! JPEG and GIF (first frame) through the pure-Rust `zune-jpeg` and `gif` crates. An op whose asset
//! is missing or undecodable draws a light-grey placeholder outline.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use rpt_model::Color;
use rpt_pages::{
    DrawOp, EllipseOp, Fill, ImageAsset, ImageFit, ImageOp, LineOp, Page, PolygonOp, RectOp,
    Stroke, TextRun,
};
use rpt_text::FontDb;
use tiny_skia::{
    FillRule, FilterQuality, Paint, PathBuilder, Pixmap, PixmapPaint, PremultipliedColorU8,
    Rect as SkRect, Shader, Stroke as SkStroke, StrokeDash, Transform,
};

/// Default output resolution. 96 DPI matches the HTML backend's `TWIPS_PER_PX = 15` (1440/96 = 15).
pub const DEFAULT_DPI: f32 = 96.0;

/// Twips per inch (the Page IR unit; 1 twip = 1/1440 inch). Exact in `f32`.
const TWIPS_PER_INCH: f32 = rpt_render_util::TWIPS_PER_INCH as f32;
/// Typographic points per inch (font sizes are in points). Exact in `f32`.
const POINTS_PER_INCH: f32 = rpt_render_util::POINTS_PER_INCH as f32;

/// Render one [`Page`] to PNG bytes at [`DEFAULT_DPI`]. Image ops draw placeholders (no bytes are
/// available); use [`render_pages_with_assets`] to composite embedded pictures.
pub fn render_page(page: &Page) -> Vec<u8> {
    render_page_dpi(page, DEFAULT_DPI)
}

/// Render one [`Page`] to PNG bytes at a caller-chosen `dpi`.
pub fn render_page_dpi(page: &Page, dpi: f32) -> Vec<u8> {
    encode_png(&render_page_pixmap_dpi(page, dpi))
}

/// Render one [`Page`] to a `tiny_skia::Pixmap` at [`DEFAULT_DPI`] (for callers that want the raw
/// pixels — e.g. compositing several pages, or feeding an on-screen canvas).
pub fn render_page_pixmap(page: &Page) -> Pixmap {
    render_page_pixmap_dpi(page, DEFAULT_DPI)
}

/// Render one [`Page`] to a `tiny_skia::Pixmap` at a caller-chosen `dpi`.
pub fn render_page_pixmap_dpi(page: &Page, dpi: f32) -> Pixmap {
    let mut cache = ImageCache::default();
    render_page_pixmap_into(page, dpi, &BTreeMap::new(), &mut cache)
}

/// Render every [`Page`] to its own PNG (one per page, mirroring the SVG backend's per-page output).
pub fn render_pages(pages: &[Page]) -> Vec<Vec<u8>> {
    pages.iter().map(render_page).collect()
}

/// Render every [`Page`] to its own PNG at `dpi`, embedding each image op whose `image_id` resolves
/// in `assets`. Distinct images decode once and are cached for the whole document (a picture placed
/// on every page decodes a single time), then composited scaled into each placement box.
pub fn render_pages_with_assets(
    pages: &[Page],
    assets: &BTreeMap<String, ImageAsset>,
    dpi: f32,
) -> Vec<Vec<u8>> {
    let mut cache = ImageCache::default();
    pages
        .iter()
        .map(|p| encode_png(&render_page_pixmap_into(p, dpi, assets, &mut cache)))
        .collect()
}

/// Rasterize one page into a fresh pixmap, resolving image ops against `assets` and reusing decoded
/// bitmaps held in `cache` (shared across a document's pages).
fn render_page_pixmap_into(
    page: &Page,
    dpi: f32,
    assets: &BTreeMap<String, ImageAsset>,
    cache: &mut ImageCache,
) -> Pixmap {
    let scale = dpi / TWIPS_PER_INCH;
    let ctx = Ctx {
        scale,
        ox: page.origin.x.0 as f32,
        oy: page.origin.y.0 as f32,
    };
    // The paper: at least 1×1 px so a degenerate/empty page still yields a valid PNG.
    let w = ((page.size.width.0 as f32 * scale).round() as u32).max(1);
    let h = ((page.size.height.0 as f32 * scale).round() as u32).max(1);
    let mut pixmap = Pixmap::new(w, h).expect("non-zero pixmap dimensions");
    pixmap.fill(tiny_skia::Color::WHITE);

    let fonts = Fonts::system();
    for op in &page.ops {
        match op {
            DrawOp::Rect(r) => draw_rect(&mut pixmap, &ctx, r),
            DrawOp::Ellipse(e) => draw_ellipse(&mut pixmap, &ctx, e),
            DrawOp::Line(l) => draw_line(&mut pixmap, &ctx, l),
            DrawOp::Polygon(p) => draw_polygon(&mut pixmap, &ctx, p),
            DrawOp::Text(t) => draw_text(&mut pixmap, &ctx, &fonts, dpi, t),
            DrawOp::Image(i) => draw_image(&mut pixmap, &ctx, assets, cache, i),
        }
    }
    pixmap
}

/// Knobs for [`RasterBackend`]. `Default` is [`DEFAULT_DPI`].
#[derive(Debug, Clone, Copy)]
pub struct RasterOptions {
    /// Output resolution in dots per inch; twip coordinates scale by `dpi / 1440`.
    pub dpi: f32,
}

impl Default for RasterOptions {
    fn default() -> RasterOptions {
        RasterOptions { dpi: DEFAULT_DPI }
    }
}

/// The raster backend as a [`rpt_pages::PageBackend`]: one PNG per page at the chosen DPI. The `render_page*`
/// free functions (and the `Pixmap` accessors) stay available for callers that want raw pixels.
#[derive(Debug, Default, Clone, Copy)]
pub struct RasterBackend;

impl rpt_pages::PageBackend for RasterBackend {
    type Output = Vec<Vec<u8>>;
    type Options = RasterOptions;

    fn render(&self, doc: &rpt_pages::PagedDocument, opts: &RasterOptions) -> Vec<Vec<u8>> {
        render_pages_with_assets(&doc.pages, &doc.assets, opts.dpi)
    }
}

/// Encode a pixmap as PNG bytes.
fn encode_png(pixmap: &Pixmap) -> Vec<u8> {
    // `encode_png` only fails on an allocation/IO error for an in-memory target, which cannot happen
    // for a valid pixmap.
    pixmap
        .encode_png()
        .expect("PNG encode of an in-memory pixmap")
}

/// The twip→pixel transform for one page: uniform `scale`, plus the printable-area origin (`ox`,`oy`,
/// in twips) added before scaling so printable-relative ops land inside the physical margins.
struct Ctx {
    scale: f32,
    ox: f32,
    oy: f32,
}

impl Ctx {
    fn x(&self, twips: i32) -> f32 {
        (twips as f32 + self.ox) * self.scale
    }
    fn y(&self, twips: i32) -> f32 {
        (twips as f32 + self.oy) * self.scale
    }
    fn len(&self, twips: i32) -> f32 {
        twips as f32 * self.scale
    }
}

fn sk_color(c: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

fn solid_paint(c: Color) -> Paint<'static> {
    Paint {
        shader: Shader::SolidColor(sk_color(c)),
        anti_alias: true,
        ..Paint::default()
    }
}

/// A solid paint for a Page-IR fill. Gradient/hatch fills are not tiled by this backend; both fall
/// back to the fill's [`Fill::representative_color`] (a gradient's midpoint stop, a hatch's
/// foreground). A [`Fill::Solid`] paints its own colour, so solid output is pixel-identical to before.
fn fill_paint(fill: &Fill) -> Paint<'static> {
    solid_paint(fill.representative_color())
}

/// A stroked-edge spec in pixels: width and an optional dash pattern for the line style.
fn sk_stroke(stroke: &Stroke, ctx: &Ctx) -> SkStroke {
    let width = ctx.len(stroke.width.0).max(1.0);
    let dash = rpt_render_util::dash_pattern(stroke.style, width)
        .and_then(|[on, off]| StrokeDash::new(vec![on, off], 0.0));
    SkStroke {
        width,
        dash,
        ..SkStroke::default()
    }
}

fn draw_rect(pixmap: &mut Pixmap, ctx: &Ctx, r: &RectOp) {
    let (x, y) = (ctx.x(r.bounds.left.0), ctx.y(r.bounds.top.0));
    let (w, h) = (ctx.len(r.bounds.width.0), ctx.len(r.bounds.height.0));
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let radius = ctx.len(r.corner_radius.0).min(w / 2.0).min(h / 2.0);
    let path = if radius > 0.0 {
        rounded_rect_path(x, y, w, h, radius)
    } else {
        SkRect::from_xywh(x, y, w, h).map(PathBuilder::from_rect)
    };
    let Some(path) = path else { return };

    if let Some(fill) = &r.fill {
        pixmap.fill_path(
            &path,
            &fill_paint(fill),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
    if let Some(stroke) = &r.stroke {
        pixmap.stroke_path(
            &path,
            &solid_paint(stroke.color),
            &sk_stroke(stroke, ctx),
            Transform::identity(),
            None,
        );
    }
}

/// An axis-aligned ellipse inscribed in the op's bounds (tiny-skia's oval), filled and/or stroked.
fn draw_ellipse(pixmap: &mut Pixmap, ctx: &Ctx, e: &EllipseOp) {
    let (x, y) = (ctx.x(e.bounds.left.0), ctx.y(e.bounds.top.0));
    let (w, h) = (ctx.len(e.bounds.width.0), ctx.len(e.bounds.height.0));
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let Some(rect) = SkRect::from_xywh(x, y, w, h) else {
        return;
    };
    let Some(path) = PathBuilder::from_oval(rect) else {
        return;
    };
    if let Some(fill) = &e.fill {
        pixmap.fill_path(
            &path,
            &fill_paint(fill),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
    if let Some(stroke) = &e.stroke {
        pixmap.stroke_path(
            &path,
            &solid_paint(stroke.color),
            &sk_stroke(stroke, ctx),
            Transform::identity(),
            None,
        );
    }
}

/// A rounded-rectangle path with quadratic corners of `radius` pixels.
fn rounded_rect_path(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<tiny_skia::Path> {
    let (l, t, r, b) = (x, y, x + w, y + h);
    let mut pb = PathBuilder::new();
    pb.move_to(l + radius, t);
    pb.line_to(r - radius, t);
    pb.quad_to(r, t, r, t + radius);
    pb.line_to(r, b - radius);
    pb.quad_to(r, b, r - radius, b);
    pb.line_to(l + radius, b);
    pb.quad_to(l, b, l, b - radius);
    pb.line_to(l, t + radius);
    pb.quad_to(l, t, l + radius, t);
    pb.close();
    pb.finish()
}

fn draw_line(pixmap: &mut Pixmap, ctx: &Ctx, l: &LineOp) {
    let mut pb = PathBuilder::new();
    pb.move_to(ctx.x(l.from.x.0), ctx.y(l.from.y.0));
    pb.line_to(ctx.x(l.to.x.0), ctx.y(l.to.y.0));
    let Some(path) = pb.finish() else { return };
    pixmap.stroke_path(
        &path,
        &solid_paint(l.stroke.color),
        &sk_stroke(&l.stroke, ctx),
        Transform::identity(),
        None,
    );
}

fn draw_polygon(pixmap: &mut Pixmap, ctx: &Ctx, p: &PolygonOp) {
    if p.points.len() < 2 {
        return;
    }
    let mut pb = PathBuilder::new();
    pb.move_to(ctx.x(p.points[0].x.0), ctx.y(p.points[0].y.0));
    for pt in &p.points[1..] {
        pb.line_to(ctx.x(pt.x.0), ctx.y(pt.y.0));
    }
    if p.closed {
        pb.close();
    }
    let Some(path) = pb.finish() else { return };
    // Only a closed region fills; an open polyline just strokes.
    if p.closed {
        if let Some(fill) = &p.fill {
            pixmap.fill_path(
                &path,
                &fill_paint(fill),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
    if let Some(stroke) = &p.stroke {
        pixmap.stroke_path(
            &path,
            &solid_paint(stroke.color),
            &sk_stroke(stroke, ctx),
            Transform::identity(),
            None,
        );
    }
}

/// Composite an image op: resolve its bytes in `assets`, decode once (cached by content hash in
/// `cache`), and paint it scaled into the op's box. A missing or undecodable asset draws a
/// placeholder outline instead so the layout stays visible.
fn draw_image(
    pixmap: &mut Pixmap,
    ctx: &Ctx,
    assets: &BTreeMap<String, ImageAsset>,
    cache: &mut ImageCache,
    i: &ImageOp,
) {
    let (x, y) = (ctx.x(i.bounds.left.0), ctx.y(i.bounds.top.0));
    let (w, h) = (ctx.len(i.bounds.width.0), ctx.len(i.bounds.height.0));
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let decoded = assets.get(&i.image_id).and_then(|asset| {
        cache
            .entry(rpt_render_util::content_hash(&asset.bytes))
            .or_insert_with(|| decode_image(asset))
            .clone()
    });
    let Some(src) = decoded else {
        return draw_image_placeholder(pixmap, x, y, w, h);
    };
    // Scale the source bitmap into the op's box. tiny-skia bilinearly samples the source under this
    // transform (draw offset 0,0; the translate lives in the matrix). `Fill` distorts to the box;
    // `Contain` scales uniformly and centers, leaving the surrounding space empty (letterbox).
    let (sw, sh) = (src.width() as f32, src.height() as f32);
    let (sx, sy, tx, ty) = match i.fit {
        ImageFit::Fill => (w / sw, h / sh, x, y),
        ImageFit::Contain => {
            let s = (w / sw).min(h / sh);
            (s, s, x + (w - sw * s) / 2.0, y + (h - sh * s) / 2.0)
        }
    };
    let transform = Transform::from_row(sx, 0.0, 0.0, sy, tx, ty);
    let paint = PixmapPaint {
        quality: FilterQuality::Bilinear,
        ..PixmapPaint::default()
    };
    pixmap.draw_pixmap(0, 0, (*src).as_ref(), &paint, transform, None);
}

/// A light-grey dashed placeholder box for an image with no compositable bytes.
fn draw_image_placeholder(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32) {
    let Some(rect) = SkRect::from_xywh(x, y, w, h) else {
        return;
    };
    let path = PathBuilder::from_rect(rect);
    let paint = solid_paint(Color {
        a: 255,
        r: 0x88,
        g: 0x88,
        b: 0x88,
    });
    let stroke = SkStroke {
        width: 1.0,
        dash: StrokeDash::new(vec![3.0, 3.0], 0.0),
        ..SkStroke::default()
    };
    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

/// Distinct decoded images, keyed by a content hash of their encoded bytes, so a picture placed many
/// times (e.g. a per-row thumbnail) decodes once for the whole document. `None` caches a decode
/// failure so it is not retried per placement.
type ImageCache = HashMap<u64, Option<Rc<Pixmap>>>;

/// Decode an image asset to a premultiplied [`Pixmap`]. PNG goes through tiny-skia; BMP (Crystal's
/// embedded OLE bitmap format), JPEG, and GIF through pure-Rust decoders. `None` for an unsupported
/// format or a decode failure (the caller then draws a placeholder).
fn decode_image(asset: &ImageAsset) -> Option<Rc<Pixmap>> {
    let pixmap = match asset.media_type.as_str() {
        "image/png" => Pixmap::decode_png(&asset.bytes).ok()?,
        "image/bmp" => {
            let (rgba, w, h) = rpt_render_util::decode_bmp_rgba(&asset.bytes)?;
            pixmap_from_rgba(rgba, w, h)?
        }
        "image/jpeg" => {
            let (rgba, w, h) = decode_jpeg_rgba(&asset.bytes)?;
            pixmap_from_rgba(rgba, w, h)?
        }
        "image/gif" => {
            let (rgba, w, h) = decode_gif_rgba(&asset.bytes)?;
            pixmap_from_rgba(rgba, w, h)?
        }
        _ => return None,
    };
    Some(Rc::new(pixmap))
}

/// Decode a JPEG to top-down straight RGBA8 + dimensions via the pure-Rust `zune-jpeg`. The decoder
/// converts any source colourspace (YCbCr/CMYK/greyscale) to RGBA with opaque alpha. `None` on a
/// malformed stream.
fn decode_jpeg_rgba(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    use zune_jpeg::JpegDecoder;

    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(data), options);
    let rgba = decoder.decode().ok()?;
    let info = decoder.info()?;
    Some((rgba, info.width as u32, info.height as u32))
}

/// Decode a GIF's first frame to straight RGBA8 + dimensions via the pure-Rust `gif` crate. A report
/// picture is a still image, so only the first frame is composited; its transparent index (if any)
/// yields transparent pixels. `None` on a malformed stream or an empty (frameless) GIF.
fn decode_gif_rgba(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(data).ok()?;
    let frame = decoder.read_next_frame().ok()??;
    let (w, h) = (frame.width as u32, frame.height as u32);
    Some((frame.buffer.to_vec(), w, h))
}

/// Build a [`Pixmap`] from straight (non-premultiplied) RGBA8 rows. Alpha-premultiplies each channel,
/// as tiny-skia's buffer requires; for the opaque BMPs Crystal emits this is a copy.
fn pixmap_from_rgba(rgba: Vec<u8>, w: u32, h: u32) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(w, h)?;
    for (dst, src) in pixmap.pixels_mut().iter_mut().zip(rgba.chunks_exact(4)) {
        let a = src[3];
        let prem = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
        *dst = PremultipliedColorU8::from_rgba(prem(src[0]), prem(src[1]), prem(src[2]), a)?;
    }
    Some(pixmap)
}

fn draw_text(pixmap: &mut Pixmap, ctx: &Ctx, fonts: &Fonts, dpi: f32, t: &TextRun) {
    if t.text.is_empty() {
        return;
    }
    let Some(font) = fonts.resolve(&t.font) else {
        return; // no resolvable face — nothing to draw (a warning path could log this)
    };
    // Point size → device pixels at this DPI (1pt = 1/72 inch).
    let px = (t.font.size_pt * dpi / POINTS_PER_INCH).max(1.0);

    // Split into single-face segments (primary family, else the bundled symbol fallback for ⚠ etc.)
    // and rasterize each char with its own covering face — so a missing glyph draws from the
    // fallback instead of a `.notdef` box. Each glyph's fontdue metrics come from its own face, so
    // its vertical bearing is correct while every glyph shares the one run baseline (below).
    let glyphs: Vec<(char, fontdue::Metrics, Vec<u8>)> = fonts
        .db
        .segment_by_coverage(&t.font, &t.text)
        .into_iter()
        .filter_map(|seg| Some((seg.range.clone(), fonts.resolve_face(seg.face)?)))
        .flat_map(|(range, face)| {
            t.text[range]
                .chars()
                .map(move |c| {
                    let (m, cov) = face.rasterize(c, px);
                    (c, m, cov)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    // Alignment anchor: the run's stored advance when the layout engine measured it (device px via
    // the twip scale), else the sum of fontdue advances.
    let text_w: f32 = match &t.metrics {
        Some(m) => ctx.len(m.advance.0),
        None => glyphs.iter().map(|(_, m, _)| m.advance_width).sum(),
    };

    let (bx, bw) = (ctx.x(t.bounds.left.0), ctx.len(t.bounds.width.0));
    let mut pen_x = rpt_render_util::aligned_x(t.align, bx, bw, text_w);
    // Justified: spread the slack (box width − shaped width) evenly across the inter-word gaps so the
    // line reaches both edges. Anchored flush-left by `aligned_x`; the extra is added after each space.
    let just_extra = rpt_render_util::justify_gap_extra(t.align, &t.text, bw, text_w);
    // Baseline: the run's top edge plus the ascent (device px). Use the run's stored ascent when
    // present (via the twip scale), else the resolved face's fontdue ascent, else the em fallback.
    let ascent = match &t.metrics {
        Some(m) => ctx.len(m.ascent.0),
        None => font
            .horizontal_line_metrics(px)
            .map(|m| m.ascent)
            .unwrap_or(px * rpt_render_util::ASCENT_FALLBACK_EM as f32),
    };
    let baseline = ctx.y(t.bounds.top.0) + ascent;

    // Rotation is CCW degrees about the run's origin (top-left of bounds). `0.0` uses the plain
    // axis-aligned blit (byte-identical to before); a non-zero angle forward-maps each glyph pixel
    // about the origin (exact for 90/180/270°, best-effort for arbitrary angles).
    let rot = (t.rotation != 0.0).then(|| {
        let r = t.rotation.to_radians();
        Rot {
            ox: ctx.x(t.bounds.left.0),
            oy: ctx.y(t.bounds.top.0),
            cos: r.cos(),
            sin: r.sin(),
        }
    });

    for (c, m, cov) in &glyphs {
        // fontdue coverage is row-major, y-down; `ymin` is the bitmap's bottom offset above the
        // baseline, so its top edge sits `ymin + height` above the baseline.
        let gx = pen_x + m.xmin as f32;
        let gy = baseline - (m.ymin as f32 + m.height as f32);
        match &rot {
            None => blit_coverage(pixmap, cov, m.width, m.height, gx, gy, t.color),
            Some(rot) => blit_coverage_rot(pixmap, cov, m.width, m.height, gx, gy, t.color, rot),
        }
        pen_x += m.advance_width;
        if *c == ' ' {
            pen_x += just_extra;
        }
    }

    // Underline / strikethrough as thin filled bars across the drawn extent. The axis-aligned bars
    // are only meaningful for upright text; a rotated run omits them (best-effort).
    if rot.is_some() {
        return;
    }
    let x0 = rpt_render_util::aligned_x(t.align, bx, bw, text_w);
    let thickness = (px * 0.06).max(1.0);
    if t.font.underline {
        fill_bar(pixmap, x0, baseline + thickness, text_w, thickness, t.color);
    }
    if t.font.strikethrough {
        fill_bar(
            pixmap,
            x0,
            baseline - ascent * 0.3,
            text_w,
            thickness,
            t.color,
        );
    }
}

/// Fill an axis-aligned bar (used for underline/strikethrough).
fn fill_bar(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, color: Color) {
    if let Some(rect) = SkRect::from_xywh(x, y, w, h.max(1.0)) {
        pixmap.fill_path(
            &PathBuilder::from_rect(rect),
            &solid_paint(color),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// A rotation of a glyph blit about a pivot `(ox, oy)` in device px: CCW `cos`/`sin` for the run's
/// angle (visual CCW in the y-down buffer).
struct Rot {
    ox: f32,
    oy: f32,
    cos: f32,
    sin: f32,
}

/// Alpha-composite a glyph coverage bitmap (one byte/pixel) onto the pixmap at `(ox, oy)` in the
/// run's colour, source-over on tiny-skia's premultiplied buffer.
fn blit_coverage(
    pixmap: &mut Pixmap,
    cov: &[u8],
    gw: usize,
    gh: usize,
    ox: f32,
    oy: f32,
    color: Color,
) {
    if gw == 0 || gh == 0 {
        return;
    }
    let pw = pixmap.width() as i32;
    let ph = pixmap.height() as i32;
    let x0 = ox.round() as i32;
    let y0 = oy.round() as i32;
    let pixels = pixmap.pixels_mut();
    for row in 0..gh {
        let py = y0 + row as i32;
        for col in 0..gw {
            let px = x0 + col as i32;
            composite_px(pixels, pw, ph, px, py, cov[row * gw + col], color);
        }
    }
}

/// Like [`blit_coverage`], but forward-maps each source pixel through a rotation about the pivot
/// (nearest-neighbour). Exact for 90/180/270°; higher angles are best-effort (minor sampling holes).
#[allow(clippy::too_many_arguments)]
fn blit_coverage_rot(
    pixmap: &mut Pixmap,
    cov: &[u8],
    gw: usize,
    gh: usize,
    gx: f32,
    gy: f32,
    color: Color,
    rot: &Rot,
) {
    if gw == 0 || gh == 0 {
        return;
    }
    let pw = pixmap.width() as i32;
    let ph = pixmap.height() as i32;
    let pixels = pixmap.pixels_mut();
    for row in 0..gh {
        for col in 0..gw {
            let coverage = cov[row * gw + col];
            if coverage == 0 {
                continue;
            }
            // Unrotated device position of this source pixel, rotated CCW about the pivot (y-down).
            let (dx, dy) = (gx + col as f32 - rot.ox, gy + row as f32 - rot.oy);
            let fx = rot.ox + dx * rot.cos + dy * rot.sin;
            let fy = rot.oy - dx * rot.sin + dy * rot.cos;
            composite_px(
                pixels,
                pw,
                ph,
                fx.round() as i32,
                fy.round() as i32,
                coverage,
                color,
            );
        }
    }
}

/// Source-over composite one `coverage` sample of `color` onto the premultiplied pixel at `(px, py)`
/// (a no-op when out of bounds or fully transparent).
fn composite_px(
    pixels: &mut [PremultipliedColorU8],
    pw: i32,
    ph: i32,
    px: i32,
    py: i32,
    coverage: u8,
    color: Color,
) {
    if px < 0 || px >= pw || py < 0 || py >= ph {
        return;
    }
    let a = (coverage as f32 / 255.0) * (color.a as f32 / 255.0);
    if a <= 0.0 {
        return;
    }
    let idx = (py * pw + px) as usize;
    let dst = pixels[idx];
    // Source-over in premultiplied space: out = src + dst*(1-a).
    let inv = 1.0 - a;
    let out_r = (color.r as f32 * a + dst.red() as f32 * inv).round() as u8;
    let out_g = (color.g as f32 * a + dst.green() as f32 * inv).round() as u8;
    let out_b = (color.b as f32 * a + dst.blue() as f32 * inv).round() as u8;
    let out_a = ((a * 255.0) + dst.alpha() as f32 * inv).round() as u8;
    // Clamp the premultiplied channels to alpha so `from_rgba` accepts them.
    let (out_r, out_g, out_b) = (out_r.min(out_a), out_g.min(out_a), out_b.min(out_a));
    if let Some(p) = PremultipliedColorU8::from_rgba(out_r, out_g, out_b, out_a) {
        pixels[idx] = p;
    }
}

/// A shared [`FontDb`] plus a cache of parsed [`fontdue::Font`]s, keyed by the resolved face id.
/// Built once per render; resolution (family with bold/italic, generic sans-serif fallback) is the
/// shared policy in [`rpt_text::FontDb`], so most runs get real glyphs even when the exact family is
/// missing; this backend keeps only its own fontdue parse+cache step.
struct Fonts {
    db: FontDb,
    cache: RefCell<HashMap<fontdb::ID, Option<Rc<fontdue::Font>>>>,
}

impl Fonts {
    fn system() -> Fonts {
        Fonts {
            db: FontDb::with_system_fonts(),
            cache: RefCell::new(HashMap::new()),
        }
    }

    fn resolve(&self, spec: &rpt_pages::FontSpec) -> Option<Rc<fontdue::Font>> {
        self.resolve_face(self.db.query(spec)?)
    }

    /// The parsed fontdue face for a resolved face id, read once then cached. Used both for the
    /// primary family and for the per-glyph symbol-fallback segments.
    fn resolve_face(&self, id: fontdb::ID) -> Option<Rc<fontdue::Font>> {
        if let Some(hit) = self.cache.borrow().get(&id) {
            return hit.clone();
        }
        let parsed = self
            .db
            .with_face_data(id, |data, index| {
                fontdue::Font::from_bytes(
                    data,
                    fontdue::FontSettings {
                        collection_index: index,
                        ..fontdue::FontSettings::default()
                    },
                )
                .ok()
                .map(Rc::new)
            })
            .flatten();
        self.cache.borrow_mut().insert(id, parsed.clone());
        parsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_model::{Color, Rect, Twips};
    use rpt_pages::{FontSpec, LineOp, LineStyle, Point, RectOp, Stroke, TextAlign, TextRun};
    use rpt_pages::{ObjectKind, ObjectRef, PageSize};

    fn page() -> Page {
        let mut p = Page::new(
            1,
            PageSize {
                width: Twips(12240),
                height: Twips(15840),
            },
        );
        // A filled + stroked rect near the top-left.
        p.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(200),
                top: Twips(200),
                width: Twips(3000),
                height: Twips(1000),
            },
            fill: Some(
                Color {
                    a: 255,
                    r: 200,
                    g: 40,
                    b: 40,
                }
                .into(),
            ),
            stroke: Some(Stroke {
                color: Color {
                    a: 255,
                    r: 0,
                    g: 0,
                    b: 0,
                },
                width: Twips(30),
                style: LineStyle::Single,
            }),
            corner_radius: Twips(0),
            source: Some(ObjectRef::new("Details", ObjectKind::Box).named("Box1")),
        }));
        p.push(DrawOp::Line(LineOp {
            from: Point::new(200, 1500),
            to: Point::new(3200, 1500),
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
        p.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(250),
                top: Twips(250),
                width: Twips(2900),
                height: Twips(400),
            },
            text: "Hello".to_string(),
            font: FontSpec {
                bold: true,
                ..FontSpec::default()
            },
            color: Color {
                a: 255,
                r: 255,
                g: 255,
                b: 255,
            },
            align: TextAlign::Left,
            rotation: 0.0,
            metrics: None,
            source: Some(ObjectRef::new("Details", ObjectKind::Field).named("greeting")),
        }));
        p
    }

    #[test]
    fn png_has_magic_and_is_nonempty() {
        let png = render_page(&page());
        assert!(
            png.len() > 100,
            "PNG should be non-trivial, got {}",
            png.len()
        );
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "PNG magic header");
    }

    #[test]
    fn pixmap_has_expected_dims_and_nonwhite_pixels() {
        let pm = render_page_pixmap(&page());
        // 12240 twips * 96/1440 = 816 px wide; 15840 → 1056 px tall.
        assert_eq!(pm.width(), 816);
        assert_eq!(pm.height(), 1056);

        // The red rect sits around (200,200)+origin twips → sample a pixel inside it and confirm it
        // is not the white paper background.
        let scale = DEFAULT_DPI / TWIPS_PER_INCH;
        let sx = ((1000.0) * scale) as u32; // ~66 px, inside the rect (left 200..3200 twips)
        let sy = ((600.0) * scale) as u32; // ~40 px, inside the rect (top 200..1200 twips)
        let idx = (sy * pm.width() + sx) as usize;
        let px = pm.pixels()[idx];
        assert!(
            !(px.red() == 255 && px.green() == 255 && px.blue() == 255),
            "expected drawn (non-white) content inside the rect, got white"
        );
    }

    #[test]
    fn png_output_is_deterministic() {
        // The same Page IR must render byte-identically — a golden PNG hash would be font-dependent
        // across machines, but non-determinism (unstable ordering, timestamps) is a real regression
        // this catches within a run.
        assert_eq!(
            render_page(&page()),
            render_page(&page()),
            "raster output must be deterministic for a fixed Page IR"
        );
    }

    #[test]
    fn render_pages_is_one_png_each() {
        let pages = vec![page(), page()];
        let pngs = render_pages(&pages);
        assert_eq!(pngs.len(), 2);
        for png in &pngs {
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        }
    }

    #[test]
    fn ellipse_fills_its_centre_not_its_corner() {
        use rpt_pages::EllipseOp;
        let mut p = Page::new(
            1,
            PageSize {
                width: Twips(3000),
                height: Twips(3000),
            },
        );
        p.push(DrawOp::Ellipse(EllipseOp {
            bounds: Rect {
                left: Twips(0),
                top: Twips(0),
                width: Twips(3000),
                height: Twips(3000),
            },
            fill: Some(
                Color {
                    a: 255,
                    r: 0,
                    g: 0,
                    b: 0,
                }
                .into(),
            ),
            stroke: None,
            source: None,
        }));
        let pm = render_page_pixmap(&p);
        // Centre pixel is inside the ellipse → drawn (black); the top-left corner is outside → white.
        let (cx, cy) = (pm.width() / 2, pm.height() / 2);
        let centre = pm.pixels()[(cy * pm.width() + cx) as usize];
        assert!(centre.red() < 128, "ellipse centre should be filled");
        let corner = pm.pixels()[0];
        assert!(
            corner.red() == 255 && corner.green() == 255 && corner.blue() == 255,
            "ellipse must not fill the bounding-box corner"
        );
    }

    #[test]
    fn origin_shifts_content_into_the_page() {
        // With a non-zero origin, the same 0-based op lands further down/right.
        let mut p = Page::new(
            1,
            PageSize {
                width: Twips(2880),
                height: Twips(2880),
            },
        );
        p.origin = Point::new(720, 720); // half-inch margin
        p.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(0),
                top: Twips(0),
                width: Twips(200),
                height: Twips(200),
            },
            fill: Some(
                Color {
                    a: 255,
                    r: 0,
                    g: 0,
                    b: 0,
                }
                .into(),
            ),
            stroke: None,
            corner_radius: Twips(0),
            source: None,
        }));
        let pm = render_page_pixmap(&p);
        let scale = DEFAULT_DPI / TWIPS_PER_INCH;
        // The op at twip 0 with a 720-twip origin paints around pixel 48 — the true (0,0) corner
        // stays white.
        let corner = pm.pixels()[0];
        assert!(corner.red() == 255 && corner.green() == 255 && corner.blue() == 255);
        let shifted_x = (760.0 * scale) as u32; // inside 720..920 twips
        let shifted_y = (760.0 * scale) as u32;
        let idx = (shifted_y * pm.width() + shifted_x) as usize;
        let px = pm.pixels()[idx];
        assert!(
            px.red() < 128,
            "expected the origin-shifted black rect here"
        );
    }

    /// Build a minimal 24-bit BI_RGB (bottom-up) BMP for `w`x`h`, each pixel BGR from `px(x, y)`.
    fn bmp_24(w: usize, h: usize, px: impl Fn(usize, usize) -> (u8, u8, u8)) -> Vec<u8> {
        let stride = (w * 3 + 3) & !3;
        let pixel_offset = 54usize;
        let mut data = vec![0u8; pixel_offset + stride * h];
        data[0..2].copy_from_slice(b"BM");
        data[10..14].copy_from_slice(&(pixel_offset as u32).to_le_bytes());
        data[14..18].copy_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER
        data[18..22].copy_from_slice(&(w as i32).to_le_bytes());
        data[22..26].copy_from_slice(&(h as i32).to_le_bytes()); // positive = bottom-up
        data[28..30].copy_from_slice(&24u16.to_le_bytes());
        for y in 0..h {
            let src_row = h - 1 - y; // bottom-up
            let base = pixel_offset + src_row * stride;
            for x in 0..w {
                let (b, g, r) = px(x, y);
                let p = base + x * 3;
                data[p] = b;
                data[p + 1] = g;
                data[p + 2] = r;
            }
        }
        data
    }

    fn image_page(id: &str, w: i32, h: i32) -> Page {
        image_page_fit(id, w, h, ImageFit::Fill)
    }

    fn image_page_fit(id: &str, w: i32, h: i32, fit: ImageFit) -> Page {
        let mut p = Page::new(
            1,
            PageSize {
                width: Twips(w),
                height: Twips(h),
            },
        );
        p.push(DrawOp::Image(ImageOp {
            bounds: Rect {
                left: Twips(0),
                top: Twips(0),
                width: Twips(w),
                height: Twips(h),
            },
            image_id: id.to_string(),
            fit,
            source: Some(ObjectRef::new("Details", ObjectKind::Image).named(id)),
        }));
        p
    }

    #[test]
    fn composites_bmp_pixels_scaled_into_the_box() {
        // A solid-red BMP must paint red across the placement box, not a grey placeholder outline.
        let bmp = bmp_24(4, 4, |_, _| (0, 0, 255)); // BGR red
        let mut assets = BTreeMap::new();
        assets.insert(
            "Pic".to_string(),
            ImageAsset {
                media_type: "image/bmp".to_string(),
                bytes: bmp,
            },
        );
        let pngs = render_pages_with_assets(&[image_page("Pic", 2880, 2880)], &assets, DEFAULT_DPI);
        assert_eq!(pngs.len(), 1);
        // Re-render to a pixmap for pixel inspection via the shared path.
        let mut cache = ImageCache::default();
        let pm = render_page_pixmap_into(
            &image_page("Pic", 2880, 2880),
            DEFAULT_DPI,
            &assets,
            &mut cache,
        );
        let (cx, cy) = (pm.width() / 2, pm.height() / 2);
        let centre = pm.pixels()[(cy * pm.width() + cx) as usize];
        assert!(
            centre.red() > 200 && centre.green() < 60 && centre.blue() < 60,
            "expected composited red image, got rgb ({},{},{})",
            centre.red(),
            centre.green(),
            centre.blue()
        );
    }

    #[test]
    fn contain_letterboxes_a_square_image_in_a_wide_box() {
        // A 1:1 blue image in a 2:1 box scales to fit the height and centers: the box's vertical
        // mid-line is blue at the center but white near the left/right edges (empty padding).
        let bmp = bmp_24(4, 4, |_, _| (0, 0, 255)); // BGR red
        let mut assets = BTreeMap::new();
        assets.insert(
            "Pic".to_string(),
            ImageAsset {
                media_type: "image/bmp".to_string(),
                bytes: bmp,
            },
        );
        let mut cache = ImageCache::default();
        let pm = render_page_pixmap_into(
            &image_page_fit("Pic", 2880, 1440, ImageFit::Contain),
            DEFAULT_DPI,
            &assets,
            &mut cache,
        );
        let cy = pm.height() / 2;
        let px = |x: u32| pm.pixels()[(cy * pm.width() + x) as usize];
        let centre = px(pm.width() / 2);
        assert!(
            centre.red() > 200 && centre.green() < 60 && centre.blue() < 60,
            "centre should be the composited image"
        );
        let left = px(2);
        let right = px(pm.width() - 3);
        assert!(
            left.red() == 255 && left.green() == 255 && left.blue() == 255,
            "left edge should be empty (letterbox padding)"
        );
        assert!(
            right.red() == 255 && right.green() == 255 && right.blue() == 255,
            "right edge should be empty (letterbox padding)"
        );
    }

    #[test]
    fn missing_asset_draws_placeholder_without_panicking() {
        // No assets for the op → placeholder outline; the box interior stays white.
        let pm = render_page_pixmap(&image_page("Pic", 2880, 2880));
        let (cx, cy) = (pm.width() / 2, pm.height() / 2);
        let centre = pm.pixels()[(cy * pm.width() + cx) as usize];
        assert!(
            centre.red() == 255 && centre.green() == 255 && centre.blue() == 255,
            "placeholder must not fill the box interior"
        );
    }

    #[test]
    fn identical_bytes_decode_once() {
        // Two ids backed by identical bytes share one cache entry (decoded once for the document).
        let bmp = bmp_24(2, 2, |_, _| (0, 0, 255));
        let mut assets = BTreeMap::new();
        for id in ["A", "B"] {
            assets.insert(
                id.to_string(),
                ImageAsset {
                    media_type: "image/bmp".to_string(),
                    bytes: bmp.clone(),
                },
            );
        }
        let mut cache = ImageCache::default();
        let _ = render_page_pixmap_into(
            &image_page("A", 2880, 2880),
            DEFAULT_DPI,
            &assets,
            &mut cache,
        );
        let _ = render_page_pixmap_into(
            &image_page("B", 2880, 2880),
            DEFAULT_DPI,
            &assets,
            &mut cache,
        );
        assert_eq!(cache.len(), 1, "identical bytes → a single decoded entry");
    }

    // RED_JPEG_8X8: 632 bytes (8x8 solid rgb(220,30,40), no chroma subsampling)
    const RED_JPEG_8X8: &[u8] = &[
        0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00,
        0x01, 0x00, 0x01, 0x00, 0x00, 0xff, 0xdb, 0x00, 0x43, 0x00, 0x03, 0x02, 0x02, 0x03, 0x02,
        0x02, 0x03, 0x03, 0x03, 0x03, 0x04, 0x03, 0x03, 0x04, 0x05, 0x08, 0x05, 0x05, 0x04, 0x04,
        0x05, 0x0a, 0x07, 0x07, 0x06, 0x08, 0x0c, 0x0a, 0x0c, 0x0c, 0x0b, 0x0a, 0x0b, 0x0b, 0x0d,
        0x0e, 0x12, 0x10, 0x0d, 0x0e, 0x11, 0x0e, 0x0b, 0x0b, 0x10, 0x16, 0x10, 0x11, 0x13, 0x14,
        0x15, 0x15, 0x15, 0x0c, 0x0f, 0x17, 0x18, 0x16, 0x14, 0x18, 0x12, 0x14, 0x15, 0x14, 0xff,
        0xdb, 0x00, 0x43, 0x01, 0x03, 0x04, 0x04, 0x05, 0x04, 0x05, 0x09, 0x05, 0x05, 0x09, 0x14,
        0x0d, 0x0b, 0x0d, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14,
        0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14,
        0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14,
        0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x08,
        0x00, 0x08, 0x03, 0x01, 0x11, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01, 0xff, 0xc4, 0x00,
        0x1f, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
        0xff, 0xc4, 0x00, 0xb5, 0x10, 0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05, 0x05,
        0x04, 0x04, 0x00, 0x00, 0x01, 0x7d, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21,
        0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08,
        0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a,
        0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x34, 0x35, 0x36, 0x37,
        0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56,
        0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75,
        0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93,
        0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9,
        0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6,
        0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
        0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7,
        0xf8, 0xf9, 0xfa, 0xff, 0xc4, 0x00, 0x1f, 0x01, 0x00, 0x03, 0x01, 0x01, 0x01, 0x01, 0x01,
        0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05,
        0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0xff, 0xc4, 0x00, 0xb5, 0x11, 0x00, 0x02, 0x01, 0x02,
        0x04, 0x04, 0x03, 0x04, 0x07, 0x05, 0x04, 0x04, 0x00, 0x01, 0x02, 0x77, 0x00, 0x01, 0x02,
        0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13, 0x22,
        0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0, 0x15,
        0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26,
        0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47,
        0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66,
        0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84,
        0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a,
        0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7,
        0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4,
        0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea,
        0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xff, 0xda, 0x00, 0x0c, 0x03, 0x01,
        0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3f, 0x00, 0xf1, 0x4a, 0xfc, 0xdc, 0xfe, 0xfb, 0x3f,
        0xff, 0xd9,
    ];

    // RED_GIF_4X4: 48 bytes, 4x4 solid, palette index 0 = rgb(220,30,40)
    const RED_GIF_4X4: &[u8] = &[
        0x47, 0x49, 0x46, 0x38, 0x37, 0x61, 0x04, 0x00, 0x04, 0x00, 0x81, 0x00, 0x00, 0xdc, 0x1e,
        0x28, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00,
        0x04, 0x00, 0x04, 0x00, 0x00, 0x08, 0x09, 0x00, 0x01, 0x08, 0x1c, 0x48, 0xb0, 0x20, 0x80,
        0x80, 0x00, 0x3b,
    ];

    #[test]
    fn decodes_jpeg_to_rgba() {
        let (rgba, w, h) = decode_jpeg_rgba(RED_JPEG_8X8).expect("valid JPEG decodes");
        assert_eq!((w, h), (8, 8));
        assert_eq!(rgba.len(), 8 * 8 * 4);
        // Lossy, so allow a wide tolerance: the interior pixel is reddish and opaque.
        let c = &rgba[(8 * 4 + 4) * 4..][..4]; // pixel (4,4)
        assert!(c[0] > 180 && c[1] < 80 && c[2] < 90, "reddish, got {c:?}");
        assert_eq!(c[3], 255, "JPEG decodes to opaque alpha");
    }

    #[test]
    fn rejects_truncated_jpeg() {
        assert!(decode_jpeg_rgba(&RED_JPEG_8X8[..20]).is_none());
    }

    #[test]
    fn decodes_gif_first_frame_to_rgba() {
        let (rgba, w, h) = decode_gif_rgba(RED_GIF_4X4).expect("valid GIF decodes");
        assert_eq!((w, h), (4, 4));
        assert_eq!(rgba.len(), 4 * 4 * 4);
        // GIF is lossless — the palette colour comes back exactly.
        assert_eq!(&rgba[0..4], &[220, 30, 40, 255]);
    }

    #[test]
    fn rejects_truncated_gif() {
        assert!(decode_gif_rgba(&RED_GIF_4X4[..8]).is_none());
    }

    #[test]
    fn composites_jpeg_and_gif_into_the_box() {
        // Both formats must paint their (reddish) pixels across the box, not the grey placeholder.
        for (media, bytes) in [
            ("image/jpeg", RED_JPEG_8X8.to_vec()),
            ("image/gif", RED_GIF_4X4.to_vec()),
        ] {
            let mut assets = BTreeMap::new();
            assets.insert(
                "Pic".to_string(),
                ImageAsset {
                    media_type: media.to_string(),
                    bytes,
                },
            );
            let mut cache = ImageCache::default();
            let pm = render_page_pixmap_into(
                &image_page("Pic", 2880, 2880),
                DEFAULT_DPI,
                &assets,
                &mut cache,
            );
            let (cx, cy) = (pm.width() / 2, pm.height() / 2);
            let centre = pm.pixels()[(cy * pm.width() + cx) as usize];
            assert!(
                centre.red() > 180 && centre.green() < 90 && centre.blue() < 100,
                "{media}: expected composited red image, got rgb ({},{},{})",
                centre.red(),
                centre.green(),
                centre.blue()
            );
        }
    }

    /// A deterministic, font-free page: a filled+stroked rect, a dashed line, and an embedded BMP.
    /// It draws no text, so its rasterization is independent of the host font stack.
    fn golden_page() -> (Page, BTreeMap<String, ImageAsset>) {
        let mut p = Page::new(
            1,
            PageSize {
                width: Twips(1600),
                height: Twips(1200),
            },
        );
        p.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(100),
                top: Twips(100),
                width: Twips(900),
                height: Twips(500),
            },
            fill: Some(
                Color {
                    a: 255,
                    r: 40,
                    g: 120,
                    b: 200,
                }
                .into(),
            ),
            stroke: Some(Stroke {
                color: Color {
                    a: 255,
                    r: 0,
                    g: 0,
                    b: 0,
                },
                width: Twips(30),
                style: LineStyle::Single,
            }),
            corner_radius: Twips(0),
            source: None,
        }));
        p.push(DrawOp::Line(LineOp {
            from: Point::new(100, 700),
            to: Point::new(1500, 700),
            stroke: Stroke {
                color: Color {
                    a: 255,
                    r: 200,
                    g: 40,
                    b: 40,
                },
                width: Twips(20),
                style: LineStyle::Dashed,
            },
            source: None,
        }));
        // A 4×4 gradient BMP, composited near the bottom.
        let bmp = bmp_24(4, 4, |x, _| ((x * 60) as u8, 200, 0));
        p.push(DrawOp::Image(ImageOp {
            bounds: Rect {
                left: Twips(100),
                top: Twips(800),
                width: Twips(500),
                height: Twips(300),
            },
            image_id: "Pic".to_string(),
            fit: ImageFit::Fill,
            source: None,
        }));
        let mut assets = BTreeMap::new();
        assets.insert(
            "Pic".to_string(),
            ImageAsset {
                media_type: "image/bmp".to_string(),
                bytes: bmp,
            },
        );
        (p, assets)
    }

    #[test]
    fn golden_font_free_page_pixels() {
        // Freeze the composited pixels of a font-free page (rect + line + image) so a rect/line/image
        // geometry regression is caught. The golden is the raw pixmap buffer, not the PNG container:
        // tiny-skia's PNG encoder is not byte-reproducible across process/load contexts (identical
        // pixels compress to different bytes), whereas the rasterized pixels are fully deterministic.
        let (page, assets) = golden_page();
        let mut cache = ImageCache::default();
        let pixmap = render_page_pixmap_into(&page, DEFAULT_DPI, &assets, &mut cache);
        rpt_test_support::assert_golden_bytes(
            env!("CARGO_MANIFEST_DIR"),
            "page.rgba",
            pixmap.data(),
        );
    }
}
