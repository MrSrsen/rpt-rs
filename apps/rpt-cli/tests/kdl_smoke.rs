//! Smoke test for `rpt kdl`: export each report fixture and assert the output is valid KDL.
//!
//! The corpus lives at the workspace root, reached through [`rpt_test_support::fixture`] rather than
//! this crate's own `CARGO_MANIFEST_DIR`. Every `.rpt` under `tests/` is committed, so an empty walk
//! means the walk is looking in the wrong place — [`corpus`] fails on it instead of skipping, or a
//! misresolved path would report `ok` having exported nothing.
#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The compiled `rpt` binary under test.
const RPT: &str = env!("CARGO_BIN_EXE_rpt");

/// The smallest corpus this suite accepts as real, below the committed count so adding or retiring
/// a fixture does not move it, and far above what a wrong directory could yield.
const MIN_REPORTS: usize = 100;

/// How many corpus reports carry a record-level sort, and so exercise the backstop below. Pinned to
/// the count rather than to "at least one", so retiring the fixtures that cover this path is a
/// failure rather than a quiet loss of coverage.
const MIN_RECORD_SORTS: usize = 5;

/// Every `.rpt` under `dir`, recursively.
fn collect_reports(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(collect_reports(&p));
        } else if p.extension().is_some_and(|x| x == "rpt") {
            out.push(p);
        }
    }
    out
}

/// The committed report corpus, sorted, asserting the walk found it.
fn corpus() -> Vec<PathBuf> {
    let dir = rpt_test_support::fixture("tests/fixtures/reports");
    let mut reports = collect_reports(&dir);
    assert!(
        reports.len() >= MIN_REPORTS,
        "the corpus walk found only {} report(s) under {} — it is looking in the wrong place, and a \
         smoke test that exports nothing passes",
        reports.len(),
        dir.display()
    );
    reports.sort();
    reports
}

/// Run `rpt <subcommand> <report>` and return its stdout as text (asserting success).
fn run(subcommand: &str, report: &Path) -> String {
    let out = Command::new(RPT)
        .arg(subcommand)
        .arg(report)
        .output()
        .unwrap_or_else(|e| panic!("run rpt {subcommand}: {e}"));
    assert!(
        out.status.success(),
        "rpt {subcommand} failed for {}: {}",
        report.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8 output")
}

#[test]
fn kdl_export_is_valid_kdl() {
    let reports = corpus();
    for report in &reports {
        let text = run("kdl", report);
        kdl::KdlDocument::parse(&text)
            .unwrap_or_else(|e| panic!("KDL from {} does not parse: {e}", report.display()));
    }
}

/// Losslessness backstop: every report the decoder gives a record-level sort must also carry a
/// `record-sort` node in its KDL export, so the sparse KDL surface never silently drops one. The
/// reference is the exhaustive JSON dump — the decoded model itself, not another projection of it.
#[test]
fn kdl_emits_record_sort_when_the_model_has_one() {
    let reports = corpus();
    let mut checked = 0usize;
    for report in &reports {
        let dump: serde_json::Value = serde_json::from_str(&run("json-dump", report))
            .unwrap_or_else(|e| panic!("json-dump of {} is not JSON: {e}", report.display()));
        let has_record_sort = dump["model"]["data_definition"]["record_sorts"]
            .as_array()
            .is_some_and(|sorts| sorts.iter().any(|s| s["kind"] == "RecordSortField"));
        if !has_record_sort {
            continue;
        }
        checked += 1;
        assert!(
            run("kdl", report).contains("record-sort"),
            "{} has a RecordSortField in the model but no record-sort in KDL",
            report.display()
        );
    }
    // The backstop is only a backstop while some fixture still carries a record sort; a corpus that
    // lost them all would leave the assertion above unexecuted and still report `ok`.
    assert!(
        checked >= MIN_RECORD_SORTS,
        "only {checked} report(s) in the corpus carry a RecordSortField, expected at least \
         {MIN_RECORD_SORTS} — the backstop is no longer covering the KDL record-sort path"
    );
}
