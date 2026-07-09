//! # rpt-text — the font stack for the rpt-rs render pipeline
//!
//! Two pieces, split by weight:
//!
//! - [`FontDb`] (always compiled, `fontdb` only): the shared **face-resolution policy** the physical
//!   backends need — locate an OS face for a [`rpt_pages::FontSpec`] family (with bold/italic), then
//!   hand its bytes to the backend's own parser. Depend on this crate with `default-features = false`
//!   to get just this without the shaping stack.
//! - [`CosmicLayout`] / [`FontProvider`] (the default `cosmic` feature): the font-accurate
//!   [`rpt_layout::TextLayout`] backed by [cosmic-text] — real per-glyph `hmtx` advances (matching the
//!   native engine's GDI `GetCharWidthW`), Unicode/CJK line-breaking, bidi, and font fallback. Inject
//!   it via [`rpt_layout::layout_with`]. This is the metric-accurate, international upgrade over
//!   `rpt-layout`'s dependency-free `ApproxLayout`.
//!
//! [cosmic-text]: https://github.com/pop-os/cosmic-text

mod font_db;
pub use font_db::FontDb;

#[cfg(feature = "cosmic")]
mod cosmic;
#[cfg(feature = "cosmic")]
pub use cosmic::{CosmicLayout, FontProvider};
