//! The krilla PDF backend (default): real embedded fonts, subset `/Widths`, `/FlateDecode` streams.
//!
//! Drives [krilla] so text is drawn with real TrueType/CFF font subset embedding and content streams
//! are compressed. Its surface is y-down with a top-left origin — the same convention as the Page IR —
//! so there is no y-flip here (the basic writer flips y for raw PDF's bottom-left origin).
//!
//! [krilla]: https://docs.rs/krilla

use crate::common::{
    aligned_x, approx_text_width, baseline_offset_twips, pt, solid_of, KAPPA, TWIPS_PER_PT,
};
use rpt_model::Color;
use rpt_pages::{DrawOp, EllipseOp, FontSpec, LineOp, Page, PolygonOp, RectOp, TextRun};
use rpt_text::FontDb;
use std::collections::HashMap;

use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Transform};
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, Stroke};
use krilla::surface::Surface;
use krilla::text::{Font, TextDirection};
use krilla::Document;

/// Render `pages` to PDF bytes via krilla. Returns `None` if serialization fails, so the caller can
/// fall back to the basic writer (a valid PDF is better than none).
pub fn render(pages: &[Page]) -> Option<Vec<u8>> {
    let mut fonts = FontCache::new();
    let mut document = Document::new();

    // krilla always emits at least one page; mirror the basic writer's "≥1 page" contract by
    // letting an empty slice produce a single blank Letter page.
    if pages.is_empty() {
        document.start_page_with(PageSettings::from_wh(612.0, 792.0)?);
    }

    for page in pages {
        let w = pt(page.size.width.0) as f32;
        let h = pt(page.size.height.0) as f32;
        let settings = PageSettings::from_wh(w.max(1.0), h.max(1.0))?;
        let mut kpage = document.start_page_with(settings);
        let mut surface = kpage.surface();

        // Draw-op coordinates are printable-relative (0-based); translate by the page origin (the
        // report margin) so content sits inside the physical margins. krilla's surface is y-down
        // with a top-left origin — the same convention as the Page IR — so there is no y-flip here.
        let (ox, oy) = (pt(page.origin.x.0) as f32, pt(page.origin.y.0) as f32);
        let shifted = ox != 0.0 || oy != 0.0;
        if shifted {
            surface.push_transform(&Transform::from_translate(ox, oy));
        }

        for op in &page.ops {
            match op {
                DrawOp::Rect(r) => draw_rect(&mut surface, r),
                DrawOp::Ellipse(e) => draw_ellipse(&mut surface, e),
                DrawOp::Line(l) => draw_line(&mut surface, l),
                DrawOp::Polygon(p) => draw_polygon(&mut surface, p),
                DrawOp::Text(t) => draw_text(&mut surface, &mut fonts, t),
                DrawOp::Image(_) => {} // image bytes are not threaded through this entry point
            }
        }

        if shifted {
            surface.pop();
        }
        surface.finish();
        kpage.finish();
    }

    document.finish().ok()
}

/// Convert an `rpt` colour to a krilla RGB paint plus a normalized alpha for opacity.
fn paint(color: Color) -> (rgb::Color, NormalizedF32) {
    let opacity = NormalizedF32::new(color.a as f32 / 255.0).unwrap_or(NormalizedF32::ONE);
    (rgb::Color::new(color.r, color.g, color.b), opacity)
}

fn fill_for(color: Color) -> Fill {
    let (rgb, opacity) = paint(color);
    Fill {
        paint: rgb.into(),
        opacity,
        ..Fill::default()
    }
}

/// A krilla fill for a Page-IR fill: solid renders exactly, gradient/hatch fall back to the
/// fill's representative solid colour (see [`solid_of`]).
fn fill_of(fill: &rpt_pages::Fill) -> Fill {
    fill_for(solid_of(fill))
}

fn stroke_for(color: Color, width_twips: i32) -> Stroke {
    let (rgb, opacity) = paint(color);
    Stroke {
        paint: rgb.into(),
        // A stored width of 0 (hairline) still needs to render; clamp to a thin visible line.
        width: (pt(width_twips) as f32).max(0.25),
        opacity,
        ..Stroke::default()
    }
}

