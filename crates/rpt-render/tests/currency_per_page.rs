//! `OneCurrencySymbolPerPage` end to end, over the committed report that sets it.
//!
//! The rule is decided after pagination — which of a field's values is a page's first is not a
//! property of the value — so it is only observable once a real report has been laid out into pages.
//! The L4a listing for the same fixture freezes the resulting operators; this states the rule the
//! listing is supposed to encode, so removing the pass fails by name rather than as a diff.
//!
//! Hermetic: the report renders from its own embedded saved data, with the locale pinned (the stored
//! symbol wins over the locale's, but a host locale still decides the separators) and the default
//! bundled faces, so pagination is the same on every machine.

use rpt_pages::DrawOp;
use rpt_render::{Locale, RenderOptions};
use rpt_test_support::fixture;

/// Every text run on `page`, in emission (print) order.
fn page_texts(doc: &rpt_pages::PagedDocument, page: usize) -> Vec<&str> {
    doc.pages[page]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect()
}

/// The flagged field prints its `$` on its first value of each page and a two-space blank on the
/// rest, so every page carries exactly one symbol and the amounts keep their column.
#[test]
fn the_flagged_field_prints_one_symbol_per_page() {
    let path = fixture("tests/fixtures/reports/synthetic/currency_symbol_per_page.rpt");
    let rpt = rpt_reader::Rpt::open(&path).expect("open");
    let doc = rpt_render::render_with(
        rpt.report(),
        RenderOptions {
            locale: Locale::from_tag("en-US"),
            ..Default::default()
        },
    );

    assert!(doc.pages.len() > 1, "the report paginates");
    for page in 0..doc.pages.len() {
        let amounts: Vec<&str> = page_texts(&doc, page)
            .into_iter()
            .filter(|t| t.starts_with('$') || t.starts_with("  "))
            .collect();
        assert!(
            amounts.len() > 1,
            "page {page} draws several amounts: {amounts:?}"
        );
        let with_symbol: Vec<&&str> = amounts.iter().filter(|t| t.starts_with('$')).collect();
        assert_eq!(
            with_symbol.len(),
            1,
            "page {page} keeps exactly one symbol: {amounts:?}"
        );
        assert!(
            amounts[0].starts_with('$'),
            "the page's first value is the one that keeps it: {amounts:?}"
        );
    }
}
