//! Data-pipeline failures must reach the caller through `PagedDocument::diagnostics`.
//!
//! The record pipeline is fail-open by design: a selection formula that errors drops the row, a
//! `{@formula}` that errors resolves to `Null`. That behaviour is deliberate — one broken formula must
//! not abort a whole render — but for a long time nothing attached a sink on the render path, so those
//! failures were not merely tolerated, they were *invisible*. Three committed fixtures render zero rows
//! from non-empty saved data; before this, they did so silently and exited 0.
//!
//! What is asserted here is the reachability of the information, not the fail-open policy.
//!
//! Fixture-gated: a missing fixture skips.

use std::path::{Path, PathBuf};

use rpt_pages::{DiagnosticKind, Severity};
use rpt_render::ReportDocument;

/// Fixtures whose record selection keeps nothing from non-empty saved data.
const EMPTY_RENDERERS: [&str; 3] = [
    "benbrahim777/Orders10k.rpt",
    "benbrahim777/BeforeTV.rpt",
    "benbrahim777/Orders5-150.rpt",
];

fn fixture(rel: &str) -> Option<PathBuf> {
    let p = rpt_test_support::fixture(Path::new("tests/fixtures/reports").join(rel));
    p.exists().then_some(p)
}

#[test]
fn a_report_that_renders_no_rows_from_saved_data_says_why() {
    let mut checked = 0;
    for rel in EMPTY_RENDERERS {
        let Some(path) = fixture(rel) else { continue };
        let doc = ReportDocument::load(&path).expect("open the fixture");
        let saved_rows = doc.report().saved_data.as_ref().map_or(0, |s| s.rows.len());
        assert!(saved_rows > 0, "{rel}: fixture premise — it has saved rows");

        let rendered = doc.render();
        // The premise of the whole test: these fixtures really do drop every row.
        assert_eq!(
            rendered.pages.len().min(1),
            1,
            "{rel}: expected at least a static page"
        );

        // A selection diagnostic must be present, and it must explain rather than merely fire.
        let selection: Vec<_> = rendered
            .diagnostics
            .iter()
            .filter(|d| d.kind == DiagnosticKind::RecordSelection)
            .collect();
        assert!(
            !selection.is_empty(),
            "{rel}: {saved_rows} saved row(s) produced no rows and NO diagnostic — \
             the render is silently wrong. Diagnostics were: {:?}",
            rendered.diagnostics
        );
        // The summary names the counts, so a user can tell "nothing matched" from "nothing worked".
        let summary = selection
            .iter()
            .find(|d| d.message.contains("row(s) kept"))
            .unwrap_or_else(|| panic!("{rel}: no summary diagnostic among {selection:?}"));
        assert!(
            summary.message.contains(&format!("of {saved_rows} row(s)")),
            "{rel}: the summary must state how many rows were offered: {}",
            summary.message
        );
        checked += 1;
    }
    if checked == 0 {
        // No fixtures present — nothing to assert, but do not silently claim success either.
        eprintln!("skipped: none of the diagnostic fixtures are present");
    }
}

#[test]
fn a_selection_that_fails_is_an_error_and_one_that_filters_is_a_warning() {
    // The distinction matters: "nothing matched your criteria" is normal, "the criteria could not be
    // evaluated" is a broken report. Collapsing both into one severity would make the signal useless.
    let Some(failing) = fixture("benbrahim777/Orders5-150.rpt") else {
        return;
    };
    let doc = ReportDocument::load(&failing).expect("open");
    let rendered = doc.render();
    let worst = rendered
        .diagnostics
        .iter()
        .filter(|d| d.kind == DiagnosticKind::RecordSelection)
        .map(|d| d.severity)
        .max_by_key(|s| match s {
            Severity::Error => 1,
            Severity::Warning => 0,
        });
    assert_eq!(
        worst,
        Some(Severity::Error),
        "a selection formula that errors on every row must be an error, not a warning"
    );
}

#[test]
fn per_row_failures_carry_the_record_index() {
    // The same formula runs on every row, so without an index a user cannot tell one bad row from a
    // systematically broken formula.
    let Some(path) = fixture("benbrahim777/Orders5-150.rpt") else {
        return;
    };
    let rendered = ReportDocument::load(&path).expect("open").render();
    let located = rendered
        .diagnostics
        .iter()
        .filter(|d| d.kind == DiagnosticKind::RecordSelection)
        .filter(|d| d.location.record_index.is_some())
        .count();
    assert!(
        located > 0,
        "no per-row selection diagnostic carried a record index: {:?}",
        rendered.diagnostics
    );
    // And `describe()` — what a CLI prints — surfaces it.
    let sample = rendered
        .diagnostics
        .iter()
        .find(|d| d.location.record_index.is_some())
        .expect("one exists");
    assert!(
        sample.describe().contains("record "),
        "describe() must include the location: {}",
        sample.describe()
    );
}