fn draw_rect(surface: &mut Surface, r: &RectOp) {
    if r.fill.is_none() && r.stroke.is_none() {
        return; // nothing to paint (the basic writer emits a no-op `n` here)
    }
    let x = pt(r.bounds.left.0) as f32;
    let y = pt(r.bounds.top.0) as f32;
    let w = pt(r.bounds.width.0) as f32;
    let ht = pt(r.bounds.height.0) as f32;
    let mut pb = PathBuilder::new();
    pb.move_to(x, y);
    pb.line_to(x + w, y);
    pb.line_to(x + w, y + ht);
    pb.line_to(x, y + ht);
    pb.close();
    let Some(path) = pb.finish() else {
        return;
    };
    // `draw_path` fills and/or strokes according to the surface's current fill/stroke state, so set
    // exactly the ones this rect has (and clear the other) before drawing.
    surface.set_fill(r.fill.as_ref().map(fill_of));
    surface.set_stroke(r.stroke.map(|s| stroke_for(s.color, s.width.0)));
    surface.draw_path(&path);
}

/// An axis-aligned ellipse inscribed in the op's bounds, built from four cubic-Bézier quarter
/// arcs (krilla has no native ellipse primitive).
fn draw_ellipse(surface: &mut Surface, e: &EllipseOp) {
    if e.bounds.width.0 <= 0 || e.bounds.height.0 <= 0 {
        return;
    }
    if e.fill.is_none() && e.stroke.is_none() {
        return;
    }
    let cx = pt(e.bounds.left.0) as f32 + pt(e.bounds.width.0) as f32 / 2.0;
    let cy = pt(e.bounds.top.0) as f32 + pt(e.bounds.height.0) as f32 / 2.0;
    let rx = pt(e.bounds.width.0) as f32 / 2.0;
    let ry = pt(e.bounds.height.0) as f32 / 2.0;
    let k = KAPPA as f32;
    let (kx, ky) = (rx * k, ry * k);
    let mut pb = PathBuilder::new();
    pb.move_to(cx + rx, cy);
    pb.cubic_to(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry);
    pb.cubic_to(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy);
    pb.cubic_to(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry);
    pb.cubic_to(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy);
    pb.close();
    let Some(path) = pb.finish() else {
        return;
    };
    surface.set_fill(e.fill.as_ref().map(fill_of));
    surface.set_stroke(e.stroke.map(|s| stroke_for(s.color, s.width.0)));
    surface.draw_path(&path);
}

fn draw_line(surface: &mut Surface, l: &LineOp) {
    let mut pb = PathBuilder::new();
    pb.move_to(pt(l.from.x.0) as f32, pt(l.from.y.0) as f32);
    pb.line_to(pt(l.to.x.0) as f32, pt(l.to.y.0) as f32);
    let Some(path) = pb.finish() else {
        return;
    };
    surface.set_fill(None);
    surface.set_stroke(Some(stroke_for(l.stroke.color, l.stroke.width.0)));
    surface.draw_path(&path);
}

fn draw_polygon(surface: &mut Surface, p: &PolygonOp) {
    if p.points.len() < 2 {
        return;
    }
    let mut pb = PathBuilder::new();
    pb.move_to(pt(p.points[0].x.0) as f32, pt(p.points[0].y.0) as f32);
    for pt_ in &p.points[1..] {
        pb.line_to(pt(pt_.x.0) as f32, pt(pt_.y.0) as f32);
    }
    if p.closed {
        pb.close();
    }
    let Some(path) = pb.finish() else {
        return;
    };
    // Only a closed region fills; an open polyline just strokes.
    surface.set_fill(if p.closed {
        p.fill.as_ref().map(fill_of)
    } else {
        None
    });
    surface.set_stroke(p.stroke.map(|s| stroke_for(s.color, s.width.0)));
    surface.draw_path(&path);
}

