//! # rpt-layout — the layout & pagination engine
//!
//! Walks a report's areas/sections over the [`Dataset`] instance tree, places each object at its
//! absolute twip position, paginates band-by-band, and emits the [`rpt_pages`] Page IR.
//! This is the pull-driven formatter over the push-built tree:
//! [`rpt_data`] built the tree; here we iterate it and pull each object's value.
//!
//! Pagination follows the native checkpoint model at the band level: the page
//! header repeats on every page, the page footer pins to the bottom, and a band that would overflow
//! the body starts a new page. A [`PageCheckpoint`] is recorded at each page top.
//!
//! Text wrapping + `can-grow`: a can-grow text/field wraps to multiple lines and grows its band
//! (pushing later content down) — see the `text` module. Metrics + line-breaking come from an injected
//! [`TextLayout`] (default [`ApproxLayout`], dependency-free and approximate; inject
//! `rpt-text::CosmicLayout` via [`layout_with`] for real font metrics + Unicode/CJK line-breaking).
//!
//! Charts (the `chart` module) and cross-tabs (`crosstab`) render as native draw-ops (bars / a pivot grid)
//! computed from the dataset (the series/pivot builders live in `aggregate`). Subreports render
//! recursively: a subreport object lays out its nested [`Report`] (sharing this formatter's text
//! stack) and its draw-ops are translated into the object's box. Summary resolution is best-effort;
//! a cross-tab supports a single row × column axis pair; and only page-1, unlinked subreports render.
//!
//! The formatter is split across a few modules over a shared `Formatter` state holder:
//! `paginate` owns the page-break cursor and band walk, `place` emits each object's draw-ops,
//! `aggregate` builds chart/cross-tab series from the dataset, and `chart`/`crosstab` draw
//! them. This module keeps the state struct, the public entry points, and the shared leaf helpers.

mod aggregate;
mod chart;
mod crosstab;
mod currency;
pub mod diagnostics;
mod emf;
mod format;
mod paginate;
mod place;
mod resolve;
mod sections;
mod tabs;
mod text;

pub use rpt_format_value::Locale;
pub use text::{ApproxLayout, TextLayout, TWIPS_PER_PT};

