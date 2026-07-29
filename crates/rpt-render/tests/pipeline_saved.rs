//! Runs the full pipeline on a real report's saved data and sanity-checks the materialized instance
//! tree. Uses a committed public-demo fixture that retains its saved-data batch, so the saved-data
//! path is always exercised in CI.

use rpt_data::{build_dataset, SavedDataSource};
use rpt_test_support::fixture;

#[test]
fn worrall_pipeline_materializes_rows() {
    let path = fixture("tests/fixtures/reports/worrall/AlphaISOsByCountry.rpt");
    let rpt = rpt_reader::Rpt::open(&path).expect("open");
    let report = rpt.report();
    let saved = report.saved_data.as_ref().expect("saved data");

    let source = SavedDataSource::new(saved);
    let dataset = build_dataset(&source, &report.data_definition);

    // All 249 stored rows must reach the dataset — an equality, not a floor, because the failure
    // this guards against is silent: a pipeline that drops rows still renders and still blesses, and
    // a floor cannot tell "this report has N rows" from "it has more and we lost some". The rows
    // span two memo-descriptor batches (170 + 79), so reading only the first batch of a class yields
    // 170 here; row completeness at the decode layer is covered by `rpt-reader`'s own `saved_rows` suite.
    assert_eq!(saved.record_count, 249, "stored record count");
    assert_eq!(saved.rows.len(), 249, "decoded rows");
    assert_eq!(
        dataset.row_count, 249,
        "rows carried through selection+sort"
    );
    assert_eq!(dataset.row_count, saved.rows.len());
    assert_eq!(dataset.iter_detail_rows().len(), dataset.row_count);

    // A known field resolves on the first row.
    let first = dataset.iter_detail_rows()[0];
    assert!(first.get("countries_all_iso.id").is_some());
    assert!(first.get("id").is_some(), "short-name lookup works");
}
