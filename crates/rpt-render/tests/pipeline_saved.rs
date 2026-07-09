//! Runs the full pipeline on a real report's saved data and sanity-checks the materialized instance
//! tree. Uses a committed public-demo fixture that retains its saved-data batch, so the saved-data
//! path is always exercised in CI.

use rpt_data::{build_dataset, SavedDataSource};
use rpt_test_support::fixture;

#[test]
fn worrall_pipeline_materializes_rows() {
    let path = fixture("tests/fixtures/reports/worrall/AlphaISOsByCountry.rpt");
    let rpt = rpt::Rpt::open(&path).expect("open");
    let report = rpt.report();
    let saved = report.saved_data.as_ref().expect("saved data");

    let source = SavedDataSource::new(saved);
    let dataset = build_dataset(&source, &report.data_definition);

    // The decoder yields 170 of 249 rows for this batch; the pipeline must carry
    // them through selection+sort intact (this report has no grouping).
    assert_eq!(dataset.row_count, saved.rows.len());
    assert!(
        dataset.row_count >= 170,
        "row_count = {}",
        dataset.row_count
    );
    assert_eq!(dataset.iter_detail_rows().len(), dataset.row_count);

    // A known field resolves on the first row.
    let first = dataset.iter_detail_rows()[0];
    assert!(first.get("countries_all_iso.id").is_some());
    assert!(first.get("id").is_some(), "short-name lookup works");
}
