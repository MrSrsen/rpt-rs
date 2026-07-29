//! [`CosmicLayout`] — the font-accurate [`TextLayout`] backed by cosmic-text (real `hmtx` advances +
//! Unicode/CJK line-breaking + bidi + font fallback), plus the [`FontProvider`] that configures where
//! its fonts come from. Gated behind the `cosmic` feature so a backend that only needs [`FontDb`]
//! (crate::font_db) does not pull the shaping stack.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, Weight};
// One typographic point = 20 twips: the single definition lives in `rpt_pages` (cosmic-text is
// unit-agnostic; we drive it in points and scale the resulting advances to twips).
use rpt_pages::FontSpec;
use rpt_pages::{TextLayout, TWIPS_PER_PT};

use crate::FontSource;

/// Default leading as a multiple of the em, until we read the font's real ascent+descent+line-gap.
const DEFAULT_LEADING: f32 = 1.2;

/// Configures where fonts are sourced. Resolution priority is **local dirs first** (so a deployment
/// can pin a report's exact fonts — e.g. drop `Rubik` into a `fonts/` dir — without touching the
/// system), then the OS font registry, then the **bundled Liberation fallback** (always loaded, so a
/// named-but-absent font resolves to a metric-compatible face and rendering never fails for lack of
/// fonts — see the `bundled` module), and finally cosmic-text's per-glyph fallback.
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

    /// The bundled fallback faces only — no OS fonts, no local dirs. Every family a report names
    /// resolves through the generic defaults to a compiled-in Liberation face, so the same report
    /// lays out **identically on every machine**. That makes it the right stack for a WASM host with
    /// no font registry, a minimal container, and any render whose geometry is compared against a
    /// committed baseline (where a difference in the host's installed faces would otherwise read as
    /// a layout regression).
    pub fn bundled() -> FontProvider {
        FontProvider {
            local_dirs: Vec::new(),
            use_system_fonts: false,
        }
    }

    /// The provider a [`FontSource`] names — the metrics-side counterpart of [`FontSource::load`], so
    /// one value configures both halves of the font stack (the faces layout measures with and the
    /// faces a backend embeds) and they cannot drift apart.
    pub fn from_source(source: FontSource) -> FontProvider {
        match source {
            FontSource::System => FontProvider::system(),
            FontSource::Bundled => FontProvider::bundled(),
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
        // system font (a true override) rather than merely filling gaps.
        let mut db = cosmic_text::fontdb::Database::new();
        for dir in &self.local_dirs {
            db.load_fonts_dir(dir);
        }
        if self.use_system_fonts {
            db.load_system_fonts();
        }
        crate::bundled::register_fallback(&mut db);
        FontSystem::new_with_locale_and_db(detect_locale(), db)
    }
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

/// A resolved-metrics cache key: the same handful of `FontSpec`s recur thousands of times per pass,
/// so `(line_height, ascent)` is memoized on `family + size + bold + italic` (size stored by bit
/// pattern to keep the key hashable and exact).
#[derive(Clone, PartialEq, Eq, Hash)]
struct FontKey {
    family: String,
    size_bits: u32,
    bold: bool,
    italic: bool,
}

impl FontKey {
    fn new(font: &FontSpec) -> FontKey {
        FontKey {
            family: font.family.clone(),
            size_bits: font.size_pt.to_bits(),
            bold: font.bold,
            italic: font.italic,
        }
    }
}

/// A [`TextLayout`] backed by cosmic-text. Holds a `FontSystem` (the font DB + shaping cache) behind
/// a `RefCell` because the trait measures through `&self` while cosmic-text shapes through
/// `&mut FontSystem`. Single-threaded use (one per layout pass); not `Sync`.
pub struct CosmicLayout {
    font_system: RefCell<FontSystem>,
    /// Lowercased family names actually present in the DB. A requested family that is **not** here
    /// resolves through the sans-serif generic (the metric-compatible bundled Liberation) rather than
    /// letting cosmic-text's built-in per-script fallback list pick an arbitrary loaded sans (e.g. the
    /// bundled DejaVu symbol face) — which would break Arial/Times metric-compatibility. The symbol
    /// face is still reached, but only through per-glyph coverage fallback for glyphs Liberation lacks.
    known_families: HashSet<String>,
    /// Memoized `(line_height_twips, ascent_twips)` per [`FontKey`], computed lazily on first use.
    metrics_cache: RefCell<HashMap<FontKey, (f64, f64)>>,
}

impl std::fmt::Debug for CosmicLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CosmicLayout")
    }
}

