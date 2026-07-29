//! JSON baseline regression tests — the project's decode regression net.
//!
//! For every committed `.rpt`, run `rpt json-dump` and compare its output against a committed
//! baseline. The dump is the full serde serialization of the decoded model, so *any* change to a
//! decoded value shows up here — including fields no curated export projects.
//!
//! The reports live in two trees, and both are in the net (see [`CORPORA`]): the general fixture
//! corpus under `tests/fixtures/reports/`, and the synthetic Meridian corpus under
//! `tests/meridian/`. A report outside the net is a decode change nobody can see at L1, so the net
//! is defined by *walking* each tree rather than by a list — every `.rpt` found must have a
//! baseline, and every baseline must have a report ([`every_baseline_has_a_report`]).
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
//! machine runs it — so it is hermetic on every platform. The corpus is committed, so a walk that
//! finds no reports fails rather than skipping: a suite that compares nothing must not report `ok`.
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

/// A report tree and the baseline tree that mirrors it.
#[derive(Debug)]
struct Corpus {
    /// Names the tree in failure messages.
    name: &'static str,
    /// Walked recursively for `.rpt` files, relative to the workspace root.
    reports: &'static str,
    /// Holds one `<relative-stem>.json` per report found, relative to the workspace root.
    baselines: &'static str,
    /// The smallest count this tree is accepted as real at — below the committed number so adding
    /// or retiring a report does not move it, and far above what a wrong directory could yield.
    min_reports: usize,
}

/// Every report tree in the decode net. The Meridian root is walked whole rather than at
/// `reports/`, so the template — and anything else authored beside it — is covered too.
const CORPORA: &[Corpus] = &[
    Corpus {
        name: "fixtures",
        reports: "tests/fixtures/reports",
        baselines: "tests/fixtures/baselines/json",
        min_reports: 100,
    },
    Corpus {
        name: "meridian",
        reports: "tests/meridian",
        baselines: "tests/meridian/baselines/json",
        min_reports: 20,
    },
];

/// One report under test: where it lives, where its baseline lives, and the name that identifies it
/// in a failure message.
#[derive(Debug)]
struct Entry {
    label: String,
    report: PathBuf,
    baseline: PathBuf,
}

/// Every report in every corpus, sorted.
///
/// The corpora are committed, so an empty walk means the walk is wrong, not that the corpus is
/// absent — asserting here rather than skipping keeps a misresolved path from reporting `ok` having
/// compared nothing.
fn corpus() -> Vec<Entry> {
    let root = workspace_root();
    let mut out = Vec::new();
    for c in CORPORA {
        let reports_dir = root.join(c.reports);
        let mut reports = collect_reports(&reports_dir);
        assert!(
            reports.len() >= c.min_reports,
            "the {} corpus walk found only {} report(s) under {} — it is looking in the wrong \
             place, and a baseline suite that compares nothing passes",
            c.name,
            reports.len(),
            reports_dir.display()
        );
        reports.sort();
        out.extend(reports.into_iter().map(|(rel, report)| Entry {
            label: format!("{}/{rel}", c.name),
            baseline: root.join(c.baselines).join(format!("{rel}.json")),
            report,
        }));
    }
    out
}

