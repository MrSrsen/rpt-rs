//! Record selection still filters, and a parameter still decides how many rows survive it.
//!
//! Every render fixture in this tier is hermetic saved data, which is deliberately **not**
//! re-filtered — a stored batch is the result set as it stood *after* selection ran, and
//! re-evaluating the formula over it would drop rows the report is meant to show. So the
//! selection-formula path, which only runs against a live fetch, has no other coverage in this tier.
//!
//! The gap is not theoretical: selection is what a live-DB render still runs for every row, and its
//! failures are fail-open, so a broken predicate or an unresolved parameter costs rows silently.
//!
//! These tests reach it by presenting a committed saved batch through a source that reports itself
//! UNFILTERED, which is what a live source is. That is the one honest way to exercise the path
//! without a database: the rows and the report's own formulas are real, only the claim about whether
//! they have already been through selection is different.
//!
//! Both fixtures are committed, so a missing one fails rather than skipping.

use std::path::{Path, PathBuf};

use rpt_data::{
    build_dataset_opts, CollectingSink, Column, DatasetOptions, Parameters, Row, RowSource,
    SavedDataSource,
};
use rpt_formula::eval::Value;

/// A committed saved batch, presented as though it had never been through record selection.
///
/// Only [`RowSource::already_selected`] differs from [`SavedDataSource`]; the columns and rows are
/// the fixture's own.
#[derive(Debug)]
struct Unfiltered(SavedDataSource);

impl RowSource for Unfiltered {
    fn columns(&self) -> &[Column] {
        self.0.columns()
    }
    fn rows(&self) -> Vec<Row> {
        self.0.rows()
    }
    fn already_selected(&self) -> bool {
        false
    }
}

/// Both fixtures are committed, so an absent one means the corpus moved under this harness rather
/// than that the check is unavailable — skipping there would report green having asserted nothing.
fn fixture(rel: &str) -> PathBuf {
    let p = rpt_test_support::fixture(Path::new("tests/fixtures/reports").join(rel));
    assert!(
        p.exists(),
        "committed fixture {rel} is missing from tests/fixtures/reports"
    );
    p
}

/// Run a fixture's own record selection over its own rows, and report what survived.
fn kept(rel: &str, params: Parameters) -> (usize, usize, Vec<String>) {
    let path = fixture(rel);
    let rpt = rpt_reader::Rpt::open(&path).expect("open the fixture");
    let report = rpt.report();
    let saved = report.saved_data.as_ref().expect("fixture has saved data");
    let offered = saved.rows.len();

    let source = Unfiltered(SavedDataSource::from_report(saved, report));
    let sink = CollectingSink::new();
    let dataset = build_dataset_opts(
        &source,
        &report.data_definition,
        DatasetOptions {
            params: Some(&params),
            sink: Some(&sink),
            ..Default::default()
        },
    );
    let messages = sink
        .into_diagnostics()
        .into_iter()
        .map(|d| d.detail)
        .collect();
    (offered, dataset.row_count, messages)
}

/// A selection predicate that drops rows must still drop them.
///
/// `'Pink' IN {web_colours.name}` is case-sensitive in the formula language, so it matches 9 of this
/// fixture's 52 stored rows — the rest are lowercase "pink" ("Amaranth pink", "Baby pink"). The
/// engine renders all 52 from the saved batch precisely because it does not re-run this, which is
/// what makes the same fixture a clean test of the predicate itself once the rows are offered as
/// unfiltered.
#[test]
fn a_selection_predicate_still_filters_when_the_source_is_unfiltered() {
    let (offered, kept_rows, _) = kept("worrall/PinkPaletteSampler.rpt", Parameters::new());
    assert_eq!(offered, 52, "fixture premise: the stored batch");
    assert_eq!(
        kept_rows, 9,
        "the case-sensitive predicate must keep only the exact-case matches"
    );
}

/// A parameter must still decide how many rows survive.
///
/// `{Orders.Order Amount} = {?Order_Amt_Range}` — equality against a range means membership. The
/// value supplied is one of the fixture's own saved order amounts, so exactly one row matches, and
/// a `{?Param}` that resolved to `Null` or was looked up under the wrong key would keep none.
///
/// It is the amount as the report means it, not as the batch stores it: saved numeric cells are
/// written scaled by 100 and the decoder divides that out, so this is `10_259.10`, not the
/// `1_025_910` the bytes hold.
#[test]
fn a_parameter_still_decides_how_many_rows_survive_selection() {
    let mut params = Parameters::new();
    params.insert("order_amt_range".to_string(), Value::Currency(10_259.10));
    let (offered, kept_rows, _) = kept("benbrahim777/Orders10k.rpt", params);
    assert_eq!(offered, 27, "fixture premise: the stored batch");
    assert_eq!(
        kept_rows, 1,
        "the supplied parameter must select exactly its own row"
    );
}

/// An unresolved parameter must be *reported*, not silently emptied.
///
/// This is the failure the fail-open pipeline is most able to hide: with no value supplied the
/// formula cannot be evaluated, every row is dropped, and the render is a blank report that exited
/// successfully. The diagnostic is the only thing standing between that and a silent wrong answer,
/// and it must name the counts so "nothing matched" is distinguishable from "nothing worked".
#[test]
fn an_unresolved_parameter_empties_the_report_and_says_so() {
    let (offered, kept_rows, messages) = kept("benbrahim777/Orders10k.rpt", Parameters::new());
    assert_eq!(kept_rows, 0, "an unresolvable predicate keeps nothing");
    assert!(
        messages.iter().any(|m| m.contains("row(s) kept")),
        "no summary diagnostic among {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains(&format!("of {offered} row(s)"))),
        "the summary must state how many rows were offered: {messages:?}"
    );
}
