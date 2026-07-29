//! PDF output backend for the [`rpt_pages`] Page IR.
//!
//! Coordinate model: `pt = twips / 20` — see [`rpt_render_util`] for the cross-backend coordinate
//! reference.
//!
//! The single writer drives [krilla] — typst's standalone PDF library — so text is drawn with **real
//! TrueType/CFF font subset embedding** (`/Widths` + `/FontDescriptor`, Type0/CID for Unicode with a
//! `/ToUnicode` map) and content streams are `/FlateDecode`-compressed. Fonts are located by family
//! name via [`fontdb`], with the bundled fallback face covering what they lack.
//!
//! Which faces exist is the caller's choice, via [`PdfOptions::fonts`]: the crate's own bundled faces
//! by default, so the bytes do not depend on this machine's installed set — the only source a committed
//! PDF baseline can be blessed against — or [`FontSource::System`] to resolve and subset the host's
//! library instead (an installed Arial embeds Arial). It reaches the writer through [`PdfBackend`] or
//! [`try_render_pages_with_options`]; the entry points that take no options use the default.
//!
//! Draw-op coordinates are printable-relative (0-based); [`Page::origin`](rpt_pages::Page::origin) —
//! the report margin — is added to place content on the physical page. krilla's surface is y-down with
//! a top-left origin, the same convention as the Page IR, so nothing flips y.
//!
//! Images: each picture whose bytes are present in the document's out-of-band
//! [`assets`](rpt_pages::PagedDocument::assets) is embedded — reached through the [`PdfBackend`]
//! whole-document entry point, which is the only path that carries them. The pages-only
//! [`render_pages`] free function has no assets, so it draws no pictures.
//!
//! Archival and accessibility conformance: [`PdfOptions::conformance`] exports against a PDF/A or
//! PDF/UA level instead of an ordinary PDF. Off by default, because the standards forbid things an
//! ordinary report may legally contain — see [`Conformance`].
//!
//! Tagging: [`PdfOptions::tagged`] adds a **structure tree** — what the marks mean and in what order
//! they are read — reconstructed from the Page IR's object refs, paint order and band dictionary (see
//! the `tagging` module). Also off by default; the accessible conformance levels imply it, and they
//! additionally require the [`Semantics`] the Page IR cannot supply. They also need the band
//! dictionary, so they are refused on the pages-only entry points — render a whole document with
//! [`try_render_document`].
//!
//! Provenance: every document names its producer ([`RPT_RS_PRODUCER`] by default), so a file can be
//! traced back to the engine that wrote it. The identity is a build constant, never a clock or a
//! host name, which is what keeps a render byte-reproducible — see [`Producer`].
//!
//! Failure: serialization can fail on a resource krilla will not embed (a host font with no outline
//! table, a raster whose pixels do not decode) or on a document that does not meet the requested
//! conformance level. [`try_render_pages`] reports it as a [`PdfError`]; the infallible entry points
//! return a one-page PDF naming the failure, so a failed render is a document that says so rather
//! than one that silently looks ordinary.
//!
//! [krilla]: https://docs.rs/krilla

use rpt_pages::{ImageAsset, Page, PagedDocument};
use std::collections::BTreeMap;

/// Re-exported so choosing the font library needs no direct `rpt-text` dependency: it is a field of
/// [`PdfOptions`], hence part of this crate's surface.
pub use rpt_text::FontSource;

mod common;
mod fill;
mod tagging;
mod writer_krilla;

#[cfg(test)]
mod tests;

