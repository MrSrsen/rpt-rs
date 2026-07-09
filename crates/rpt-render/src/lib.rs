//! # rpt-render — the end-to-end render orchestrator
//!
//! Ties the whole stack together: a decoded [`Report`] → [`rpt_data`] pipeline
//! → [`rpt_layout`] → the [`rpt_pages`] Page IR, then out to a backend. `render(report)` returns
//! paginated pages a caller renders to SVG/PDF/raster/HTML.
//!
//! The data feed is the report's **saved data** when present (the offline path); with no saved
//! data, the pipeline runs over zero rows (headers/footers still format). A live feed can also be
//! supplied via a custom [`RowSource`] (`render_with`/`render_dataset_with`).
//!
//! ```no_run
//! use rpt_render::ReportDocument;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Load a report, render its saved data, and write the PDF bytes.
//! let doc = ReportDocument::load("report.rpt")?;
//! std::fs::write("report.pdf", doc.to_pdf())?;
//! # Ok(())
//! # }
//! ```

use rpt::model::Report;
use rpt_data::{build_dataset, compile_formulas, Dataset, EmptySource, RowSource, SavedDataSource};
use rpt_pages::PagedDocument;
use std::path::Path;

pub use rpt_data::{Parameters, ScopeData};

// Text-layout stack: the trait + dependency-free default from rpt-layout, and (with the `cosmic`
// feature) the font-accurate cosmic-text impl + font configuration, re-exported for callers who
// want to build their own layout for `render_dataset_with`.
pub use rpt_layout::{ApproxLayout, Locale, TextLayout};
#[cfg(feature = "cosmic")]
pub use rpt_text::{CosmicLayout, FontProvider};

/// An SDK-shaped facade over the load → model → render/export flow, mirroring
/// `CrystalDecisions.CrystalReports.Engine.ReportDocument`: **one object that loads a report, holds
/// its model, and exports it.** It owns an [`rpt::Rpt`] and delegates rendering to the free
/// functions in this crate — so the crate layering is untouched (`rpt` stays pure I/O; the
/// dependency arrow points one way, `rpt-render` → `rpt`). Method names echo the SDK's
/// `Load`/`ExportToDisk` while staying Rust-idiomatic (`Result`, not exceptions).
///
/// This is *optional sugar* for SDK-familiar callers; the free functions ([`render`], [`render_pdf`],
/// …) and the layered crates remain the primary API.
#[derive(Debug)]
pub struct ReportDocument {
    rpt: rpt::Rpt,
}

impl ReportDocument {
    /// SDK: `ReportDocument.Load(path)`.
    ///
    /// ```no_run
    /// use rpt_render::ReportDocument;
    /// let doc = ReportDocument::load("report.rpt")?;
    /// let _report = doc.report();
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn load(path: impl AsRef<Path>) -> rpt::Result<ReportDocument> {
        Ok(ReportDocument {
            rpt: rpt::Rpt::open(path)?,
        })
    }

    /// The decoded report model (SDK: the `ReportDocument`'s own object graph).
    pub fn report(&self) -> &Report {
        self.rpt.report()
    }

    /// The underlying [`rpt::Rpt`] (stream access, saved data, re-save).
    pub fn inner(&self) -> &rpt::Rpt {
        &self.rpt
    }

    /// Render to the Page IR from the report's saved data (SDK: format for view/print). The
    /// zero-config path — never fails.
    ///
    /// ```no_run
    /// # use rpt_render::ReportDocument;
    /// # fn demo(doc: &ReportDocument) {
    /// let pages = doc.render();
    /// println!("{} page(s)", pages.pages.len());
    /// # }
    /// ```
    pub fn render(&self) -> PagedDocument {
        render(self.report())
    }