use resolve::{context, ResolveState};
use rpt_data::DataContext;
use rpt_data::{
    Column, Dataset, EvalSchedule, FormulaRegistry, GroupInstance, Row, RowSource, RunningTotals,
    ScheduledValues, ScopeData, SharedState, Summary,
};
use rpt_formula::eval::Value;
use rpt_model::{
    field_object_value_type, Alignment, AreaSectionKind, Color, Font, GroupAreaFormat, ImageFormat,
    ReadingOrder, Report, ReportObject, ReportObjectKind, Section, Twips,
};
use rpt_pages::{
    Diagnostic, DrawOp, FontSpec, ImageAsset, ObjectKind, Page, PageCheckpoint, PageSize,
    PagedDocument, Point, TextAlign,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// Whether a decoded image format is one browsers can render inline (so it is worth carrying as a
/// [`PagedDocument`] asset); other formats (TIFF/WMF/EMF/…) get no asset and draw a placeholder.
fn browser_renderable(fmt: ImageFormat) -> bool {
    matches!(
        fmt,
        ImageFormat::Bmp
            | ImageFormat::Dib
            | ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::Gif
    )
}

/// A shared sink the render pass records fidelity [`Diagnostic`]s into (interior mutability so the
/// `&self` resolve path and the `&mut self` emit path can both push). Drained into
/// [`PagedDocument::diagnostics`] at the end of the pass.
pub(crate) type DiagSink = RefCell<Vec<Diagnostic>>;

/// Record a diagnostic, de-duplicating identical ones — an unsupported object in a detail band or a
/// formula that errors would otherwise emit one per record (thousands of copies).
pub(crate) fn push_diag(sink: &DiagSink, d: Diagnostic) {
    let mut v = sink.borrow_mut();
    if !v
        .iter()
        .any(|e| e.kind == d.kind && e.message == d.message && e.source == d.source)
    {
        v.push(d);
    }
}

/// US-Letter fallback when the report carries no page geometry.
const DEFAULT_PAGE_W: i32 = 12240; // 8.5in
const DEFAULT_PAGE_H: i32 = 15840; // 11in

/// One placed line of a text object: its text plus the horizontal offset and usable width for its
/// [`rpt_pages::TextRun`] bounds. `x_offset`/`width` carry per-paragraph indentation (left/right and
/// the first-line indent on the paragraph's first wrapped line). For an un-indented paragraph
/// `x_offset` is zero and `width` is the object's full width, so placement is byte-identical to the
/// pre-indent behaviour.
pub(crate) struct LayoutLine {
    pub(crate) text: String,
    /// Left offset from the object's left edge, in twips: `left_indent` plus `first_line_indent` on
    /// the paragraph's first wrapped line.
    pub(crate) x_offset: Twips,
    /// The line's usable width, in twips: the object width minus `x_offset` and the paragraph's
    /// `right_indent`.
    pub(crate) width: Twips,
    /// The line's own resolved font — the owning paragraph's font (a run-level override, else the
    /// object font), so a multi-paragraph text object with mixed point sizes renders each paragraph at
    /// its own size.
    pub(crate) font: FontSpec,
    /// The vertical advance from this line's top to the next, in twips: the font's natural line height
    /// scaled by the paragraph's `Multiple` line spacing, or its `Exact` twip pitch.
    pub(crate) line_height: Twips,
    /// The baseline offset below this line's top edge, in twips (the line font's ascent).
    pub(crate) ascent: Twips,
    /// Horizontal alignment of this line. Usually the object alignment, but a justified paragraph's
    /// last wrapped line falls back to left (typography does not stretch a paragraph's final line).
    pub(crate) align: TextAlign,
    /// The owning paragraph's rigid character spacing, in twips — extra advance after every Unicode
    /// scalar. It decided this line's wrap point and is carried onto the emitted run so the backend
    /// draws at the same width it was measured at.
    pub(crate) character_spacing: Twips,
}

/// One child page of a formatted subreport: its box-local draw-ops (the subreport's printable
/// top-left mapped to `0,0`) and the content height (the deepest op bottom, floored at the box
/// height so a short subreport never shrinks its band). A subreport with an internal forced page
/// break (a section's `NewPage`) produces one chunk per child page.
pub(crate) struct SubreportChunk {
    pub(crate) ops: Vec<DrawOp>,
    pub(crate) height: i32,
    /// The child's unresolved currency fixups for this chunk, each paired with its index into
    /// [`Self::ops`]. The parent re-anchors each to the physical page it merges that op onto; an op
    /// the parent clips or slices away simply drops its fixup.
    pub(crate) currency: Vec<(usize, CurrencyFixup)>,
}

/// A subreport formatted once for the band it sits in, cached so the band-planning phase can grow the
/// band to fit it and the emit phase can place it — flowing it across parent pages when it is taller
/// than one page — without re-running the child (a second run would double-fire the subreport's
/// `Shared`/`Global` variable writes). `chunks` hold the child's pages' box-local ops (one chunk per
/// forced internal page break).
pub(crate) struct SubreportRender {
    pub(crate) chunks: Vec<SubreportChunk>,
}

impl SubreportRender {
    /// The total content height the enclosing band grows to accommodate (the single-chunk fit path
    /// uses this to extend the band; a multi-chunk or overflowing subreport flows instead).
    pub(crate) fn used_height(&self) -> i32 {
        self.chunks.iter().map(|c| c.height).sum()
    }
}

/// A resolved text/field object's display: its wrapped lines and rendering attributes, computed
/// once per band so the grown height and the emitted runs stay consistent.
pub(crate) struct TextPlan {
    pub(crate) lines: Vec<LayoutLine>,
    pub(crate) color: Color,
    pub(crate) kind: ObjectKind,
    /// Set when the field asks for one currency symbol per page: the amount and the currency spec,
    /// carried so [`currency::apply`] can blank the symbol without re-evaluating the field. A plan is
    /// pure data — a plan that is built and then discarded (a suppressed or blank band) records
    /// nothing, because the fixup is pushed by the emit path, not here.
    pub(crate) currency: Option<resolve::CurrencyMark>,
}

/// Lay out a whole report against its dataset, producing the paginated Page IR. Uses the
/// dependency-free [`ApproxLayout`] for text metrics — for engine-accurate wrap points and
/// international scripts, inject a real [`TextLayout`] via [`layout_with`].
pub fn layout(report: &Report, dataset: &Dataset, formulas: &FormulaRegistry) -> PagedDocument {
    layout_with(report, dataset, formulas, Box::new(ApproxLayout))
}

/// Lay out a report with an injected [`TextLayout`] (e.g. `rpt-text::CosmicLayout` for real font
/// metrics + Unicode line-breaking). The layout engine stays dependency-free; the caller supplies
/// the text stack.
pub fn layout_with(
    report: &Report,
    dataset: &Dataset,
    formulas: &FormulaRegistry,
    text_layout: Box<dyn TextLayout>,
) -> PagedDocument {
    layout_scoped(
        report,
        dataset,
        formulas,
        text_layout,
        None,
        Locale::default(),
    )
}

/// Like [`layout_with`] but with an optional [`ScopeData`] provider: subreports fetch their rows from
/// it (a live datasource) instead of only their saved data. `None` keeps the offline behaviour
/// (subreports render from saved data). The provider lets the native render CLI feed each subreport
/// scope's live rows without `rpt-layout` depending on any DB crate.
pub fn layout_scoped(
    report: &Report,
    dataset: &Dataset,
    formulas: &FormulaRegistry,
    text_layout: Box<dyn TextLayout>,
    scope_data: Option<&dyn ScopeData>,
    locale: Locale,
) -> PagedDocument {
    // The report-lifetime store for Global/Shared variables — one instance for the whole print pass
    // so running totals / WhilePrintingRecords counters accumulate across records.
    let state_vars = SharedState::new();
    // Print-order running totals and the evaluation-time schedule: the
    // read pass fires BeforeReading/WhileReading side-effects into `state_vars` in read order before
    // the print walk, so the print pass reuses the recorded values (single-fire).
    let running_totals = RunningTotals::from_data_def(&report.data_definition);
    let scheduled = run_schedule(report, dataset, formulas, &state_vars);
    Formatter::new(
        report,
        dataset,
        formulas,
        text_layout.as_ref(),
        &state_vars,
        &running_totals,
        &scheduled,
        scope_data,
        locale,
    )
    .run()
    .0
}

/// Run the evaluation-time schedule's read pass for a (sub)report: classify its
/// formulas, then fire `BeforeReading` (once) and `WhileReading` (per record in **read order**)
/// side-effects into `state_vars`, recording each formula's value for the print pass to reuse.
pub(crate) fn run_schedule(
    report: &Report,
    dataset: &Dataset,
    formulas: &FormulaRegistry,
    state_vars: &SharedState,
) -> ScheduledValues {
    let schedule = EvalSchedule::classify(&report.data_definition);
    if schedule.is_empty() {
        return ScheduledValues::default();
    }
    // Read order = source order (after selection, before sort/group), recovered from the stamped
    // read index — the print/tree order differs once sorting or grouping reorders records.
    let mut read_rows: Vec<Row> = dataset.iter_detail_rows().into_iter().cloned().collect();
    read_rows.sort_by_key(|r| r.read_index());
    schedule.run(&read_rows, formulas, state_vars, &dataset.params)
}

/// Sections grouped by their role in the emit sequence.
struct Bands<'a> {
    report_header: Vec<&'a Section>,
    page_header: Vec<&'a Section>,
    group_headers: Vec<Vec<&'a Section>>, // by group level
    detail: Vec<&'a Section>,
    group_footers: Vec<Vec<&'a Section>>, // by group level
    report_footer: Vec<&'a Section>,
    page_footer: Vec<&'a Section>,
    /// The `GroupAreaFormat` for each group level (parallel to `group_headers`), carrying
    /// `keep_group_together` and `visible_groups_per_page` — the group-header area's format, since
    /// the section vectors drop it.
    group_formats: Vec<GroupAreaFormat>,
    /// The Detail area's "Records per page" cap (`visible_records_per_page`); `0` = no limit.
    records_per_page: i32,
}

