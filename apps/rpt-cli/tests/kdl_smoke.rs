//! Smoke test for `rpt kdl`: export each report fixture and assert the output is valid KDL.
//!
//! Fixture-gated in the same way as the baseline suite — it passes (skips) when no `.rpt` fixtures
//! are present under `tests/fixtures/reports/`, so a checkout without the private/local report
//! corpus still builds and tests cleanly.
#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The compiled `rpt` binary under test.
const RPT: &str = env!("CARGO_BIN_EXE_rpt");

fn reports_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reports")
}

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
    let reports = collect_reports(&reports_dir());
    if reports.is_empty() {
        eprintln!("[skip] no fixtures at {}", reports_dir().display());
        return;
    }
    for report in &reports {
        let text = run("kdl", report);
        kdl::KdlDocument::parse(&text)
            .unwrap_or_else(|e| panic!("KDL from {} does not parse: {e}", report.display()));
    }
}

/// Losslessness backstop: every report the decoder gives a record-level sort must also carry a
/// `record-sort` node in its KDL export, so the sparse KDL surface never silently drops one. The
/// reference is the exhaustive JSON dump — the decoded model itself, not another projection of it.
/// Fixture-gated like the smoke test above.
#[test]
fn kdl_emits_record_sort_when_the_model_has_one() {
    let reports = collect_reports(&reports_dir());
    if reports.is_empty() {
        eprintln!("[skip] no fixtures at {}", reports_dir().display());
        return;
    }
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
    eprintln!("[record-sort backstop] checked {checked} report(s) with a record sort");
}
