//! Dev-only test helpers shared across the workspace's test suites.
//!
//! This crate carries the small amount of boilerplate that several crates' tests would otherwise
//! duplicate: resolving a fixture path relative to the workspace root, and hand-building a
//! [`SavedData`] batch from literal columns and rows. It is depended on only under
//! `[dev-dependencies]` and pulls in nothing beyond `rpt-model`.

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
/// compared; a missing golden panics with a regenerate hint. `str` variant for text backends (HTML,
/// SVG, Page-IR JSON) — see [`assert_golden_bytes`] for binary backends.
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
