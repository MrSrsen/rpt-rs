//! PDF output backend for the [`rpt_pages`] Page IR.
//!
//! Coordinate model: `pt = twips / 20` — see [`rpt_render_util`] for the cross-backend coordinate
//! reference (including the y-axis difference between the two writers below).
//!
//! Two writers share the public [`render_pages`] entry point:
//!
//! - The **krilla backend** (default, `krilla-backend` feature) drives [krilla] — typst's standalone
//!   PDF library — so text is drawn with **real TrueType/CFF font subset embedding** (`/Widths` +
//!   `/FontDescriptor`, Type0/CID for Unicode with a `/ToUnicode` map) and content streams are
//!   `/FlateDecode`-compressed. Fonts are located on the host by family name via [`fontdb`]. This is
//!   the fidelity path: it produces the same kind of PDF a real print pipeline would.
//! - The **basic writer** ([`render_pages_basic`]) is a self-contained, dependency-free fallback that
//!   emits a minimal PDF 1.4 with a single Helvetica base-14 font and uncompressed content streams.
//!   It is always available and is used when the `krilla-backend` feature is disabled (or if krilla
//!   fails to serialize a document).
//!
//! Both convert twips to PDF points as `pt = twips / 20`. Draw-op coordinates are printable-relative
//! (0-based); [`Page::origin`](rpt_pages::Page::origin) — the report margin — is added
//! to place content on the physical page. The two writers differ in coordinate handling only because
//! raw PDF is y-up (origin bottom-left) while krilla's surface is y-down (origin top-left, matching the
//! Page IR): the basic writer flips y and the krilla backend does not.
//!
//! Images are not embedded by either writer through this entry point — the resolved image bytes live
//! out-of-band from the Page IR and are not threaded through [`render_pages`].
//!
//! [krilla]: https://docs.rs/krilla

use rpt_pages::Page;

mod common;
mod writer_basic;
#[cfg(feature = "krilla-backend")]
mod writer_krilla;

#[cfg(test)]
mod tests;

pub use writer_basic::render_pages_basic;

/// Render a slice of pages to a single PDF document (bytes).
///
/// Uses the krilla backend when the `krilla-backend` feature is enabled (the default), falling back to
/// the dependency-free [`render_pages_basic`] writer if krilla is disabled or fails to serialize.
pub fn render_pages(pages: &[Page]) -> Vec<u8> {
    #[cfg(feature = "krilla-backend")]
    {
        if let Some(bytes) = writer_krilla::render(pages) {
            return bytes;
        }
    }
    render_pages_basic(pages)
}

/// Render one page to a single-page PDF.
pub fn render_page(page: &Page) -> Vec<u8> {
    render_pages(std::slice::from_ref(page))
}

/// Which writer [`PdfBackend`] uses.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PdfWriter {
    /// The krilla backend when the `krilla-backend` feature is on (real embedded fonts), falling back
    /// to the basic writer otherwise — the same policy as [`render_pages`].
    #[default]
    Auto,
    /// Force the dependency-free basic writer (Helvetica base-14, uncompressed).
    Basic,
}

/// Knobs for [`PdfBackend`].
#[derive(Debug, Default, Clone, Copy)]
pub struct PdfOptions {
    /// Which PDF writer to use (full-featured vs. the dependency-free basic writer).
    pub writer: PdfWriter,
}

/// The PDF backend as a [`rpt_pages::PageBackend`]: one multi-page PDF document. The [`render_pages`] /
/// [`render_pages_basic`] free functions stay available.
#[derive(Debug, Default, Clone, Copy)]
pub struct PdfBackend;

impl rpt_pages::PageBackend for PdfBackend {
    type Output = Vec<u8>;
    type Options = PdfOptions;

    fn render(&self, doc: &rpt_pages::PagedDocument, opts: &PdfOptions) -> Vec<u8> {
        match opts.writer {
            PdfWriter::Auto => render_pages(&doc.pages),
            PdfWriter::Basic => render_pages_basic(&doc.pages),
        }
    }
}
