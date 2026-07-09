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
//! stack) and its draw-ops are translated into the object's box. Remaining first-cut scope:
//! best-effort summary resolution; single row×column cross-tab axes; and page-1-only, unlinked
//! subreports.
//!
//! The formatter is split across a few modules over a shared `Formatter` state holder:
//! `paginate` owns the page-break cursor and band walk, `place` emits each object's draw-ops,
//! `aggregate` builds chart/cross-tab series from the dataset, and `chart`/`crosstab` draw
//! them. This module keeps the state struct, the public entry points, and the shared leaf helpers.

mod aggregate;
mod chart;
mod crosstab;
mod emf;
mod format;
mod paginate;
mod place;
mod resolve;
mod text;

pub use rpt_format_value::Locale;
pub use text::{ApproxLayout, TextLayout, TWIPS_PER_PT};

use crystal_formula::eval::Value;
use resolve::{context, ResolveState};
use rpt_data::DataContext;
use rpt_data::{
    Column, Dataset, EvalSchedule, FormulaRegistry, GroupInstance, Row, RowSource, RunningTotals,
    ScheduledValues, ScopeData, SharedState, Summary,
};
use rpt_model::{
    Alignment, AreaSectionKind, Color, Font, GroupAreaFormat, ImageFormat, Report, Section, Twips,
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

/// A resolved text/field object's display: its wrapped lines and rendering attributes, computed
/// once per band so the grown height and the emitted runs stay consistent.
pub(crate) struct TextPlan {
    pub(crate) lines: Vec<String>,
    pub(crate) font: FontSpec,
    pub(crate) color: Color,
    pub(crate) align: TextAlign,
    pub(crate) kind: ObjectKind,
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
    /// `keep_group_together` — the group-header area's format, since the section vectors drop it.
    group_formats: Vec<GroupAreaFormat>,
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
        };
        for area in &report.report_definition.areas {
            let sections: Vec<&Section> = area.sections.iter().collect();
            match area.kind {
                AreaSectionKind::ReportHeader => b.report_header.extend(sections),
                AreaSectionKind::PageHeader => b.page_header.extend(sections),
                AreaSectionKind::GroupHeader => {
                    b.group_formats.push(area.format.group.unwrap_or_default());
                    b.group_headers.push(sections);
                }
                AreaSectionKind::Detail => b.detail.extend(sections),
                AreaSectionKind::GroupFooter => b.group_footers.push(sections),
                AreaSectionKind::ReportFooter => b.report_footer.extend(sections),
                AreaSectionKind::PageFooter => b.page_footer.extend(sections),
                _ => {}
            }
        }
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
    content_left: i32,
    content_top: i32,
    body_bottom: i32,
    page_footer_top: i32,

    pages: Vec<Page>,
    checkpoints: Vec<PageCheckpoint>,
    cur: Page,
    cursor_y: i32,
    page_number: i64,
    record_number: i64,
    /// Text metrics + line-breaking (default: [`ApproxLayout`]; inject a font-accurate impl for
    /// engine parity and international scripts). Borrowed so nested subreport layouts share it.
    text_layout: &'a dyn TextLayout,
    /// Multi-column detail layout, if the report uses "Format with Multiple Columns".
    multi_column: Option<rpt_model::MultiColumn>,
    /// Horizontal offset added to every object's x, for placing a detail record in a given column
    /// (0 for single-column and for all non-detail bands).
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
    /// A `NewPageAfter` on a just-emitted section defers a page break to the next flow band (so a
    /// trailing `NewPageAfter` doesn't leave a blank page at the end of the report).
    pending_page_break: bool,
    /// A `ResetPageNumberAfter` on a just-emitted section resets the page-number counter at the next
    /// page top (so the following page prints as page 1). `PageNumber`/`PageNofM` honour the reset;
    /// `TotalPageCount` stays the whole-document count (a per-reset-section total needs a second pass).
    pending_page_number_reset: bool,
    /// The next per-placement instance id to hand out (see [`rpt_pages::ObjectRef::instance`]). One id
    /// per [`Formatter::emit_object`] call, shared by that object's text runs and its border/fill box;
    /// monotonic across the report, with subreport ids remapped into this space on merge.
    next_instance_id: u32,
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
        // and re-applied per backend (SVG/PDF add it, HTML uses a CSS-margin container). This keeps
        // the coordinate model in one place instead of scattering ±margin across every position site
        // and every backend.
        let origin = Point::new(m.left.0, m.top.0);
        let content_left = 0;
        let content_top = 0;
        // Bottom of the printable area in printable-relative coords (= printable height).
        let content_bottom = page_h - m.bottom.0 - m.top.0;
        let page_footer_height: i32 = Bands::collect(report)
            .page_footer
            .iter()
            .map(|s| s.height.0)
            .sum();
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
            bands: Bands::collect(report),
            page_size,
            origin,
            content_left,
            content_top,
            body_bottom: content_bottom - page_footer_height,
            page_footer_top: content_bottom - page_footer_height,
            pages: Vec::new(),
            checkpoints: Vec::new(),
            cur,
            cursor_y: content_top,
            page_number: 0,
            record_number: 0,
            text_layout,
            multi_column: po.multi_column,
            col_offset: 0,
            state_vars,
            running_totals,
            scheduled,
            group_stack: Vec::new(),
            group_summaries: Rc::new(Vec::new()),
            scope_data,
            locale,
            diagnostics: RefCell::new(Vec::new()),
            assets: RefCell::new(BTreeMap::new()),
            pending_page_break: false,
            pending_page_number_reset: false,
            next_instance_id: 0,
        }
    }

    fn run(mut self) -> PagedDocument {
        // Note any raw SQL Command / stored-proc tables: their SQL is author-written for a specific
        // database and passed through verbatim (never translated), so the report renders with live
        // data only against that database — one aggregated diagnostic.
        self.note_command_tables();
        // Approximate layout drives pagination: fixed average advance + space-only wrapping (no real
        // metrics, no CJK). Wrap points, can-grow heights, and page counts are then NOT cross-platform
        // byte-parity with a real font stack — the same report on WASM (default ApproxLayout) vs native
        // (CosmicLayout) can paginate differently. Emit one aggregated diagnostic so the divergence is
        // not silent; inject a font-loaded CosmicLayout (`render_dataset_with`) for identical output.
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
        // begin_page emits the report header (page 1 only) above the page header, then the page
        // header — the correct top-of-page band order.
        self.begin_page();
        // Body: grouped or flat.
        if self.dataset.groups.is_empty() {
            self.emit_details(&self.dataset.details, &self.dataset.grand_total);
        } else {
            // `dataset` is borrowed for the whole formatter lifetime, so copy the reference out and
            // iterate the group tree in place — no need to deep-clone the recursively-owned
            // `GroupInstance` tree just to satisfy the borrow checker against `&mut self`.
            let dataset = self.dataset;
            for g in &dataset.groups {
                self.emit_group(g);
            }
        }
        // Report footer (once) — the grand-total summaries are in scope so a 1-argument grand-total
        // summary object resolves.
        let rf: Vec<&Section> = self.bands.report_footer.clone();
        let mut rf_state = self.state(None);
        rf_state.summaries = self.dataset.grand_total.clone();
        for s in rf {
            self.emit_band(s, None, &rf_state);
        }
        self.finish_page();

        // Second pass would fix TotalPageCount; first-cut sets it to the final page count on every
        // page's checkpoint. (A real two-pass or checkpoint replay is a flagged follow-up.)
        PagedDocument {
            pages: self.pages,
            checkpoints: self.checkpoints,
            diagnostics: self.diagnostics.into_inner(),
            assets: self.assets.into_inner(),
        }
    }

    /// Build the print [`ResolveState`] for the current position: the in-scope group summaries (one
    /// entry per enclosing group), the print specials, and (optionally) the current group key.
    pub(crate) fn state(&self, group_key: Option<Value>) -> ResolveState {
        ResolveState {
            group_key,
            summaries: Vec::new(),
            group_summaries: Rc::clone(&self.group_summaries),
            page_number: self.page_number,
            total_pages: self.pages.len() as i64 + 1,
            record_number: self.record_number,
        }
    }

    /// Rebuild the [`Self::group_summaries`] projection from the current [`Self::group_stack`]. Called
    /// once whenever the stack changes (a group is entered or left) so per-record [`Self::state`]
    /// calls only bump the shared `Rc` instead of re-projecting and deep-cloning the whole stack.
    pub(crate) fn refresh_group_summaries(&mut self) {
        self.group_summaries = Rc::new(
            self.group_stack
                .iter()
                .map(|g| (g.condition_field.clone(), g.summaries.clone()))
                .collect(),
        );
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
    pub(crate) fn context<'r>(&self, row: &'r Row, state: &ResolveState) -> DataContext<'r>
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

#[cfg(test)]
mod tests;
