//! Phase-6 gate: the full stack on a real report — decode → data → layout → Page IR → SVG.
//! Runs on a committed public-demo fixture, so the full render path is always exercised in CI.

use rpt_test_support::fixture;

#[test]
fn worrall_renders_end_to_end() {
    let path = fixture("tests/fixtures/reports/worrall/AlphaISOsByCountry.rpt");
    let rpt = rpt::Rpt::open(&path).expect("open");
    let report = rpt.report();

    let doc = rpt_render::render(report);
    assert!(!doc.pages.is_empty(), "produced at least one page");
    assert_eq!(doc.pages.len(), doc.checkpoints.len());

    // The pages carry draw-ops (this report has detail fields + text).
    let total_ops: usize = doc.pages.iter().map(|p| p.ops.len()).sum();
    assert!(total_ops > 0, "pages have draw-ops");

    // Every page renders to well-formed-ish SVG.
    let svgs = rpt_render::render_svg_pages(report);
    assert_eq!(svgs.len(), doc.pages.len());
    for svg in &svgs {
        assert!(svg.starts_with("<svg"));
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    // The normalized IR JSON (the parity contract) is valid for page 1.
    let json = doc.pages[0].to_normalized_json();
    assert!(json.contains("\"ops\""));
}