/// The level-matching key a group header and its footer share: the area name with its band token
/// (`Header`/`Footer`) removed once, so `nameHeader`↔`nameFooter`, `orderdateHeader`↔`orderdateFooter`,
/// and `GroupHeaderArea3`↔`GroupFooterArea3` all pair up regardless of the (UI-creation-order) digit
/// suffix that the file's area names carry.
fn group_area_key(name: &str, token: &str) -> String {
    name.replacen(token, "", 1)
}

/// Order group footers by group level (0 = outermost), pairing each footer to the header level that
/// shares its [`group_area_key`]. This is robust to whatever order the file/decoder presents footers
/// in. When the names don't pair up cleanly (an unmatched footer, or a mismatched count), fall back
/// to the canonical assumption that footers are stored innermost-first and reverse them.
fn order_group_footers<'a>(
    header_keys: &[String],
    footer_entries: Vec<(String, Vec<&'a Section>)>,
) -> Vec<Vec<&'a Section>> {
    let mut placed: Vec<Option<Vec<&Section>>> = (0..header_keys.len()).map(|_| None).collect();
    let mut all_matched = footer_entries.len() == header_keys.len();
    let mut fallback = Vec::with_capacity(footer_entries.len());
    for (key, sections) in footer_entries {
        fallback.push(sections.clone());
        match header_keys.iter().position(|k| *k == key) {
            Some(level) if placed[level].is_none() => placed[level] = Some(sections),
            _ => all_matched = false,
        }
    }
    if all_matched && placed.iter().all(Option::is_some) {
        placed.into_iter().map(Option::unwrap).collect()
    } else {
        fallback.reverse();
        fallback
    }
}

impl<'a> Bands<'a> {
    fn collect(report: &'a Report) -> Bands<'a> {
        let mut b = Bands {
            report_header: Vec::new(),
            page_header: Vec::new(),
            group_headers: Vec::new(),
            detail: Vec::new(),
            group_footers: Vec::new(),
            report_footer: Vec::new(),
            page_footer: Vec::new(),
            group_formats: Vec::new(),
            records_per_page: 0,
        };
        // Group headers appear in group-level order (outermost first); record each level's matching
        // key so its footer can be paired back to it by name, not by position.
        let mut header_keys: Vec<String> = Vec::new();
        let mut footer_entries: Vec<(String, Vec<&Section>)> = Vec::new();
        for area in &report.report_definition.areas {
            // "Hide (Drill-Down OK)" hides the whole area in the normal (non-drill-down) render, so it
            // contributes no bands — but its structural bookkeeping (group level, header/footer key,
            // group format) is kept by zeroing only its sections, so group-level indexing stays aligned.
            let sections: Vec<&Section> = if area.format.hide_for_drill_down {
                Vec::new()
            } else {
                area.sections.iter().collect()
            };
            match area.kind {
                AreaSectionKind::ReportHeader => b.report_header.extend(sections),
                AreaSectionKind::PageHeader => b.page_header.extend(sections),
                AreaSectionKind::GroupHeader => {
                    b.group_formats.push(area.format.group.unwrap_or_default());
                    header_keys.push(group_area_key(&area.name, "Header"));
                    b.group_headers.push(sections);
                }
                AreaSectionKind::Detail => {
                    b.records_per_page = area.format.visible_records_per_page;
                    b.detail.extend(sections);
                }
                AreaSectionKind::GroupFooter => {
                    footer_entries.push((group_area_key(&area.name, "Footer"), sections));
                }
                AreaSectionKind::ReportFooter => b.report_footer.extend(sections),
                AreaSectionKind::PageFooter => b.page_footer.extend(sections),
                _ => {}
            }
        }
        b.group_footers = order_group_footers(&header_keys, footer_entries);
        b
    }
}

pub(crate) struct Formatter<'a> {
    report: &'a Report,
    dataset: &'a Dataset,
    formulas: &'a FormulaRegistry,
    bands: Bands<'a>,
    page_size: PageSize,
    /// The printable-area origin (report top-left margin) stamped on every emitted [`Page`] so
    /// physical backends can re-apply it.
    origin: Point,
    /// Bottom of the flow body in printable-relative twips: the printable height minus the reserved
    /// page-footer band. Bands paginate when they would cross it, and the page footer pins to it.
    body_bottom: i32,

