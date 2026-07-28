//! Tests for [`rpt::Rpt::anonymize`] — the authoring-metadata scrub.
//!
//! **These tests build their own dirty input.** They cannot lean on the fixture corpus carrying
//! authoring metadata, because the point of the command is that it does not: the corpus is scrubbed,
//! and `corpus_is_already_clean` below asserts it stays that way. So the path case injects a source
//! path into a real report through the writer, then scrubs it back out. The property-set side is
//! covered where it belongs, as a unit test over a synthesized OLEPS set in `container`.
//!
//! The guarantees under test: the identifying value is gone, its replacement keeps `IsImported`
//! true, *nothing else in the decoded model moved*, the result re-opens, and a second pass is a
//! no-op. Every edit is same-length, so "the result re-opens" is a real check on that claim rather
//! than a formality — a mis-sized write would corrupt the record framing and fail the decode.
//!
//! Fixture-gated: a missing corpus skips, so the suite stays green on a bare checkout.

use rpt::raw::RecordTag;
use rpt::Rpt;
use std::path::{Path, PathBuf};

/// A report with a `0x0142` re-import record in its main `Contents` — the record the dirty-input
/// helper rewrites. Its stored path is empty, as it is everywhere in the clean corpus.
const WITH_REIMPORT_RECORD: &str = "benbrahim777/USAvsFrance.rpt";

/// The record type carrying a re-imported subreport's source path.
const REIMPORT_INFO: RecordTag = RecordTag(0x0142);

/// A path shaped like the ones this command exists to remove: a UNC host, a user directory, and a
/// deep tree above the report's own name.
const DIRTY_PATH: &str = r"\\FILESRV\ada\Documents\Reports\Quarterly Sales.rpt";

fn fixture(rel: &str) -> PathBuf {
    rpt_test_support::fixture(Path::new("tests/fixtures/reports").join(rel))
}

fn open(rel: &str) -> Option<Rpt> {
    Rpt::open(fixture(rel)).ok()
}

/// Every `.rpt` under the fixture corpus.
fn corpus() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rpt") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(
        &rpt_test_support::fixture(Path::new("tests/fixtures/reports")),
        &mut out,
    );
    out.sort();
    out
}

/// Write `path` into the report's first `0x0142` record, producing the dirty input the scrub is
/// meant to clean.
///
/// The leaf is `[u32 BE len][path, len bytes including its NUL][fixed 17-byte trailer]`, and in the
/// clean corpus `len` is 1 (the NUL alone). Replacing leaf bytes `0..5` — the length prefix plus
/// that lone NUL — with a longer prefix + path grows the record, which is exactly what
/// `patch_record_leaf_resize` recomputes enclosing lengths for. The trailer is left alone.
fn with_source_path(rpt: &Rpt, path: &str) -> Rpt {
    let mut leaf = Vec::new();
    let mut stored = path.as_bytes().to_vec();
    stored.push(0);
    leaf.extend_from_slice(&(stored.len() as u32).to_be_bytes());
    leaf.extend_from_slice(&stored);

    let bytes = rpt
        .patch_record_leaf_resize(REIMPORT_INFO, 0, 0..5, &leaf)
        .expect("inject a source path");
    let dirty = Rpt::read(std::io::Cursor::new(bytes)).expect("injected report re-opens");
    assert_eq!(
        dirty
            .report()
            .reimport
            .as_ref()
            .map(|r| r.source_path.as_str()),
        Some(path),
        "the helper must actually produce dirty input"
    );
    dirty
}

/// Re-open anonymized bytes through the reader, so every assertion is on a real decode.
fn anonymized(rpt: &Rpt) -> (Rpt, rpt::AnonymizeReport) {
    let (bytes, report) = rpt.anonymize().expect("anonymize");
    let reread = Rpt::read(std::io::Cursor::new(bytes)).expect("anonymized report re-opens");
    (reread, report)
}

#[test]
fn reimport_path_is_reduced_to_its_file_name() {
    let Some(rpt) = open(WITH_REIMPORT_RECORD) else {
        eprintln!("[skip] {WITH_REIMPORT_RECORD} absent");
        return;
    };
    let dirty = with_source_path(&rpt, DIRTY_PATH);
    let (clean, report) = anonymized(&dirty);

    let removal = report
        .removals
        .iter()
        .find(|r| r.field == "reimport.source_path")
        .expect("the injected path was removed");
    assert_eq!(removal.value, DIRTY_PATH);
    assert_eq!(removal.replacement, "Quarterly Sales.rpt");
    assert_eq!(removal.stream, "Contents");

    let after = clean
        .report()
        .reimport
        .as_ref()
        .expect("reimport info survives")
        .source_path
        .clone();
    assert_eq!(after, "Quarterly Sales.rpt");
    assert!(!after.contains('\\'), "no directory survives: {after:?}");
}