    /// Render to the Page IR with explicit [`RenderOptions`] — a live datasource, parameter values, a
    /// locale, and/or a subreport scope (SDK analogue: `SetDataSource` + parameters + refresh).
    ///
    /// The [`datasource`](RenderOptions::datasource) picks where rows come from — the report's saved
    /// data by default, or a custom [`RowSource`]:
    ///
    /// ```no_run
    /// use rpt_render::{RenderOptions, RenderSource, ReportDocument};
    /// use rpt_data::EmptySource;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let doc = ReportDocument::load("report.rpt")?;
    ///
    /// // Default: render the report's own saved data.
    /// let from_saved = doc.render_with(RenderOptions::default())?;
    ///
    /// // Or feed rows from a custom `RowSource` (a live DB feed, an in-memory source, …).
    /// let source = EmptySource;
    /// let from_rows = doc.render_with(RenderOptions {
    ///     datasource: RenderSource::Rows(&source),
    ///     ..Default::default()
    /// })?;
    /// # let _ = (from_saved, from_rows);
    /// # Ok(())
    /// # }
    /// ```
    pub fn render_with(&self, opts: RenderOptions) -> Result<PagedDocument, RenderError> {
        render_with(self.report(), opts)
    }

    /// SDK: `ExportToDisk(ExportFormatType.PortableDocFormat, path)`.
    pub fn export_pdf_to_disk(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        std::fs::write(path, render_pdf(self.report()))
    }

    /// SDK: `ExportToDisk(ExportFormatType.HTML40, path)` (single self-contained document).
    pub fn export_html_to_disk(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        std::fs::write(path, render_html(self.report()))
    }

    /// The full report as PDF bytes (SDK: `ExportToStream(PortableDocFormat)`).
    ///
    /// ```no_run
    /// # use rpt_render::ReportDocument;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let doc = ReportDocument::load("report.rpt")?;
    /// std::fs::write("report.pdf", doc.to_pdf())?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn to_pdf(&self) -> Vec<u8> {
        render_pdf(self.report())
    }

    /// The full report as one self-contained HTML document.
    pub fn to_html(&self) -> String {
        render_html(self.report())
    }

    /// One standalone SVG per page.
    pub fn export_svg_pages(&self) -> Vec<String> {
        render_svg_pages(self.report())
    }
}

/// Where [`render_with`] gets its rows.
///
/// The default is [`Saved`](RenderSource::Saved) — the report's own saved data — so
/// `RenderOptions::default()` is the zero-config render. [`Rows`](RenderSource::Rows) feeds a live or
/// custom [`RowSource`]; [`Dataset`](RenderSource::Dataset) hands in a pipeline result the caller
/// already built (its own params/grouping are used as-is, so [`RenderOptions::params`] is ignored for
/// that variant).
#[derive(Default, Clone, Copy)]
pub enum RenderSource<'a> {
    /// The report's saved data if present, else no rows (only static bands format).
    #[default]
    Saved,
    /// A live or custom [`RowSource`] (a DB feed, an in-memory source, …). Report parameters and the
    /// datasource itself are applied by [`render_with`].
    Rows(&'a dyn RowSource),
    /// A [`Dataset`] the caller already built (skips the record pipeline). Its own params are used;
    /// [`RenderOptions::params`] is ignored for this variant.
    Dataset(&'a Dataset),
}

impl std::fmt::Debug for RenderSource<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderSource::Saved => f.write_str("Saved"),
            RenderSource::Rows(_) => f.write_str("Rows(..)"),
            RenderSource::Dataset(_) => f.write_str("Dataset(..)"),
        }
    }
}

/// Everything [`render_with`] needs beyond the report itself: the datasource, report parameter
/// values, the render locale, and an optional subreport-scope provider. [`Default`] is the zero-config
/// render (saved data, no parameters, en-US locale, offline subreports) — the same as [`render`].
#[derive(Default)]
pub struct RenderOptions<'a> {
    /// Where rows come from. Default: the report's saved data.
    pub datasource: RenderSource<'a>,
    /// Report parameter current-values, so formulas referencing `{?Name}` resolve. Ignored when
    /// [`datasource`](RenderOptions::datasource) is [`RenderSource::Dataset`] (the dataset carries its
    /// own params). See [`Parameters`] / [`rpt_data::normalize_param_name`].
    pub params: Parameters,
    /// The render locale (the `--locale`/host locale), merged with each field's stored format leaf to
    /// produce the effective display format. Default: en-US.
    pub locale: Locale,
    /// A [`ScopeData`] provider so subreports render from live data (their scope's rows) instead of
    /// only their saved data. `None` keeps the offline behaviour.
    pub scope: Option<&'a dyn ScopeData>,
}