    pages: Vec<Page>,
    checkpoints: Vec<PageCheckpoint>,
    cur: Page,
    cursor_y: i32,
    page_number: i64,
    record_number: i64,
    /// The detail record currently in scope during the print walk. A page header/footer resolves its
    /// field/formula objects against the record straddling the page boundary (the first record placed
    /// on a page for the header, the last for the footer) — this tracks that record, updated as each
    /// detail row is emitted. `None` before the first record prints.
    current_row: Option<&'a Row>,
    /// Text metrics + line-breaking (default: [`ApproxLayout`]; inject a font-accurate impl for
    /// engine parity and international scripts). Borrowed so nested subreport layouts share it.
    text_layout: &'a dyn TextLayout,
    /// Multi-column detail layout, if the report uses "Format with Multiple Columns".
    multi_column: Option<rpt_model::MultiColumn>,
    /// Horizontal offset added to every object's x. Two things shift a band sideways and they
    /// compose: the column a multi-column detail record sits in, and the depth of a hierarchically
    /// grouped instance in its parent/child tree. `0` for an ordinary single-column band.
    col_offset: i32,
    /// Report-lifetime Global/Shared variable store, threaded into every record's [`DataContext`]
    /// so running variables accumulate across the print pass.
    state_vars: &'a SharedState,
    /// Report-lifetime print-order running-total accumulators. Advanced once per
    /// record as it prints, then read back by a `{#name}` field/text object.
    running_totals: &'a RunningTotals,
    /// Pre-scheduled formula values from the read pass, threaded into each record's
    /// context so `BeforeReading`/`WhileReading` formulas return their recorded value (single-fire).
    scheduled: &'a ScheduledValues,
    /// The enclosing group instances (outermost first) as the print walk descends, used to reset
    /// `OnChangeOfGroup` running totals (the key path) and to resolve group-scoped 2-argument
    /// summaries.
    group_stack: Vec<GroupScope>,
    /// The `(condition field, summaries)` projection of [`Self::group_stack`], rebuilt once per
    /// group-stack change (see [`Self::refresh_group_summaries`]) and handed to each per-record
    /// [`ResolveState`] as a cheap `Rc` clone rather than deep-cloned on every band emit.
    group_summaries: Rc<Vec<(String, Vec<Summary>)>>,
    /// The group keys of [`Self::group_stack`], outermost first — rebuilt alongside
    /// [`Self::group_summaries`] and positionally parallel to it, so a `GroupName ({cond})`
    /// reference can resolve the key of the level it names rather than the nearest one.
    group_keys: Rc<Vec<Value>>,
    /// The report grand-total summaries, shared into every [`ResolveState`] so a 1-argument
    /// (grand-total) summary — `Sum({field})`, whether a placed object or a summary function in a
    /// formula — resolves to the report total from any band, not the innermost group's subtotal.
    grand_summaries: Rc<Vec<Summary>>,
    /// Optional live-row provider for subreports. Threaded into nested subreport
    /// formatters so a whole tree renders from live data; `None` = subreports use saved data.
    scope_data: Option<&'a dyn ScopeData>,
    /// The render locale (`--locale` / host): the "system default" layer merged with each field's
    /// stored format leaf to produce the effective display format.
    locale: Locale,
    /// Fidelity diagnostics collected during the pass (unsupported objects, formula errors), drained
    /// into [`PagedDocument::diagnostics`] for the CLI to surface.
    diagnostics: DiagSink,
    /// Embedded image bytes collected as pictures are emitted, drained into
    /// [`PagedDocument::assets`] so every backend can inline images automatically.
    assets: RefCell<BTreeMap<String, ImageAsset>>,
    /// What band each named section is, seeded from this report's areas and merged with each
    /// subreport's as it is formatted; drained into [`PagedDocument::sections`].
    sections: RefCell<crate::sections::SectionMap>,
    /// A `NewPageAfter` on a just-emitted section defers a page break to the next flow band (so a
    /// trailing `NewPageAfter` doesn't leave a blank page at the end of the report).
    pending_page_break: bool,
    /// A `ResetPageNumberAfter` on a just-emitted section resets the page-number counter at the next
    /// page top (so the following page prints as page 1). `PageNumber`/`PageNofM` honour the reset;
    /// `TotalPageCount` stays the whole-document count (a per-reset-section total needs a second pass).
    pending_page_number_reset: bool,
    /// Set while the Report Header is being emitted. The page header belongs *below* the report
    /// header, so no page a report header occupies carries one — including the continuation pages a
    /// tall subreport inside it flows onto (native behavior: the page header first prints on the
    /// report's first body page).
    in_report_header: bool,
    /// The next per-placement instance id to hand out (see [`rpt_pages::ObjectRef::instance`]). One id
    /// per [`Formatter::emit_object`] call, shared by that object's text runs and its border/fill box;
    /// monotonic across the report, with subreport ids remapped into this space on merge.
    next_instance_id: u32,
    /// The next subreport-placement id to hand out: one per merged subreport placement, monotonic
    /// across the report. It is what separates two instances of the *same* subreport definition,
    /// which are otherwise indistinguishable in a merged op's identity (see
    /// [`Formatter::merge_currency_fixup`]).
    next_subreport_placement: u32,
    /// Placed `TotalPageCount`/`PageNofM` runs to rewrite once the final page count is known (the
    /// forward reference a single pass cannot resolve; see [`Self::resolve_page_totals`]).
    page_count_fixups: Vec<PageCountFixup>,
    /// Placed currency values whose field prints one symbol per page, resolved once page membership
    /// is final (see [`Self::resolve_currency_symbols`]).
    currency_fixups: Vec<CurrencyFixup>,
    /// Whether this formatter is laying out a subreport. A subreport's "pages" are its own flow
    /// chunks, not the parent's physical pages, so the page-scoped currency pass does not run on
    /// them — its pending values are handed back for the parent to resolve against the physical
    /// pages it merges them onto.
    nested: bool,
    /// Visible detail records placed on the current page, against the Detail area's "Records per
    /// page" cap. Reset at every page top (see [`Formatter::open_page`]).
    records_on_page: i32,
    /// Group instances present on the current page, per group level (parallel to
    /// `Bands::group_formats`), against each level's "Groups per page" cap. A group carried onto the
    /// page by a break inside it counts, so this is reset to 1 — not 0 — for every level the page
    /// opens inside.
    groups_on_page: Vec<i32>,
    /// Open "Underlay Following Sections" spans on the current page: the bottom each underlaid
    /// section reached, and the companion band that ends it. Cleared at every page top.
    underlay_spans: Vec<paginate::UnderlaySpan>,
}

