//! JSON baseline regression tests — the project's decode regression net.
//!
//! For each `.rpt` fixture under `tests/fixtures/reports/`, run `rpt json-dump` and compare its
//! output against the committed baseline in `tests/fixtures/baselines/json/`. The dump is the full
//! serde serialization of the decoded model, so *any* change to a decoded value shows up here —
//! including fields no curated export projects.
//!
//! It is a two-sided guarantee, and the second side matters as much as the first: the dump carries
//! **stored facts only**, nothing derived. A derived value in the baseline would move when the
//! *derivation* changed, with the bytes untouched, and a diff would stop meaning "the decoder's
//! reading of this file changed". [`json_dump_is_the_model_and_only_the_model`] enforces that.
//!
//! Regenerate the baselines after an intentional output change with:
//!
//! ```sh
//! RPT_BLESS=1 cargo test -p rpt-cli --test json_baseline
//! ```
//!
//! The test needs no sandbox: `json-dump` takes no locale, `source_path` is a stored field rather
//! than the invocation path, and nothing else in the output depends on where the report sits or which
//! machine runs it — so it is hermetic on every platform. It skips only when the fixture corpus is
//! absent, so a bare checkout still tests clean.
#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use std::process::Command;

use rpt_test_support::workspace_root;

/// The compiled `rpt` binary under test; the exporter is its `json-dump` subcommand.
const EXPORTER: &str = env!("CARGO_BIN_EXE_rpt");

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

/// Run `rpt json-dump <report>` and return its stdout, normalized to LF so baselines are plain
/// Unix-newline text and the comparison is line-ending agnostic.
fn json_dump(report: &Path) -> String {
    let out = Command::new(EXPORTER)
        .arg("json-dump")
        .arg(report)
        .output()
        .unwrap_or_else(|e| panic!("run rpt json-dump: {e}"));
    assert!(
        out.status.success(),
        "exporter failed for {}:\n{}",
        report.display(),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8(out.stdout)
        .expect("exporter emitted valid UTF-8")
        .replace("\r\n", "\n")
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

/// The fixture corpus as `(relative-stem, path)` pairs, sorted; `None` (skip) when it is absent.
fn corpus() -> Option<Vec<(String, PathBuf)>> {
    let reports_dir = workspace_root().join("tests/fixtures/reports");
    let mut reports = collect_reports(&reports_dir);
    if reports.is_empty() {
        eprintln!("[skip] no fixtures at {}", reports_dir.display());
        return None;
    }
    reports.sort();
    Some(reports)
}

#[test]
fn json_matches_baselines() {
    let Some(reports) = corpus() else { return };
    // Baselines mirror the report tree under `baselines/json/<group>/<name>.json`.
    let baselines_dir = workspace_root().join("tests/fixtures/baselines/json");
    let bless = std::env::var_os("RPT_BLESS").is_some();

    let mut failures = Vec::new();
    for (rel, report) in &reports {
        let baseline = baselines_dir.join(format!("{rel}.json"));
        let actual = json_dump(report);

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

/// The baselines are only a regression net if the dump is reproducible; a second run of the same
/// report must be byte-identical, or a green suite would only mean "nothing changed *this* run".
#[test]
fn json_dump_is_deterministic() {
    let Some(reports) = corpus() else { return };
    for (rel, report) in &reports {
        let a = json_dump(report);
        let b = json_dump(report);
        assert_eq!(a, b, "{rel}: json-dump output differs between runs");
        assert!(a.ends_with('\n'), "{rel}: output must end with a newline");
    }
}
/// The document's contract: valid JSON, the `model` section, and **nothing derived**. A dump that
/// lost a whole section — or quietly grew a computed one — would still diff cleanly against a
/// re-blessed baseline, so the shape is asserted independently of the baselines.
#[test]
fn json_dump_is_the_model_and_only_the_model() {
    let Some(reports) = corpus() else { return };
    for (rel, report) in &reports {
        let text = json_dump(report);
        let doc: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{rel}: json-dump output must be valid JSON: {e}"));

        let obj = doc.as_object().expect("top level is a JSON object");
        let model = obj
            .get("model")
            .and_then(serde_json::Value::as_object)
            .unwrap_or_else(|| panic!("{rel}: missing `model` object"));

        // The model is the serialized report: exhaustive, so core sections are always present.
        for key in [
            "version",
            "report_definition",
            "data_definition",
            "database",
        ] {
            assert!(model.contains_key(key), "{rel}: model missing `{key}`");
        }

        // Stored facts only. These are values the engine *computes*; if one reappears, a derivation
        // has leaked back into the baseline surface, and a change to that derivation would start
        // moving baselines with the decode untouched — exactly what this surface must not do.
        assert!(
            !obj.contains_key("analysis"),
            "{rel}: the dump must not carry a derived `analysis` section"
        );
        for derived in ["preferred_view", "use_count", "field_use_counts"] {
            assert!(
                !text.contains(&format!("\"{derived}\"")),
                "{rel}: derived key `{derived}` leaked into the dump"
            );
        }
    }
}
