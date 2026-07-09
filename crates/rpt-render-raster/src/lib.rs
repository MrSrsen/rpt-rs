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
//! [`ImageOp`]s carry only an `image_id` (bytes live out-of-band); this backend draws a light-grey
//! placeholder outline so the layout is still visible. Embedding decoded picture bytes is a later
//! refinement (the HTML backend already inlines them).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rpt_model::Color;
use rpt_pages::{
    DrawOp, EllipseOp, Fill, ImageOp, LineOp, Page, PolygonOp, RectOp, Stroke, TextAlign, TextRun,
};
use rpt_text::FontDb;
use tiny_skia::{
    FillRule, Paint, PathBuilder, Pixmap, PremultipliedColorU8, Rect as SkRect, Shader,
    Stroke as SkStroke, StrokeDash, Transform,
};

/// Default output resolution. 96 DPI matches the HTML backend's `TWIPS_PER_PX = 15` (1440/96 = 15).
pub const DEFAULT_DPI: f32 = 96.0;

/// Twips per inch (the Page IR unit; 1 twip = 1/1440 inch). Exact in `f32`.
const TWIPS_PER_INCH: f32 = rpt_render_util::TWIPS_PER_INCH as f32;
/// Typographic points per inch (font sizes are in points). Exact in `f32`.
const POINTS_PER_INCH: f32 = rpt_render_util::POINTS_PER_INCH as f32;

/// Render one [`Page`] to PNG bytes at [`DEFAULT_DPI`].
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
            DrawOp::Image(i) => draw_image(&mut pixmap, &ctx, i),
        }
    }
    pixmap
}

/// Render every [`Page`] to its own PNG (one per page, mirroring the SVG backend's per-page output).
pub fn render_pages(pages: &[Page]) -> Vec<Vec<u8>> {
    pages.iter().map(render_page).collect()
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
        doc.pages
            .iter()
            .map(|p| render_page_dpi(p, opts.dpi))
            .collect()
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

/// Draw a placeholder outline for an image (bytes live out-of-band; see the module docs).
fn draw_image(pixmap: &mut Pixmap, ctx: &Ctx, i: &ImageOp) {
    let (x, y) = (ctx.x(i.bounds.left.0), ctx.y(i.bounds.top.0));
    let (w, h) = (ctx.len(i.bounds.width.0), ctx.len(i.bounds.height.0));
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

fn draw_text(pixmap: &mut Pixmap, ctx: &Ctx, fonts: &Fonts, dpi: f32, t: &TextRun) {
    if t.text.is_empty() {
        return;
    }
    let Some(font) = fonts.resolve(&t.font) else {
        return; // no resolvable face — nothing to draw (a warning path could log this)
    };
    // Point size → device pixels at this DPI (1pt = 1/72 inch).
    let px = (t.font.size_pt * dpi / POINTS_PER_INCH).max(1.0);

    // Rasterize each char once; keep the per-glyph advance for pen flow.
    let glyphs: Vec<(fontdue::Metrics, Vec<u8>)> =
        t.text.chars().map(|c| font.rasterize(c, px)).collect();
    // Alignment anchor: the run's stored advance when the layout engine measured it (device px via
    // the twip scale), else the sum of fontdue advances.
    let text_w: f32 = match &t.metrics {
        Some(m) => ctx.len(m.advance.0),
        None => glyphs.iter().map(|(m, _)| m.advance_width).sum(),
    };

    let (bx, bw) = (ctx.x(t.bounds.left.0), ctx.len(t.bounds.width.0));
    let mut pen_x = match t.align {
        TextAlign::Left | TextAlign::Justified => bx,
        TextAlign::Center => bx + (bw - text_w) / 2.0,
        TextAlign::Right => bx + bw - text_w,
    };
    // Baseline: the run's top edge plus the ascent (device px). Use the run's stored ascent when
    // present (via the twip scale), else the resolved face's fontdue ascent.
    let ascent = match &t.metrics {
        Some(m) => ctx.len(m.ascent.0),
        None => font
            .horizontal_line_metrics(px)
            .map(|m| m.ascent)
            .unwrap_or(px * 0.8),
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

    for (m, cov) in &glyphs {
        // fontdue coverage is row-major, y-down; `ymin` is the bitmap's bottom offset above the
        // baseline, so its top edge sits `ymin + height` above the baseline.
        let gx = pen_x + m.xmin as f32;
        let gy = baseline - (m.ymin as f32 + m.height as f32);
        match &rot {
            None => blit_coverage(pixmap, cov, m.width, m.height, gx, gy, t.color),
            Some(rot) => blit_coverage_rot(pixmap, cov, m.width, m.height, gx, gy, t.color, rot),
        }
        pen_x += m.advance_width;
    }

    // Underline / strikethrough as thin filled bars across the drawn extent. The axis-aligned bars
    // are only meaningful for upright text; a rotated run omits them (best-effort).
    if rot.is_some() {
        return;
    }
    let x0 = match t.align {
        TextAlign::Left | TextAlign::Justified => bx,
        TextAlign::Center => bx + (bw - text_w) / 2.0,
        TextAlign::Right => bx + bw - text_w,
    };
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
        let id = self.db.query(spec)?;
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
    use rpt_pages::{FontSpec, LineOp, LineStyle, Point, RectOp, Stroke, TextRun};
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
}