/// Why a PDF could not be serialized.
///
/// Every case is krilla declining a resource the document references, or the document failing the
/// requested [`Conformance`]. There is no second, degraded writer to absorb it: a PDF with
/// substituted fonts and no images is not an acceptable stand-in for the requested one, so the
/// failure is reported instead.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PdfError {
    /// A font the document draws with could not be embedded (no outline table, subsetting failed).
    #[error("PDF font embedding failed: {0}")]
    Font(String),
    /// An image the document places could not be encoded (its pixels did not decode).
    #[error("PDF image encoding failed: {0}")]
    Image(String),
    /// The document did not meet the requested archival standard, with one message per unmet
    /// requirement. Reported instead of the bytes: a file that claims conformance it does not have is
    /// worse than no file.
    #[error("{level} conformance failed: {}", .reasons.join("; "))]
    Conformance {
        /// The standard the render was exported against.
        level: Conformance,
        /// One message per requirement the document did not satisfy.
        reasons: Vec<String>,
    },
    /// krilla declined the document for another reason.
    #[error("PDF serialization failed: {0}")]
    Serialize(String),
}

/// Render a slice of pages to a single PDF document (bytes).
///
/// Infallible: a serialization failure yields a one-page PDF naming it (see [`try_render_pages`] to
/// handle the failure instead).
pub fn render_pages(pages: &[Page]) -> Vec<u8> {
    render_pages_with_assets(pages, &BTreeMap::new())
}

/// Like [`render_pages`], but reports a serialization failure rather than rendering it onto a page.
///
/// # Errors
///
/// [`PdfError`] if krilla declines a resource the pages reference — see
/// [`try_render_pages_with_assets`].
pub fn try_render_pages(pages: &[Page]) -> Result<Vec<u8>, PdfError> {
    try_render_pages_with_assets(pages, &BTreeMap::new())
}

/// Like [`render_pages`], but embeds each image op whose `image_id` resolves in `assets`. The
/// [`PdfBackend`] whole-document entry point calls this with the document's assets.
///
/// # Panics
///
/// Never in practice. A serialization failure produces the one-page failure PDF instead, and that page
/// is a few text runs in the crate's own bundled face with no images — the two things krilla can
/// decline (a host font, a report asset) are both absent from it, so its own serialization has nothing
/// left to fail on. If it somehow does, the panic names the original failure rather than hiding it.
pub fn render_pages_with_assets(pages: &[Page], assets: &BTreeMap<String, ImageAsset>) -> Vec<u8> {
    or_failure_page(try_render_pages_with_assets(pages, assets))
}

/// The infallible entry points' shared tail: the rendered bytes, or the one-page PDF naming the
/// failure.
fn or_failure_page(result: Result<Vec<u8>, PdfError>) -> Vec<u8> {
    result.unwrap_or_else(|err| writer_krilla::render_failure_page(&err))
}

/// Like [`render_pages_with_assets`], but reports a serialization failure rather than rendering it
/// onto a page — the entry point for a caller that surfaces the failure itself (the CLI).
///
/// # Errors
///
/// [`PdfError`] if krilla declines a resource this document references: a host font it cannot embed
/// ([`PdfError::Font`]) or an asset whose pixels do not decode ([`PdfError::Image`]). Neither is a
/// property of the Page IR — both come from the host's fonts and the report's stored image bytes — so
/// neither can be ruled out by construction. Page geometry cannot fail: a degenerate page size is
/// clamped rather than abandoning the document.
pub fn try_render_pages_with_assets(
    pages: &[Page],
    assets: &BTreeMap<String, ImageAsset>,
) -> Result<Vec<u8>, PdfError> {
    try_render_pages_with_options(pages, assets, &PdfOptions::default())
}

/// Like [`try_render_pages_with_assets`], but with explicit [`PdfOptions`] — the free-function form of
/// the [`PdfBackend`] seam, for a caller that has pages rather than a whole document and wants to
/// choose the [`FontSource`] (a hermetic baseline render, a fontless host) or a [`Conformance`]
/// level.
///
/// # Errors
///
/// [`PdfError`], on the same resources as [`try_render_pages_with_assets`], plus
/// [`PdfError::Conformance`] if `opts` requests an archival level this document does not meet.
pub fn try_render_pages_with_options(
    pages: &[Page],
    assets: &BTreeMap<String, ImageAsset>,
    opts: &PdfOptions,
) -> Result<Vec<u8>, PdfError> {
    writer_krilla::render(pages, assets, &BTreeMap::new(), opts)
}

