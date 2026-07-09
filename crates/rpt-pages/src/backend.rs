//! The [`PageBackend`] trait — one uniform entry point every output backend implements.
//!
//! The four backends (SVG/PDF/raster/HTML) consume the same [`PagedDocument`] but historically each
//! exposed its own free-function surface (`render_page`, `render_pages`, `render_pages_with_assets`,
//! `render_page_dpi`, …) and the render facade called each by hand. [`PageBackend`] gives them a
//! common shape: a backend is a value whose [`render`](PageBackend::render) turns a document into that
//! backend's output type, with per-backend knobs carried in an [`Options`](PageBackend::Options)
//! struct instead of method-name suffixes.
//!
//! The trait lives here (not in the facade) because the backend crates depend on `rpt-pages`; a trait
//! in the facade would invert that arrow. The existing free functions stay — this is an additive seam,
//! not a replacement, so callers that want the plain function keep it.
//!
//! Whole-document input: [`render`](PageBackend::render) takes the whole [`PagedDocument`] (not just
//! its pages) so a backend that needs the out-of-band [`assets`](PagedDocument::assets) — the HTML
//! backend inlines images from them — reads them uniformly, without a separate `_with_assets` entry.
//! It takes `&self` (not an associated function) so a future stateful backend (e.g. one holding a
//! shared font system) fits the same trait without a breaking change.

use crate::PagedDocument;

/// A render backend: turns a [`PagedDocument`] into this backend's output bytes/strings, tuned by an
/// [`Options`](PageBackend::Options) value. Implemented by each `rpt-render-*` backend crate.
pub trait PageBackend {
    /// The backend's output (e.g. `String` for HTML, `Vec<u8>` for a single PDF, `Vec<Vec<u8>>` for
    /// one image per page).
    type Output;
    /// Per-backend knobs (writer choice, DPI, …). `Default` gives the backend's standard behaviour,
    /// so a caller with no special needs passes `&Default::default()`.
    type Options: Default;

    /// Render the whole document to this backend's output.
    fn render(&self, doc: &PagedDocument, opts: &Self::Options) -> Self::Output;
}
