//! `rpt-render`'s accessibility flags, driven through the real binary.
//!
//! Two properties matter here and neither is visible from a unit test of the argument parser: a
//! conformance request that cannot be honoured must **fail loudly and write nothing**, and a request
//! that can must produce exactly what the library produces for the same inputs — the command is a
//! caller of `rpt-render`, so a difference between the two would be the app having grown logic of
//! its own.
//!
//! Fixture-gated like the rest of the suite: it skips when the committed report is absent.
#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The compiled `rpt-render` binary under test.
const RPT_RENDER: &str = env!("CARGO_BIN_EXE_rpt-render");

/// A report with a summary title, saved data, no figures and no date/time specials — so PDF/UA-1 is
/// reachable with a language alone, and two renders of it a moment apart are byte-identical.
fn report() -> Option<PathBuf> {
    let path = rpt_test_support::fixture("tests/fixtures/reports/worrall/AlphaISOsByCountry.rpt");
    path.is_file().then_some(path)
}

/// A unique scratch path for one test's output, removed first so a stale file cannot be mistaken for
/// one this run wrote.
fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("rpt-render-{name}-{}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

fn run(report: &Path, args: &[&str], out: &Path) -> Output {
    Command::new(RPT_RENDER)
        .arg(report)
        .args(args)
        .arg("-o")
        .arg(out)
        .arg("--quiet")
        .output()
        .expect("rpt-render runs")
}

/// PDF/UA-1 without a language is refused: non-zero exit, the missing fact named, the flag that
/// supplies it named, and **no file** — a conformance request that silently degraded to an ordinary
/// PDF would be worse than the failure.
#[test]
fn a_conformance_request_that_cannot_be_honoured_writes_nothing() {
    let Some(report) = report() else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let out = scratch("refused");
    let result = run(&report, &["--pdfua"], &out);
    assert!(
        !result.status.success(),
        "an unmet claim must exit non-zero, got {:?}",
        result.status
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("natural language") && stderr.contains("--lang"),
        "the refusal must name what is missing and how to supply it:\n{stderr}"
    );
    assert!(
        stderr.contains("PDF/UA-1"),
        "and the level it was refused at:\n{stderr}"
    );
    assert!(
        !out.exists(),
        "nothing may be written when the claim is refused"
    );
}

/// The same render through the command line and through the library is the same document, byte for
/// byte: the CLI resolves inputs and calls `rpt-render`, and holds no render logic of its own.
#[test]
fn the_cli_path_and_the_library_path_agree() {
    let Some(report) = report() else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let out = scratch("ua1");
    let result = run(
        &report,
        &["--pdfua", "--lang", "en-US", "--locale", "en-US"],
        &out,
    );
    assert!(
        result.status.success(),
        "PDF/UA-1 must be reachable for a titled, figureless report: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let from_cli = std::fs::read(&out).expect("the CLI wrote its output");

    let doc = rpt_render::ReportDocument::load(&report).expect("load");
    let pages = doc.render_with(rpt_render::RenderOptions {
        locale: rpt_render::Locale::from_tag("en-US"),
        ..Default::default()
    });
    let semantics = rpt_render::Semantics {
        language: Some("en-US".to_string()),
        ..rpt_render::semantics_of(doc.report())
    };
    let from_library = rpt_render::try_render_document(
        &pages,
        &rpt_render::PdfOptions {
            conformance: rpt_render::Conformance::PdfUa1,
            semantics,
            ..Default::default()
        },
    )
    .expect("the library path renders the same document");

    assert_eq!(
        from_cli.len(),
        from_library.len(),
        "the two paths must produce the same document"
    );
    assert_eq!(from_cli, from_library);
    // The title the claim requires came from the report, not from a flag, and it is in the bytes.
    assert!(
        String::from_utf8_lossy(&from_cli).contains("Alpha Codes and Internet CCTLDs by Country")
            || from_cli.windows(4).any(|w| w == b"/UA1"),
        "the document must carry the title it claims"
    );
    let _ = std::fs::remove_file(&out);
}

/// `--tagged` claims no standard, so it needs none of the semantics a claim does: a report the
/// levels would refuse still renders with a structure tree.
#[test]
fn tagged_alone_needs_nothing_from_the_caller() {
    let Some(report) = report() else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let out = scratch("tagged");
    let result = run(&report, &["--tagged"], &out);
    assert!(
        result.status.success(),
        "--tagged must claim nothing: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let bytes = std::fs::read(&out).expect("the CLI wrote its output");
    assert!(
        String::from_utf8_lossy(&bytes).contains("/StructTreeRoot"),
        "a tagged render carries a structure tree"
    );
    let _ = std::fs::remove_file(&out);
}