/// Render a whole [`PagedDocument`] — the entry point that sees everything the producer recorded, and
/// the only one a tagged or conforming render should use.
///
/// Beyond the pages and their assets, a document carries its
/// [`sections`](PagedDocument::sections): what band each section is, which is what tells a structure
/// tree that a page header is running furniture rather than content read out on all 42 pages. The
/// pages-only entry points cannot know it, so a [`Conformance`] level requiring tagging is refused
/// there unless the caller classifies the bands itself through [`Semantics::artifact_sections`].
///
/// # Errors
///
/// [`PdfError`], on the same resources as [`try_render_pages_with_assets`], plus
/// [`PdfError::Conformance`] if `opts` requests a level this document does not meet.
pub fn try_render_document(doc: &PagedDocument, opts: &PdfOptions) -> Result<Vec<u8>, PdfError> {
    writer_krilla::render(&doc.pages, &doc.assets, &doc.sections, opts)
}

/// Render one page to a single-page PDF.
pub fn render_page(page: &Page) -> Vec<u8> {
    render_pages(std::slice::from_ref(page))
}

/// The archival or accessibility standard a render is exported against.
///
/// A level is a claim that is checked, not a flag: a document that does not meet it fails the render
/// with [`PdfError::Conformance`] rather than being written with a conformance claim it does not
/// honour. The levels differ in what they forbid, so the choice is not just "newer is better".
///
/// **The level B (basic) archival levels** require a self-contained, reproducible file — embedded
/// fonts, device-independent color, an output intent, XMP metadata — but no accessibility semantics,
/// so a report needs no structure tree to conform:
///
/// - [`PdfA1b`](Conformance::PdfA1b) is PDF 1.4 and **forbids transparency** — any translucent
///   color in the report, and any 16-bit image, fails it. The most portable, the most restrictive.
/// - [`PdfA2b`](Conformance::PdfA2b) is PDF 1.7 and permits transparency and JPEG2000. The usual
///   default for archiving.
/// - [`PdfA3b`](Conformance::PdfA3b) is PDF/A-2b plus the right to embed arbitrary attachments,
///   which this backend does not emit; it exists for archives that mandate the -3 family.
///
/// All of them forbid a `.notdef` glyph, a CMYK color without an ICC profile, and PostScript
/// functions, none of which this backend produces from a well-formed Page IR.
///
/// **The level A (accessible) levels and PDF/UA-1** additionally require a **structure tree** — the
/// document's logical content and reading order, with page furniture marked as artifacts. That tree
/// is reconstructed from the Page IR (see [`Semantics`] for what the IR cannot supply and the caller
/// must):
///
/// - [`PdfA1a`](Conformance::PdfA1a) / [`PdfA2a`](Conformance::PdfA2a) /
///   [`PdfA3a`](Conformance::PdfA3a) are the level-B levels plus tagging, a document language, and
///   alternate text on every figure.
/// - [`PdfUa1`](Conformance::PdfUa1) (ISO 14289-1) is the accessibility standard proper. It is not
///   archival — it mandates no output intent — but it does mandate a document **title**, and the
///   viewer must show it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Conformance {
    /// No conformance claim: an ordinary PDF, and the only setting that leaves the bytes untouched.
    #[default]
    None,
    /// PDF/A-1b (ISO 19005-1 level B).
    PdfA1b,
    /// PDF/A-2b (ISO 19005-2 level B).
    PdfA2b,
    /// PDF/A-3b (ISO 19005-3 level B).
    PdfA3b,
    /// PDF/A-1a (ISO 19005-1 level A) — PDF/A-1b plus a structure tree.
    PdfA1a,
    /// PDF/A-2a (ISO 19005-2 level A) — PDF/A-2b plus a structure tree.
    PdfA2a,
    /// PDF/A-3a (ISO 19005-3 level A) — PDF/A-3b plus a structure tree.
    PdfA3a,
    /// PDF/UA-1 (ISO 14289-1), the universal-accessibility standard.
    PdfUa1,
}

