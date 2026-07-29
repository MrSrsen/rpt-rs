//! # rpt-render — the end-to-end render orchestrator
//!
//! Ties the whole stack together: a decoded [`Report`] → [`rpt_data`] pipeline
//! → [`rpt_layout`] → the [`rpt_pages`] Page IR, then out to a backend. `render(report)` returns
//! paginated pages, which [`render_pdf`] turns into a PDF document.
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

use rpt_data::{
    build_dataset_opts, compile_formulas_at, CollectingSink, Dataset, DatasetOptions, EmptySource,
    RowSource, SavedDataSource,
};
use rpt_pages::PagedDocument;
use rpt_reader::model::Report;
use std::path::{Path, PathBuf};

pub use rpt_data::{DateTimeSpecials, Parameters, ScopeData};

// Text-layout stack: the trait and the dependency-free approximate impl from rpt-layout, plus the
// font-accurate cosmic-text impl plus font configuration, re-exported for callers who want to build
// their own layout for `render_dataset_with`.
/// The bridge from the data pipeline's diagnostics to the Page IR's one vocabulary, re-exported so a
/// caller that builds its own [`Dataset`] (and so collects its own pipeline diagnostics) can convert
/// them without depending on `rpt-layout` directly.
pub use rpt_layout::diagnostics as data_diagnostics;
pub use rpt_layout::{ApproxLayout, Locale, TextLayout};
/// The font inventory types and the directories a system scan reads, re-exported so a caller can
/// report what a render would resolve without depending on `rpt-text` directly.
pub use rpt_text::{system_font_dirs, FaceReport, FontInventory};
pub use rpt_text::{CosmicLayout, FontProvider};

/// An SDK-shaped facade over the load → model → render/export flow, mirroring
/// `CrystalDecisions.CrystalReports.Engine.ReportDocument`: **one object that loads a report, holds
/// its model, and exports it.** It owns an [`rpt_reader::Rpt`] and delegates rendering to the free
/// functions in this crate — so the crate layering is untouched (`rpt-reader` stays pure I/O; the
/// dependency arrow points one way, `rpt-render` → `rpt-reader`). Method names echo the SDK's
/// `Load`/`ExportToDisk` while staying Rust-idiomatic (`Result`, not exceptions).
///
/// This is *optional sugar* for SDK-familiar callers; the free functions ([`render`], [`render_pdf`],
/// …) and the layered crates remain the primary API.
#[derive(Debug)]
pub struct ReportDocument {
    rpt: rpt_reader::Rpt,
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
    ///
    /// # Errors
    ///
    /// Whatever [`rpt_reader::Rpt::open`] returns: [`rpt_reader::Error::Io`] (naming `path`),
    /// [`rpt_reader::Error::Container`], [`rpt_reader::Error::Codec`], or [`rpt_reader::Error::Crypto`].
    pub fn load(path: impl AsRef<Path>) -> rpt_reader::Result<ReportDocument> {
        Ok(ReportDocument {
            rpt: rpt_reader::Rpt::open(path)?,
        })
    }

    /// The decoded report model (SDK: the `ReportDocument`'s own object graph).
    pub fn report(&self) -> &Report {
        self.rpt.report()
    }

