//! Exercises the full stack on a real report — decode → data → layout → Page IR.
//! Runs on a committed public-demo fixture, so the full render path is always exercised in CI.

use rpt_test_support::fixture;

#[test]
fn worrall_renders_end_to_end() {
    let path = fixture("tests/fixtures/reports/worrall/AlphaISOsByCountry.rpt");
    let rpt = rpt_reader::Rpt::open(&path).expect("open");
    let report = rpt.report();

    let doc = rpt_render::render(report);
    assert!(!doc.pages.is_empty(), "produced at least one page");
    assert_eq!(doc.pages.len(), doc.checkpoints.len());

    // The pages carry draw-ops (this report has detail fields + text).
    let total_ops: usize = doc.pages.iter().map(|p| p.ops.len()).sum();
    assert!(total_ops > 0, "pages have draw-ops");

    // The normalized IR JSON is valid for page 1.
    let json = doc.pages[0].to_normalized_json();
    assert!(json.contains("\"ops\""));
}
