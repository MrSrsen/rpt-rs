//! [`CosmicLayout`] — the font-accurate [`TextLayout`] backed by cosmic-text (real `hmtx` advances +
//! Unicode/CJK line-breaking + bidi + font fallback), plus the [`FontProvider`] that configures where
//! its fonts come from. Gated behind the `cosmic` feature so a backend that only needs [`FontDb`]
//! (crate::font_db) does not pull the shaping stack.

use std::cell::RefCell;
use std::path::PathBuf;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, Weight};
// One typographic point = 20 twips: the single definition lives in `rpt_layout` (cosmic-text is
// unit-agnostic; we drive it in points and scale the resulting advances to twips).
use rpt_layout::{TextLayout, TWIPS_PER_PT};
use rpt_pages::FontSpec;

/// Default leading as a multiple of the em, until we read the font's real ascent+descent+line-gap.
const DEFAULT_LEADING: f32 = 1.2;

/// Configures where fonts are sourced. Resolution priority is **local dirs first** (so a deployment
/// can pin a report's exact fonts — e.g. drop `Rubik` into a `fonts/` dir — without touching the
/// system), then the OS font registry, then the **bundled Liberation fallback** (always loaded, so a
/// named-but-absent font resolves to a metric-compatible face and rendering never fails for lack of
/// fonts — see the private `load_bundled_fallback`), and finally cosmic-text's per-glyph fallback.
#[derive(Debug, Clone, Default)]
pub struct FontProvider {
    /// Extra directories scanned for fonts, highest priority. Loaded in order.
    pub local_dirs: Vec<PathBuf>,
    /// Whether to also load the OS-installed fonts (native only; ignore on WASM).
    pub use_system_fonts: bool,
}

impl FontProvider {
    /// OS fonts only (the common native default).
    pub fn system() -> FontProvider {
        FontProvider {
            local_dirs: Vec::new(),
            use_system_fonts: true,
        }
    }

    /// Local override dirs plus the OS fonts (the recommended deployment default: pinned report
    /// fonts win, but the system library is still available).
    pub fn from_font_dirs(dirs: impl IntoIterator<Item = PathBuf>) -> FontProvider {
        FontProvider {
            local_dirs: dirs.into_iter().collect(),
            use_system_fonts: true,
        }
    }

    fn into_font_system(self) -> FontSystem {
        // Build the db with the **local dirs first**, then the OS fonts: fontdb resolves a family to
        // the first matching face in insertion order, so a pinned report font shadows a same-named
        // system font (true override, decision O1) rather than merely filling gaps.
        let mut db = cosmic_text::fontdb::Database::new();
        for dir in &self.local_dirs {
            db.load_fonts_dir(dir);
        }
        if self.use_system_fonts {
            db.load_system_fonts();
        }
        load_bundled_fallback(&mut db);
        FontSystem::new_with_locale_and_db(detect_locale(), db)
    }
}

/// The bundled Liberation fonts (SIL OFL 1.1) — see `crates/rpt-text/fonts/LICENSE`. Liberation is
/// **metric-compatible** with Arial / Times New Roman / Courier New (identical advance widths), so a
/// report authored in those (the Crystal defaults) lays out at the same positions here even when the
/// originals are not installed.
const BUNDLED_FONTS: &[&[u8]] = &[
    include_bytes!("../fonts/LiberationSans-Regular.ttf"),
    include_bytes!("../fonts/LiberationSans-Bold.ttf"),
    include_bytes!("../fonts/LiberationSans-Italic.ttf"),
    include_bytes!("../fonts/LiberationSans-BoldItalic.ttf"),
    include_bytes!("../fonts/LiberationSerif-Regular.ttf"),
    include_bytes!("../fonts/LiberationSerif-Bold.ttf"),
    include_bytes!("../fonts/LiberationMono-Regular.ttf"),
    include_bytes!("../fonts/LiberationMono-Bold.ttf"),
];

/// Register the always-present guaranteed fallback set: load the bundled Liberation
/// faces **last** (lowest priority — a real system/pinned font of the same name still wins), then
/// point the generic CSS family defaults at them. So any font a report names that is not installed
/// resolves through the generic fallback to a metric-compatible bundled face — the render is
/// deterministic and never fails for lack of fonts (headless CI, minimal containers, wasm).
fn load_bundled_fallback(db: &mut cosmic_text::fontdb::Database) {
    for bytes in BUNDLED_FONTS {
        db.load_font_data(bytes.to_vec());
    }
    db.set_sans_serif_family("Liberation Sans");
    db.set_serif_family("Liberation Serif");
    db.set_monospace_family("Liberation Mono");
}

