//! The always-present bundled fallback fonts, shared by both font paths ([`crate::FontDb`] for the
//! physical backends and [`crate::CosmicLayout`] for shaping) so an unknown family resolves to the
//! same metric-compatible face on either path.

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

/// The bundled symbol fallback face (DejaVu Sans, Bitstream Vera license — see
/// `crates/rpt-text/fonts/LICENSE-DejaVu`). It covers the Unicode symbols/dingbats the text faces
/// lack — ⚠ U+26A0, ✓, ✗, arrows, box-drawing — so per-glyph fallback resolves them instead of
/// emitting a `.notdef` box. Kept as a *distinct* family (not wired to a generic default), so normal
/// family resolution never picks it up: it is used only by the explicit per-glyph coverage fallback.
const SYMBOL_FALLBACK_FONT: &[u8] = include_bytes!("../fonts/DejaVuSans.ttf");

/// The family name the [`SYMBOL_FALLBACK_FONT`] registers under (its internal name-table family).
pub(crate) const SYMBOL_FALLBACK_FAMILY: &str = "DejaVu Sans";

/// Register the always-present guaranteed fallback set: load the bundled Liberation faces **last**
/// (lowest priority — a real system/pinned font of the same name still wins), then point the generic
/// CSS family defaults at them. So any font a report names that is not installed resolves through the
/// generic fallback to a metric-compatible bundled face — the render is deterministic and never fails
/// for lack of fonts (headless CI, minimal containers, wasm). The symbol fallback face is loaded too
/// (for per-glyph coverage fallback) but left off the generic defaults so it never shadows a text
/// family.
pub(crate) fn register_fallback(db: &mut fontdb::Database) {
    for bytes in BUNDLED_FONTS {
        db.load_font_data(bytes.to_vec());
    }
    db.load_font_data(SYMBOL_FALLBACK_FONT.to_vec());
    db.set_sans_serif_family("Liberation Sans");
    db.set_serif_family("Liberation Serif");
    db.set_monospace_family("Liberation Mono");
}
