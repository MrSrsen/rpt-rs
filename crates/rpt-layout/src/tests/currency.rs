//! `OneCurrencySymbolPerPage`: the symbol prints on a field's first printed value of each page and
//! is blanked on the rest.

use super::*;
use rpt_model::{CurrencySymbolFormat, FieldFormat};

/// A detail-band currency field bound to `reference`, showing `symbol` as a leading currency symbol
/// from its own stored format. `per_page` sets `OneCurrencySymbolPerPage`.
fn currency_field(name: &str, reference: &str, symbol: &str, per_page: bool) -> ReportObject {
    let mut o = db_field_object(name, reference, 0);
    let mut fmt = FieldFormat::default();
    fmt.common.use_system_defaults = false;
    // A Currency-valued field is formatted by the currency slot, not the number slot.
    fmt.currency_numeric.currency_symbol = CurrencySymbolFormat::FloatingSymbol;
    fmt.currency_numeric.currency_symbol_text = symbol.to_string();
    fmt.currency_numeric.decimal_places = 2;
    fmt.currency_numeric.one_currency_symbol_per_page = per_page;
    if let ReportObjectKind::Field(f) = &mut o.kind {
        f.value_type = FieldValueType::Currency;
        f.format = Some(fmt);
    }
    o
}

/// The single-field case: `{t.amt}` with a `$` symbol.
fn amount_field(per_page: bool) -> ReportObject {
    currency_field("Amount", "{t.amt}", "$", per_page)
}

/// Four rows over a page that holds two detail bands, so the run is two pages of two values. The
/// saved batch carries both `{t.amt}` and `{t.fee}`; a doc that places only one field ignores the
/// other column.
fn doc_with(fields: Vec<ReportObject>) -> rpt_pages::PagedDocument {
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(700);
    report.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(AreaSectionKind::Detail, "Details", 300, fields)],
    )];
    let saved = saved_data(
        &[
            ("t.amt", FieldValueType::Currency),
            ("t.fee", FieldValueType::Currency),
        ],
        &[&["1", "5"], &["2", "6"], &["3", "7"], &["4", "8"]],
    );
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    layout(&report, &ds, &formulas)
}

/// Four amounts through one flagged field.
fn amounts_doc(per_page: bool) -> rpt_pages::PagedDocument {
    doc_with(vec![amount_field(per_page)])
}

/// Every text run on `page`, in emission (print) order.
fn page_texts(doc: &rpt_pages::PagedDocument, page: usize) -> Vec<String> {
    doc.pages[page]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_symbol_prints_once_per_page_on_the_first_value() {
    let doc = amounts_doc(true);
    assert_eq!(doc.pages.len(), 2, "four values at two bands per page");
    for page in 0..2 {
        let texts = page_texts(&doc, page);
        assert_eq!(texts.len(), 2, "page {page} holds two values");
        assert!(
            texts[0].contains('$'),
            "page {page}'s first value keeps the symbol: {texts:?}"
        );
        assert!(
            !texts[1].contains('$'),
            "page {page}'s second value loses it: {texts:?}"
        );
    }
}

/// The symbol is blanked, not deleted: the engine draws spaces covering the symbol's own advance so
/// the amount keeps its column, and the run's stored advance is re-measured against the new text.
#[test]
fn a_blanked_symbol_leaves_its_space_and_re_measures_the_run() {
    let doc = amounts_doc(true);
    let first = &doc.pages[0].ops[0];
    let second = &doc.pages[0].ops[1];
    let (DrawOp::Text(first), DrawOp::Text(second)) = (first, second) else {
        panic!("both ops are text runs");
    };
    assert_eq!(first.text, "$1.00");
    assert_eq!(
        second.text, "  2.00",
        "a one-character symbol is blanked by two spaces"
    );
    let advance = |run: &rpt_pages::TextRun| {
        crate::text::spaced_width_twips(
            &rpt_pages::ApproxLayout,
            &run.text,
            &run.font,
            run.character_spacing,
        ) as i32
    };
    assert_eq!(second.metrics.expect("measured").advance.0, advance(second));
}

/// The blank covers the symbol's characters, not its encoded bytes: a symbol that is one character
/// of several UTF-8 bytes is blanked by the same two spaces as a one-byte one.
#[test]
fn a_multi_byte_symbol_is_blanked_by_its_character_count() {
    let doc = doc_with(vec![currency_field("Amount", "{t.amt}", "\u{20ac}", true)]);
    let texts = page_texts(&doc, 0);
    assert_eq!(texts[0], "\u{20ac}1.00");
    assert_eq!(texts[1], "  2.00");
}

/// The grant is per field object, not per page: two flagged fields carry their own symbol, their own
/// placement and their own flag, so each keeps its symbol on its own first value of the page.
#[test]
fn each_flagged_field_keeps_its_own_symbol_on_the_page() {
    let mut fee = currency_field("Fee", "{t.fee}", "#", true);
    fee.bounds.left = Twips(4000);
    let doc = doc_with(vec![amount_field(true), fee]);
    assert_eq!(doc.pages.len(), 2, "four rows at two bands per page");
    for page in 0..2 {
        let texts = page_texts(&doc, page);
        assert_eq!(texts.len(), 4, "page {page} holds two rows of two values");
        assert!(
            texts[0].starts_with('$') && texts[1].starts_with('#'),
            "each field keeps its symbol on the page's first row: {texts:?}"
        );
        assert!(
            texts[2].starts_with("  ") && texts[3].starts_with("  "),
            "both lose it on the second row: {texts:?}"
        );
    }
}

/// A subreport holding one flagged `$` field over `rows` amounts, named `Sub`.
fn currency_subreport(rows: usize) -> rpt_model::Subreport {
    let mut sub = Report::default();
    sub.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(
            AreaSectionKind::Detail,
            "SubDetail",
            300,
            vec![currency_field("Amount", "{s.amt}", "$", true)],
        )],
    )];
    sub.saved_data = Some(SavedData {
        record_count: rows as u32,
        columns: vec![SavedColumn {
            name: "s.amt".into(),
            value_type: FieldValueType::Currency,
        }],
        rows: (1..=rows).map(|i| vec![Some(i.to_string())]).collect(),
    });
    let mut sr = rpt_model::Subreport::default();
    sr.name = "Sub".to_string();
    sr.report = Box::new(sub);
    sr
}