impl std::fmt::Debug for RenderOptions<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderOptions")
            .field("datasource", &self.datasource)
            .field("params", &self.params)
            .field("locale", &self.locale)
            .field("scope", &self.scope.map(|_| "..").unwrap_or("None"))
            .finish()
    }
}

/// Render a report to the paginated Page IR using its saved data (if any). The zero-config path:
/// equivalent to [`render_with`] with [`RenderOptions::default`], and infallible.
pub fn render(report: &Report) -> PagedDocument {
    render_options(report, RenderOptions::default())
}

/// The single options-driven entry point: render a report with an explicit datasource, parameters,
/// locale, and/or subreport scope (see [`RenderOptions`]).
///
/// Fallible because the datasource can fail: the built-in [`RenderSource`] variants (saved data, an
/// already-materialized [`RowSource`], a pre-built [`Dataset`]) always succeed, and the render CLI's
/// live-DB fetch surfaces its failures through the same [`RenderError`]. For the always-infallible
/// zero-config render use [`render`].
pub fn render_with(report: &Report, opts: RenderOptions) -> Result<PagedDocument, RenderError> {
    Ok(render_options(report, opts))
}

/// The shared body behind [`render`], [`render_with`], and the deprecated wrappers: resolve the
/// datasource to a [`Dataset`] (attaching parameters), then lay it out. Infallible — the built-in
/// sources cannot fail.
fn render_options(report: &Report, opts: RenderOptions) -> PagedDocument {
    let RenderOptions {
        datasource,
        params,
        locale,
        scope,
    } = opts;
    match datasource {
        RenderSource::Dataset(dataset) => layout_dataset(report, dataset, scope, locale),
        RenderSource::Rows(source) => {
            let mut dataset = build_dataset(source, &report.data_definition);
            dataset.params = params;
            layout_dataset(report, &dataset, scope, locale)
        }
        RenderSource::Saved => {
            let saved_holder;
            let source: &dyn RowSource = match &report.saved_data {
                Some(saved) => {
                    saved_holder = SavedDataSource::from_report(saved, report);
                    &saved_holder
                }
                None => &EmptySource,
            };
            let mut dataset = build_dataset(source, &report.data_definition);
            dataset.params = params;
            layout_dataset(report, &dataset, scope, locale)
        }
    }
}

/// Compile the report's formulas and lay out a [`Dataset`] with the default text layout — the last
/// step shared by every non-BYO-layout entry point.
fn layout_dataset(
    report: &Report,
    dataset: &Dataset,
    scope: Option<&dyn ScopeData>,
    locale: Locale,
) -> PagedDocument {
    let formulas = compile_formulas(&report.data_definition);
    rpt_layout::layout_scoped(
        report,
        dataset,
        &formulas,
        default_text_layout(),
        scope,
        locale,
    )
}

/// Render a pre-built [`Dataset`] with an explicit [`TextLayout`] (font stack). Lets a caller reuse
/// a `CosmicLayout` (avoids re-scanning fonts per render) or inject host-supplied fonts on WASM. This
/// is the bring-your-own-layout entry [`RenderOptions`] does not model, so it stays as a free
/// function.
pub fn render_dataset_with(
    report: &Report,
    dataset: &Dataset,
    text_layout: Box<dyn TextLayout>,
) -> PagedDocument {
    let formulas = compile_formulas(&report.data_definition);
    rpt_layout::layout_with(report, dataset, &formulas, text_layout)
}

