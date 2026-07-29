//! `rpt patch`'s raw byte form must refuse an edit to a record type that is not cleared for safe
//! editing — and refusing means writing **nothing**.
//!
//! The write path is the only one in the project that can damage a file, and the damage it causes is
//! silent: a record whose bytes carry an internal offset table, count, or checksum can be
//! overwritten into a `.rpt` that re-encodes, re-opens, and re-decodes without complaint while being
//! semantically corrupt. So the refusal is asserted end-to-end, through the binary, including the
//! absence of the output file.
//!
//! Addressing the same record by **field name** is gated differently, on the record type's field
//! table reproducing the record byte for byte, so it needs no entry on the clearance list — which is
//! asserted here too, since the two gates only make sense against each other.
//!
//! The input is a committed fixture reached from the workspace root, and its absence fails: a gate
//! that skips itself when it cannot find its input is indistinguishable from a gate that holds.
#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The compiled `rpt` binary under test.
const RPT: &str = env!("CARGO_BIN_EXE_rpt");

/// The Section record (`0x008c`) — present in every report, deliberately *not* on the clearance
/// list, and described exactly by a field table.
const UNCLEARED_TAG: &str = "0x008c";

/// The committed fixture to patch: the smallest report in the corpus, since the gate refuses before
/// reading any content and the forced write only has to round-trip.
const FIXTURE: &str = "tests/fixtures/reports/synthetic/blank_report.rpt";

fn fixture() -> PathBuf {
    let p = rpt_test_support::fixture(FIXTURE);
    assert!(
        p.exists(),
        "the patch-gate fixture is missing at {} — the gate cannot be exercised, and a test that \
         returns here would pass having patched nothing",
        p.display()
    );
    p
}

fn out_path(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("rpt-patch-gate-{name}.rpt"));
    std::fs::remove_file(&p).ok();
    p
}

fn patch(input: &Path, out: &Path, extra: &[&str], target: &str, value: &str) -> Output {
    let mut cmd = Command::new(RPT);
    cmd.arg("patch");
    cmd.args(extra);
    cmd.args([input.as_os_str(), UNCLEARED_TAG.as_ref(), "0".as_ref()]);
    cmd.args([target, value]);
    cmd.arg(out.as_os_str());
    cmd.output().expect("run rpt patch")
}

/// A raw edit at offset 0, one byte written as itself: the gate runs before any bytes are touched,
/// so the payload does not matter.
fn patch_bytes(input: &Path, out: &Path, extra: &[&str]) -> Output {
    patch(input, out, extra, "@0", "00")
}

#[test]
fn an_uncleared_record_is_refused_and_no_output_file_appears() {
    let input = fixture();
    let out = out_path("refused");

    let o = patch_bytes(&input, &out, &[]);
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
    let input = fixture();
    let out = out_path("forced");

    let o = patch_bytes(&input, &out, &["--force"]);
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success(), "--force must permit the edit: {stderr}");
    assert!(out.exists(), "--force must write the output file");

    // And what it wrote is a report: the escape hatch bypasses the clearance judgement, not the
    // writer's own correctness.
    assert!(
        rpt_reader::Rpt::open(&out).is_ok(),
        "the forced output must re-open"
    );
    std::fs::remove_file(&out).ok();
}

/// The same record, addressed by field name, needs no forcing: its field table reproduces it byte
/// for byte, which is the evidence the clearance list stands in for.
#[test]
fn the_same_record_is_editable_by_field_name() {
    let input = fixture();
    let out = out_path("by-name");

    let o = patch(&input, &out, &[], "height", "1234");
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(
        o.status.success(),
        "a named field needs no --force: {stderr}"
    );
    assert!(out.exists(), "the edit must write the output file");

    let patched = rpt_reader::Rpt::open(&out).expect("the output re-opens");
    assert_eq!(
        patched
            .record_field(rpt_reader::raw::RecordTag(0x008c), 0, "height")
            .expect("the section still carries a height")
            .value,
        rpt_reader::fields::FieldValue::Int(1234)
    );
    std::fs::remove_file(&out).ok();
}