/// The load-bearing guarantee behind shortening rather than blanking: a non-empty source path is the
/// only evidence in the file that a subreport was imported, and `SubreportObject.IsImported` is
/// resolved from it. A replacement must therefore never be empty, or the scrub would silently turn a
/// true fact false.
#[test]
fn a_removed_path_is_always_replaced_not_emptied() {
    let Some(rpt) = open(WITH_REIMPORT_RECORD) else {
        eprintln!("[skip] {WITH_REIMPORT_RECORD} absent");
        return;
    };
    // A path with no directory part at all is already clean and must be left alone entirely.
    for path in [DIRTY_PATH, r"C:\a.rpt", "/tmp/unix/b.rpt"] {
        let dirty = with_source_path(&rpt, path);
        let (_, report) = anonymized(&dirty);
        for r in report
            .removals
            .iter()
            .filter(|r| r.field.ends_with("source_path"))
        {
            assert!(
                !r.replacement.is_empty(),
                "{path}: replacement must be non-empty so IsImported survives: {r:?}"
            );
            assert!(r.value.ends_with(&r.replacement), "{path}: {r:?}");
        }
    }
    let bare = with_source_path(&rpt, "already-clean.rpt");
    let (_, report) = anonymized(&bare);
    assert!(
        !report
            .removals
            .iter()
            .any(|r| r.field.ends_with("source_path")),
        "a bare file name has nothing identifying to strip: {report:?}"
    );
}

/// Nothing outside the scrubbed field may move. This is what makes the command safe to run over a
/// whole corpus: the report decodes to the same model, so no baseline shifts except that field.
#[test]
fn nothing_else_in_the_model_changes() {
    let Some(rpt) = open(WITH_REIMPORT_RECORD) else {
        eprintln!("[skip] {WITH_REIMPORT_RECORD} absent");
        return;
    };
    let dirty = with_source_path(&rpt, DIRTY_PATH);
    let (clean, _) = anonymized(&dirty);
    let (before, after) = (dirty.report(), clean.report());

    assert_eq!(before.subreports.len(), after.subreports.len());
    assert_eq!(before.objects().count(), after.objects().count());
    assert_eq!(before.data_definition, after.data_definition);
    assert_eq!(before.database, after.database);
    assert_eq!(before.report_definition, after.report_definition);
    assert_eq!(before.print_options, after.print_options);
    assert_eq!(before.report_options, after.report_options);
    assert_eq!(before.summary_info, after.summary_info);
}

/// A second pass finds nothing, and a report with nothing to remove is returned byte-identical
/// rather than needlessly re-deflated — so re-running the command over a corpus is free and leaves
/// no diff.
#[test]
fn anonymize_is_idempotent() {
    let Some(rpt) = open(WITH_REIMPORT_RECORD) else {
        eprintln!("[skip] {WITH_REIMPORT_RECORD} absent");
        return;
    };
    let dirty = with_source_path(&rpt, DIRTY_PATH);
    let (clean, first) = anonymized(&dirty);
    assert!(!first.is_empty(), "expected removals on the first pass");

    let (bytes, second) = clean.anonymize().expect("second pass");
    assert!(second.is_empty(), "second pass found {second:?}");
    assert_eq!(
        bytes,
        clean.original_bytes(),
        "a no-op pass must not rewrite the file"
    );
}

/// The committed corpus carries no authoring metadata, and must not start carrying any: a fixture
/// added with a real author or a real authoring path fails here. This is the guard that keeps the
/// repository publishable, and it is also why the tests above synthesize their own dirty input.
#[test]
fn corpus_is_already_clean() {
    let reports = corpus();
    if reports.is_empty() {
        eprintln!("[skip] fixture corpus absent");
        return;
    }
    let mut dirty = Vec::new();
    for path in &reports {
        let rpt = Rpt::open(path).expect("fixture opens");
        let (_, report) = rpt.anonymize().expect("anonymize");
        for r in &report.removals {
            dirty.push(format!(
                "{}: {} = {:?}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                r.field,
                r.value
            ));
        }
    }
    assert!(
        dirty.is_empty(),
        "{} fixture(s) carry authoring metadata — run `rpt anonymize` on them:\n{}",
        dirty.len(),
        dirty.join("\n")
    );
}