/// A placed text run whose value depends on the final page count (`TotalPageCount` / `PageNofM`),
/// recorded during the single layout pass and rewritten once the true page total is known (the
/// forward reference the single pass cannot resolve up front — see [`Formatter::resolve_page_totals`]).
struct PageCountFixup {
    /// Index into [`Formatter::pages`] of the page carrying the run.
    page_index: usize,
    /// Index of the [`DrawOp::Text`] within that page's ops.
    op_index: usize,
    /// The page number the run displays (it honours any page-number reset, so only the total is
    /// stale), and the provisional total it was rendered with.
    page_number: i64,
    provisional_total: i64,
    /// What in the run has to be recomputed.
    kind: PageCountFixupKind,
}

/// A placed currency value on a page whose field prints its symbol only once per page, recorded
/// during the single layout pass and resolved once page membership is final (see
/// [`Formatter::resolve_currency_symbols`]). The page a band lands on is decided *after* its text is
/// resolved, so "the first value on this page" cannot be known while formatting.
#[derive(Clone)]
struct CurrencyFixup {
    /// Index into [`Formatter::pages`] of the page carrying the run.
    page_index: usize,
    /// Index of the [`DrawOp::Text`] within that page's ops.
    op_index: usize,
    /// What the symbol is granted to, once per page: the report object's name, since the flag, the
    /// symbol and its placement are all stored on the object's own format leaf. A value merged from
    /// a subreport is keyed by its **placement** as well as the subreport and object names, because
    /// the grant restarts for every subreport instance (see [`Formatter::merge_currency_fixup`]).
    object: String,
    /// The amount and the currency spec it was rendered through.
    mark: resolve::CurrencyMark,
}

/// The two shapes a page-total reference takes on the page.
#[derive(Clone)]
enum PageCountFixupKind {
    /// A placed special field object: the run's whole text is its value, so the run is re-resolved.
    Field(Box<rpt_model::FieldObject>),
    /// A page-total special embedded in a text object, held as its placeholder text: the run also
    /// carries literal text and other references, so only this special's rendered fragment is
    /// substituted. Re-resolving the whole text object instead would re-evaluate its formulas, firing
    /// their `WhilePrintingRecords` side effects a second time.
    Embedded(String),
}

/// One enclosing group's render-time state: its key, its condition field, and its computed summaries
/// (for group-scoped 2-argument summary resolution and `OnChangeOfGroup` running-total resets).
pub(crate) struct GroupScope {
    pub(crate) key: Value,
    pub(crate) condition_field: String,
    pub(crate) summaries: Vec<Summary>,
}

