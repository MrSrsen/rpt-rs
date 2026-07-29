//! L4a — committed PDF **content-stream** baselines: the regression surface for the PDF backend's
//! serialization.
//!
//! Where this sits. L3 (`postgres_fixtures.rs`, `typography_baselines.rs`) already pins the Page IR
//! across the whole corpus, so layout and pagination are covered before this layer runs. What is left
//! unproven is the step from Page IR to PDF operators: which font resource a run selects, the text and
//! transform matrices, path construction for shapes and chart interiors, image XObject placement. A
//! diff here with L3 green means the *writer* changed.
//!
//! One baseline per fixture:
//!   tests/fixtures/baselines/pdf/<group>/<name>.txt   — the normalized operator listing, page by page
//!
//! **Why the content stream and not the PDF bytes.** Our PDF output is byte-deterministic, so a byte
//! golden is technically possible — but it is unreadable before blessing (a one-twip move reads as
//! "binary files differ") and it pins incidental structure: object numbering, xref offsets, the
//! per-subset font tag. The operator listing is the writer's actual output as text, so an intentional
//! change is reviewable in the diff and an unintentional one is loud.
//! [`rpt_test_support::pdf::operator_listing`] documents exactly what it normalizes and why.
//!
//! **Hermetic by construction.** No database: every fixture renders from its own embedded saved data
//! (or, for the data-free typography set, from an empty source), so this harness runs on every
//! checkout under a plain `cargo test`. Both halves of the font stack are pinned to the bundled faces
//! — layout metrics via [`CosmicLayout`] over [`FontProvider::bundled`], embedding via
//! [`PdfOptions::fonts`] — because a PDF blessed against the host's installed faces embeds different
//! face bytes on the next machine and reads as a render regression.
//!
//! Regenerate after an intentional writer change with:
//!
//! ```sh
//! RPT_BLESS=1 cargo test -p rpt-render --test pdf_baselines
//! ```

// The listing pins font-accurate geometry, so it only means anything against the cosmic-text stack.

use std::path::{Path, PathBuf};

use rpt_data::RowSource;
use rpt_render::{
    render_backend, render_dataset_with, CosmicLayout, DateTimeSpecials, FontProvider, FontSource,
    Locale, PdfBackend, PdfOptions,
};
use rpt_test_support::{pdf::operator_listing, workspace_root};

/// The frozen "now" every fixture renders against, so a report reading a date special yields the same
/// page today and next year. 2023-11-14T22:13:20Z — the same instant the Page IR harnesses use.
const AS_OF_UNIX: i64 = 1_700_000_000;

/// The fixtures this layer covers, and the writer path each one buys.
///
/// Deliberately a handful, not the corpus: L3 covers layout everywhere, so a second full sweep here
/// would mostly re-pay for coverage we already have. Each entry earns its place with an operator path
/// the others do not reach.
const FIXTURES: &[&str] = &[
    // Font resolution and fallback: twelve distinct families on one page, so the /Font resource map
    // records which face each one resolved to and the listing shows one /fN per resolved face.
    "typography/font_faces",
    // The `Tf` size operand across 10-48 pt through one face, and the advances that follow from it.
    "typography/font_sizes",
    // Four subsets of one family (regular/bold/italic/bold-italic) embedded side by side — the
    // multi-face page, where selecting the wrong /fN is invisible in the Page IR.
    "typography/font_styles",
    // Rotation and color: 90°/270° runs, which ride a non-axis-aligned `cm` rather than the text
    // matrix, plus a `rg` color change per run and every horizontal alignment.
    "typography/text_color_align",
    // Two pages, so the listing carries more than one content stream and a page-count change is a
    // visible diff rather than a truncated comparison.
    "typography/paragraph_typography",
    // An image XObject (`/x0 Do` plus the image dictionary the listing summarizes), stroked rules
    // (`w`/`RG`/`S`), and real saved rows formatted into text. Also an L3 fixture, so the two layers
    // share a report and an L4a-only diff is unambiguous.
    "worrall/USStatesWithAbbreviations",
    // A chart interior: ~200 polygon paths with fill-and-stroke (`B`), the only vector work in the
    // corpus that is neither a rectangle nor a rule.
    "benbrahim777/Top5USA_piechart",
    // `OneCurrencySymbolPerPage` over three pages: the only report that exercises the
    // post-pagination currency pass, so the listing pins both the symbol-bearing first value of each
    // page and the re-measured advance of the blanked ones that follow.
    "synthetic/currency_symbol_per_page",
    // Four pages of grouped data: repeated page bands, per-page resources, page-number text.
    "benbrahim777/USA Orders, Percentages",
];