    /// The underlying [`rpt_reader::Rpt`] (stream access, saved data, re-save).
    pub fn inner(&self) -> &rpt_reader::Rpt {
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
    /// let from_saved = doc.render_with(RenderOptions::default());
    ///
    /// // Or feed rows from a custom `RowSource` (a live DB feed, an in-memory source, …).
    /// let source = EmptySource;
    /// let from_rows = doc.render_with(RenderOptions {
    ///     datasource: RenderSource::Rows(&source),
    ///     ..Default::default()
    /// });
    /// # let _ = (from_saved, from_rows);
    /// # Ok(())
    /// # }
    /// ```
    pub fn render_with(&self, opts: RenderOptions) -> PagedDocument {
        render_with(self.report(), opts)
    }

    /// SDK: `ExportToDisk(ExportFormatType.PortableDocFormat, path)`.
    ///
    /// # Errors
    ///
    /// [`ExportError`] if `path` cannot be written. Rendering itself is infallible, so the write is
    /// the only thing that can fail.
    pub fn export_pdf_to_disk(&self, path: impl AsRef<Path>) -> Result<(), ExportError> {
        write_to_disk(path.as_ref(), &render_pdf(self.report()))
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
    ///
    /// Infallible: if krilla declines a resource (a host font it cannot embed, an asset whose pixels do
    /// not decode) the bytes are a one-page PDF naming the failure. Use
    /// [`try_render_pages_with_assets`] over [`render`]'s pages to get the [`PdfError`] instead.
    pub fn to_pdf(&self) -> Vec<u8> {
        render_pdf(self.report())
    }
}

/// Writing an exported document to disk failed.
///
/// It names the file, which a bare [`std::io::Error`] does not, so an embedding caller gets the same
/// "which one?" answer the CLI does when a path is wrong or unwritable. The underlying failure is the
/// [`source`](std::error::Error::source) and is not interpolated, so a cause-chain walk reports it
/// exactly once.
#[derive(Debug, thiserror::Error)]
#[error("cannot write `{}`", .path.display())]
pub struct ExportError {
    /// The file the export was being written to.
    pub path: PathBuf,
    /// The underlying I/O failure.
    #[source]
    pub source: std::io::Error,
}

/// Write exported bytes to `path`, attaching the path to any I/O failure.
fn write_to_disk(path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    std::fs::write(path, bytes).map_err(|source| ExportError {
        path: path.to_path_buf(),
        source,
    })
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
    /// The render's as-of instant, resolving the date/time specials (`CurrentDate`/`Today`/
    /// `CurrentDateTime`/`CurrentTime`) that a formula may read. `None` = capture the system clock
    /// once at render start (see [`default_as_of`]); set it explicitly to make the render fully
    /// reproducible against a frozen baseline.
    pub as_of: Option<DateTimeSpecials>,
    /// Which face library the default text layout takes its metrics from — the half of the font stack
    /// that decides wrap points, can-grow heights and therefore pagination. Default:
    /// [`FontSource::Bundled`], so the geometry is a property of the report rather than of the host's
    /// installed faces; set [`FontSource::System`] to measure with the host's own library instead. The
    /// backend's half is [`PdfOptions::fonts`] — set both the same way, or text is laid out to one
    /// face's advances and drawn in another's. Ignored when the caller injects its own layout
    /// ([`render_dataset_with`]), which brings whatever metrics source it was built on — the
    /// approximate layout reads no fonts at all.
    pub fonts: FontSource,
}

impl std::fmt::Debug for RenderOptions<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderOptions")
            .field("datasource", &self.datasource)
            .field("params", &self.params)
            .field("locale", &self.locale)
            .field("scope", &self.scope.map(|_| "..").unwrap_or("None"))
            .field("as_of", &self.as_of)
            .field("fonts", &self.fonts)
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
/// Infallible: every built-in [`RenderSource`] variant (saved data, an already-materialized
/// [`RowSource`], a pre-built [`Dataset`]) always succeeds. A live datasource can fail, but that
/// happens *before* this call — the caller fetches its rows into a [`RowSource`]/[`Dataset`] and
/// surfaces any fetch failure itself, then hands the ready rows here.
pub fn render_with(report: &Report, opts: RenderOptions) -> PagedDocument {
    render_options(report, opts)
}

/// The shared body behind [`render`] and [`render_with`]: resolve the datasource to a [`Dataset`]
/// (attaching parameters), then lay it out. Infallible — the built-in sources cannot fail.
fn render_options(report: &Report, opts: RenderOptions) -> PagedDocument {
    let RenderOptions {
        datasource,
        params,
        locale,
        scope,
        as_of,
        fonts,
    } = opts;
    // Resolve the render's as-of instant once, so the record pipeline and the layout pass share a
    // single fixed value for `CurrentDate`/… (deterministic across the whole render).
    let as_of = as_of.unwrap_or_else(default_as_of);
    match datasource {
        // A caller-supplied dataset was built outside this function, so its pipeline diagnostics (if
        // any were collected) belong to that caller.
        RenderSource::Dataset(dataset) => {
            layout_dataset(report, dataset, scope, locale, as_of, fonts)
        }
        RenderSource::Rows(source) => {
            build_and_lay_out(report, source, params, scope, locale, as_of, fonts)
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
            build_and_lay_out(report, source, params, scope, locale, as_of, fonts)
        }
    }
}

/// Build the dataset **with a diagnostic sink attached**, lay it out, and merge the pipeline's
/// diagnostics into the document's.
///
/// The record pipeline is fail-open: a selection formula that errors drops the row, a `{@formula}` that
/// errors resolves to `Null`. Without a sink those failures are simply invisible — a report can render
/// zero rows from non-empty data and report success. Attaching the sink here, on the one path every
/// render takes, is what makes them reach [`PagedDocument::diagnostics`] and so the caller.
fn build_and_lay_out(
    report: &Report,
    source: &dyn RowSource,
    params: Parameters,
    scope: Option<&dyn ScopeData>,
    locale: Locale,
    as_of: DateTimeSpecials,
    fonts: FontSource,
) -> PagedDocument {
    let sink = CollectingSink::new();
    let dataset = build_dataset_opts(
        source,
        &report.data_definition,
        DatasetOptions {
            params: Some(&params),
            sink: Some(&sink),
            datetime: Some(as_of),
            ..Default::default()
        },
    );
    let mut doc = layout_dataset(report, &dataset, scope, locale, as_of, fonts);
    // Pipeline diagnostics come first: a selection failure explains an empty page, so it should be
    // read before the layout consequences of that emptiness.
    let mut diagnostics = rpt_layout::diagnostics::from_evals(&sink.into_diagnostics());
    diagnostics.append(&mut doc.diagnostics);
    doc.diagnostics = diagnostics;
    doc
}

/// The default render as-of instant: the system clock captured once at render start (UTC). A WASM
/// (`wasm32`) build has no wall clock, so it falls back to the Unix epoch — a WASM host that needs a
/// real date supplies [`RenderOptions::as_of`] explicitly.
pub fn default_as_of() -> DateTimeSpecials {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        DateTimeSpecials::from_unix_seconds(secs)
    }
    #[cfg(target_arch = "wasm32")]
    {
        DateTimeSpecials::from_unix_seconds(0)
    }
}

/// Compile the report's formulas and lay out a [`Dataset`] with the default text layout — the last
/// step shared by every non-BYO-layout entry point.
fn layout_dataset(
    report: &Report,
    dataset: &Dataset,
    scope: Option<&dyn ScopeData>,
    locale: Locale,
    as_of: DateTimeSpecials,
    fonts: FontSource,
) -> PagedDocument {
    let formulas = compile_formulas_at(&report.data_definition, as_of);
    rpt_layout::layout_scoped(
        report,
        dataset,
        &formulas,
        default_text_layout(fonts),
        scope,
        locale,
    )
}

/// Render a pre-built [`Dataset`] with an explicit [`TextLayout`] (font stack). Lets a caller reuse
/// a `CosmicLayout` (avoids re-scanning fonts per render) or inject host-supplied fonts on WASM. This
/// is the bring-your-own-layout entry [`RenderOptions`] does not model, so it stays as a free
/// function — but it still carries the same reproducibility knobs [`RenderOptions`] does: the render
/// `locale`, an optional subreport `scope`, and the `as_of` instant (`None` captures the system clock
/// once, like [`default_as_of`]; set it to freeze the date/time specials for a reproducible render).
pub fn render_dataset_with(
    report: &Report,
    dataset: &Dataset,
    text_layout: Box<dyn TextLayout>,
    locale: Locale,
    scope: Option<&dyn ScopeData>,
    as_of: Option<DateTimeSpecials>,
) -> PagedDocument {
    let as_of = as_of.unwrap_or_else(default_as_of);
    let formulas = compile_formulas_at(&report.data_definition, as_of);
    rpt_layout::layout_scoped(report, dataset, &formulas, text_layout, scope, locale)
}

/// The default text layout: font-accurate cosmic-text over the face library `fonts` names.
///
/// Deliberately not a build choice: a font-accurate/approximate layout choice must never be a
/// build-time flag, since two builds of the same commit could then paginate a report differently —
/// not a difference worth shipping. The approximate layout stays available by passing it explicitly
/// to [`render_dataset_with`] (what the data-driven baselines do), never through a silent
/// build-time fallback.
fn default_text_layout(fonts: FontSource) -> Box<dyn TextLayout> {
    Box::new(rpt_text::CosmicLayout::new(
        rpt_text::FontProvider::from_source(fonts),
    ))
}

/// The uniform backend trait plus the PDF backend, its option struct (including the [`FontSource`],
/// [`Conformance`], [`Timestamp`] and [`Producer`] it carries) and the fallible PDF entry points,
/// re-exported so a caller can drive output through [`render_backend`] without depending on the
/// backend crate. PDF is the only output backend; the trait is what keeps the Page IR independent
/// of it.
pub use rpt_pages::PageBackend;
pub use rpt_render_pdf::{
    try_render_document, try_render_pages_with_assets, try_render_pages_with_options, ArtifactRole,
    Conformance, FontSource, PdfBackend, PdfError, PdfOptions, Producer, Semantics, Timestamp,
    RPT_RS_PRODUCER,
};

/// The document semantics a tagged or accessible render needs, as far as the **report itself**
/// states them — the caller-side half of [`PdfOptions::semantics`].
///
/// A [`Conformance`] level that requires tagging refuses a render whose semantics it cannot honour,
/// naming each one. Some of what it asks for is in the file and some is not, and this function draws
/// that line:
///
/// - **[`title`](Semantics::title)** — the report's own `SummaryInfo.title`, when the author filled
///   it in. Left `None` when it is empty rather than substituted from the file name, which is not the
///   document's title. Most reports leave it empty.
/// - **[`alt_text`](Semantics::alt_text)** — each picture's and chart's stored `ToolTipText`, which is
///   the one place a report describes a graphic, keyed by object name and taken from the main report
///   and every subreport. **Literal values only**: a tooltip can instead be a conditional formula,
///   and resolving that needs an eval context and a row. A figure with neither is left undescribed,
///   so the level is refused naming it — inventing a description would grant exactly the
///   accessibility claim the caller could not support.
/// - **[`language`](Semantics::language)** — never derived, and always `None` here. Nothing in a
///   `.rpt` records the language of its text, and the render [`Locale`] states number and date
///   conventions rather than language: a US report formatted for a German subsidiary is `de-DE` and
///   still reads in English. It is the caller's to state.
/// - **[`artifact_sections`](Semantics::artifact_sections)** — left `None`, which defers to the
///   document. A [`PagedDocument`] classifies its own bands, so the override is for a caller that
///   disagrees with it.
///
/// ```no_run
/// use rpt_render::{semantics_of, Conformance, PdfOptions, ReportDocument, Semantics};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let doc = ReportDocument::load("report.rpt")?;
/// let pages = doc.render();
/// let semantics = Semantics {
///     // Only the caller knows this; the report does not state it.
///     language: Some("en-US".to_string()),
///     ..semantics_of(doc.report())
/// };
/// let pdf = rpt_render::try_render_document(
///     &pages,
///     &PdfOptions {
///         conformance: Conformance::PdfUa1,
///         semantics,
///         ..Default::default()
///     },
/// )?;
/// # let _ = pdf;
/// # Ok(())
/// # }
/// ```
pub fn semantics_of(report: &Report) -> Semantics {
    let mut alt_text = std::collections::BTreeMap::new();
    collect_alt_text(report, &mut alt_text);
    Semantics {
        title: non_empty(&report.summary_info.title),
        language: None,
        alt_text,
        artifact_sections: None,
    }
}

/// Collect every figure's stored tooltip as its alternate text, recursing into subreports.
///
/// A subreport's objects are merged into the parent document under their own names, so they are
/// collected under those names too; first writer wins on a collision, matching how the producer
/// merges a subreport's section dictionary.
fn collect_alt_text(report: &Report, out: &mut std::collections::BTreeMap<String, String>) {
    use rpt_reader::model::ReportObjectKind;
    for obj in report.objects() {
        let is_figure = matches!(
            obj.kind,
            ReportObjectKind::Picture(_)
                | ReportObjectKind::BlobField(_)
                | ReportObjectKind::Chart(_)
        );
        if !is_figure {
            continue;
        }
        if let Some(text) = obj.format.tooltip_text.as_deref().and_then(non_empty) {
            out.entry(obj.name.clone()).or_insert(text);
        }
    }
    for sub in &report.subreports {
        collect_alt_text(&sub.report, out);
    }
}

/// A stored string as a value, or `None` when it is empty or blank — the difference between a fact
/// the report states and a field the author left alone.
fn non_empty(value: &str) -> Option<String> {
    match value.trim().is_empty() {
        true => None,
        false => Some(value.to_string()),
    }
}

/// Render a [`PagedDocument`] through a [`PageBackend`] — the trait seam over the concrete
/// `render_*` functions. Lets a caller hold a backend as a value and pass its
/// [`Options`](PageBackend::Options), which is also how an out-of-tree backend attaches to the Page
/// IR without this crate knowing about it.
pub fn render_backend<B: PageBackend>(
    doc: &PagedDocument,
    backend: &B,
    opts: &B::Options,
) -> B::Output {
    backend.render(doc, opts)
}

// The named format helper goes through the one [`render_backend`]/[`PageBackend`] seam (proven
// byte-identical to the backend free functions by the `render_backend_seam_matches_free_functions`
// test), so there is a single documented render path.

/// Render the whole report to a single multi-page PDF document (bytes).
pub fn render_pdf(report: &Report) -> Vec<u8> {
    render_backend(&render(report), &PdfBackend, &PdfOptions::default())
}

/// The normalized Page-IR JSON for every page — a stable surface for diffing layout output across
/// renders.
pub fn render_ir_json(report: &Report) -> Vec<String> {
    render(report)
        .pages
        .iter()
        .map(|p| p.to_normalized_json())
        .collect()
}
