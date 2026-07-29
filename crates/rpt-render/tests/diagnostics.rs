//! What the render facade does and does not do to a saved batch, and that it says so when it fails.
//!
//! Three properties, all asserted through `ReportDocument`/`render_with` rather than against a
//! hand-built dataset, because the seam being tested is the facade's — the sink that `build_and_lay_out`
//! attaches to the dataset build, whose result it folds into `PagedDocument::diagnostics`. Without
//! that sink the record pipeline is fail-open *and silent*: a report renders zero rows from non-empty
//! data and exits 0.
//!
//! 1. The record selection is **not** re-applied to a saved batch. A saved batch is the result set as
//!    it stood after selection ran, so re-evaluating the formula over it drops rows the report is meant
//!    to show. The three fixtures here are the ones a re-applied selection formula would visibly drop
//!    rows from.
//! 2. The **saved-data** selection formula *is* applied — it is the one filter the engine runs over an
//!    already-fetched rowset, so it must survive the trip to the page.
//! 3. When that formula cannot be evaluated, the failure reaches `PagedDocument::diagnostics` instead
//!    of quietly emptying the report.
//!
//! Fixture-gated: a missing fixture skips.

use std::path::{Path, PathBuf};

use rpt_data::{Column, Row, RowSource, SavedDataSource};
use rpt_formula::eval::Value;
use rpt_pages::{DiagnosticKind, DrawOp, Severity};
use rpt_render::{render_with, RenderOptions, RenderSource, ReportDocument};

/// Fixtures whose record selection, if it were re-applied to their saved batch, keeps nothing.
///
/// That is the point of choosing them: each one's selection genuinely excludes every stored row when
/// evaluated now, so any regression that reinstates the re-filtering empties them again and is caught
/// here rather than showing up as a blank page in someone's report.
const POST_SELECTION_BATCHES: [&str; 3] = [
    "benbrahim777/Orders10k.rpt",
    "benbrahim777/BeforeTV.rpt",
    "benbrahim777/Orders5-150.rpt",
];

/// Resolve a committed fixture.
///
/// # Panics
///
/// If it is absent. Every report named in this file is committed, so a missing one is the corpus
/// having moved rather than a check being unavailable, and skipping would report green having
/// rendered nothing.
fn fixture(rel: &str) -> PathBuf {
    let p = rpt_test_support::fixture(Path::new("tests/fixtures/reports").join(rel));
    assert!(
        p.exists(),
        "committed fixture {rel} is missing from tests/fixtures/reports"
    );
    p
}

#[test]
fn a_saved_batch_is_rendered_whole_and_raises_no_selection_diagnostic() {
    let mut checked = 0;
    for rel in POST_SELECTION_BATCHES {
        let path = fixture(rel);
        let doc = ReportDocument::load(&path).expect("open the fixture");
        let saved_rows = doc.report().saved_data.as_ref().map_or(0, |s| s.rows.len());
        assert!(saved_rows > 0, "{rel}: fixture premise — it has saved rows");

        let rendered = doc.render();

        // No selection diagnostic, because no selection ran. A diagnostic here means the record
        // selection is being applied to a batch that had already passed it.
        let selection: Vec<_> = rendered
            .diagnostics
            .iter()
            .filter(|d| d.kind == DiagnosticKind::RecordSelection)
            .collect();
        assert!(
            selection.is_empty(),
            "{rel}: the record selection was re-applied to a saved batch — {selection:?}"
        );

        // And the rows actually reached a page. Asserting only the absence of a diagnostic would pass
        // just as well if the rows vanished quietly, which is the exact failure this file exists for.
        assert!(
            !rendered.pages.is_empty(),
            "{rel}: {saved_rows} saved row(s) produced no page at all"
        );
        checked += 1;
    }
    assert_eq!(
        checked,
        POST_SELECTION_BATCHES.len(),
        "only {checked} of the {} committed batch fixtures were rendered",
        POST_SELECTION_BATCHES.len()
    );
}

/// The one committed report with a non-empty saved-data selection formula.
///
/// `{orders.id} > 15` over 30 stored rows whose ids run 1..=30, with both the record selection and the
/// group selection empty — so it isolates that formula and nothing else. Its monthly group on
/// `created_at` makes the surviving rows visible on the page without a detail-band field: the 30 rows
/// fall five to a month across Jan–Jun 2024, so keeping ids 16..=30 keeps exactly the last three
/// months' group headers.
const SAVED_DATA_FILTER: &str = "synthetic/saved_data_filter.rpt";

