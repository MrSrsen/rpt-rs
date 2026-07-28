//! Shared fixture discovery for the integration tests.
//!
//! Reports come from the committed public corpus under `tests/fixtures/reports/`, so these tests
//! run on any checkout. Fixture bytes, names, and strings never appear in assertions or output.
//!
//! `mod`-included into test binaries that need it; not every binary uses every helper.
#![allow(dead_code)]

use std::path::PathBuf;

/// The workspace root, from the shared [`rpt_test_support`] helper.
pub use rpt_test_support::workspace_root;

/// Every committed `.rpt` fixture, recursively, sorted for a stable iteration order.
pub fn public_rpts() -> Vec<PathBuf> {
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rpt") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&workspace_root().join("tests/fixtures/reports"), &mut out);
    out.sort();
    out
}

/// Print a uniform skip notice (visible with `cargo test -- --nocapture`).
pub fn skip(reason: &str) {
    eprintln!("[skip] {reason}");
}