impl CosmicLayout {
    /// Build from a [`FontProvider`].
    pub fn new(provider: FontProvider) -> CosmicLayout {
        let font_system = provider.into_font_system();
        let known_families = font_system
            .db()
            .faces()
            .flat_map(|f| f.families.iter().map(|(name, _)| name.to_lowercase()))
            .collect();
        CosmicLayout {
            font_system: RefCell::new(font_system),
            known_families,
            metrics_cache: RefCell::new(HashMap::new()),
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
        // A known family shapes by name; an absent one goes through the generic its name implies, so
        // it lands on the DB's metric-compatible Liberation face for that class — NOT cosmic-text's
        // built-in per-script list, which would pick an arbitrary loaded sans (the bundled DejaVu
        // symbol face, say) and lose metric compatibility entirely. Per-glyph fallback still covers
        // glyphs the chosen face lacks.
        let family = if self.known_families.contains(&font.family.to_lowercase()) {
            Family::Name(&font.family)
        } else {
            match crate::font_db::generic_for(&font.family) {
                crate::font_db::GenericFamily::Serif => Family::Serif,
                crate::font_db::GenericFamily::Monospace => Family::Monospace,
                crate::font_db::GenericFamily::SansSerif => Family::SansSerif,
            }
        };
        let mut attrs = Attrs::new().family(family);
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

    /// Resolve `font` (after cosmic-text fallback) to the vertical metrics of the face that actually
    /// renders it, in font design units: `(units_per_em, ascent, descent, leading)`. Shapes a probe
    /// glyph to discover the resolved `font_id`. `None` when nothing resolves or the face has no em
    /// square — the callers substitute their own em-relative fallback.
    fn resolved_metrics(&self, font: &FontSpec) -> Option<(u16, f32, f32, f32)> {
        let font_id = self
            .shaped("x", font, None)
            .layout_runs()
            .flat_map(|run| run.glyphs.iter())
            .map(|g| g.font_id)
            .next()?;
        let weight = if font.bold {
            cosmic_text::fontdb::Weight::BOLD
        } else {
            cosmic_text::fontdb::Weight::NORMAL
        };
        let resolved = self.font_system.borrow_mut().get_font(font_id, weight)?;
        let m = resolved.metrics();
        if m.units_per_em == 0 {
            return None;
        }
        Some((m.units_per_em, m.ascent, m.descent, m.leading))
    }

    /// Memoized `(line_height_twips, ascent_twips)` for `font`. The first call per [`FontKey`] shapes
    /// a probe glyph to resolve the face; subsequent calls hit the cache. Values are identical to
    /// computing them directly — this is pure memoization of an expensive per-`FontSpec` resolution.
    fn line_metrics(&self, font: &FontSpec) -> (f64, f64) {
        let key = FontKey::new(font);
        if let Some(&cached) = self.metrics_cache.borrow().get(&key) {
            return cached;
        }
        let metrics = match self.resolved_metrics(font) {
            // Real vertical metrics: `(ascent − descent + line-gap) / units_per_em`, scaled to the
            // point size (skrifa reports descent negative, so the subtraction adds its magnitude).
            Some((units_per_em, ascent, descent, leading)) => {
                let line_units = (ascent - descent + leading) as f64;
                let line_height =
                    line_units / units_per_em as f64 * font.size_pt as f64 * TWIPS_PER_PT;
                let ascent =
                    ascent as f64 / units_per_em as f64 * font.size_pt as f64 * TWIPS_PER_PT;
                (line_height, ascent)
            }
            // Unresolvable font: em-relative fallbacks (1.2×em line height, ~0.8×em ascent).
            None => (
                font.size_pt as f64 * TWIPS_PER_PT * DEFAULT_LEADING as f64,
                font.size_pt as f64 * TWIPS_PER_PT * 0.8,
            ),
        };
        self.metrics_cache.borrow_mut().insert(key, metrics);
        metrics
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
        // Real line height from the resolved font's vertical metrics, falling back to 1.2×em when the
        // font can't be resolved. Computed (and memoized) in `line_metrics`.
        self.line_metrics(font).0
    }

    fn ascent_twips(&self, font: &FontSpec) -> f64 {
        // The resolved face's ascent scaled to the point size, falling back to ~0.8×em. Computed
        // (and memoized) in `line_metrics`.
        self.line_metrics(font).1
    }

    fn substituted_chars(&self, text: &str, font: &FontSpec) -> String {
        // Resolve the primary face for the requested family (same query FontDb uses on the backend
        // side), then report every char that face lacks — those render through the shared symbol
        // fallback (DejaVu, registered on the same path). Deterministic and font-DB-driven, so the
        // reported set matches what the physical backends actually substitute.
        use cosmic_text::fontdb::{Family, Query, Stretch, Style, Weight};
        let fs = self.font_system.borrow();
        let db = fs.db();
        let primary = db.query(&Query {
            families: &[Family::Name(&font.family), Family::SansSerif],
            weight: if font.bold {
                Weight::BOLD
            } else {
                Weight::NORMAL
            },
            stretch: Stretch::Normal,
            style: if font.italic {
                Style::Italic
            } else {
                Style::Normal
            },
        });
        let Some(primary) = primary else {
            return String::new();
        };
        // Parse the face once and probe every char against it, rather than re-parsing per char.
        db.with_face_data(primary, |data, index| {
            let face = ttf_parser::Face::parse(data, index).ok();
            let mut out = String::new();
            for c in text.chars() {
                if c.is_control() || out.contains(c) {
                    continue;
                }
                if face.as_ref().and_then(|f| f.glyph_index(c)).is_none() {
                    out.push(c);
                }
            }
            out
        })
        .unwrap_or_default()
    }

    fn wrap(&self, text: &str, max_width: f64, font: &FontSpec) -> Vec<String> {
        let max_width_pt = (max_width / TWIPS_PER_PT) as f32;
        let buffer = self.shaped(text, font, Some(max_width_pt));
        // Each layout run is one visual (wrapped) line; reconstruct its text from the glyph byte
        // ranges into the logical line. Correct for LTR/CJK; RTL visual order is not reconstructed
        // correctly.
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
    fn symbol_fallback_converges_with_the_backends() {
        // Measurement must fall back to the SAME bundled symbol face the
        // physical backends render with, so summed advances agree and text after a symbol stays put.
        // With only the bundled set loaded, ⚠ in "Arial" measures identically to naming DejaVu Sans
        // directly — proving cosmic-text's per-glyph fallback lands on the bundled DejaVu.
        let m = CosmicLayout::new(FontProvider {
            local_dirs: vec![],
            use_system_fonts: false,
        });
        let arial = FontSpec {
            family: "Arial".into(),
            ..font(12.0)
        };
        let dejavu = FontSpec {
            family: "DejaVu Sans".into(),
            ..font(12.0)
        };
        let w_arial = m.width_twips("\u{26A0}", &arial);
        let w_dejavu = m.width_twips("\u{26A0}", &dejavu);
        assert!(w_arial > 0.0, "⚠ shapes to a real glyph, not tofu");
        assert_eq!(
            w_arial, w_dejavu,
            "⚠ in Arial falls back to the same bundled DejaVu face used by the backends"
        );
    }

    #[test]
    fn substituted_chars_reports_only_uncovered_glyphs() {
        let m = CosmicLayout::new(FontProvider {
            local_dirs: vec![],
            use_system_fonts: false,
        });
        let arial = FontSpec {
            family: "Arial".into(),
            ..font(12.0)
        };
        // ASCII is fully covered by the Latin face → nothing substituted.
        assert_eq!(m.substituted_chars("HAZ", &arial), "");
        // ⚠ is not in the Latin face → reported (once, deduped) and the ASCII around it is not.
        assert_eq!(
            m.substituted_chars("\u{26A0} HAZ \u{26A0}", &arial),
            "\u{26A0}"
        );
    }

    #[test]
    fn metrics_are_stable_and_deterministic_across_the_cache() {
        // The per-FontSpec metrics cache is pure memoization: repeated calls and a fresh instance
        // must return bit-identical values (the guarantee the render gate relies on).
        let f = FontSpec {
            family: "Arial".into(),
            bold: true,
            ..font(11.5)
        };
        let a = CosmicLayout::with_system_fonts();
        let first = (a.line_height_twips(&f), a.ascent_twips(&f));
        let second = (a.line_height_twips(&f), a.ascent_twips(&f));
        assert_eq!(first, second, "cached call returns identical values");
        let b = CosmicLayout::with_system_fonts();
        let fresh = (b.line_height_twips(&f), b.ascent_twips(&f));
        assert_eq!(first, fresh, "resolution is deterministic across instances");
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
