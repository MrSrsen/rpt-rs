//! The krilla PDF backend (default): real embedded fonts, subset `/Widths`, `/FlateDecode` streams.
//!
//! Drives [krilla] so text is drawn with real TrueType/CFF font subset embedding and content streams
//! are compressed. Its surface is y-down with a top-left origin — the same convention as the Page IR —
//! so there is no y-flip here (the basic writer flips y for raw PDF's bottom-left origin).
//!
//! [krilla]: https://docs.rs/krilla

use crate::common::{
    aligned_x, approx_text_width, baseline_offset_twips, pt, solid_of, KAPPA, MIN_STROKE_PT,
    TWIPS_PER_PT,
};
use rpt_model::Color;
use rpt_pages::{
    DrawOp, EllipseOp, ImageAsset, ImageFit, ImageOp, LineOp, Page, PolygonOp, RectOp, TextRun,
};
use rpt_text::FontDb;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Size, Transform};
use krilla::image::Image;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, Stroke};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph, TextDirection};
use krilla::Document;

/// Render `pages` to PDF bytes via krilla, embedding each [`DrawOp::Image`] whose `image_id` resolves
/// in `assets`. Returns `None` if serialization fails, so the caller can fall back to the basic writer
/// (a valid PDF is better than none).
pub fn render(pages: &[Page], assets: &BTreeMap<String, ImageAsset>) -> Option<Vec<u8>> {
    let mut fonts = FontCache::new();
    // Decode each distinct image once, keyed by a content hash of its bytes: identical bytes (the
    // per-page logo, duplicate thumbnails) then share one krilla image handle, so krilla emits a
    // single XObject referenced many times instead of duplicating it per placement.
    let mut images: HashMap<u64, Option<Image>> = HashMap::new();
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
                DrawOp::Image(i) => draw_image(&mut surface, assets, &mut images, i),
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
        width: (pt(width_twips) as f32).max(MIN_STROKE_PT as f32),
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
    let radius = (pt(r.corner_radius.0) as f32).clamp(0.0, (w / 2.0).min(ht / 2.0));
    let mut pb = PathBuilder::new();
    if radius > 0.0 {
        // A rounded rect: straight edges joined by four cubic-Bézier quarter arcs (kappa control
        // offset), matching the ellipse's corner construction.
        let k = radius * KAPPA as f32;
        let (l, t, rr, b) = (x, y, x + w, y + ht);
        pb.move_to(l + radius, t);
        pb.line_to(rr - radius, t);
        pb.cubic_to(rr - radius + k, t, rr, t + radius - k, rr, t + radius);
        pb.line_to(rr, b - radius);
        pb.cubic_to(rr, b - radius + k, rr - radius + k, b, rr - radius, b);
        pb.line_to(l + radius, b);
        pb.cubic_to(l + radius - k, b, l, b - radius + k, l, b - radius);
        pb.line_to(l, t + radius);
        pb.cubic_to(l, t + radius - k, l + radius - k, t, l + radius, t);
    } else {
        pb.move_to(x, y);
        pb.line_to(x + w, y);
        pb.line_to(x + w, y + ht);
        pb.line_to(x, y + ht);
    }
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

/// Draw an image op: look up its bytes in `assets`, decode to a krilla image, and paint it filling
/// the op's box. An unresolved id or an undecodable/unsupported raster is skipped (the box's own
/// border, if any, still shows) — matching the other backends' "no bytes, no picture" behaviour.
fn draw_image(
    surface: &mut Surface,
    assets: &BTreeMap<String, ImageAsset>,
    images: &mut HashMap<u64, Option<Image>>,
    i: &ImageOp,
) {
    let Some(asset) = assets.get(&i.image_id) else {
        return;
    };
    // Decode once per distinct byte string; a repeated image reuses the cached krilla handle.
    let image = images
        .entry(rpt_render_util::content_hash(&asset.bytes))
        .or_insert_with(|| load_image(asset))
        .clone();
    let Some(image) = image else {
        return;
    };
    let (bw, bh) = (pt(i.bounds.width.0) as f32, pt(i.bounds.height.0) as f32);
    // `Fill` draws the raster to the whole box (distorting aspect); `Contain` scales it uniformly to
    // the largest fit and centers it, leaving the surrounding space empty — Crystal letterboxes.
    let (dw, dh, ox, oy) = match i.fit {
        ImageFit::Fill => (bw, bh, 0.0, 0.0),
        ImageFit::Contain => {
            let (iw, ih) = image.size();
            let (iw, ih) = (iw as f32, ih as f32);
            if iw <= 0.0 || ih <= 0.0 {
                (bw, bh, 0.0, 0.0)
            } else {
                let s = (bw / iw).min(bh / ih);
                let (dw, dh) = (iw * s, ih * s);
                (dw, dh, (bw - dw) / 2.0, (bh - dh) / 2.0)
            }
        }
    };
    let Some(size) = Size::from_wh(dw, dh) else {
        return;
    };
    let (x, y) = (pt(i.bounds.left.0) as f32, pt(i.bounds.top.0) as f32);
    surface.push_transform(&Transform::from_translate(x + ox, y + oy));
    surface.draw_image(image, size);
    surface.pop();
}

/// Decode an image asset into a krilla [`Image`]. PNG/JPEG/GIF go through krilla's decoders; a BMP
/// (the format Crystal stores embedded OLE bitmaps as) is decoded here to RGBA and embedded raw,
/// since krilla has no BMP path. `None` for anything else or a decode failure.
fn load_image(asset: &ImageAsset) -> Option<Image> {
    match asset.media_type.as_str() {
        "image/png" => Image::from_png(asset.bytes.clone().into(), true).ok(),
        "image/jpeg" => Image::from_jpeg(asset.bytes.clone().into(), true).ok(),
        "image/gif" => Image::from_gif(asset.bytes.clone().into(), true).ok(),
        "image/bmp" => {
            let (rgba, w, h) = rpt_render_util::decode_bmp_rgba(&asset.bytes)?;
            Some(Image::from_rgba8(rgba, w, h))
        }
        _ => None,
    }
}

/// Draw a justified line word by word, flushing both edges: each word is shaped and placed at a
/// running pen, and every inter-word gap advances by the space width plus `extra`.
fn draw_justified_words(
    surface: &mut Surface,
    face: &LoadedFace,
    size: f32,
    text: &str,
    x: f32,
    baseline_y: f32,
    extra: f32,
) {
    let space_w = shape_run(" ", face, size).1;
    let mut pen = x;
    for (wi, word) in text.split(' ').enumerate() {
        if wi > 0 {
            pen += space_w + extra;
        }
        if !word.is_empty() {
            surface.draw_text(
                Point::from_xy(pen, baseline_y),
                face.font.clone(),
                size,
                word,
                false,
                TextDirection::Auto,
            );
            pen += shape_run(word, face, size).1;
        }
    }
}

fn draw_text(surface: &mut Surface, fonts: &mut FontCache, t: &TextRun) {
    if t.text.is_empty() {
        return;
    }
    // Split the run into maximal single-face segments: the primary family where it has the glyph,
    // else the bundled symbol fallback for the chars it lacks (⚠ etc.) — so a missing glyph renders
    // from the fallback face instead of a `.notdef` box. Segments carry byte ranges into `t.text`.
    let segments = fonts.db.segment_by_coverage(&t.font, &t.text);
    if segments.is_empty() {
        return; // no usable face on this host — skip rather than emit nothing-glyphs
    }
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
    // Horizontal alignment: shift x by the run's stored advance for centre/right (else the
    // approximate width). Shared anchor math with the basic writer; only the point conversion
    // differs. Segments then flow left-to-right from this anchor by their own shaped advances.
    let text_w = match &t.metrics {
        Some(m) => pt(m.advance.0),
        None => approx_text_width(&t.text, size as f64),
    };
    let x = aligned_x(t.align, pt(t.bounds.left.0), pt(t.bounds.width.0), text_w) as f32;
    // Text is fill-only. Clear any stroke left active by a preceding path op (a rule/line/rect) so
    // krilla does not fill *and* stroke the glyphs — a leaked stroke renders the run doubled/haloed.
    surface.set_fill(Some(fill_for(t.color)));
    surface.set_stroke(None);
    // Rotation is CCW degrees about the run's origin (top-left of bounds). krilla's surface is
    // y-down (like `from_rotate_at`'s CW-positive angle), so negate to render CCW. `0.0` pushes no
    // transform, keeping upright output identical.
    let rotated = t.rotation != 0.0;
    if rotated {
        let (px, py) = (pt(t.bounds.left.0) as f32, pt(t.bounds.top.0) as f32);
        surface.push_transform(&Transform::from_rotate_at(-t.rotation, px, py));
    }
    // Draw each segment with its own face at a running pen, all sharing the one baseline. Shaping
    // each single-face segment with krilla's shaper (rustybuzz) yields the same glyph positions
    // krilla's own `draw_text` would; the pen advances by the segment's shaped width so text after a
    // fallback glyph keeps its place.
    // Common case — one non-substituted segment spanning the whole run (any pure-primary-face text,
    // ASCII or not): let krilla shape and draw it in one call. This is the pre-fallback path and its
    // cheapest; the per-segment glyph path below is reserved for genuinely mixed-face runs.
    if let [seg] = segments.as_slice() {
        if !seg.substituted {
            if let Some(face) = fonts.face(seg.face) {
                // Justified: spread the box's slack across the inter-word gaps so both edges flush.
                // The layout marks a paragraph's last line `Left`, so only interior wrapped lines
                // reach this. Falls back to one shaped draw when there is no slack/gap.
                let extra = rpt_render_util::justify_gap_extra(
                    t.align,
                    &t.text,
                    pt(t.bounds.width.0),
                    text_w,
                ) as f32;
                if extra > 0.0 {
                    draw_justified_words(surface, &face, size, &t.text, x, baseline_y, extra);
                } else {
                    surface.draw_text(
                        Point::from_xy(x, baseline_y),
                        face.font.clone(),
                        size,
                        &t.text,
                        false,
                        TextDirection::Auto,
                    );
                }
            }
            draw_text_decoration(surface, t, x, baseline_y, ascent_pt, size, text_w);
            if rotated {
                surface.pop();
            }
            return;
        }
    }
    // Mixed-face run: draw each segment with its own face at a running pen, all sharing the one
    // baseline. Shaping each single-face segment with krilla's shaper (rustybuzz) yields the same
    // glyph positions krilla's own `draw_text` would; the pen advances by the segment's shaped width
    // so text after a fallback glyph keeps its place.
    let mut pen_x = x;
    for seg in &segments {
        let Some(face) = fonts.face(seg.face) else {
            continue;
        };
        let seg_text = &t.text[seg.range.clone()];
        let (glyphs, advance) = shape_run(seg_text, &face, size);
        if !glyphs.is_empty() {
            surface.draw_glyphs(
                Point::from_xy(pen_x, baseline_y),
                &glyphs,
                face.font.clone(),
                seg_text,
                size,
                false,
            );
        }
        pen_x += advance;
    }
    draw_text_decoration(surface, t, x, baseline_y, ascent_pt, size, text_w);
    if rotated {
        surface.pop();
    }
}

/// Underline / strikethrough: thin filled bars across the run's drawn extent (called inside any
/// active rotation transform so they follow rotated text). Underline sits just below the baseline;
/// strikethrough crosses the x-height. `text_w` is the same advance used for anchoring.
#[allow(clippy::too_many_arguments)]
fn draw_text_decoration(
    surface: &mut Surface,
    t: &TextRun,
    x: f32,
    baseline_y: f32,
    ascent_pt: f32,
    size: f32,
    text_w: f64,
) {
    if !(t.font.underline || t.font.strikethrough) {
        return;
    }
    let thickness = (size * 0.06).max(MIN_STROKE_PT as f32);
    let bar_w = text_w as f32;
    if t.font.underline {
        fill_bar(
            surface,
            x,
            baseline_y + thickness,
            bar_w,
            thickness,
            t.color,
        );
    }
    if t.font.strikethrough {
        fill_bar(
            surface,
            x,
            baseline_y - ascent_pt * 0.3,
            bar_w,
            thickness,
            t.color,
        );
    }
}

/// Fill an axis-aligned bar (underline / strikethrough) as a rectangle path.
fn fill_bar(surface: &mut Surface, x: f32, y: f32, w: f32, h: f32, color: Color) {
    if w <= 0.0 {
        return;
    }
    let h = h.max(MIN_STROKE_PT as f32);
    let mut pb = PathBuilder::new();
    pb.move_to(x, y);
    pb.line_to(x + w, y);
    pb.line_to(x + w, y + h);
    pb.line_to(x, y + h);
    pb.close();
    let Some(path) = pb.finish() else {
        return;
    };
    surface.set_fill(Some(fill_for(color)));
    surface.set_stroke(None);
    surface.draw_path(&path);
}

/// A face loaded once for the PDF backend: the krilla [`Font`] (subset-embedded on write) plus the
/// raw bytes kept for re-shaping each segment with rustybuzz (krilla does not expose a face's bytes
/// back). `upem` is cached so glyph advances normalise without re-parsing.
struct LoadedFace {
    font: Font,
    data: Rc<Vec<u8>>,
    index: u32,
    upem: f32,
}

/// Resolves host fonts via the shared [`rpt_text::FontDb`] and memoizes each loaded face **by id**,
/// so a face is read and subset once no matter how many families or fallback segments reference it —
/// this is what keeps krilla's per-face subsetting to one embedded copy. The resolution/coverage
/// policy (`segment_by_coverage`) lives in `FontDb`; this backend keeps only the krilla+shaping parse.
struct FontCache {
    db: FontDb,
    /// `None` value = we looked and the face failed to load (don't re-read).
    faces: HashMap<fontdb::ID, Option<Rc<LoadedFace>>>,
}

impl FontCache {
    fn new() -> FontCache {
        FontCache {
            db: FontDb::with_system_fonts(),
            faces: HashMap::new(),
        }
    }

    /// The loaded face for `id`, read and parsed once then cached (one krilla `Font` per face → one
    /// subset embed). Keeps the raw bytes so segments can be shaped with rustybuzz.
    fn face(&mut self, id: fontdb::ID) -> Option<Rc<LoadedFace>> {
        if let Some(cached) = self.faces.get(&id) {
            return cached.clone();
        }
        let loaded = self
            .db
            .with_face_data(id, |data, index| {
                let bytes = Rc::new(data.to_vec());
                let font = Font::new(bytes.as_ref().clone().into(), index)?;
                let upem = font.units_per_em();
                Some(Rc::new(LoadedFace {
                    font,
                    data: bytes,
                    index,
                    upem,
                }))
            })
            .flatten();
        self.faces.insert(id, loaded.clone());
        loaded
    }
}

/// Shape one single-face segment with rustybuzz (krilla's shaper) into krilla glyphs, returning them
/// and the segment's total advance in points. Advances are normalised by the face's units-per-em (as
/// `KrillaGlyph` expects) then scaled by `size` for the returned pen advance. LTR/auto direction —
/// each segment is one face and, in practice, one script; RTL visual order is deferred.
fn shape_run(text: &str, face: &LoadedFace, size: f32) -> (Vec<KrillaGlyph>, f32) {
    let Some(rb) = rustybuzz::Face::from_slice(&face.data, face.index) else {
        return (Vec::new(), 0.0);
    };
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    let output = rustybuzz::shape(&rb, &[], buffer);
    let positions = output.glyph_positions();
    let infos = output.glyph_infos();
    let upem = face.upem;
    let mut glyphs = Vec::with_capacity(output.len());
    let mut advance_norm = 0.0f32;
    for i in 0..output.len() {
        let pos = positions[i];
        let info = infos[i];
        // Cluster → byte range in `text`: this glyph's cluster start up to the next differing
        // cluster (LTR). rustybuzz clusters are byte offsets into the pushed string, so the range is
        // always on a UTF-8 boundary — correct for the ToUnicode mapping krilla derives from it.
        let start = info.cluster as usize;
        let end = infos[i + 1..]
            .iter()
            .map(|n| n.cluster as usize)
            .find(|&c| c != start)
            .unwrap_or(text.len());
        let x_adv = pos.x_advance as f32 / upem;
        glyphs.push(KrillaGlyph::new(
            GlyphId::new(info.glyph_id),
            x_adv,
            pos.x_offset as f32 / upem,
            pos.y_offset as f32 / upem,
            pos.y_advance as f32 / upem,
            start..end,
            None,
        ));
        advance_norm += x_adv;
    }
    (glyphs, advance_norm * size)
}