/// Best-effort system locale (affects locale-specific family resolution, e.g. CJK), from `LANG`
/// (`en_US.UTF-8` → `en-US`), defaulting to `en-US`.
fn detect_locale() -> String {
    std::env::var("LANG")
        .ok()
        .and_then(|l| l.split('.').next().map(|s| s.replace('_', "-")))
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "en-US".to_string())
}

/// A [`TextLayout`] backed by cosmic-text. Holds a `FontSystem` (the font DB + shaping cache) behind
/// a `RefCell` because the trait measures through `&self` while cosmic-text shapes through
/// `&mut FontSystem`. Single-threaded use (one per layout pass); not `Sync`.
pub struct CosmicLayout {
    font_system: RefCell<FontSystem>,
}

impl std::fmt::Debug for CosmicLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CosmicLayout")
    }
}

impl CosmicLayout {
    /// Build from a [`FontProvider`].
    pub fn new(provider: FontProvider) -> CosmicLayout {
        CosmicLayout {
            font_system: RefCell::new(provider.into_font_system()),
        }
    }

    /// Convenience: OS fonts only.
    pub fn with_system_fonts() -> CosmicLayout {
        CosmicLayout::new(FontProvider::system())
    }

    /// Load additional font bytes (e.g. host-supplied fonts on WASM, or a report's embedded font).
    pub fn load_font_bytes(&self, data: Vec<u8>) {
        self.font_system.borrow_mut().db_mut().load_font_data(data);
    }

    /// Build a shaped buffer for `text` in `font`, optionally width-constrained (for wrapping).
    fn shaped(&self, text: &str, font: &FontSpec, max_width_pt: Option<f32>) -> Buffer {
        let mut fs = self.font_system.borrow_mut();
        let size = font.size_pt.max(1.0);
        let mut buffer = Buffer::new(&mut fs, Metrics::new(size, size * DEFAULT_LEADING));
        // set_size/set_text just store config in 0.19; shape_until_scroll does the shaping with fonts.
        buffer.set_size(max_width_pt, None);
        let mut attrs = Attrs::new().family(Family::Name(&font.family));
        if font.bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        if font.italic {
            attrs = attrs.style(Style::Italic);
        }
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut fs, false);
        buffer
    }
}