impl<'a> Formatter<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        report: &'a Report,
        dataset: &'a Dataset,
        formulas: &'a FormulaRegistry,
        text_layout: &'a dyn TextLayout,
        state_vars: &'a SharedState,
        running_totals: &'a RunningTotals,
        scheduled: &'a ScheduledValues,
        scope_data: Option<&'a dyn ScopeData>,
        locale: Locale,
    ) -> Formatter<'a> {
        let po = &report.print_options;
        let m = &po.margins;
        // `content_width`/`content_height` are the **printable** area (paper minus margins — e.g.
        // Letter 11520×15120 = 12240×15840 − 720 twips of margins). Reconstruct the full paper so both
        // the emitted page size and the body height are right — treating the printable size as the
        // whole page double-subtracts the margins and loses ~1 detail row per page.
        let page_w = if po.content_width.0 > 0 {
            po.content_width.0 + m.left.0 + m.right.0
        } else {
            DEFAULT_PAGE_W
        };
        let page_h = if po.content_height.0 > 0 {
            po.content_height.0 + m.top.0 + m.bottom.0
        } else {
            DEFAULT_PAGE_H
        };
        // Draw-op coordinates are **printable-relative**: the base is 0,0 (the top-left of the
        // printable area), not the physical margin — the margin is carried once as the page origin
        // and re-applied once per backend. This keeps
        // the coordinate model in one place instead of scattering ±margin across every position site
        // and every backend.
        let origin = Point::new(m.left.0, m.top.0);
        // Bottom of the printable area in printable-relative coords (= printable height).
        let content_bottom = page_h - m.bottom.0 - m.top.0;
        // Draw-op coordinates are printable-relative (0-based), so the content origin is (0, 0); it is
        // not threaded as a field. The margin is carried once as the page origin (above).
        let bands = Bands::collect(report);
        let group_levels = bands.group_formats.len();
        let page_footer_height: i32 = bands.page_footer.iter().map(|s| s.height.0).sum();
        let page_size = PageSize {
            width: Twips(page_w),
            height: Twips(page_h),
        };
        let mut cur = Page::new(1, page_size);
        cur.origin = origin;
        Formatter {
            report,
            dataset,
            formulas,
            bands,
            page_size,
            origin,
            body_bottom: content_bottom - page_footer_height,
            pages: Vec::new(),
            checkpoints: Vec::new(),
            cur,
            cursor_y: 0,
            page_number: 0,
            record_number: 0,
            current_row: None,
            text_layout,
            multi_column: po.multi_column,
            col_offset: 0,
            state_vars,
            running_totals,
            scheduled,
            group_stack: Vec::new(),
            group_summaries: Rc::new(Vec::new()),
            group_keys: Rc::new(Vec::new()),
            grand_summaries: Rc::new(dataset.grand_total.clone()),
            scope_data,
            locale,
            diagnostics: RefCell::new(Vec::new()),
            assets: RefCell::new(BTreeMap::new()),
            sections: RefCell::new(crate::sections::SectionMap::from_report(report)),
            pending_page_break: false,
            pending_page_number_reset: false,
            in_report_header: false,
            next_instance_id: 0,
            next_subreport_placement: 0,
            page_count_fixups: Vec::new(),
            currency_fixups: Vec::new(),
            nested: false,
            records_on_page: 0,
            groups_on_page: vec![0; group_levels],
            underlay_spans: Vec::new(),
        }
    }

    /// Switch this formatter into flow mode: never paginate, so the whole report lays out onto a
    /// single tall page. Used to format an inline subreport into one continuous op list whose height
    /// the enclosing band grows to fit (see `place::format_subreport`).
    fn set_flow_mode(&mut self) {
        // A body bottom far past any realistic content height: the band-overflow checks never trip,
        // so `begin_page`/`finish_page` run exactly once and every band flows onto page 1.
        self.body_bottom = i32::MAX / 4;
        // The per-section paging limits count bands rather than measure them, so height alone cannot
        // keep them from breaking this flow onto a second page: clear them too.
        self.bands.records_per_page = 0;
        for f in &mut self.bands.group_formats {
            f.visible_groups_per_page = 0;
        }
    }

    /// Lay the report out into pages. The second half of the pair is the currency fixups this run
    /// did **not** resolve: a nested formatter leaves them to its parent, whose physical pages are
    /// the ones the rule is scoped to (see [`Self::resolve_currency_symbols`]). A top-level run
    /// resolves its own and returns none.
    fn run(mut self) -> (PagedDocument, Vec<CurrencyFixup>) {
        // Note any raw SQL Command / stored-proc tables: their SQL is author-written for a specific
        // database and passed through verbatim (never translated), so the report renders with live
        // data only against that database — one aggregated diagnostic.
        self.note_command_tables();
        // Approximate layout drives pagination: fixed average advance + space-only wrapping (no real
        // metrics, no CJK). Wrap points, can-grow heights, and page counts are then NOT byte-parity
        // with a real font stack, so a report laid out through this impl can paginate differently
        // from the same report laid out through CosmicLayout. Emit one aggregated diagnostic so the
        // divergence is not silent; inject a font-loaded CosmicLayout (`render_dataset_with`) for
        // identical output.
        if self.text_layout.is_approximate() {
            push_diag(
                &self.diagnostics,
                Diagnostic::warn(
                    rpt_pages::DiagnosticKind::Other,
                    "pagination used the approximate text layout (ApproxLayout): wrap points, \
                     can-grow heights, and page counts are not guaranteed to match a real font \
                     stack (e.g. native CosmicLayout) and it cannot wrap CJK; inject a font-loaded \
                     TextLayout for cross-platform-identical pagination",
                ),
            );
        }
        // Open page 1, then emit the report header above the page header (the correct top-of-page
        // band order). The report header is emitted here — not in `begin_page` — because a tall
        // subreport in it flows across continuation pages, each of which repeats the page header via
        // `begin_page`; every later page turn opens through `begin_page` (report header page-1 only).
        self.open_page();
        self.emit_report_header();
        // `dataset` is borrowed for the whole formatter lifetime, so copy the reference out and walk
        // it in place — no need to deep-clone the recursively-owned `GroupInstance` tree to satisfy
        // the borrow checker against `&mut self`.
        let dataset = self.dataset;
        // Body: grouped or flat.
        if dataset.groups.is_empty() {
            if self.bands.group_headers.is_empty() && self.bands.group_footers.is_empty() {
                self.emit_details(&dataset.details, &dataset.grand_total);
            } else {
                // A report that defines group bands but produced no group (an empty dataset) still
                // lays out the group skeleton once, so its static content renders.
                self.emit_empty_group_skeleton();
            }
        } else {
            for g in &dataset.groups {
                self.emit_group(g);
            }
        }
        // Report footer (once) — resolved against the report's last record (Crystal's report-footer
        // record context) so its field/formula objects populate; the grand-total summaries are in
        // scope so a 1-argument grand-total summary object also resolves. It is the Report Header's
        // underlay companion, so a span the header opened closes before it prints.
        self.close_underlay_spans(paginate::UnderlayEnd::ReportFooter);
        let rf: Vec<&Section> = self.bands.report_footer.clone();
        let mut rf_state = self.state(None);
        rf_state.summaries = Rc::new(dataset.grand_total.clone());
        let last = dataset.iter_detail_rows().last().copied();
        for s in rf {
            self.emit_band(s, last, &rf_state, None);
        }
        self.finish_page();

        // The final page count is now known: rewrite every `TotalPageCount`/`PageNofM` run that was
        // placed with a provisional total during the single pass.
        self.resolve_page_totals();
        // Page membership is now final too: blank the repeated currency symbols.
        let deferred = self.resolve_currency_symbols();
        let doc = PagedDocument {
            pages: self.pages,
            checkpoints: self.checkpoints,
            diagnostics: self.diagnostics.into_inner(),
            assets: self.assets.into_inner(),
            sections: self.sections.into_inner().finish(),
        };
        (doc, deferred)
    }

    /// Push one draw-op onto the current page. Every op the placement path emits goes through here so
    /// a text run's horizontal tabs are resolved into positioned runs in one place (see
    /// [`crate::tabs`]); returns how many ops landed, since a tabbed run splits into one per segment.
    pub(crate) fn push_op(&mut self, op: DrawOp) -> usize {
        match op {
            DrawOp::Text(run) if run.text.contains('\t') => {
                let runs = tabs::expand_tabs(run, self.text_layout);
                let n = runs.len();
                for r in runs {
                    self.cur.push(DrawOp::Text(r));
                }
                n
            }
            other => {
                self.cur.push(other);
                1
            }
        }
    }

    /// Build the print [`ResolveState`] for the current position: the in-scope group summaries (one
    /// entry per enclosing group), the print specials, and (optionally) the current group key.
    pub(crate) fn state(&self, group_key: Option<Value>) -> ResolveState {
        ResolveState {
            group_key,
            summaries: Rc::new(Vec::new()),
            group_summaries: Rc::clone(&self.group_summaries),
            grand_summaries: Rc::clone(&self.grand_summaries),
            group_keys: Rc::clone(&self.group_keys),
            page_number: self.page_number,
            total_pages: self.pages.len() as i64 + 1,
            record_number: self.record_number,
        }
    }

    /// Rewrite every recorded `TotalPageCount`/`PageNofM` run with the true final page count. Each
    /// run's displayed page number is preserved (it honoured any page-number reset already); only the
    /// stale total changes. The run's stored advance is recomputed so a right/centre-aligned footer
    /// re-anchors to the new width.
    fn resolve_page_totals(&mut self) {
        if self.page_count_fixups.is_empty() {
            return;
        }
        let total = self.pages.len() as i64;
        let diag: DiagSink = RefCell::new(Vec::new());
        for fx in std::mem::take(&mut self.page_count_fixups) {
            let at = |total_pages| ResolveState {
                page_number: fx.page_number,
                total_pages,
                ..ResolveState::default()
            };
            let Some(DrawOp::Text(run)) = self
                .pages
                .get_mut(fx.page_index)
                .and_then(|p| p.ops.get_mut(fx.op_index))
            else {
                continue;
            };
            let text = match &fx.kind {
                PageCountFixupKind::Field(f) => {
                    resolve::field_text(self.report, f, None, &at(total), &self.locale, &diag)
                }
                // The embedded case substitutes the special's own rendering inside the run: the
                // fragment it produced with the provisional total, replaced by the same rendering
                // with the true one.
                PageCountFixupKind::Embedded(placeholder) => {
                    let render = |total_pages| {
                        resolve::special_display(
                            placeholder,
                            self.report,
                            &at(total_pages),
                            &self.locale,
                        )
                    };
                    run.text
                        .replacen(&render(fx.provisional_total), &render(total), 1)
                }
            };
            if let Some(m) = run.metrics.as_mut() {
                m.advance = Twips(crate::text::spaced_width_twips(
                    self.text_layout,
                    &text,
                    &run.font,
                    run.character_spacing,
                ) as i32);
            }
            run.text = text;
        }
    }

    /// Apply `OneCurrencySymbolPerPage`: on each page, the first printed value of each such field
    /// keeps its symbol and every later one on that page has it blanked (see [`crate::currency`]).
    ///
    /// It runs here, not while formatting, because a band's text is resolved *before* the page-break
    /// decision that follows it — so at format time the value's page is not yet settled. Blanking can
    /// only shorten a single-line run, so nothing pagination computed is invalidated by it.
    ///
    /// A nested formatter resolves nothing and hands its fixups back instead: a subreport's pages are
    /// its own flow chunks, and the rule is scoped to the physical page the parent finally places the
    /// value on. The parent re-anchors them onto [`SubreportChunk::currency`] as it merges the
    /// child's ops.
    fn resolve_currency_symbols(&mut self) -> Vec<CurrencyFixup> {
        if self.nested {
            return std::mem::take(&mut self.currency_fixups);
        }
        currency::apply(&mut self.pages, &self.currency_fixups, self.text_layout);
        Vec::new()
    }

    /// Rebuild the [`Self::group_summaries`] and [`Self::group_keys`] projections from the current
    /// [`Self::group_stack`]. Called once whenever the stack changes (a group is entered or left) so
    /// per-record [`Self::state`] calls only bump the shared `Rc`s instead of re-projecting and
    /// deep-cloning the whole stack.
    pub(crate) fn refresh_group_summaries(&mut self) {
        self.group_summaries = Rc::new(
            self.group_stack
                .iter()
                .map(|g| (g.condition_field.clone(), g.summaries.clone()))
                .collect(),
        );
        self.group_keys = Rc::new(self.group_stack.iter().map(|g| g.key.clone()).collect());
    }

    /// The current enclosing-group key path signature (`OnChangeOfGroup` running-total reset key):
    /// it changes exactly when the record's group path changes.
    fn group_signature(&self) -> Option<String> {
        if self.group_stack.is_empty() {
            return None;
        }
        Some(
            self.group_stack
                .iter()
                .map(|g| format!("{:?}", g.key))
                .collect::<Vec<_>>()
                .join("\u{1}"),
        )
    }

    /// Advance every running total by `row` (print order), so a `{#name}` object in the band about to
    /// be emitted reads the value accumulated up to and including this record.
    pub(crate) fn advance_running_totals(&self, row: &Row, state: &ResolveState) {
        if self.running_totals.is_empty() {
            return;
        }
        let sig = self.group_signature();
        let ctx = self.context(row, state);
        self.running_totals.advance(&ctx, sig.as_deref());
    }

    /// Build the per-record [`DataContext`] from this Formatter's report-lifetime state (formulas,
    /// params, shared vars, running totals, scheduled values) plus one `row` and the print `state`.
    /// The context borrows `row` for its own lifetime, so it never keeps `self` borrowed.
    pub(crate) fn context<'r>(&self, row: &'r Row, state: &'r ResolveState) -> DataContext<'r>
    where
        'a: 'r,
    {
        context(
            row,
            self.formulas,
            &self.dataset.params,
            state,
            self.state_vars,
            self.running_totals,
            self.scheduled,
        )
    }
}

