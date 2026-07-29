//! The advance contract between the producer's measurement and this writer's drawing.
//!
//! The layout engine measures a run's width — and picks its wrap point — as the natural advance plus
//! `character_spacing` once per Unicode scalar. This writer re-shapes the same string to obtain
//! glyphs, so it must arrive at the same total, or the text is wrapped as if spaced and drawn as if
//! not. Asserted at the shaper, where the rule lives, and once through the serialized content stream,
//! where its effect is observable.

use super::*;
use rpt_pages::TextMetrics;
use rpt_test_support::pdf::operator_listing;

/// 12 pt, the size the typography fixtures use.
const SIZE: f32 = 12.0;
/// 0.5 pt = 10 twips per scalar — the spacing the `paragraph_typography` fixture stores.
const SPACING_PT: f32 = 0.5;

/// The single bundled face that covers `text` for `family`, loaded for shaping.
fn face_of(fonts: &mut FontCache, family: &str, text: &str) -> Rc<LoadedFace> {
    let spec = FontSpec {
        family: family.to_string(),
        size_pt: SIZE,
        ..FontSpec::default()
    };
    let segments = fonts.db.segment_by_coverage(&spec, text);
    assert_eq!(segments.len(), 1, "{text:?} must shape in a single face");
    fonts.face(segments[0].face).expect("a bundled face loads")
}

/// Spacing is charged once per scalar of the run, the trailing one included — the producer's advance
/// includes it, so a run drawn without it ends short of the box it was measured into.
#[test]
fn character_spacing_is_charged_once_per_scalar_including_the_trailing_one() {
    let text = "Order Amount";
    let mut fonts = FontCache::new(FontSource::Bundled);
    let face = face_of(&mut fonts, "Liberation Sans", text);
    let shaper = Shaper::new(&face).expect("the bundled face parses for shaping");

    let natural = shaper.shape(text, SIZE, 0.0).1;
    let spaced = shaper.shape(text, SIZE, SPACING_PT).1;
    let expected = natural + SPACING_PT * text.chars().count() as f32;
    assert!(
        (spaced - expected).abs() < 0.001,
        "drawn {spaced} vs measured {expected}"
    );
    // Zero spacing must leave the advances exactly as the face shaped them.
    assert_eq!(shaper.shape(text, SIZE, 0.0).1, natural);
}

/// A ligature shapes several scalars into one glyph, so the per-glyph reading of the rule loses the
/// spacing of every extra scalar it covers. This is the divergence the field exists to prevent, and
/// the per-glyph total is asserted *not* to reconcile so the loop cannot quietly adopt it.
#[test]
fn a_ligature_owes_spacing_for_every_scalar_it_covers() {
    // DejaVu Sans is the bundled symbol face and the only one with a ligature table.
    let text = "office";
    let mut fonts = FontCache::new(FontSource::Bundled);
    let face = face_of(&mut fonts, "DejaVu Sans", text);
    let shaper = Shaper::new(&face).expect("the bundled face parses for shaping");

    let (glyphs, natural) = shaper.shape(text, SIZE, 0.0);
    let scalars = text.chars().count();
    assert!(
        glyphs.len() < scalars,
        "the fixture needs a real ligature: {} glyphs for {scalars} scalars",
        glyphs.len()
    );

    let spaced = shaper.shape(text, SIZE, SPACING_PT).1;
    let per_scalar = natural + SPACING_PT * scalars as f32;
    let per_glyph = natural + SPACING_PT * glyphs.len() as f32;
    assert!(
        (spaced - per_scalar).abs() < 0.001,
        "drawn {spaced} vs measured {per_scalar}"
    );
    assert!(
        (spaced - per_glyph).abs() > 0.1,
        "the per-glyph rule must not reconcile — that is the bug this charges per scalar to avoid"
    );
}

/// The spacing reaches the page: the writer emits it as the `TJ` adjustment between glyphs, in
/// thousandths of the text-space unit, which is what a viewer applies. A backend that dropped it
/// would still serialize a clean PDF, so the operator listing is where the omission is visible.
#[test]
fn the_content_stream_carries_the_spacing_between_glyphs() {
    let mut page = Page::new(
        1,
        PageSize {
            width: Twips(12240),
            height: Twips(15840),
        },
    );
    // No kern pair in the string, so the only `TJ` adjustment a viewer sees is the spacing.
    let text = "nun";
    page.push(DrawOp::Text(TextRun {
        bounds: Rect {
            left: Twips(720),
            top: Twips(720),
            width: Twips(4000),
            height: Twips(320),
        },
        text: text.to_string(),
        font: FontSpec {
            family: "Liberation Sans".to_string(),
            size_pt: SIZE,
            ..FontSpec::default()
        },
        color: Color {
            a: 255,
            r: 0,
            g: 0,
            b: 0,
        },
        align: TextAlign::Left,
        rotation: 0.0,
        metrics: Some(TextMetrics {
            advance: Twips(1000),
            ascent: Twips(240),
            line_height: Twips(276),
        }),
        character_spacing: Twips(10),
        source: None,
    }));
    let listing = operator_listing(&crate::render_pages(&[page]));
    // -41.667 = -(10 twips / 20 twips-per-pt) / 12 pt × 1000, once after each of the first two
    // scalars; the trailing one moves the pen past the end of the run and needs no adjustment.
    assert!(
        listing.contains("-41.667"),
        "no per-scalar TJ adjustment in:\n{listing}"
    );
}
