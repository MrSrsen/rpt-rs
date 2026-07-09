//! XML baseline regression tests.
//!
//! For each `.rpt` fixture under `tests/fixtures/reports/`, run the exporter and compare its XML
//! against the committed baseline in `tests/fixtures/baselines/`. The exporter runs inside a
//! Bubblewrap sandbox with the report bind-mounted at a fixed path, so path-derived attributes
//! (e.g. the report's file name) are identical on every machine and the output is deterministic.
//! Output is normalized to LF; a mismatch prints a git-style unified diff.
//!
//! Regenerate the baselines after an intentional output change with:
//!
//! ```sh
//! RPT_BLESS=1 cargo test -p rpt-cli --test baseline
//! ```
//!
//! The test skips (rather than fails) when it cannot run hermetically: on non-Linux platforms,
//! when `bwrap` is not installed, or when the fixtures are absent.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The fixed path the report is mounted at inside the sandbox (on a writable tmpfs, so the
/// mountpoint can be created over the read-only host root).
const SANDBOX_RPT: &str = "/mnt/report.rpt";

/// The compiled `rpt` binary under test; the exporter is its `xml-dump` subcommand.
const EXPORTER: &str = env!("CARGO_BIN_EXE_rpt");

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

/// Collect every `.rpt` under `dir` (recursively) as `(relative-stem, path)`, where the relative
/// stem is the path below `dir` without the extension (e.g. `worrall/SportsTeams`). Reports are
/// grouped one directory deep by set, so each baseline mirrors the report's `<group>/<name>` path.
fn collect_reports(dir: &Path) -> Vec<(String, PathBuf)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else if p.extension().is_some_and(|x| x == "rpt") {
                let rel = p.strip_prefix(base).unwrap().with_extension("");
                out.push((rel.to_string_lossy().replace('\\', "/"), p));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

fn bwrap_available() -> bool {
    Command::new("bwrap")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Export one report to XML inside a Bubblewrap sandbox. The host root is bind-mounted read-only
/// (so the binary and its libraries resolve), and the report is bind-mounted at [`SANDBOX_RPT`].
fn export_in_sandbox(report: &Path) -> String {
    let report = report.canonicalize().expect("fixture path");
    let out = Command::new("bwrap")
        .args([
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--tmpfs",
            "/mnt",
            "--ro-bind",
            report.to_str().expect("utf-8 path"),
            SANDBOX_RPT,
            EXPORTER,
            "xml-dump",
            SANDBOX_RPT,
        ])
        .output()
        .expect("run bwrap");
    assert!(
        out.status.success(),
        "exporter failed for {}:\n{}",
        report.display(),
        String::from_utf8_lossy(&out.stderr),
    );
    // Normalize to LF so baselines are plain Unix-newline text and the comparison is
    // line-ending agnostic.
    let xml = String::from_utf8(out.stdout).expect("exporter emitted valid UTF-8");
    xml.replace("\r\n", "\n")
}

/// A git-style unified diff between the baseline and the current output, with line numbers and
/// `-`/`+` markers showing exactly which lines changed.
fn unified_diff(name: &str, baseline: &str, current: &str) -> String {
    let body = similar::TextDiff::from_lines(baseline, current)
        .unified_diff()
        .context_radius(3)
        .header(&format!("{name} (baseline)"), &format!("{name} (current)"))
        .to_string();
    format!("{name}: output differs\n{body}")
}

#[test]
fn xml_matches_baselines() {
    // Locally the test skips when it cannot run hermetically. CI sets RPT_REQUIRE_SANDBOX to turn
    // those skips into failures, so the blocking CI step can never pass by silently skipping.
    let require = std::env::var_os("RPT_REQUIRE_SANDBOX").is_some();
    let bail = |msg: &str| {
        assert!(!require, "{msg}");
        eprintln!("[skip] {msg}");
    };
    if std::env::consts::OS != "linux" {
        bail("baseline test requires Linux (Bubblewrap)");
        return;
    }
    if !bwrap_available() {
        bail("baseline test requires `bwrap` (bubblewrap) on PATH");
        return;
    }

    let root = workspace_root();
    let reports_dir = root.join("tests/fixtures/reports");
    // XML baselines mirror the report tree under `baselines/xml/<group>/<name>.xml`.
    let baselines_dir = root.join("tests/fixtures/baselines/xml");
    let bless = std::env::var_os("RPT_BLESS").is_some();

    // Reports are grouped one directory deep by set (`reports/<group>/<name>.rpt`); collect them with
    // their `<group>/<name>` relative key so each baseline mirrors the report's path.
    let mut reports = collect_reports(&reports_dir);
    if reports.is_empty() {
        eprintln!("[skip] no fixtures at {}", reports_dir.display());
        return;
    }
    reports.sort();

    let mut failures = Vec::new();
    for (rel, report) in &reports {
        let baseline = baselines_dir.join(format!("{rel}.xml"));
        let actual = export_in_sandbox(report);

        if bless {
            if let Some(parent) = baseline.parent() {
                std::fs::create_dir_all(parent).expect("create baselines dir");
            }
            std::fs::write(&baseline, &actual).expect("write baseline");
            continue;
        }

        match std::fs::read_to_string(&baseline) {
            Ok(expected) => {
                let expected = expected.replace("\r\n", "\n");
                if expected != actual {
                    failures.push(unified_diff(rel, &expected, &actual));
                }
            }
            Err(_) => failures.push(format!("{rel}: missing baseline (run with RPT_BLESS=1)")),
        }
    }

    if bless {
        eprintln!("blessed {} baseline(s)", reports.len());
        return;
    }
    assert!(
        failures.is_empty(),
        "{} baseline mismatch(es):\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