impl Conformance {
    /// The standard's name, as it is written in the document's XMP metadata.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "no",
            Self::PdfA1b => "PDF/A-1B",
            Self::PdfA2b => "PDF/A-2B",
            Self::PdfA3b => "PDF/A-3B",
            Self::PdfA1a => "PDF/A-1A",
            Self::PdfA2a => "PDF/A-2A",
            Self::PdfA3a => "PDF/A-3A",
            Self::PdfUa1 => "PDF/UA-1",
        }
    }

    /// Whether this level requires a populated structure tree, and so turns tagging on.
    pub fn requires_tagging(self) -> bool {
        matches!(
            self,
            Self::PdfA1a | Self::PdfA2a | Self::PdfA3a | Self::PdfUa1
        )
    }

    /// Whether this level requires the document to carry a title.
    pub fn requires_title(self) -> bool {
        self == Self::PdfUa1
    }
}

/// What a page-furniture section's marks are, as PDF names them.
///
/// PDF/UA-1 §7.8 asks that running headers and footers be marked as the corresponding artifact, and
/// the level-A standards ask the same of page numbers. A whole-document render reads the band from
/// [`PagedDocument::sections`] and assigns the role itself — which band maps to which artifact is
/// this backend's policy, not something the Page IR states. A caller that disagrees, or that renders
/// bare pages, names them through [`Semantics::artifact_sections`] instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactRole {
    /// A running page header.
    Header,
    /// A running page footer.
    Footer,
    /// Pagination furniture that is neither (a watermark band, a page-number-only section).
    Pagination,
}

/// The document semantics a structure tree needs and the Page IR does not carry.
///
/// The Page IR is a flat list of positioned marks. Two things a *conforming* structure tree needs are
/// genuinely absent from it, and neither can be inferred soundly, so both are supplied by the caller
/// — which, unlike this backend, holds the decoded report:
///
/// - **A natural language.** Needed to determine the language of every text run. Never guessed: a
///   French report declared `en` is read aloud in the wrong voice.
/// - **Alternate text.** Nothing in the IR describes a picture or a chart. A conforming document must
///   describe every figure, so a report with a picture and no [`alt_text`](Self::alt_text) entry for
///   it fails the render with [`PdfError::Conformance`] naming the object rather than being written
///   with an empty description a screen reader would announce as an unlabelled graphic. An entry
///   present but **empty** is the HTML `alt=""` convention: the caller looked, and the graphic is
///   decorative, so it is emitted as an artifact instead of a figure.
///
/// Which band a section is decides whether a run of text is document content or the running header
/// repeated on every page, and the stored section name (`Section3`, `TSection7`) does not say. A
/// [`PagedDocument`] carries that classification directly, so
/// [`artifact_sections`](Self::artifact_sections) is only an **override**.
///
/// [`Default`] supplies none of it, which is why it cannot claim a level and can still be tagged.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Semantics {
    /// The document's title. Required by PDF/UA-1, which also makes the viewer display it.
    pub title: Option<String>,
    /// The document's natural language as an RFC 3066 tag (`en-US`). Required by the level-A
    /// standards, and what makes every text run's language determinable under PDF/UA-1 §7.2.
    pub language: Option<String>,
    /// Alternate text describing a picture or chart, keyed by the report object's name. An empty
    /// string marks the graphic decorative.
    pub alt_text: BTreeMap<String, String>,
    /// The sections whose content is page furniture rather than document content, keyed by the
    /// section name as it appears in [`rpt_pages::ObjectRef::section`].
    ///
    /// An **override**: `Some` replaces the classification derived from
    /// [`PagedDocument::sections`] wholesale, for a caller that disagrees with the document or that
    /// renders bare pages. `Some` of an empty map therefore says "classified, and nothing is
    /// furniture" — which is the one way a pages-only render can still claim a level that requires
    /// tagging. `None` defers to the document.
    pub artifact_sections: Option<BTreeMap<String, ArtifactRole>>,
}