/// A placeholder box for the `Sub` subreport, `height` tall.
fn subreport_object(height: i32) -> ReportObject {
    let mut obj = ReportObject::default();
    obj.name = "SubObj".into();
    obj.bounds = Rect {
        left: Twips(0),
        top: Twips(0),
        width: Twips(6000),
        height: Twips(height),
    };
    let mut so = rpt_model::SubreportObject::default();
    so.subreport_name = "Sub".into();
    obj.kind = ReportObjectKind::Subreport(so);
    obj
}

/// A flagged field inside a subreport is scoped to the **host** page, not to the subreport's own
/// page sequence: a subreport flowing across three parent pages shows the symbol once on each of
/// them, not once for the whole subreport.
#[test]
fn a_subreport_field_is_scoped_to_the_host_page() {
    let mut main = Report::default();
    main.print_options.content_width = Twips(12240);
    // A body far shorter than the subreport's 6000 twips, so it flows across parent pages.
    main.print_options.content_height = Twips(2000);
    main.report_definition.areas = vec![area(
        AreaSectionKind::ReportHeader,
        vec![section(
            AreaSectionKind::ReportHeader,
            "RH",
            300,
            vec![subreport_object(300)],
        )],
    )];
    main.subreports = vec![currency_subreport(20)];

    let ds = build_dataset(&rpt_data::EmptySource, &main.data_definition);
    let formulas = rpt_data::compile_formulas(&main.data_definition);
    let doc = layout(&main, &ds, &formulas);

    assert!(
        doc.pages.len() > 2,
        "the subreport spans several host pages, got {}",
        doc.pages.len()
    );
    for page in 0..doc.pages.len() {
        let amounts: Vec<String> = page_texts(&doc, page)
            .into_iter()
            .filter(|t| t.starts_with('$') || t.starts_with("  "))
            .collect();
        assert_eq!(
            amounts.iter().filter(|t| t.starts_with('$')).count(),
            1,
            "host page {page} keeps exactly one symbol: {amounts:?}"
        );
        assert!(
            amounts[0].starts_with('$'),
            "the host page's first value is the one that keeps it: {amounts:?}"
        );
    }
}

/// The grant restarts for every subreport **instance**, not once per host page: two rows of a host
/// detail band each place the same subreport definition on one host page, and each instance keeps
/// the symbol on its own first value.
#[test]
fn two_subreport_instances_on_one_page_each_keep_their_symbol() {
    let mut main = Report::default();
    main.print_options.content_width = Twips(12240);
    // Room for both grown instances (2 rows of 300 each), so they share one host page.
    main.print_options.content_height = Twips(5000);
    main.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(
            AreaSectionKind::Detail,
            "Details",
            300,
            vec![subreport_object(300)],
        )],
    )];
    main.subreports = vec![currency_subreport(2)];

    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["1"], &["2"]]);
    let ds = build_dataset(&SavedDataSource::new(&saved), &main.data_definition);
    let formulas = rpt_data::compile_formulas(&main.data_definition);
    let doc = layout(&main, &ds, &formulas);

    assert_eq!(doc.pages.len(), 1, "both instances share one host page");
    assert_eq!(
        page_texts(&doc, 0),
        vec!["$1.00", "  2.00", "$1.00", "  2.00"],
        "each instance keeps the symbol on its own first value"
    );
}

/// With the flag off, every value prints its symbol — the pass is what makes the difference, not the
/// formatting.
#[test]
fn without_the_flag_every_value_keeps_its_symbol() {
    let doc = amounts_doc(false);
    for page in 0..2 {
        for text in page_texts(&doc, page) {
            assert!(text.contains('$'), "page {page}: {text}");
        }
    }
}
