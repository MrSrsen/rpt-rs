//! `rpt patch` must refuse an edit to a record type that is not cleared for safe editing — and
//! refusing means writing **nothing**.
//!
//! The write path is the only one in the project that can damage a file, and the damage it causes is
//! silent: a record whose leaf carries an internal offset table, count, or checksum can be
//! overwritten into a `.rpt` that re-encodes, re-opens, and re-decodes without complaint while being
//! semantically corrupt. So the refusal is asserted end-to-end, through the binary, including the
//! absence of the output file.
//!
//! Fixture-gated like the rest of the suite: no `.rpt` fixtures means skip, not fail.
#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The compiled `rpt` binary under test.
const RPT: &str = env!("CARGO_BIN_EXE_rpt");

/// The Section record (`0x008c`) — present in every report and deliberately *not* cleared.
const UNCLEARED_TAG: &str = "0x008c";

/// A committed fixture to patch. `None` skips the test.
fn fixture() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reports/synthetic/blank_report.rpt");
    p.exists().then_some(p)
}

fn out_path(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("rpt-patch-gate-{name}.rpt"));
    std::fs::remove_file(&p).ok();
    p
}

fn patch(input: &Path, out: &Path, extra: &[&str]) -> Output {
    let mut cmd = Command::new(RPT);
    cmd.arg("patch");
    cmd.args(extra);
    // Offset 0, one byte, written as itself would still be an edit — the gate runs before any bytes
    // are touched, so the payload does not matter.
    cmd.args([input.as_os_str(), UNCLEARED_TAG.as_ref(), "0".as_ref()]);
    cmd.args(["0", "00"]);
    cmd.arg(out.as_os_str());
    cmd.output().expect("run rpt patch")
}

#[test]
fn an_uncleared_record_is_refused_and_no_output_file_appears() {
    let Some(input) = fixture() else { return };
    let out = out_path("refused");

    let o = patch(&input, &out, &[]);
    let stderr = String::from_utf8_lossy(&o.stderr);

    assert!(!o.status.success(), "the edit must be refused: {stderr}");
    assert!(
        !out.exists(),
        "a refused patch wrote `{}` — the refusal must happen before anything is written",
        out.display()
    );
    // The refusal has to be actionable: what was refused, why, and how to proceed anyway.
    assert!(stderr.contains("uncleared record edit"), "{stderr}");
    assert!(stderr.contains("0x008c"), "{stderr}");
    assert!(stderr.contains("--force"), "{stderr}");
}

#[test]
fn force_writes_the_same_edit() {
    let Some(input) = fixture() else { return };
    let out = out_path("forced");

    let o = patch(&input, &out, &["--force"]);
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success(), "--force must permit the edit: {stderr}");
    assert!(out.exists(), "--force must write the output file");

    // And what it wrote is a report: the escape hatch bypasses the clearance judgement, not the
    // writer's own correctness.
    assert!(
        rpt::Rpt::open(&out).is_ok(),
        "the forced output must re-open"
    );
    std::fs::remove_file(&out).ok();
}
