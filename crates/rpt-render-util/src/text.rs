//! Text-placement math for the render backends: the horizontal-alignment anchor, the justification
//! slack, and the metric-less baseline fallback. Each is generic over (or returns) the numeric unit
//! the caller works in, so it holds one definition of the rule rather than one per output unit.

use core::ops::{Add, Div, Sub};

use rpt_pages::{TextAlign, TextRun};

use crate::units::TWIPS_PER_POINT;

/// Float backing for justification slack math — the numeric unit a caller computes slack in, so
/// [`justify_gap_extra`] holds one definition independent of that unit.
pub trait JustifyUnit: Copy + PartialOrd + Sub<Output = Self> + Div<Output = Self> {
    /// The additive identity, both the "no slack" return value and the slack comparison threshold.
    const ZERO: Self;
    /// The inter-word gap count as this unit, the divisor slack is spread across.
    fn from_gaps(n: usize) -> Self;
}

impl JustifyUnit for f64 {
    const ZERO: Self = 0.0;
    fn from_gaps(n: usize) -> Self {
        n as f64
    }
}

/// The extra advance added at each inter-word gap to justify a run: the box slack (`box_w − text_w`)
/// spread across the run's inter-word gaps. `0` when the run is not justified, has no inter-word
/// gap, or already fills its box. Generic over the slack unit; a caller that needs a narrower result
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

/// Fraction of a run's point size used as its ascent when it carries no measured metrics (~0.8 em).
/// Matches the native engine's baseline placement.
const ASCENT_FALLBACK_EM: f64 = 0.8;

/// The x anchor for a text run given its box left/width and shaped text width, in whatever numeric
/// unit the caller works in. Left/justified anchor at the box left; centre and right offset by the
/// slack (`box_w - text_w`).
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
/// A justified line's slack (`box_w − text_w`) is spread across this many gaps.
fn word_gap_count(text: &str) -> usize {
    text.chars().filter(|c| *c == ' ').count()
}

/// The baseline offset below a text run's top edge, in twips: the run's measured ascent when it
/// carries metrics, else ~0.8 em of its point size converted to twips. Returned unrounded (`f64`)
/// so a twips-space caller stays exact and a point-space one can scale it.
pub fn baseline_offset_twips(run: &TextRun) -> f64 {
    match &run.metrics {
        Some(m) => m.ascent.0 as f64,
        None => run.font.size_pt as f64 * ASCENT_FALLBACK_EM * TWIPS_PER_POINT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_model::{Color, Rect, Twips};
    use rpt_pages::{FontSpec, TextMetrics};

    #[test]
    fn aligned_x_anchors_by_alignment() {
        // Left/justified anchor at the box left; centre/right offset by the slack.
        assert_eq!(aligned_x(TextAlign::Left, 10.0_f64, 100.0, 40.0), 10.0);
        assert_eq!(aligned_x(TextAlign::Justified, 10.0_f64, 100.0, 40.0), 10.0);
        assert_eq!(aligned_x(TextAlign::Center, 10.0_f64, 100.0, 40.0), 40.0);
        assert_eq!(aligned_x(TextAlign::Right, 10.0_f64, 100.0, 40.0), 70.0);
    }

    #[test]
    fn word_gap_count_counts_spaces() {
        assert_eq!(word_gap_count("one two three"), 2);
        assert_eq!(word_gap_count("single"), 0);
        assert_eq!(word_gap_count(""), 0);
    }

    #[test]
    fn justify_gap_extra_spreads_slack_across_gaps() {
        // 30 slack over 2 gaps → 15 per gap.
        assert_eq!(
            justify_gap_extra(TextAlign::Justified, "a b c", 100.0_f64, 70.0),
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
    fn baseline_offset_prefers_measured_ascent_over_the_em_fallback() {
        let mut run = TextRun {
            bounds: Rect::default(),
            text: "x".to_string(),
            font: FontSpec {
                size_pt: 10.0,
                ..FontSpec::default()
            },
            color: Color::default(),
            align: TextAlign::Left,
            rotation: 0.0,
            metrics: None,
            character_spacing: Twips(0),
            source: None,
        };
        // No metrics: 0.8 em of the 10 pt size, in twips.
        assert_eq!(baseline_offset_twips(&run), 10.0 * 0.8 * TWIPS_PER_POINT);
        // Measured metrics win outright.
        run.metrics = Some(TextMetrics {
            advance: Twips(500),
            ascent: Twips(137),
            line_height: Twips(240),
        });
        assert_eq!(baseline_offset_twips(&run), 137.0);
    }
}