pub(crate) fn first_row(g: &GroupInstance) -> Option<&Row> {
    g.details
        .first()
        .or_else(|| g.subgroups.iter().find_map(first_row))
}

/// The last detail record of a group subtree, in print order — the group-footer record context.
pub(crate) fn last_row(g: &GroupInstance) -> Option<&Row> {
    g.details
        .last()
        .or_else(|| g.subgroups.iter().rev().find_map(last_row))
}

/// A [`RowSource`] with no columns and no rows — for a subreport that carries no saved data (only
/// its static content formats).
pub(crate) struct EmptyRows;

impl RowSource for EmptyRows {
    fn columns(&self) -> &[Column] {
        &[]
    }
    fn rows(&self) -> Vec<Row> {
        Vec::new()
    }
}

/// Shift a draw-op by `(dx, dy)` twips and remap its instance id by `id_offset` (for placing a
/// subreport's ops into its box on the containing page). The geometry shift is [`DrawOp::translate`];
/// `id_offset` lifts the subreport's own 0-based instance ids into the parent's id space so they
/// don't collide with the parent's.
pub(crate) fn translate_op(op: &DrawOp, dx: i32, dy: i32, id_offset: u32) -> DrawOp {
    let mut moved = op.translate(dx, dy);
    if id_offset != 0 {
        if let Some(inst) = moved.source_mut().and_then(|s| s.instance.as_mut()) {
            *inst += id_offset;
        }
    }
    moved
}