/// Every text a render drew, in page then draw order.
fn drawn_text(doc: &rpt_pages::PagedDocument) -> Vec<String> {
    doc.pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

/// The report's saved-data selection formula must run, and must drop the rows it excludes.
///
/// The complement of the test above: the record selection is skipped on a saved batch precisely
/// because this formula is the engine's filter for one. A regression that skipped *both* would leave
/// the previous test passing and this one failing, which is why the two are asserted separately.
#[test]
fn a_saved_data_selection_formula_filters_the_batch_through_the_facade() {
    let path = fixture(SAVED_DATA_FILTER);
    let doc = ReportDocument::load(&path).expect("open the fixture");
    assert_eq!(
        doc.report().saved_data.as_ref().map_or(0, |s| s.rows.len()),
        30,
        "fixture premise: the stored batch"
    );

    let text = drawn_text(&doc.render());

    // The group headers of the months whose rows the formula keeps (a monthly group prints
    // `M/YYYY`).
    for kept in ["4/2024", "5/2024", "6/2024"] {
        assert!(
            text.iter().any(|t| t == kept),
            "the {kept} group is missing — the batch was over-filtered: {text:?}"
        );
    }
    // And of the months it excludes. These are what appear if the formula is decoded but never
    // consumed: the report renders all 30 rows and looks perfectly healthy doing it.
    for dropped in ["1/2024", "2/2024", "3/2024"] {
        assert!(
            !text.iter().any(|t| t == dropped),
            "the {dropped} group survived — the saved-data selection formula did not run: {text:?}"
        );
    }
}

/// A saved batch whose rows are missing the column the report's saved-data filter reads.
///
/// The report and its formula are the fixture's own; only the rows differ, which is what makes the
/// evaluation fail on every one of them. It reports itself already selected because that is what a
/// saved batch is — and it is what routes the pipeline to the saved-data formula rather than the
/// record selection.
#[derive(Debug)]
struct BatchWithoutTheFilteredColumn {
    columns: Vec<Column>,
    rows: Vec<Row>,
}

impl BatchWithoutTheFilteredColumn {
    /// Rebuild a source's rows keeping only `keep`, dropping every other column.
    fn from(source: &dyn RowSource, keep: &str) -> Self {
        let columns = source
            .columns()
            .iter()
            .filter(|c| c.name == keep)
            .cloned()
            .collect();
        let rows = source
            .rows()
            .iter()
            .map(|row| {
                let mut out = Row::default();
                out.insert(keep, row.get(keep).cloned().unwrap_or(Value::Null));
                out
            })
            .collect();
        Self { columns, rows }
    }
}

impl RowSource for BatchWithoutTheFilteredColumn {
    fn columns(&self) -> &[Column] {
        &self.columns
    }
    fn rows(&self) -> Vec<Row> {
        self.rows.clone()
    }
    fn already_selected(&self) -> bool {
        true
    }
}

/// A selection failure in the record pipeline must reach `PagedDocument::diagnostics`.
///
/// This is the fail-open path at its worst: every row is dropped, the report renders as a blank one,
/// and the render call succeeds. The diagnostics are the only thing distinguishing that from a report
/// whose data genuinely matched nothing, and they only get there because the facade attaches a sink to
/// the dataset build — the merge is one function, and this is what covers it.
#[test]
fn a_pipeline_selection_failure_reaches_the_document_through_the_facade() {
    let path = fixture(SAVED_DATA_FILTER);
    let rpt = rpt_reader::Rpt::open(&path).expect("open the fixture");
    let report = rpt.report();
    let saved = report.saved_data.as_ref().expect("fixture has saved data");
    let source = BatchWithoutTheFilteredColumn::from(
        &SavedDataSource::from_report(saved, report),
        "orders.created_at",
    );

    let doc = render_with(
        report,
        RenderOptions {
            datasource: RenderSource::Rows(&source),
            ..Default::default()
        },
    );

    let selection: Vec<_> = doc
        .diagnostics
        .iter()
        .filter(|d| d.kind == DiagnosticKind::RecordSelection)
        .collect();
    assert!(
        !selection.is_empty(),
        "the selection failed on all 30 rows and the document said nothing: {:?}",
        doc.diagnostics
    );

    // A per-row failure, located. The record index survives the hand-off from the pipeline's
    // vocabulary to the Page IR's, which is what lets a user find the offending row.
    assert!(
        selection
            .iter()
            .any(|d| d.severity == Severity::Error && d.location.record_index.is_some()),
        "no located per-row failure among {selection:?}"
    );

    // And the summary, which is the part a user reads first: it must distinguish "the formula failed"
    // from "nothing matched", and name the counts that make the difference legible.
    let summary = selection
        .iter()
        .find(|d| d.message.contains("row(s) kept"))
        .unwrap_or_else(|| panic!("no summary diagnostic among {selection:?}"));
    assert!(
        summary.message.contains("0 of 30 row(s) kept"),
        "the summary must state how many rows were offered: {}",
        summary.message
    );
    assert!(
        summary
            .message
            .contains("the saved-data selection formula FAILED"),
        "the summary must name the formula that failed, and say it failed rather than filtered: {}",
        summary.message
    );

    // The render itself succeeded and produced pages. That is the point: nothing about the output
    // says anything is wrong, so the diagnostics are the whole signal.
    assert!(!doc.pages.is_empty(), "the render produced no page at all");
}
