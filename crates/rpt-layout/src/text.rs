//! Text layout — measurement + line-breaking for `can-grow` fields.
//!
//! [`TextLayout`] abstracts "how wide is this string, how tall is a line, and where does it wrap"
//! so the layout engine is independent of the metrics/shaping source. It is injected into the
//! formatter ([`crate::layout_with`]) so the heavy real font stack lives in a separate crate.
//!
//! The default [`ApproxLayout`] is dependency-free and only *approximate*: a fixed average advance
//! per em, greedy space-based wrapping. Enough to trigger wrapping and stack lines, but NOT
//! metric-accurate and NOT script-aware (it cannot wrap CJK, which has no spaces). The
//! metric-accurate, international impl is `rpt-text`'s `CosmicLayout` (cosmic-text: real `hmtx`
//! advances + Unicode line-breaking + bidi + font fallback), which overrides every method here.

use rpt_pages::FontSpec;

/// One typographic point = 20 twips.
pub use rpt_render_util::TWIPS_PER_POINT as TWIPS_PER_PT;

/// Measures and line-breaks text for the layout engine, in twips. Implementors range from the
/// dependency-free [`ApproxLayout`] to a full shaping stack (`rpt-text::CosmicLayout`).
pub trait TextLayout: std::fmt::Debug {
    /// Width of a single line of `text` in `font`, in twips.
    fn width_twips(&self, text: &str, font: &FontSpec) -> f64;

    /// A line's height in twips (font size × leading). Default: 1.2× the em; a real impl overrides
    /// with the font's ascent+descent+line-gap.
    fn line_height_twips(&self, font: &FontSpec) -> f64 {
        font.size_pt as f64 * TWIPS_PER_PT * 1.2
    }

    /// The baseline offset below a line's top edge in twips (the font's ascent). Default: ~0.8× the em
    /// (the point-size heuristic the backends used to inline); a real impl overrides with the resolved
    /// face's ascent so text sits on the same baseline across every backend.
    fn ascent_twips(&self, font: &FontSpec) -> f64 {
        font.size_pt as f64 * TWIPS_PER_PT * 0.8
    }

    /// Break `text` into display lines that each fit `max_width` twips in `font`. Explicit newlines
    /// always break. The default is greedy and **space-based** (no mid-word or CJK breaking); a
    /// script-aware impl overrides this. Returns at least one line (possibly empty).
    fn wrap(&self, text: &str, max_width: f64, font: &FontSpec) -> Vec<String> {
        greedy_wrap(text, max_width, font, self)
    }

    /// Whether this layout is only *approximate* — a fixed average advance with space-only greedy
    /// wrapping (no real metrics, no CJK breaking). Wrap points, `can-grow` heights, and therefore
    /// **page counts** derived from an approximate layout are NOT cross-platform byte-identical with a
    /// real font stack, so the paginator emits a one-shot diagnostic when this is true. A
    /// metric-accurate impl (`rpt-text::CosmicLayout`) leaves the default `false`.
    fn is_approximate(&self) -> bool {
        false
    }
}

/// Dependency-free approximate layout: average Latin proportional advance as a fraction of the em,
/// nudged for weight; greedy space-based wrapping. Good enough to decide *where* to wrap and how
/// tall a field grows for Latin text; not accurate enough for pixel parity or non-spaced scripts.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApproxLayout;

impl TextLayout for ApproxLayout {
    fn width_twips(&self, text: &str, font: &FontSpec) -> f64 {
        let em = font.size_pt as f64 * TWIPS_PER_PT;
        // Average advance / em for Latin proportional text (empirical); bold is a touch wider.
        let avg = if font.bold { 0.56 } else { 0.50 };
        text.chars().count() as f64 * em * avg
    }

    fn is_approximate(&self) -> bool {
        true
    }
}

/// Greedy word-wrap shared as the trait's default `wrap`. Explicit newlines always break; a single
/// word wider than the box is kept whole (overflow rather than split mid-word).
pub fn greedy_wrap(
    text: &str,
    max_width: f64,
    font: &FontSpec,
    m: &(impl TextLayout + ?Sized),
) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        for word in para.split_whitespace() {
            if cur.is_empty() {
                cur.push_str(word);
                continue;
            }
            let candidate = format!("{cur} {word}");
            if m.width_twips(&candidate, font) <= max_width {
                cur = candidate;
            } else {
                lines.push(std::mem::take(&mut cur));
                cur.push_str(word);
            }
        }
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font(size: f32) -> FontSpec {
        FontSpec {
            size_pt: size,
            ..FontSpec::default()
        }
    }

    #[test]
    fn short_text_is_one_line() {
        let lines = ApproxLayout.wrap("Hello world", 100_000.0, &font(10.0));
        assert_eq!(lines, vec!["Hello world"]);
    }

    #[test]
    fn long_text_wraps_to_multiple_lines() {
        let m = ApproxLayout;
        // A narrow box forces wrapping.
        let lines = m.wrap(
            "the quick brown fox jumps over the lazy dog",
            1500.0,
            &font(10.0),
        );
        assert!(lines.len() > 1, "expected wrapping, got {lines:?}");
        // Every line fits (except an unsplittable single word).
        for l in &lines {
            assert!(
                m.width_twips(l, &font(10.0)) <= 1500.0 || !l.contains(' '),
                "line over width: {l:?}"
            );
        }
        // No content lost.
        assert_eq!(lines.join(" ").split_whitespace().count(), 9);
    }

    #[test]
    fn explicit_newlines_break() {
        let lines = ApproxLayout.wrap("a\nb\nc", 100_000.0, &font(10.0));
        assert_eq!(lines, vec!["a", "b", "c"]);
    }
}
