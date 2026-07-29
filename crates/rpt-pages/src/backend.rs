//! The [`PageBackend`] trait — one uniform entry point every output backend implements.
//!
//! A backend is a value whose [`render`](PageBackend::render) turns a [`PagedDocument`] into that
//! backend's output type, with its knobs carried in an [`Options`](PageBackend::Options) struct
//! instead of method-name suffixes. That uniform shape is what lets the render facade — or an
//! out-of-tree consumer such as a viewer — drive output without naming a concrete backend.
//!
//! The trait lives here (not in the facade) because the backend crates depend on `rpt-pages`; a trait
//! in the facade would invert that arrow. The existing free functions stay — this is an additive seam,
//! not a replacement, so callers that want the plain function keep it.
//!
//! Whole-document input: [`render`](PageBackend::render) takes the whole [`PagedDocument`] (not just
//! its pages) so a backend that needs the out-of-band [`assets`](PagedDocument::assets) — an image-
//! embedding backend reads its bytes from them — reads them uniformly, without a separate
//! `_with_assets` entry.
//! It takes `&self` (not an associated function) so a stateful backend (e.g. one holding a shared
//! font system) fits the same trait without a breaking change.

use crate::PagedDocument;

/// A render backend: turns a [`PagedDocument`] into this backend's output bytes/strings, tuned by an
/// [`Options`](PageBackend::Options) value. Implemented by each `rpt-render-*` backend crate.
pub trait PageBackend {
    /// The backend's output (e.g. `Vec<u8>` for a single PDF, `Vec<String>` for one document per
    /// page).
    type Output;
    /// Per-backend knobs (writer choice, DPI, …). `Default` gives the backend's standard behaviour,
    /// so a caller with no special needs passes `&Default::default()`.
    type Options: Default;

    /// Render the whole document to this backend's output.
    fn render(&self, doc: &PagedDocument, opts: &Self::Options) -> Self::Output;
}