impl std::fmt::Display for Conformance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A UTC instant, written as the document's creation and modification date.
///
/// Out-of-range components are clamped rather than rejected. [`Default`] is the Unix epoch — the
/// conventional "date not known", and what a conforming render falls back to so that its bytes stay
/// reproducible without reading a clock (which a WASM build does not have).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    /// Year, e.g. 2026.
    pub year: u16,
    /// Month, 1–12.
    pub month: u8,
    /// Day of month, 1–31.
    pub day: u8,
    /// Hour, 0–23.
    pub hour: u8,
    /// Minute, 0–59.
    pub minute: u8,
    /// Second, 0–59.
    pub second: u8,
}

impl Default for Timestamp {
    fn default() -> Self {
        Self {
            year: 1970,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        }
    }
}

/// This crate's own identity, as written into a document it produces: the name plus the workspace
/// version, from the single source every crate inherits (`[workspace.package] version`).
pub const RPT_RS_PRODUCER: &str = concat!("rpt-rs ", env!("CARGO_PKG_VERSION"));

/// The software a document names as having produced it — `/Producer` and `/Creator` in the info
/// dictionary, `pdf:Producer` and `xmp:CreatorTool` in the XMP packet.
///
/// A version string is a property of the build, not of the run, so naming one costs nothing in
/// reproducibility: the same input renders the same bytes until the version itself changes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum Producer {
    /// [`RPT_RS_PRODUCER`] — this crate produced the file, and says so.
    #[default]
    RptRs,
    /// The embedding application's own identity instead, for a caller that ships this backend under
    /// its own name.
    Named(std::borrow::Cow<'static, str>),
    /// Name nothing. The document then carries no producer or creator at all.
    Anonymous,
}

impl Producer {
    /// The string to write, or `None` for [`Producer::Anonymous`].
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::RptRs => Some(RPT_RS_PRODUCER),
            Self::Named(name) => Some(name),
            Self::Anonymous => None,
        }
    }
}

/// Knobs for [`PdfBackend`] — the [`PageBackend`](rpt_pages::PageBackend) options seam.
///
/// [`Default`] is the bundled face library and no conformance claim, so an ordinary render's bytes
/// are a property of the document rather than of this machine; set [`fonts`](PdfOptions::fonts) to
/// [`FontSource::System`] to resolve and subset the report's real faces from the host instead, and
/// [`conformance`](PdfOptions::conformance) to export an archival PDF.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PdfOptions {
    /// Which face library text is resolved, shaped and subset from.
    pub fonts: FontSource,
    /// The archival or accessibility standard to export against, and to fail the render on if unmet.
    pub conformance: Conformance,
    /// Emit a structure tree — what the marks mean and in what order they are read — without claiming
    /// any standard. Implied by a [`conformance`](PdfOptions::conformance) level that requires
    /// tagging; on its own it makes the document more usable to assistive technology and to
    /// text extraction, and asserts nothing.
    pub tagged: bool,
    /// The document semantics a structure tree needs and the Page IR does not carry. Its title and
    /// language are ordinary document metadata, written whenever they are set; the rest only shapes a
    /// structure tree, so it is inert unless the document is tagged.
    pub semantics: Semantics,
    /// The document's creation date. `None` writes no date at all for an ordinary render, and the
    /// [`Timestamp::default()`] epoch for a conforming one, which every PDF/A level requires.
    pub created: Option<Timestamp>,
    /// The software the document names as its producer and creator.
    pub producer: Producer,
}

/// The PDF backend as a [`rpt_pages::PageBackend`]: one multi-page PDF document. The
/// [`render_pages`] / [`try_render_pages_with_assets`] free functions stay available.
#[derive(Debug, Default, Clone, Copy)]
pub struct PdfBackend;

impl rpt_pages::PageBackend for PdfBackend {
    type Output = Vec<u8>;
    type Options = PdfOptions;

    fn render(&self, doc: &PagedDocument, opts: &PdfOptions) -> Vec<u8> {
        or_failure_page(try_render_document(doc, opts))
    }
}
