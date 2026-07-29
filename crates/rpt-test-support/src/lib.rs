//! Dev-only test helpers shared across the workspace's test suites.
//!
//! This crate carries the small amount of boilerplate that several crates' tests would otherwise
//! duplicate: resolving a fixture path relative to the workspace root, hand-building a [`SavedData`]
//! batch from literal columns and rows, and inspecting rendered PDF bytes ([`pdf`]). It is depended on
//! only under `[dev-dependencies]`.

pub mod pdf;

use rpt_model::{FieldValueType, SavedColumn, SavedData};
use std::path::{Path, PathBuf};

/// The workspace root directory — the parent of `crates/`.
///
/// Resolved from this crate's own `CARGO_MANIFEST_DIR` (`<root>/crates/rpt-test-support`), so it is
/// stable regardless of which crate's tests call it (each crate's own `CARGO_MANIFEST_DIR` differs).
///
/// # Panics
///
/// If the workspace root cannot be located from `CARGO_MANIFEST_DIR`. Test-support only, where an
/// unusable layout should fail loudly and immediately.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("rpt-test-support lives two levels below the workspace root")
        .to_path_buf()
}

/// Resolve `rel` (a path relative to the workspace root) to an absolute fixture path.
///
/// Lets any crate's tests reach shared fixtures under `tests/fixtures/…` without hard-coding a
/// per-crate `../..` prefix.
pub fn fixture(rel: impl AsRef<Path>) -> PathBuf {
    workspace_root().join(rel)
}

/// Every `.rpt` a corpus-wide sweep should see, sorted and deduplicated.
///
/// A sweep that walks *some* of the report trees can only support a claim scoped to those trees, and
/// a claim of the form "no record in the corpus does X" is exactly the kind that decides whether a
/// field gets decoded at all. So the corpus is discovered rather than named: every `.rpt` under
/// `tests/` (the committed fixtures, the Meridian corpus, and any newly added tree), plus any
/// directory listed in the colon-separated `RPT_EXTRA_CORPUS`. Callers that genuinely want a subset
/// should walk that subset explicitly and say so where the conclusion is recorded.
///
/// `RPT_EXTRA_CORPUS` is how an out-of-tree report set joins a sweep. Nothing outside the repository
/// is walked by default, so a claim of the form "no record in the corpus does X" is a claim about
/// the committed trees unless a run says otherwise.
///
/// Colon-separated `RPT_CORPUS` *replaces* the discovered roots, which is how a run narrows the
/// sweep to one tree to attribute a disagreement to it.
///
/// # Panics
///
/// If the walk finds implausibly few reports, which means it is rooted in the wrong place: a sweep
/// that scans nothing passes, and that failure mode is the one this helper exists to prevent.
/// Not checked when `RPT_CORPUS` names the roots, since a deliberate subset may be small.
pub fn corpus_reports() -> Vec<PathBuf> {
    let root = workspace_root();
    let narrowed = std::env::var("RPT_CORPUS").ok().filter(|s| !s.is_empty());
    let mut roots = match &narrowed {
        Some(list) => list.split(':').map(PathBuf::from).collect(),
        None => vec![root.join("tests")],
    };
    if let Ok(extra) = std::env::var("RPT_EXTRA_CORPUS") {
        roots.extend(
            extra
                .split(':')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
        );
    }
    let mut out = Vec::new();
    for dir in &roots {
        collect_rpt_files(dir, &mut out);
    }
    out.sort();
    out.dedup();
    assert!(
        narrowed.is_some() || out.len() > 100,
        "the corpus walk found only {} report(s) — it is looking in the wrong place",
        out.len()
    );
    out
}

/// Append every `.rpt` under `dir`, recursively. A directory that does not exist contributes
/// nothing, so an optional corpus needs no separate existence check.
pub fn collect_rpt_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_rpt_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rpt") {
            out.push(p);
        }
    }
}

/// Build a [`SavedData`] batch from column `(name, type)` pairs and row-major string cells.
///
/// Every cell is stored as a present (`Some`) value and `record_count` is taken from `rows.len()`.
/// For null cells or programmatically generated rows, construct [`SavedData`] directly.
pub fn saved_data(columns: &[(&str, FieldValueType)], rows: &[&[&str]]) -> SavedData {
    SavedData {
        record_count: rows.len() as u32,
        columns: columns
            .iter()
            .map(|(name, ty)| SavedColumn {
                name: (*name).to_string(),
                value_type: *ty,
            })
            .collect(),
        rows: rows
            .iter()
            .map(|row| row.iter().map(|c| Some((*c).to_string())).collect())
            .collect(),
    }
}

/// Compare `actual` text against the committed golden at `<manifest_dir>/tests/golden/<name>`.
///
/// The caller passes its own `env!("CARGO_MANIFEST_DIR")` as `manifest_dir` (this crate can't read
/// the caller's manifest at runtime). With `RPT_BLESS` set the golden is (re)written instead of
/// compared; a missing golden panics with a regenerate hint. `str` variant for text output (the
/// Page-IR JSON) — see [`assert_golden_bytes`] for binary backends.
///
/// # Panics
///
/// If `actual` differs from the golden file, or the golden cannot be read or written. That is the
/// assertion's purpose.
pub fn assert_golden(manifest_dir: &str, name: &str, actual: &str) {
    let dir = format!("{manifest_dir}/tests/golden");
    let path = format!("{dir}/{name}");
    if std::env::var_os("RPT_BLESS").is_some() {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing golden {path}; regenerate with RPT_BLESS=1"));
    assert_eq!(
        actual, expected,
        "golden mismatch for {name}; if intentional, regenerate with RPT_BLESS=1"
    );
}

/// Compare `actual` bytes against the committed golden at `<manifest_dir>/tests/golden/<name>`.
///
/// Byte-exact counterpart to [`assert_golden`] for binary backends (PDF, PNG). The caller passes its
/// own `env!("CARGO_MANIFEST_DIR")` as `manifest_dir`. With `RPT_BLESS` set the golden is (re)written
/// instead of compared; a missing golden panics with a regenerate hint.
///
/// # Panics
///
/// As [`assert_golden`], for byte content.
pub fn assert_golden_bytes(manifest_dir: &str, name: &str, actual: &[u8]) {
    let dir = format!("{manifest_dir}/tests/golden");
    let path = format!("{dir}/{name}");
    if std::env::var_os("RPT_BLESS").is_some() {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read(&path)
        .unwrap_or_else(|_| panic!("missing golden {path}; regenerate with RPT_BLESS=1"));
    assert!(
        actual == expected.as_slice(),
        "golden mismatch for {name} ({} vs {} bytes); if intentional, regenerate with RPT_BLESS=1",
        actual.len(),
        expected.len()
    );
}
