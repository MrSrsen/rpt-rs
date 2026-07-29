//! The [`TextLayout`] trait lives in the leaf crate [`rpt_pages`] (beside `FontSpec`), so the
//! layout engine and `rpt-text`'s real font stack share one definition without `rpt-text` depending
//! on all of `rpt-layout`. Re-exported here so `crate::text::…` paths keep resolving.
//!
//! This module also owns the **advance model** the engine measures with: the font's natural advances
//! plus a paragraph's rigid character spacing. Every site that measures, wraps, or reports a width
//! goes through [`spaced_width_twips`], so a run's `TextMetrics::advance`, its wrap point, and its
//! tab stops can never be computed three slightly different ways.

use rpt_model::Twips;
use rpt_pages::FontSpec;

pub use rpt_pages::{ApproxLayout, TextLayout, TWIPS_PER_PT};

/// The width of one line of `text` under a rigid `spacing`: the font's natural advance plus the
/// extra inserted after **every Unicode scalar**, the trailing one included (GDI
/// `SetTextCharacterExtra`, which the engine's spaced text is drawn through).
///
/// At the overwhelmingly common `spacing` of zero this is exactly `layout.width_twips`.
pub(crate) fn spaced_width_twips(
    layout: &dyn TextLayout,
    text: &str,
    font: &FontSpec,
    spacing: Twips,
) -> f64 {
    layout.width_twips(text, font) + f64::from(spacing.0) * text.chars().count() as f64
}

/// A [`TextLayout`] that measures through [`spaced_width_twips`], so a spaced paragraph breaks at the
/// width it is actually drawn at. Wrapping against the unspaced width instead would put the break in
/// the wrong place — a divergence that shows up as wrong text on the page *before* the spaced one.
///
/// Wrapping falls back to the trait's greedy, space-based default: the adjusted width has to drive
/// the break, and a real stack's script-aware `wrap` measures its own way. Spacing is a Latin
/// designer control, so this only matters for a spaced non-spaced-script run.
#[derive(Debug)]
pub(crate) struct SpacedLayout<'a> {
    inner: &'a dyn TextLayout,
    spacing: Twips,
}

impl<'a> SpacedLayout<'a> {
    pub(crate) fn new(inner: &'a dyn TextLayout, spacing: Twips) -> SpacedLayout<'a> {
        SpacedLayout { inner, spacing }
    }
}

impl TextLayout for SpacedLayout<'_> {
    fn width_twips(&self, text: &str, font: &FontSpec) -> f64 {
        spaced_width_twips(self.inner, text, font, self.spacing)
    }

    fn line_height_twips(&self, font: &FontSpec) -> f64 {
        self.inner.line_height_twips(font)
    }

    fn ascent_twips(&self, font: &FontSpec) -> f64 {
        self.inner.ascent_twips(font)
    }

    fn is_approximate(&self) -> bool {
        self.inner.is_approximate()
    }

    fn substituted_chars(&self, text: &str, font: &FontSpec) -> String {
        self.inner.substituted_chars(text, font)
    }
}
