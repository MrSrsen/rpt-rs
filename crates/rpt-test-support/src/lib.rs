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