#[test]
fn json_matches_baselines() {
    let reports = corpus();
    let bless = std::env::var_os("RPT_BLESS").is_some();

    let mut failures = Vec::new();
    for entry in &reports {
        let actual = json_dump(&entry.report);

        if bless {
            if let Some(parent) = entry.baseline.parent() {
                std::fs::create_dir_all(parent).expect("create baselines dir");
            }
            std::fs::write(&entry.baseline, &actual).expect("write baseline");
            continue;
        }

        match std::fs::read_to_string(&entry.baseline) {
            Ok(expected) => {
                let expected = expected.replace("\r\n", "\n");
                if expected != actual {
                    failures.push(unified_diff(&entry.label, &expected, &actual));
                }
            }
            Err(_) => failures.push(format!(
                "{}: missing baseline (run with RPT_BLESS=1)",
                entry.label
            )),
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
    for entry in &corpus() {
        let a = json_dump(&entry.report);
        let b = json_dump(&entry.report);
        assert_eq!(
            a, b,
            "{}: json-dump output differs between runs",
            entry.label
        );
        assert!(
            a.ends_with('\n'),
            "{}: output must end with a newline",
            entry.label
        );
    }
}

/// The other half of the exact match: a baseline with no report behind it compares nothing, and
/// would sit in the tree unnoticed after its report was renamed or retired. Walking the baseline
/// trees back to the corpus is what makes the net exact in both directions.
#[test]
fn every_baseline_has_a_report() {
    let root = workspace_root();
    let expected: std::collections::BTreeSet<PathBuf> =
        corpus().into_iter().map(|e| e.baseline).collect();

    let mut orphans = Vec::new();
    for c in CORPORA {
        let dir = root.join(c.baselines);
        let mut found = Vec::new();
        collect_baselines(&dir, &mut found);
        assert!(
            found.len() >= c.min_reports,
            "the {} baseline walk found only {} file(s) under {} — a suite that compares nothing \
             passes",
            c.name,
            found.len(),
            dir.display()
        );
        orphans.extend(found.into_iter().filter(|p| !expected.contains(p)));
    }

    assert!(
        orphans.is_empty(),
        "{} baseline(s) have no report behind them:\n{}",
        orphans.len(),
        orphans
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Collect every `.json` under `dir`, recursively.
fn collect_baselines(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_baselines(&p, out);
        } else if p.extension().is_some_and(|x| x == "json") {
            out.push(p);
        }
    }
}

/// Model keys that must never exist, because each names a value the decoder would have to *compute*
/// rather than read. Every one of these was on a model struct and in this dump at some point; each
/// was removed when its derivation moved to the consumer that needed it, and this list is what stops
/// one coming back unnoticed.
///
/// Adding a key here is cheap; adding a *field* whose value is not in the bytes is the thing to
/// resist. The four values the engine computes that rpt-rs deliberately does not reconstruct at all
/// (`HasSavedData`, a formula's runtime `NumberOfBytes`, `Field.UseCount`, the effective display
/// format) are documented in `CLAUDE.md`.
const DERIVED_KEYS: &[&str] = &[
    // A derived analytics surface the export once carried.
    "preferred_view",
    "use_count",
    "field_use_counts",
    // The engine's `SectionCode`: a band-kind base plus a dense ordinal over the sorted sections.
    "section_code",
    // A picture's natural size and scale factors — recomputed at load from the embedded image's
    // OLE extent, which a consumer reads from the picture bytes itself.
    "original_width",
    "original_height",
    "x_scaling",
    "y_scaling",
    // A box/line's end section, once resolved by walking section heights. The *stored* fact is
    // `end_section_index`, the section index the `0x00a9` opener carries.
    "end_section_name",
    // A drawing shape's stroke/fill, mirrored from the authoritative `Border`.
    "line_style",
    "line_color",
    "fill_color",
    // A parameter's `DiscreteOrRangeKind`, once inferred from whether a decoded value was a range;
    // its stored `0x007a` byte is unlocated.
    "discrete_or_range_kind",
];

/// The document's contract: valid JSON, the `model` section, and **nothing derived**. A dump that
/// lost a whole section — or quietly grew a computed one — would still diff cleanly against a
/// re-blessed baseline, so the shape is asserted independently of the baselines.
#[test]
fn json_dump_is_the_model_and_only_the_model() {
    for entry in &corpus() {
        let rel = &entry.label;
        let text = json_dump(&entry.report);
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
        for derived in DERIVED_KEYS {
            assert!(
                !text.contains(&format!("\"{derived}\"")),
                "{rel}: derived key `{derived}` leaked into the dump"
            );
        }
    }
}
