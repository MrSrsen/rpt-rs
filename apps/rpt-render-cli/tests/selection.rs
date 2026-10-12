//! `rpt-render --selection`, driven through the real binary.
//!
//! The flag narrows the records a report shows with a Crystal-syntax formula. What matters end to end
//! is that it reaches the record pipeline (the row count the render reports changes) and that a
//! formula that does not parse is refused up front instead of silently dropping every record.
//!
//! Fixture-gated like the rest of the suite: it skips when the committed report is absent.
#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The compiled `rpt-render` binary under test.
const RPT_RENDER: &str = env!("CARGO_BIN_EXE_rpt-render");

/// A report with saved data, so a render needs no database.
fn report() -> Option<PathBuf> {
    let path = rpt_test_support::fixture("tests/fixtures/reports/worrall/AlphaISOsByCountry.rpt");
    path.is_file().then_some(path)
}

fn scratch(name: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("rpt-render-sel-{name}-{}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

fn run(report: &Path, args: &[&str], out: &Path) -> Output {
    Command::new(RPT_RENDER)
        .arg(report)
        .args(args)
        .arg("-o")
        .arg(out)
        .output()
        .expect("rpt-render runs")
}

/// The `N` of the `done: N row(s)` line the render logs.
fn rows(output: &Output) -> usize {
    let log = String::from_utf8_lossy(&output.stderr);
    let line = log
        .lines()
        .find(|l| l.contains("row(s)") && l.contains("done"))
        .unwrap_or_else(|| panic!("no `done: N row(s)` line in the log:\n{log}"));
    let tail = line.split("done:").nth(1).expect("done: ").trim();
    tail.split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no row count in {line:?}"))
}

/// A selection that no record satisfies leaves no rows; one that every record satisfies leaves all.
#[test]
fn selection_reaches_the_record_pipeline() {
    let Some(report) = report() else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let all = run(&report, &[], &scratch("all"));
    assert!(all.status.success());
    let total = rows(&all);
    assert!(total > 0, "the fixture should render rows");

    let kept = run(&report, &["--selection", "1 = 1"], &scratch("kept"));
    assert!(kept.status.success());
    assert_eq!(
        rows(&kept),
        total,
        "a selection every record satisfies keeps every row"
    );

    let none = run(&report, &["--selection", "1 = 2"], &scratch("none"));
    assert!(none.status.success());
    assert_eq!(
        rows(&none),
        0,
        "a selection no record satisfies leaves no rows"
    );
}

/// A formula that is not valid Crystal syntax is refused: non-zero exit, the flag named, no file.
#[test]
fn an_invalid_selection_is_refused_and_writes_nothing() {
    let Some(report) = report() else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let out = scratch("invalid");
    let result = run(&report, &["--selection", "{t.x} = ("], &out);
    assert!(
        !result.status.success(),
        "an invalid formula must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("--selection"),
        "the flag is named: {stderr}"
    );
    assert!(!out.exists(), "no PDF is written for a refused request");
}