/// Render a fixture hermetically and project its PDF into the operator listing.
///
/// Rows come from the report's own saved data; a report that has none renders its static bands from an
/// empty source, the same as the offline path. The layout provider and the as-of instant are passed
/// explicitly (rather than inherited from the facade's defaults) so what this baseline is blessed
/// against is visible here.
fn listing_of(path: &Path, stem: &str) -> (String, usize) {
    let rpt = rpt_reader::Rpt::open(path).expect("open fixture report");
    let report = rpt.report();

    let saved_holder;
    let source: &dyn RowSource = match &report.saved_data {
        Some(saved) => {
            saved_holder = rpt_data::SavedDataSource::from_report(saved, report);
            &saved_holder
        }
        None => &rpt_data::EmptySource,
    };
    let as_of = DateTimeSpecials::from_unix_seconds(AS_OF_UNIX);
    let dataset = rpt_data::build_dataset_opts(
        source,
        &report.data_definition,
        rpt_data::DatasetOptions {
            datetime: Some(as_of),
            ..Default::default()
        },
    );
    let doc = render_dataset_with(
        report,
        &dataset,
        Box::new(CosmicLayout::new(FontProvider::bundled())),
        Locale::default(),
        None,
        Some(as_of),
    );
    assert!(!doc.pages.is_empty(), "{stem}: produced no pages");
    assert!(
        doc.pages.iter().any(|p| !p.ops.is_empty()),
        "{stem}: rendered pages carry no draw-ops"
    );

    // `Bundled` is the default, but it is stated here because it is the property the whole baseline
    // rests on: the embedded face must be this crate's own, not the host's installed one.
    let pdf = render_backend(
        &doc,
        &PdfBackend,
        &PdfOptions {
            fonts: FontSource::Bundled,
            ..Default::default()
        },
    );
    assert!(pdf.starts_with(b"%PDF"), "{stem}: not a PDF");
    let listing = operator_listing(&pdf);
    // The listing walks the PDF's own page tree, so this is a cross-check that the projection saw
    // every page the layout produced — a listing that silently covered half the document would
    // otherwise compare clean against a baseline blessed from the same half.
    assert!(
        listing.starts_with(&format!("% {} page(s)\n", doc.pages.len())),
        "{stem}: the listing's page count does not match the {} laid-out page(s):\n{}",
        doc.pages.len(),
        listing.lines().next().unwrap_or_default()
    );
    (listing, doc.pages.len())
}

/// A git-style unified diff, matching the other baseline harnesses' reporting.
fn unified_diff(name: &str, baseline: &str, current: &str) -> String {
    let body = similar::TextDiff::from_lines(baseline, current)
        .unified_diff()
        .context_radius(3)
        .header(&format!("{name} (baseline)"), &format!("{name} (current)"))
        .to_string();
    format!("{name}: PDF content stream differs from baseline\n{body}")
}

/// Compare `actual` against the committed baseline at `path`, or write it when blessing. Returns a
/// diff to report on mismatch.
fn check(label: &str, path: &Path, actual: &str, bless: bool) -> Option<String> {
    if bless {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("create baselines dir");
        }
        std::fs::write(path, actual).expect("write baseline");
        return None;
    }
    match std::fs::read_to_string(path) {
        Ok(expected) => {
            let expected = expected.replace("\r\n", "\n");
            (expected != actual).then(|| unified_diff(label, &expected, actual))
        }
        Err(_) => Some(format!(
            "{label}: missing baseline {} (run with RPT_BLESS=1)",
            path.display()
        )),
    }
}

/// Resolve a fixture's `<group>/<name>` key to the committed report, or `None` when this checkout does
/// not carry it.
fn report_path(root: &Path, rel: &str) -> Option<PathBuf> {
    let path = root
        .join("tests/fixtures/reports")
        .join(format!("{rel}.rpt"));
    path.is_file().then_some(path)
}

#[test]
fn pdf_content_streams_match_baselines() {
    let root = workspace_root();
    let bless = std::env::var_os("RPT_BLESS").is_some();
    let baselines = root.join("tests/fixtures/baselines/pdf");

    let mut ran = 0usize;
    let mut skipped = 0usize;
    let mut pages = 0usize;
    let mut failures = Vec::new();
    for rel in FIXTURES {
        let Some(path) = report_path(&root, rel) else {
            eprintln!("SKIP {rel}: not present on this checkout");
            skipped += 1;
            continue;
        };
        let (listing, page_count) = listing_of(&path, rel);
        ran += 1;
        pages += page_count;
        if let Some(d) = check(rel, &baselines.join(format!("{rel}.txt")), &listing, bless) {
            failures.push(d);
        }
    }

    eprintln!(
        "pdf baselines: {ran} fixture(s) / {pages} page(s) {}, {skipped} skipped",
        if bless { "blessed" } else { "checked" }
    );
    // Asserted in BOTH modes, deliberately. A bless that matched no fixture writes no baseline and
    // would otherwise exit green, leaving an empty baseline tree that later reads as "covered".
    assert_eq!(
        ran,
        FIXTURES.len(),
        "{skipped} of the {} committed reports listed in FIXTURES are missing, so their baselines \
         were neither checked nor blessed",
        FIXTURES.len()
    );
    if bless {
        return;
    }
    assert!(
        failures.is_empty(),
        "{} PDF baseline mismatch(es):\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
