//! Shared fixture discovery for the integration tests.
//!
//! Reports come from the committed public corpora under `tests/`, so these tests run on any
//! checkout. Fixture bytes, names, and strings never appear in assertions or output.
//!
//! `mod`-included into test binaries that need it; not every binary uses every helper.
#![allow(dead_code)]

use std::path::PathBuf;

/// The workspace root, from the shared [`rpt_test_support`] helper.
pub use rpt_test_support::workspace_root;

/// Every committed `.rpt` fixture, recursively, sorted for a stable iteration order.
///
/// Every report tree under `tests/` — a corpus named one directory at a time leaves the newest one
/// uncovered.
///
/// # Panics
///
/// If the walk finds implausibly few reports. A sweep rooted in the wrong place scans nothing and so
/// asserts nothing, which is the failure this floor exists to turn into a test failure.
pub fn public_rpts() -> Vec<PathBuf> {
    let mut out = Vec::new();
    rpt_test_support::collect_rpt_files(&workspace_root().join("tests"), &mut out);
    out.sort();
    assert!(
        out.len() > 100,
        "the committed fixture walk found only {} report(s) — it is looking in the wrong place",
        out.len()
    );
    out
}

/// Print a uniform skip notice (visible with `cargo test -- --nocapture`).
pub fn skip(reason: &str) {
    eprintln!("[skip] {reason}");
}
