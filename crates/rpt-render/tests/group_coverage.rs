//! Every group in the data must reach the page — the guard against a *silently* dropped group.
//!
//! A lost group is the one render defect that leaves no visible trace: the page stays aligned, every
//! remaining value is right, the grand total still prints, and the report is simply missing a group.
//! Nothing about the output looks wrong, so it is only ever caught by counting.
//!
//! The two assertions name their own stage. `rpt-data` owns the first (one group instance per distinct
//! key in the data) and `rpt-layout` the second (one rendered group-name run per group instance), so a
//! failure says which side lost it rather than needing a bisection between them.
//!
//! Hermetic: the fixture renders from its own embedded saved data, over the bundled faces and a frozen
//! as-of instant, so this needs no database and no host font library.

use std::collections::BTreeSet;

use rpt_data::{build_dataset_opts, DatasetOptions, DateTimeSpecials, SavedDataSource};
use rpt_formula::eval::Value;
use rpt_pages::DrawOp;
use rpt_render::{render_dataset_with, CosmicLayout, FontProvider, Locale};
use rpt_test_support::fixture;

/// The frozen "now", matching the other render harnesses.
const AS_OF_UNIX: i64 = 1_700_000_000;

/// 47 US states/districts, one group each, over 1,524 saved orders across four pages — the smallest
/// committed report where a dropped group would be invisible by inspection.
#[test]
fn every_region_group_reaches_the_page() {
    let path = fixture("tests/fixtures/reports/benbrahim777/USA Orders, Percentages.rpt");
    let rpt = rpt_reader::Rpt::open(&path).expect("open fixture report");
    let report = rpt.report();
    let saved = report
        .saved_data
        .as_ref()
        .expect("fixture carries saved data");

    // The expectation comes from the stored rows, not from the pipeline that is under test. The
    // report's record selection is `{Customer.Country} = "USA"` and every saved row is a USA row, so
    // the distinct region set of the batch is exactly the set of groups the report must produce.
    let region = saved
        .columns
        .iter()
        .position(|c| c.name == "Customer.Region")
        .expect("saved batch carries Customer.Region");
    let expected: BTreeSet<String> = saved
        .rows
        .iter()
        .filter_map(|r| r[region].clone())
        .collect();
    assert_eq!(expected.len(), 47, "distinct regions in the saved batch");

    let as_of = DateTimeSpecials::from_unix_seconds(AS_OF_UNIX);
    let source = SavedDataSource::from_report(saved, report);
    let dataset = build_dataset_opts(
        &source,
        &report.data_definition,
        DatasetOptions {
            datetime: Some(as_of),
            ..Default::default()
        },
    );

    // L2 — `rpt-data`: one group instance per distinct key, none dropped by selection or grouping.
    assert_eq!(
        dataset.row_count,
        saved.rows.len(),
        "selection keeps every saved row"
    );
    let grouped: BTreeSet<String> = dataset
        .groups
        .iter()
        .map(|g| match &g.key {
            Value::Str(s) => s.clone(),
            other => panic!("group key is not a string: {other:?}"),
        })
        .collect();
    assert_eq!(grouped, expected, "one group instance per distinct region");

    // L3 — `rpt-layout`: the group header's field object prints once per group instance. Selected by
    // object name rather than by matching the text, so a run that resolved to the wrong value is a
    // mismatch here instead of quietly passing as some other object's output.
    let doc = render_dataset_with(
        report,
        &dataset,
        Box::new(CosmicLayout::new(FontProvider::bundled())),
        Locale::default(),
        None,
        Some(as_of),
    );
    let mut printed: Vec<String> = Vec::new();
    for page in &doc.pages {
        for op in &page.ops {
            if let DrawOp::Text(t) = op {
                let is_group_name = t
                    .source
                    .as_ref()
                    .and_then(|s| s.object_name.as_deref())
                    .is_some_and(|n| n == "GroupNameRegion1");
                if is_group_name {
                    printed.push(t.text.clone());
                }
            }
        }
    }
    assert_eq!(
        printed.len(),
        expected.len(),
        "one group-name run per group, no repeats: {printed:?}"
    );
    assert_eq!(
        printed.iter().cloned().collect::<BTreeSet<String>>(),
        expected,
        "every group reached a page"
    );
}
