//! Render regression baselines for the data-free typography corpus.
//!
//! The five `tests/fixtures/reports/typography/*.rpt` fixtures are synthetic, **data-free** reports
//! (a blank report with static `TextObject`s stacked in the Report Header — no tables, no groups, no
//! formulas), each isolating one font/text axis: face, size, style flags, colour + alignment, and
//! paragraph indent/spacing. Because they bind no datasource they render with no `RowSource` at all,
//! so unlike the data-driven corpus in `postgres_fixtures.rs` this harness needs no database and runs
//! on every checkout.
//!
//! Two baselines per fixture:
//!   tests/fixtures/baselines/html/typography/<name>.html      — the HTML backend's output
//!   tests/fixtures/baselines/page-ir/typography/<name>.json   — the normalized Page IR, page by page
//!
//! The Page IR is the structural contract (op kinds, twip positions, text, resolved font), so it is
//! the baseline that actually pins layout behaviour; the HTML is the backend-serialization check on
//! top of it.
//!
//! Regenerate after an intentional render change with:
//!
//! ```sh
//! RPT_BLESS=1 cargo test -p rpt-render --test typography_baselines
//! ```
//!
//! **These baselines freeze OUR current behaviour, not engine parity.** The fixtures were compared
//! against the real SAP engine when they were authored, and the known divergences (text rotation not
//! applied, justified alignment not stretched, `LineSpacing` unmodelled, per-paragraph font size,
//! `CharacterSpacing`) are tracked as their own tickets. A baseline changing when one of those is
//! implemented is the expected, correct signal — re-bless it then.
//!
//! Like the other committed render baselines, these are blessed against the project's font
//! environment: text geometry comes from the host font stack via `rpt-text`, so a machine with a
//! different set of installed faces will legitimately differ.

use std::path::{Path, PathBuf};

use rpt_test_support::workspace_root;

/// Fixture directory, relative to the workspace root.
const REPORTS: &str = "tests/fixtures/reports/typography";

/// Collect the typography fixtures as `(name, path)`, sorted, so the harness picks up a newly added
/// fixture without being edited.
fn fixtures() -> Vec<(String, PathBuf)> {
    let dir = workspace_root().join(REPORTS);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rpt"))
        .filter_map(|p| {
            let name = p.file_stem()?.to_string_lossy().into_owned();
            Some((name, p))
        })
        .collect();
    out.sort();
    out
}

/// A git-style unified diff, matching the XML baseline harness's reporting.
fn unified_diff(name: &str, baseline: &str, current: &str) -> String {
    let body = similar::TextDiff::from_lines(baseline, current)
        .unified_diff()
        .context_radius(3)
        .header(&format!("{name} (baseline)"), &format!("{name} (current)"))
        .to_string();
    format!("{name}: output differs\n{body}")
}

/// Compare `actual` against the committed baseline at `path`, or write it when blessing. Returns a
/// diff to report on mismatch.
fn check(name: &str, path: &Path, actual: &str, bless: bool) -> Option<String> {
    if bless {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create baseline dir");
        }
        std::fs::write(path, actual).expect("write baseline");
        return None;
    }
    match std::fs::read_to_string(path) {
        Ok(expected) => {
            let expected = expected.replace("\r\n", "\n");
            (expected != actual).then(|| unified_diff(name, &expected, actual))
        }
        Err(_) => Some(format!("{name}: missing baseline (run with RPT_BLESS=1)")),
    }
}

#[test]
fn typography_render_matches_baselines() {
    let root = workspace_root();
    let bless = std::env::var_os("RPT_BLESS").is_some();

    let fixtures = fixtures();
    if fixtures.is_empty() {
        eprintln!("[skip] no typography fixtures at {REPORTS}");
        return;
    }

    let html_dir = root.join("tests/fixtures/baselines/html/typography");
    let ir_dir = root.join("tests/fixtures/baselines/page-ir/typography");

    let mut failures = Vec::new();
    for (name, path) in &fixtures {
        let rpt = rpt::Rpt::open(path).expect("open typography fixture");
        let report = rpt.report();

        // These fixtures bind no datasource, so a page must still be produced from the static text
        // alone — a zero-page render would make both baselines vacuously "match".
        let doc = rpt_render::render(report);
        assert!(
            !doc.pages.is_empty(),
            "{name}: data-free report produced no pages"
        );
        assert!(
            doc.pages.iter().any(|p| !p.ops.is_empty()),
            "{name}: rendered pages carry no draw-ops"
        );

        let html = rpt_render::render_html(report).replace("\r\n", "\n");
        if let Some(d) = check(name, &html_dir.join(format!("{name}.html")), &html, bless) {
            failures.push(d);
        }

        // One JSON document per page, concatenated with a page marker so a page count change is a
        // visible diff rather than a silently truncated comparison.
        let ir = rpt_render::render_ir_json(report)
            .iter()
            .enumerate()
            .map(|(i, p)| format!("// page {}\n{p}\n", i + 1))
            .collect::<String>();
        if let Some(d) = check(name, &ir_dir.join(format!("{name}.json")), &ir, bless) {
            failures.push(d);
        }
    }

    if bless {
        eprintln!("blessed {} typography fixture(s)", fixtures.len());
        return;
    }
    assert!(
        failures.is_empty(),
        "{} typography baseline mismatch(es):\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