fn draw_text(surface: &mut Surface, fonts: &mut FontCache, t: &TextRun) {
    if t.text.is_empty() {
        return;
    }
    let Some(font) = fonts.resolve(&t.font) else {
        return; // no font available on this host — skip rather than emit nothing-glyphs
    };
    let size = t.font.size_pt.max(1.0);
    // Baseline = top + ascent (krilla places the run at its baseline; y-down, no flip). The
    // metrics-present ascent is the shared `baseline_offset_twips`, scaled from twips to points;
    // the no-metrics fallback stays in the surface's point/f32 space (its rounding differs from the
    // twips heuristic, and krilla output must be byte-stable), so only that arm is kept local.
    let ascent_pt = match &t.metrics {
        Some(_) => (baseline_offset_twips(t) / TWIPS_PER_PT) as f32,
        None => size * 0.8,
    };
    let baseline_y = pt(t.bounds.top.0) as f32 + ascent_pt;
    // Horizontal alignment: krilla's `draw_text` does not measure, so shift x by the run's stored
    // advance for centre/right (else the approximate width). Shared anchor math with the basic
    // writer; only the point conversion differs.
    let text_w = match &t.metrics {
        Some(m) => pt(m.advance.0),
        None => approx_text_width(&t.text, size as f64),
    };
    let x = aligned_x(t.align, pt(t.bounds.left.0), pt(t.bounds.width.0), text_w) as f32;
    surface.set_fill(Some(fill_for(t.color)));
    // Rotation is CCW degrees about the run's origin (top-left of bounds). krilla's surface is
    // y-down (like `from_rotate_at`'s CW-positive angle), so negate to render CCW. `0.0` pushes no
    // transform, keeping upright output identical.
    let rotated = t.rotation != 0.0;
    if rotated {
        let (px, py) = (pt(t.bounds.left.0) as f32, pt(t.bounds.top.0) as f32);
        surface.push_transform(&Transform::from_rotate_at(-t.rotation, px, py));
    }
    surface.draw_text(
        Point::from_xy(x, baseline_y),
        font,
        size,
        &t.text,
        false,
        TextDirection::Auto,
    );
    if rotated {
        surface.pop();
    }
}

/// Resolves host fonts via the shared [`rpt_text::FontDb`] and memoizes the loaded krilla
/// [`Font`]s so a face is read and subset once per `(family, bold, italic)` combination — this
/// backend keeps only the krilla parse step; the resolution policy lives in `FontDb`.
struct FontCache {
    db: FontDb,
    /// `None` value = we looked and found no usable face for this key (don't re-query).
    cache: HashMap<(String, bool, bool), Option<Font>>,
    /// The host's first available face, used when a requested family can't be matched at all.
    fallback: Option<Font>,
    fallback_loaded: bool,
}

impl FontCache {
    fn new() -> FontCache {
        FontCache {
            db: FontDb::with_system_fonts(),
            cache: HashMap::new(),
            fallback: None,
            fallback_loaded: false,
        }
    }

    fn resolve(&mut self, spec: &FontSpec) -> Option<Font> {
        let key = (spec.family.clone(), spec.bold, spec.italic);
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }
        let font = self
            .db
            .query(spec)
            .and_then(|id| self.load_face(id))
            .or_else(|| self.fallback_font());
        self.cache.insert(key, font.clone());
        font
    }

    /// The first face in the host's font DB, loaded once — a last resort when no family matched.
    fn fallback_font(&mut self) -> Option<Font> {
        if !self.fallback_loaded {
            self.fallback_loaded = true;
            self.fallback = self.db.first_face().and_then(|id| self.load_face(id));
        }
        self.fallback.clone()
    }

    /// Read a resolved face's bytes and build a krilla [`Font`] (subset-embedded on write).
    fn load_face(&self, id: fontdb::ID) -> Option<Font> {
        self.db
            .with_face_data(id, |data, index| Font::new(data.to_vec().into(), index))
            .flatten()
    }
}
