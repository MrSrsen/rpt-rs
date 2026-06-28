//! Shared fixture discovery for the integration tests.
//!
//! `samples/` and `research/oracles/` are git-ignored. Fixture-dependent tests locate files
//! by path and skip gracefully when absent, so a clean checkout still compiles and runs green.
//! Fixture bytes, names, and strings never appear in assertions or output.
//!
//! `mod`-included into test binaries that need it; not every binary uses every helper.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// The workspace root (two levels up from this crate's `CARGO_MANIFEST_DIR`).
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

/// All `.rpt` files in `samples/`, or an empty vec if the directory is absent.
pub fn sample_rpts() -> Vec<PathBuf> {
    let dir = workspace_root().join("samples");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "rpt"))
        .collect();
    out.sort();
    out
}

/// Read an oracle file under `research/oracles/`, or `None` if absent.
pub fn oracle(rel: &str) -> Option<Vec<u8>> {
    std::fs::read(workspace_root().join("research/oracles").join(rel)).ok()
}

/// Print a uniform skip notice (visible with `cargo test -- --nocapture`).
pub fn skip(reason: &str) {
    eprintln!("[skip] {reason}");
}