impl TextLayout for CosmicLayout {
    fn width_twips(&self, text: &str, font: &FontSpec) -> f64 {
        let buffer = self.shaped(text, font, None);
        let width_pt = buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0f32, f32::max);
        width_pt as f64 * TWIPS_PER_PT
    }

    fn line_height_twips(&self, font: &FontSpec) -> f64 {
        // Real line height from the resolved font's vertical metrics (ascent − descent + line-gap,
        // in font design units), falling back to 1.2×em if the font can't be resolved. skrifa's
        // metrics use `Size::unscaled()` (font units) with a negative descent, so the em-normalized
        // line height is `(ascent − descent + leading) / units_per_em`, scaled to the point size.
        let fallback = font.size_pt as f64 * TWIPS_PER_PT * DEFAULT_LEADING as f64;
        // Shape a probe glyph to discover which font (after fallback) actually renders this spec.
        let font_id = self
            .shaped("x", font, None)
            .layout_runs()
            .flat_map(|run| run.glyphs.iter())
            .map(|g| g.font_id)
            .next();
        let Some(font_id) = font_id else {
            return fallback;
        };
        let weight = if font.bold {
            cosmic_text::fontdb::Weight::BOLD
        } else {
            cosmic_text::fontdb::Weight::NORMAL
        };
        let Some(resolved) = self.font_system.borrow_mut().get_font(font_id, weight) else {
            return fallback;
        };
        let m = resolved.metrics();
        if m.units_per_em == 0 {
            return fallback;
        }
        let line_units = (m.ascent - m.descent + m.leading) as f64;
        line_units / m.units_per_em as f64 * font.size_pt as f64 * TWIPS_PER_PT
    }

    fn ascent_twips(&self, font: &FontSpec) -> f64 {
        // The resolved face's ascent (design units), scaled to the point size; falls back to the
        // trait default (~0.8 em) when the font can't be resolved.
        let fallback = font.size_pt as f64 * TWIPS_PER_PT * 0.8;
        let font_id = self
            .shaped("x", font, None)
            .layout_runs()
            .flat_map(|run| run.glyphs.iter())
            .map(|g| g.font_id)
            .next();
        let Some(font_id) = font_id else {
            return fallback;
        };
        let weight = if font.bold {
            cosmic_text::fontdb::Weight::BOLD
        } else {
            cosmic_text::fontdb::Weight::NORMAL
        };
        let Some(resolved) = self.font_system.borrow_mut().get_font(font_id, weight) else {
            return fallback;
        };
        let m = resolved.metrics();
        if m.units_per_em == 0 {
            return fallback;
        }
        m.ascent as f64 / m.units_per_em as f64 * font.size_pt as f64 * TWIPS_PER_PT
    }

    fn wrap(&self, text: &str, max_width: f64, font: &FontSpec) -> Vec<String> {
        let max_width_pt = (max_width / TWIPS_PER_PT) as f32;
        let buffer = self.shaped(text, font, Some(max_width_pt));
        // Each layout run is one visual (wrapped) line; reconstruct its text from the glyph byte
        // ranges into the logical line. (LTR/CJK correct; RTL visual order is a later refinement.)
        let mut lines: Vec<String> = buffer
            .layout_runs()
            .map(|run| match (run.glyphs.first(), run.glyphs.last()) {
                (Some(first), Some(last)) => {
                    let (a, b) = (first.start.min(last.start), first.end.max(last.end));
                    run.text.get(a..b).unwrap_or("").to_string()
                }
                _ => String::new(),
            })
            .collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        lines
    }
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
    fn measures_wider_for_more_text() {
        let m = CosmicLayout::with_system_fonts();
        let narrow = m.width_twips("i", &font(12.0));
        let wide = m.width_twips("WWWWWWWWWW", &font(12.0));
        assert!(wide > narrow, "W-run should be wider than a single i");
        assert!(narrow > 0.0, "measured width should be positive");
    }

    #[test]
    fn wraps_long_text_to_multiple_lines() {
        let m = CosmicLayout::with_system_fonts();
        let text = "the quick brown fox jumps over the lazy dog many times";
        let lines = m.wrap(text, 1500.0, &font(12.0));
        assert!(lines.len() > 1, "expected wrapping, got {lines:?}");
        // Every word is preserved across the wrapped lines (no loss, no duplication).
        assert_eq!(
            lines.join(" ").split_whitespace().count(),
            text.split_whitespace().count(),
            "wrapped lines preserve all words: {lines:?}"
        );
    }

    #[test]
    fn explicit_newlines_break() {
        let m = CosmicLayout::with_system_fonts();
        let lines = m.wrap("alpha\nbeta", 100_000.0, &font(12.0));
        assert_eq!(lines.len(), 2, "hard newline splits: {lines:?}");
    }

    #[test]
    fn bundled_fonts_are_a_guaranteed_fallback_without_system_fonts() {
        // No system fonts and no local dirs → only the bundled Liberation set is available.
        let m = CosmicLayout::new(FontProvider {
            local_dirs: vec![],
            use_system_fonts: false,
        });
        // A report names "Arial" (not present here). It must resolve through the sans-serif generic
        // default to the bundled Liberation Sans and shape real glyphs — same width as naming the
        // bundled family directly (proving the fallback, not just notdef boxes).
        let arial = FontSpec {
            family: "Arial".into(),
            ..font(12.0)
        };
        let liberation = FontSpec {
            family: "Liberation Sans".into(),
            ..font(12.0)
        };
        let w_arial = m.width_twips("Hello World", &arial);
        let w_lib = m.width_twips("Hello World", &liberation);
        assert!(
            w_arial > 0.0,
            "render never fails for lack of fonts: {w_arial}"
        );
        assert_eq!(
            w_arial, w_lib,
            "unmatched 'Arial' falls back to the bundled Liberation Sans"
        );
    }

    #[test]
    fn line_height_is_font_derived_and_scales() {
        let m = CosmicLayout::with_system_fonts();
        let h10 = m.line_height_twips(&font(10.0));
        let h20 = m.line_height_twips(&font(20.0));
        let em10 = 10.0 * TWIPS_PER_PT; // 10pt em = 200 twips
                                        // Real fonts add leading, so line height exceeds the bare em and sits in a typical range.
        let ratio = h10 / em10;
        assert!(
            (1.0..=1.6).contains(&ratio),
            "line-height ratio {ratio} outside typical 1.0–1.6× em"
        );
        // Metrics scale linearly with the point size.
        assert!(
            (h20 / h10 - 2.0).abs() < 0.01,
            "scales with size: {h10} vs {h20}"
        );
    }
}