/// The default text layout for this build: font-accurate cosmic-text (feature `cosmic`, on by
/// default) using the OS fonts, else the dependency-free approximate layout.
fn default_text_layout() -> Box<dyn TextLayout> {
    #[cfg(feature = "cosmic")]
    {
        Box::new(rpt_text::CosmicLayout::with_system_fonts())
    }
    #[cfg(not(feature = "cosmic"))]
    {
        Box::new(rpt_layout::ApproxLayout)
    }
}

/// The uniform backend trait plus the four concrete backends and their option structs, re-exported so
/// a caller can drive any output through [`render_backend`] without depending on each backend crate.
pub use rpt_pages::PageBackend;
pub use rpt_render_html::{HtmlBackend, HtmlOptions};
pub use rpt_render_pdf::{PdfBackend, PdfOptions, PdfWriter};
pub use rpt_render_raster::{RasterBackend, RasterOptions};
pub use rpt_render_svg::SvgBackend;

/// Render a [`PagedDocument`] through any [`PageBackend`] — the trait seam over the concrete
/// `render_*` functions. Lets a caller pick a backend as a value (e.g. from a CLI flag) and pass its
/// [`Options`](PageBackend::Options), instead of matching on a format and calling each free function
/// by hand.
pub fn render_backend<B: PageBackend>(
    doc: &PagedDocument,
    backend: &B,
    opts: &B::Options,
) -> B::Output {
    backend.render(doc, opts)
}

// The named format helpers all go through the one [`render_backend`]/[`PageBackend`] seam (proven
// byte-identical to the backend free functions by the `render_backend_seam_matches_free_functions`
// test), so there is a single documented render path rather than three parallel ones.

/// Render every page to a standalone SVG string (one per page), in order.
pub fn render_svg_pages(report: &Report) -> Vec<String> {
    render_backend(&render(report), &SvgBackend, &())
}

/// Render the whole report to a single multi-page PDF document (bytes).
pub fn render_pdf(report: &Report) -> Vec<u8> {
    render_backend(&render(report), &PdfBackend, &PdfOptions::default())
}

/// Render the whole report to a single self-contained HTML document.
pub fn render_html(report: &Report) -> String {
    render_backend(&render(report), &HtmlBackend, &HtmlOptions)
}

/// The normalized Page-IR JSON for every page — the surface the render-parity tooling consumes to
/// diff our layout against a reference.
pub fn render_ir_json(report: &Report) -> Vec<String> {
    render(report)
        .pages
        .iter()
        .map(|p| p.to_normalized_json())
        .collect()
}

/// A render failure with a typed cause, so a caller can tell a datasource problem from a parameter,
/// database, or output failure — instead of matching on message strings. `#[from] rpt::Error` lets a
/// decode error propagate with `?`; the live-DB driver errors ([`rpt_db_postgres::DbError`] /
/// [`rpt_db_sqlite::DbError`]) are absorbed into [`Db`](RenderError::Db) so the portable core stays
/// free of the native DB crates when their features are off.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RenderError {
    /// Opening or decoding the `.rpt` failed.
    #[error(transparent)]
    Rpt(#[from] rpt::Error),
    /// Resolving the datasource failed: a connection URL's scheme/format, a missing connection URL,
    /// an unimplemented driver, or a scope with no live table.
    #[error("datasource error: {0}")]
    Datasource(String),
    /// A report parameter value could not be coerced to its declared type.
    #[error("parameter error: {0}")]
    Params(String),
    /// A database driver failed (connection, healthcheck, or query).
    #[error("database error: {0}")]
    Db(String),
    /// Writing the rendered output failed.
    #[error("output error: {0}")]
    Io(String),
}

#[cfg(feature = "db-postgres")]
impl From<rpt_db_postgres::DbError> for RenderError {
    fn from(e: rpt_db_postgres::DbError) -> RenderError {
        RenderError::Db(e.to_string())
    }
}

#[cfg(feature = "db-sqlite")]
impl From<rpt_db_sqlite::DbError> for RenderError {
    fn from(e: rpt_db_sqlite::DbError) -> RenderError {
        RenderError::Db(e.to_string())
    }
}