pub(crate) fn font_of(f: &Font) -> FontSpec {
    FontSpec {
        family: if f.name.is_empty() {
            "Arial".to_string()
        } else {
            f.name.clone()
        },
        size_pt: if f.size_pt > 0.0 { f.size_pt } else { 10.0 },
        bold: f.bold,
        italic: f.italic,
        underline: f.underline,
        strikethrough: f.strikethrough,
    }
}

pub(crate) fn align_of(a: Alignment) -> TextAlign {
    match a {
        Alignment::RightAlign => TextAlign::Right,
        Alignment::HorizontalCenterAlign => TextAlign::Center,
        Alignment::Justified => TextAlign::Justified,
        _ => TextAlign::Left,
    }
}

/// Resolve an object's horizontal alignment, giving [`Alignment::DefaultAlign`] the meaning the
/// engine gives it at paint time. An explicitly stored alignment always wins.
///
/// The default is not "flush left": it resolves from the content.
/// * A **field object** takes its effective value type — numeric (number / currency / integer)
///   aligns flush **right** so a column lines up on its decimal point; string, memo, date,
///   date-time and everything else align left.
/// * A **field heading** takes the field it heads: that field's own explicit alignment if it has
///   one, else the same value-type rule — so a heading sits over its column the way the column
///   itself sits.
/// * A right-to-left paragraph reads flush right, its base direction.
///
/// The stored `DefaultAlign` byte is what the file holds and what the decoder reports; only the
/// renderer needs the resolved value, so the resolution lives here.
pub(crate) fn resolved_align(
    report: &Report,
    obj: &ReportObject,
    reading_order: ReadingOrder,
) -> TextAlign {
    if !matches!(obj.format.horizontal_alignment, Alignment::DefaultAlign) {
        return align_of(obj.format.horizontal_alignment);
    }
    match &obj.kind {
        ReportObjectKind::Field(f) => {
            if field_object_value_type(report, f).is_numeric() {
                return TextAlign::Right;
            }
        }
        ReportObjectKind::FieldHeading(h) => {
            if let Some(headed) = report.objects().find(|o| o.name == h.field_object_name) {
                if !matches!(headed.format.horizontal_alignment, Alignment::DefaultAlign) {
                    return align_of(headed.format.horizontal_alignment);
                }
                if let ReportObjectKind::Field(f) = &headed.kind {
                    if field_object_value_type(report, f).is_numeric() {
                        return TextAlign::Right;
                    }
                }
            }
        }
        _ => {}
    }
    if matches!(reading_order, ReadingOrder::RightToLeft) {
        return TextAlign::Right;
    }
    TextAlign::Left
}

#[cfg(test)]
mod tests;
