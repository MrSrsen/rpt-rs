//! Text-placement math shared by the render backends: the horizontal-alignment anchor and the
//! metric-less baseline fallback. Each backend applies these in its own unit (twips, points, or
//! device pixels), so they were re-derived per backend; centralizing them keeps one definition.

use core::ops::{Add, Div, Sub};

use rpt_pages::{TextAlign, TextRun};

/// Float backing for justification slack math — the numeric types the backends compute slack in.
/// Implemented for `f32` (device pixels) and `f64` (points); lets [`justify_gap_extra`] be one
/// definition across the backends.
pub trait JustifyUnit: Copy + PartialOrd + Sub<Output = Self> + Div<Output = Self> {
    /// The additive identity, both the "no slack" return value and the slack comparison threshold.
    const ZERO: Self;
    /// The inter-word gap count as this unit, the divisor slack is spread across.
    fn from_gaps(n: usize) -> Self;
}

impl JustifyUnit for f32 {
    const ZERO: Self = 0.0;
    fn from_gaps(n: usize) -> Self {
        n as f32
    }
}

impl JustifyUnit for f64 {
    const ZERO: Self = 0.0;
    fn from_gaps(n: usize) -> Self {
        n as f64
    }
}

/// The extra advance added at each inter-word gap to justify a run: the box slack (`box_w − text_w`)
/// spread across the run's word gaps ([`word_gap_count`]). `0` when the run is not justified, has no
/// inter-word gap, or already fills its box. Generic over the slack unit so the point-space (`f64`)
/// and device-pixel (`f32`) backends share one definition; a backend that needs a narrower result
/// casts at the call site.
pub fn justify_gap_extra<T: JustifyUnit>(align: TextAlign, text: &str, box_w: T, text_w: T) -> T {
    if !matches!(align, TextAlign::Justified) {
        return T::ZERO;
    }
    let gaps = word_gap_count(text);
    let slack = box_w - text_w;
    if gaps == 0 || slack <= T::ZERO {
        T::ZERO
    } else {
        slack / T::from_gaps(gaps)
    }
}

use crate::units::TWIPS_PER_POINT;

/// Fraction of a run's point size used as its ascent when it carries no measured metrics (~0.8 em).
/// Every backend applies this same fraction for its fallback baseline; only the unit differs. Matches
/// the native engine's baseline placement.
pub const ASCENT_FALLBACK_EM: f64 = 0.8;

/// The x anchor for a text run given its box left/width and shaped text width, in whatever numeric
/// unit the caller works in. Left/justified anchor at the box left; centre and right offset by the
/// slack (`box_w - text_w`). Generic over the unit so the point-space PDF backend (`f64`) and the
/// device-pixel raster backend (`f32`) share one definition.
pub fn aligned_x<T>(align: TextAlign, left: T, box_w: T, text_w: T) -> T
where
    T: Copy + Add<Output = T> + Sub<Output = T> + Div<Output = T> + From<u8>,
{
    match align {
        TextAlign::Left | TextAlign::Justified => left,
        TextAlign::Center => left + (box_w - text_w) / T::from(2u8),
        TextAlign::Right => left + box_w - text_w,
    }
}

/// The number of inter-word gaps in `text` that justification can stretch — one per ASCII space.
/// A backend spreads a justified line's slack (`box_w − text_w`) across this many gaps.
pub fn word_gap_count(text: &str) -> usize {
    text.chars().filter(|c| *c == ' ').count()
}

/// The baseline offset below a text run's top edge, in twips: the run's measured ascent when it
/// carries metrics, else [`ASCENT_FALLBACK_EM`] of its point size converted to twips. Returned
/// unrounded (`f64`) so a twips-space backend stays exact and a point-space backend can scale it.
pub fn baseline_offset_twips(run: &TextRun) -> f64 {
    match &run.metrics {
        Some(m) => m.ascent.0 as f64,
        None => run.font.size_pt as f64 * ASCENT_FALLBACK_EM * TWIPS_PER_POINT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligned_x_matches_per_backend_spellings() {
        // Left/justified anchor at the box left; centre/right offset by the slack.
        assert_eq!(aligned_x(TextAlign::Left, 10.0_f64, 100.0, 40.0), 10.0);
        assert_eq!(aligned_x(TextAlign::Justified, 10.0_f64, 100.0, 40.0), 10.0);
        assert_eq!(aligned_x(TextAlign::Center, 10.0_f64, 100.0, 40.0), 40.0);
        assert_eq!(aligned_x(TextAlign::Right, 10.0_f64, 100.0, 40.0), 70.0);
        // Same for the f32 (device-pixel) unit.
        assert_eq!(aligned_x(TextAlign::Center, 10.0_f32, 100.0, 40.0), 40.0);
    }

    #[test]
    fn word_gap_count_counts_spaces() {
        assert_eq!(word_gap_count("one two three"), 2);
        assert_eq!(word_gap_count("single"), 0);
        assert_eq!(word_gap_count(""), 0);
    }

    #[test]
    fn justify_gap_extra_spreads_slack_across_gaps() {
        // 30 slack over 2 gaps → 15 per gap, in either unit.
        assert_eq!(
            justify_gap_extra(TextAlign::Justified, "a b c", 100.0_f64, 70.0),
            15.0
        );
        assert_eq!(
            justify_gap_extra(TextAlign::Justified, "a b c", 100.0_f32, 70.0),
            15.0
        );
        // Non-justified, no gap, and no slack all yield zero.
        assert_eq!(
            justify_gap_extra(TextAlign::Left, "a b c", 100.0_f64, 70.0),
            0.0
        );
        assert_eq!(
            justify_gap_extra(TextAlign::Justified, "single", 100.0_f64, 70.0),
            0.0
        );
        assert_eq!(
            justify_gap_extra(TextAlign::Justified, "a b c", 100.0_f64, 100.0),
            0.0
        );
    }

    #[test]
    fn ascent_fallback_ratio_agrees_across_unit_spellings() {
        // The float 0.8-em form equals the raster f32 form and the svg integer `twips*4/5` form.
        assert_eq!(ASCENT_FALLBACK_EM as f32, 0.8_f32);
        for twips in 0..20000_i32 {
            assert_eq!((twips as f64 * ASCENT_FALLBACK_EM) as i32, twips * 4 / 5);
        }
    }
}
