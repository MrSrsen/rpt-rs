use super::*;

#[test]
fn lays_out_detail_rows_onto_pages() {
    let saved = saved_data(
        &[("t.x", FieldValueType::Number)],
        &[&["1"], &["2"], &["3"]],
    );
    let report = tiny_report(15840); // full letter — everything on one page
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);

    let doc = layout(&report, &ds, &formulas);
    assert_eq!(doc.pages.len(), 1);
    let texts: Vec<&str> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    // Page header once + 3 detail rows.
    assert_eq!(texts.iter().filter(|t| **t == "REPORT").count(), 1);
    assert_eq!(texts.iter().filter(|t| **t == "line").count(), 3);
    // A checkpoint per page.
    assert_eq!(doc.checkpoints.len(), 1);
}

/// A field heading is a static column-label object with no bound field: its stored text renders as a
/// text run in the page header (which lays out without any data row), same as a text object.
#[test]
fn renders_field_heading_text() {
    let mut report = tiny_report(15840);
    report.report_definition.areas[0].sections[0]
        .objects
        .push(field_heading_object("Head", "Product", 0));
    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["1"]]);
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);

    let doc = layout(&report, &ds, &formulas);
    let texts: Vec<&str> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        texts.contains(&"Product"),
        "field heading text should render, got {texts:?}"
    );
    // No unsupported-object diagnostic for the heading.
    assert!(
        !doc.diagnostics
            .iter()
            .any(|d| d.message.contains("FieldHeading")),
        "field heading should not warn as unrendered"
    );
}

#[test]
fn paginates_when_body_overflows() {
    let saved = SavedData {
        record_count: 20,
        columns: vec![SavedColumn {
            name: "t.x".into(),
            value_type: FieldValueType::Number,
        }],
        rows: (0..20).map(|i| vec![Some(i.to_string())]).collect(),
    };
    // Tiny page: header 300 + a few detail bands (300 each) fit, then it must spill to new pages.
    let report = tiny_report(2000);
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);

    let doc = layout(&report, &ds, &formulas);
    assert!(
        doc.pages.len() > 1,
        "expected multiple pages, got {}",
        doc.pages.len()
    );
    assert_eq!(doc.pages.len(), doc.checkpoints.len());
    // Every page repeats the header.
    for page in &doc.pages {
        let headers = page
            .ops
            .iter()
            .filter(|op| matches!(op, DrawOp::Text(t) if t.text == "REPORT"))
            .count();
        assert_eq!(headers, 1, "each page repeats the page header");
    }
    // All 20 rows rendered across pages.
    let total_rows: usize = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter(|op| matches!(op, DrawOp::Text(t) if t.text == "line"))
        .count();
    assert_eq!(total_rows, 20);
}

/// A report header/footer field bound to a database field has a record context: the header resolves
/// against the report's first record, the footer against its last (Crystal's band record context).
#[test]
fn report_header_and_footer_resolve_against_first_and_last_record() {
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "ReportHeader",
                300,
                vec![db_field_object("RhVal", "t.name", 0)],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                300,
                vec![db_field_object("DetVal", "t.name", 0)],
            )],
        ),
        area(
            AreaSectionKind::ReportFooter,
            vec![section(
                AreaSectionKind::ReportFooter,
                "ReportFooter",
                300,
                vec![db_field_object("RfVal", "t.name", 0)],
            )],
        ),
    ];
    let saved = saved_data(
        &[("t.name", FieldValueType::String)],
        &[&["Alpha"], &["Beta"], &["Gamma"]],
    );
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);

    let doc = layout(&report, &ds, &formulas);
    let texts: Vec<&str> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    // Report header → first record; report footer → last record; details → each record.
    assert!(
        texts.first() == Some(&"Alpha"),
        "report header should render the first record, got {texts:?}"
    );
    assert!(
        texts.last() == Some(&"Gamma"),
        "report footer should render the last record, got {texts:?}"
    );
    assert_eq!(texts, vec!["Alpha", "Alpha", "Beta", "Gamma", "Gamma"]);
}

/// A group footer field bound to a database field resolves against the group's **last** record
/// (Crystal's group-footer record context), while the group header resolves against its first.
#[test]
fn group_header_and_footer_resolve_against_group_first_and_last_record() {
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    let mut group = rpt_model::Group::default();
    group.condition_field = "t.g".to_string();
    report.data_definition.groups = vec![group];
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::GroupHeader,
            vec![section(
                AreaSectionKind::GroupHeader,
                "GroupHeaderArea1",
                300,
                vec![db_field_object("GhVal", "t.name", 0)],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                300,
                vec![db_field_object("DetVal", "t.name", 0)],
            )],
        ),
        area(
            AreaSectionKind::GroupFooter,
            vec![section(
                AreaSectionKind::GroupFooter,
                "GroupFooterArea1",
                300,
                vec![db_field_object("GfVal", "t.name", 0)],
            )],
        ),
    ];
    // One group (g="A") with three records; header→first (Alpha), footer→last (Gamma).
    let saved = saved_data(
        &[
            ("t.g", FieldValueType::String),
            ("t.name", FieldValueType::String),
        ],
        &[&["A", "Alpha"], &["A", "Beta"], &["A", "Gamma"]],
    );
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);

    let doc = layout(&report, &ds, &formulas);
    let texts: Vec<&str> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["Alpha", "Alpha", "Beta", "Gamma", "Gamma"]);
}

#[test]
fn empty_dataset_emits_group_band_skeleton() {
    // A report that defines group bands but produced no group (empty dataset) still lays out its
    // group header/footer skeleton once, so the static captions render instead of a blank page.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    let mut group = rpt_model::Group::default();
    group.condition_field = "t.g".to_string();
    report.data_definition.groups = vec![group];
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::GroupHeader,
            vec![section(
                AreaSectionKind::GroupHeader,
                "GroupHeaderArea1",
                300,
                vec![text_object("Title", "STATEMENT", 0)],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                300,
                vec![db_field_object("DetVal", "t.name", 0)],
            )],
        ),
        area(
            AreaSectionKind::GroupFooter,
            vec![section(
                AreaSectionKind::GroupFooter,
                "GroupFooterArea1",
                300,
                vec![text_object("Total", "TOTAL DUE", 0)],
            )],
        ),
    ];
    // No records → no group instance.
    let saved = saved_data(
        &[
            ("t.g", FieldValueType::String),
            ("t.name", FieldValueType::String),
        ],
        &[],
    );
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    assert!(ds.groups.is_empty(), "expected an empty dataset");
    let formulas = rpt_data::compile_formulas(&report.data_definition);

    let doc = layout(&report, &ds, &formulas);
    let texts: Vec<&str> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    // Both the group-header and group-footer static captions render; the detail band (a record's
    // value) does not, since there is no record.
    assert!(
        texts.contains(&"STATEMENT"),
        "group-header skeleton not emitted: {texts:?}"
    );
    assert!(
        texts.contains(&"TOTAL DUE"),
        "group-footer skeleton not emitted: {texts:?}"
    );
}

#[test]
fn hide_for_drill_down_area_emits_no_bands() {
    // An area marked "Hide (Drill-Down OK)" contributes no bands to the normal render. Here the detail
    // area is hidden while the group header stays visible: the group header renders, the detail rows
    // do not.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    let mut group = rpt_model::Group::default();
    group.condition_field = "t.g".to_string();
    report.data_definition.groups = vec![group];
    let mut detail_area = area(
        AreaSectionKind::Detail,
        vec![section(
            AreaSectionKind::Detail,
            "Details",
            300,
            vec![db_field_object("DetVal", "t.name", 0)],
        )],
    );
    detail_area.format.hide_for_drill_down = true;
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::GroupHeader,
            vec![section(
                AreaSectionKind::GroupHeader,
                "GroupHeaderArea1",
                300,
                vec![text_object("Hdr", "HEADER", 0)],
            )],
        ),
        detail_area,
    ];
    let saved = saved_data(
        &[
            ("t.g", FieldValueType::String),
            ("t.name", FieldValueType::String),
        ],
        &[&["A", "Alpha"], &["A", "Beta"]],
    );
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);

    let doc = layout(&report, &ds, &formulas);
    let texts: Vec<&str> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        texts.contains(&"HEADER"),
        "group header should render: {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| *t == "Alpha" || *t == "Beta"),
        "hidden detail rows must not render: {texts:?}"
    );
}

#[test]
fn report_header_prints_above_page_header_on_page_one() {
    // A report with ReportHeader (title) + PageHeader (label) + one detail row. The report header
    // must sit ABOVE the page header at the top of page 1 (Crystal band order) — a regression guard
    // for page-1 band ordering.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "RH",
                400,
                vec![text_object("Title", "TITLE", 0)],
            )],
        ),
        area(
            AreaSectionKind::PageHeader,
            vec![section(
                AreaSectionKind::PageHeader,
                "PH",
                300,
                vec![text_object("Hdr", "COLHEAD", 0)],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                300,
                vec![text_object("Row", "row", 0)],
            )],
        ),
    ];
    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["1"]]);
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    let top_of = |text: &str| -> i32 {
        doc.pages[0]
            .ops
            .iter()
            .find_map(|op| match op {
                DrawOp::Text(t) if t.text == text => Some(t.bounds.top.0),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no text {text}"))
    };
    let (title_y, head_y, row_y) = (top_of("TITLE"), top_of("COLHEAD"), top_of("row"));
    assert!(
        title_y < head_y,
        "report header ({title_y}) must be above page header ({head_y})"
    );
    assert!(
        head_y < row_y,
        "page header ({head_y}) must be above detail ({row_y})"
    );
}

#[test]
fn can_grow_band_taller_than_body_gets_one_page_each_never_stalls() {
    // A can-grow detail that wraps taller than the whole body must still emit (at the page top) and
    // then move the next record to a fresh page — the `cursor_y > 0` guard is the only
    // protection against a page that can never fit the band looping forever.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(500); // far shorter than the wrapped can-grow band
    report.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(
            AreaSectionKind::Detail,
            "Details",
            240,
            vec![wrapping_can_grow("Memo")],
        )],
    )];
    let saved = numeric_rows(3);
    let doc = rendered(&report, &saved);

    // One over-tall record per page: it is emitted, then the next record forces a new page.
    assert_eq!(doc.pages.len(), 3, "one page per over-tall record");
    assert_eq!(doc.pages.len(), doc.checkpoints.len());
    for page in &doc.pages {
        let runs = page
            .ops
            .iter()
            .filter(|op| matches!(op, DrawOp::Text(_)))
            .count();
        assert!(runs > 1, "each page renders the wrapped can-grow band");
    }
}

#[test]
fn report_footer_overflows_onto_a_new_page() {
    // Details fill the page exactly; the report footer can't fit and paginates onto a fresh page.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(900); // fits exactly three 300-twip detail bands
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                300,
                vec![text_object("Row", "line", 0)],
            )],
        ),
        area(
            AreaSectionKind::ReportFooter,
            vec![section(
                AreaSectionKind::ReportFooter,
                "RF",
                300,
                vec![text_object("Total", "TOTAL", 0)],
            )],
        ),
    ];
    let saved = numeric_rows(3);
    let doc = rendered(&report, &saved);

    assert_eq!(doc.pages.len(), 2, "footer spills to a second page");
    // The three details are on page 1; the footer alone on page 2.
    assert_eq!(page_text_tops(&doc.pages[0], "line").len(), 3);
    assert_eq!(page_text_tops(&doc.pages[0], "TOTAL").len(), 0);
    assert_eq!(page_text_tops(&doc.pages[1], "TOTAL").len(), 1);
}

#[test]
fn page_footer_is_pinned_at_the_bottom_of_every_page() {
    // A page footer repeats pinned at the same bottom offset on every page, and body content never
    // overlaps it.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(1500);
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::PageHeader,
            vec![section(
                AreaSectionKind::PageHeader,
                "PH",
                300,
                vec![text_object("Hdr", "REPORT", 0)],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                300,
                vec![text_object("Row", "line", 0)],
            )],
        ),
        area(
            AreaSectionKind::PageFooter,
            vec![section(
                AreaSectionKind::PageFooter,
                "PF",
                300,
                vec![text_object("Foot", "FOOTER", 0)],
            )],
        ),
    ];
    let saved = numeric_rows(12);
    let doc = rendered(&report, &saved);

    assert!(doc.pages.len() > 1, "content spans multiple pages");
    let mut footer_top = None;
    for page in &doc.pages {
        let feet = page_text_tops(page, "FOOTER");
        assert_eq!(feet.len(), 1, "every page has exactly one pinned footer");
        // The footer sits at the same bottom offset on each page.
        match footer_top {
            None => footer_top = Some(feet[0]),
            Some(t) => assert_eq!(feet[0], t, "footer pinned at a consistent offset"),
        }
        // No body row overlaps the footer.
        for row_top in page_text_tops(page, "line") {
            assert!(
                row_top < feet[0],
                "detail {row_top} must sit above the footer {}",
                feet[0]
            );
        }
    }
}

#[test]
fn multi_column_page_break_keeps_a_column_row_together() {
    use rpt_model::MultiColumn;
    // Body fits one column-row; a second column-row paginates as a unit (the break is decided at
    // column 0, so records in a row are never split across a page boundary).
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(400); // one 300-twip row fits; the next does not
    report.print_options.multi_column = Some(MultiColumn {
        columns: 2,
        column_width: Twips(3000),
        gap_h: Twips(0),
        gap_v: Twips(0),
        across_then_down: true,
    });
    report.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(
            AreaSectionKind::Detail,
            "Details",
            300,
            vec![text_object("Cell", "X", 0)],
        )],
    )];
    let saved = numeric_rows(4);
    let doc = rendered(&report, &saved);

    assert_eq!(doc.pages.len(), 2, "second column-row moves to a new page");
    // Two cells per page (a full column-row), never a split row.
    assert_eq!(page_text_tops(&doc.pages[0], "X").len(), 2);
    assert_eq!(page_text_tops(&doc.pages[1], "X").len(), 2);
}

#[test]
fn new_page_after_breaks_after_each_section_without_trailing_blank() {
    // NewPageAfter on the detail band starts a fresh page after each record; the trailing one is
    // deferred so it does not leave a blank final page.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(6000); // ample room — the break is the flag, not overflow
    let mut detail = section(
        AreaSectionKind::Detail,
        "Details",
        300,
        vec![text_object("Row", "line", 0)],
    );
    detail.format.base.new_page_after = true;
    report.report_definition.areas = vec![area(AreaSectionKind::Detail, vec![detail])];
    let doc = rendered(&report, &numeric_rows(3));
    assert_eq!(doc.pages.len(), 3, "one page per record, no trailing blank");
    for page in &doc.pages {
        assert_eq!(page_text_tops(page, "line").len(), 1);
    }
}

#[test]
fn new_page_before_breaks_before_each_section_without_leading_blank() {
    // NewPageBefore on the detail band starts a fresh page before each record, but the first record
    // (already at the top of page 1) does not get a leading blank page.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(6000);
    let mut detail = section(
        AreaSectionKind::Detail,
        "Details",
        300,
        vec![text_object("Row", "line", 0)],
    );
    detail.format.base.new_page_before = true;
    report.report_definition.areas = vec![area(AreaSectionKind::Detail, vec![detail])];
    let doc = rendered(&report, &numeric_rows(3));
    assert_eq!(doc.pages.len(), 3, "one page per record, no leading blank");
    for page in &doc.pages {
        assert_eq!(page_text_tops(page, "line").len(), 1);
    }
}

#[test]
fn underlay_section_backs_following_detail() {
    let report = underlay_report(true);
    let saved = underlay_saved(2);
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);

    let doc = layout(&report, &ds, &formulas);
    assert_eq!(doc.pages.len(), 1);
    let page = &doc.pages[0];
    let (mark_i, mark_top) = text_op(page, "WMARK").expect("underlay text emitted");
    let (row_i, row_top) = text_op(page, "line").expect("detail text emitted");
    // Painter's order: the underlay is emitted first so it sits UNDER the following detail.
    assert!(mark_i < row_i, "underlay op precedes the detail op");
    // The detail overlays the underlay rather than being pushed below it: its top is at or above
    // the underlay band's bottom.
    assert!(
        row_top <= mark_top + 600,
        "detail (top {row_top}) overlaps the underlay band [{mark_top}..{}]",
        mark_top + 600
    );
    assert_eq!(row_top, mark_top, "detail starts at the underlay's top");
}

#[test]
fn non_underlay_section_pushes_detail_below() {
    // The control: with underlay off the Report Header advances the cursor, so the detail lands
    // below the header band (the normal, unchanged flow).
    let report = underlay_report(false);
    let saved = underlay_saved(2);
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);

    let doc = layout(&report, &ds, &formulas);
    let page = &doc.pages[0];
    let (_, mark_top) = text_op(page, "WMARK").expect("header text emitted");
    let (_, row_top) = text_op(page, "line").expect("detail text emitted");
    assert_eq!(
        row_top,
        mark_top + 600,
        "detail is pushed below the full header band height"
    );
}

#[test]
fn underlay_section_with_no_following_content_draws_normally() {
    // Guard: an underlay band with nothing after it still draws its own ops and paginates once.
    let report = underlay_report(true);
    let saved = underlay_saved(0); // no detail rows follow
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);

    let doc = layout(&report, &ds, &formulas);
    assert_eq!(doc.pages.len(), 1);
    assert!(
        text_op(&doc.pages[0], "WMARK").is_some(),
        "the lone underlay band still draws"
    );
    assert!(
        text_op(&doc.pages[0], "line").is_none(),
        "no detail rows to overlay it"
    );
}

#[test]
fn an_underlay_group_header_backs_the_detail_and_ends_at_its_group_footer() {
    // The span rule: the detail rows draw over the underlay from its top, and the underlay's
    // companion — the group footer of the same level — is NOT underlaid, so it prints below the
    // underlay's full 4000-twip height even though the rows it backed were only 900 twips.
    let (report, saved) = group_underlay_report(true, 4000, 15840);
    let doc = rendered(&report, &saved);
    assert_eq!(doc.pages.len(), 1);
    let page = &doc.pages[0];

    let (_, gh_top) = text_op(page, "GHNORMAL").expect("plain group header emitted");
    let (u_i, u_top) = text_op(page, "UTOP").expect("underlay emitted");
    assert_eq!(gh_top, 0);
    assert_eq!(u_top, 400, "the underlay follows the plain header band");
    // Painter's order plus a shared top: the rows overlay the underlay rather than being pushed below.
    assert_eq!(page_text_tops(page, "line"), vec![400, 700, 1000]);
    let (row_i, _) = text_op(page, "line").expect("detail emitted");
    assert!(u_i < row_i, "the underlay op precedes the detail it backs");
    let (_, gf_top) = text_op(page, "GROUPFTR").expect("group footer emitted");
    assert_eq!(
        gf_top, 4400,
        "the group footer clears the underlay's bottom"
    );
}

#[test]
fn a_plain_group_header_of_the_same_size_pushes_everything_below_it() {
    // The control: with underlay off the tall band advances the cursor, so the rows start after it.
    let (report, saved) = group_underlay_report(false, 4000, 15840);
    let doc = rendered(&report, &saved);
    let page = &doc.pages[0];
    assert_eq!(page_text_tops(page, "line"), vec![4400, 4700, 5000]);
    assert_eq!(text_op(page, "GROUPFTR").expect("footer emitted").1, 5300);
}

#[test]
fn an_underlay_shorter_than_what_it_backs_leaves_the_cursor_alone() {
    // The span only ever drops the cursor: content that outgrew the underlay (900 twips of rows over
    // a 600-twip band) keeps its own position, so the footer follows the rows, not the underlay.
    let (report, saved) = group_underlay_report(true, 600, 15840);
    let doc = rendered(&report, &saved);
    let page = &doc.pages[0];
    assert_eq!(page_text_tops(page, "line"), vec![400, 700, 1000]);
    assert_eq!(text_op(page, "GROUPFTR").expect("footer emitted").1, 1300);
}

#[test]
fn an_underlay_span_does_not_survive_a_page_turn() {
    // An underlay is drawn once, on its own page. When the rows it backs overflow, the footer lands
    // on the next page and flows from that page's top — the stale span must not push it down.
    let (report, saved) = group_underlay_report(true, 4000, 2000);
    let doc = rendered(&report, &saved);
    assert!(doc.pages.len() > 1, "the tall underlay forces a page turn");
    let last = doc.pages.last().expect("a page");
    let (_, gf_top) = text_op(last, "GROUPFTR").expect("group footer emitted");
    assert_eq!(gf_top, 0, "the footer starts at the top of its own page");
}

#[test]
fn a_nested_underlay_ends_at_its_own_level_footer() {
    // A level-2 group header's span closes at the level-2 footer, not the outer one: GF2 clears the
    // underlay, and the second inner group opens right after it rather than inheriting the span.
    let (report, saved) = nested_underlay_report();
    let doc = rendered(&report, &saved);
    let page = &doc.pages[0];
    assert_eq!(text_op(page, "GH1").expect("outer header").1, 0);
    // Inner group 1: underlay at 300, its two rows over it, GF2 below the underlay's 4000 twips.
    // Inner group 2 repeats the shape from there; GF1 closes the outer group last.
    assert_eq!(page_text_tops(page, "UTOP"), vec![300, 4600]);
    assert_eq!(page_text_tops(page, "line"), vec![300, 600, 4600, 4900]);
    assert_eq!(page_text_tops(page, "GF2"), vec![4300, 8600]);
    assert_eq!(page_text_tops(page, "GF1"), vec![8900]);
}

#[test]
fn keep_group_together_moves_a_group_that_would_split_to_a_fresh_page() {
    // Group A (header + 2 details = 900 twips) fills page 1; group B would not fit in the remaining
    // space but fits on a page by itself, so KeepGroupTogether moves the whole of group B to page 2 —
    // its header and both details stay on one page.
    let (report, saved) = keep_together_report(1200, true);
    let doc = rendered(&report, &saved);
    assert_eq!(doc.pages.len(), 2, "group B moves to a fresh page");
    assert_eq!(
        page_text_tops(&doc.pages[0], "GH").len(),
        1,
        "only group A header on page 1"
    );
    assert_eq!(page_text_tops(&doc.pages[0], "line").len(), 2);
    // Group B's header and both details are together on page 2.
    assert_eq!(page_text_tops(&doc.pages[1], "GH").len(), 1);
    assert_eq!(page_text_tops(&doc.pages[1], "line").len(), 2);
}

#[test]
fn without_keep_group_together_a_group_splits_across_the_page_break() {
    // Control: the same geometry without KeepGroupTogether lets group B's header print on page 1 and
    // its details spill onto page 2 (the group splits) — proving the flag, not the geometry, is what
    // holds the group together above.
    let (report, saved) = keep_together_report(1200, false);
    let doc = rendered(&report, &saved);
    // Group B's header lands on page 1 (right after group A), so page 1 carries two headers.
    assert_eq!(
        page_text_tops(&doc.pages[0], "GH").len(),
        2,
        "both headers on page 1 when the group is allowed to split"
    );
}

#[test]
fn print_at_bottom_of_page_pins_a_footer_to_the_body_bottom() {
    // A report footer with PrintAtBottomOfPage is pinned against the bottom of the body (above where a
    // page footer would sit), not printed directly under the last detail.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(3000);
    let mut footer = section(
        AreaSectionKind::ReportFooter,
        "RF",
        300,
        vec![text_object("Total", "TOTAL", 0)],
    );
    footer.format.base.print_at_bottom_of_page = true;
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                300,
                vec![text_object("Row", "line", 0)],
            )],
        ),
        area(AreaSectionKind::ReportFooter, vec![footer]),
    ];
    let doc = rendered(&report, &numeric_rows(2));
    assert_eq!(doc.pages.len(), 1);
    let feet = page_text_tops(&doc.pages[0], "TOTAL");
    assert_eq!(feet.len(), 1);
    // body_bottom (3000, no page footer) minus the 300-twip footer height = 2700, well below the two
    // detail rows (tops 0 and 300).
    assert_eq!(feet[0], 2700, "footer pinned at the body bottom");
}

#[test]
fn reset_page_number_after_restarts_the_page_counter() {
    // A group footer with ResetPageNumberAfter (+ NewPageAfter) restarts the page-number counter, so
    // the page that begins after group A is numbered 1 again (per-group page numbering). The counter
    // is observed via the per-page checkpoints, which record the live page number at each page top.
    let (mut report, saved) = keep_together_report(15840, false); // ample page; the break is the flag
                                                                  // Add a group footer that ends the page and resets the counter after each group.
    let mut gf = section(AreaSectionKind::GroupFooter, "GF", 300, vec![]);
    gf.format.base.new_page_after = true;
    gf.format.base.reset_page_number_after = true;
    report
        .report_definition
        .areas
        .push(area(AreaSectionKind::GroupFooter, vec![gf]));
    let doc = rendered(&report, &saved);
    assert_eq!(
        doc.pages.len(),
        2,
        "NewPageAfter splits the two groups across pages"
    );
    let page_numbers: Vec<u32> = doc.checkpoints.iter().map(|c| c.page_number).collect();
    assert_eq!(page_numbers, vec![1, 1], "page counter reset after group A");
}

#[test]
fn multi_column_new_page_after_breaks_after_each_record() {
    use rpt_model::MultiColumn;
    // NewPageAfter on a multi-column detail band breaks after each record (the deferral path, so no
    // trailing blank page) even though records would otherwise flow across two columns on one page.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(6000); // ample — the break is the flag, not overflow
    report.print_options.multi_column = Some(MultiColumn {
        columns: 2,
        column_width: Twips(3000),
        gap_h: Twips(0),
        gap_v: Twips(0),
        across_then_down: true,
    });
    let mut detail = section(
        AreaSectionKind::Detail,
        "Details",
        300,
        vec![text_object("Cell", "X", 0)],
    );
    detail.format.base.new_page_after = true;
    report.report_definition.areas = vec![area(AreaSectionKind::Detail, vec![detail])];
    let doc = rendered(&report, &numeric_rows(3));
    assert_eq!(doc.pages.len(), 3, "one record per page, no trailing blank");
    for page in &doc.pages {
        assert_eq!(page_text_tops(page, "X").len(), 1);
    }
}

#[test]
fn approximate_layout_emits_pagination_diagnostic() {
    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["1"]]);
    let report = tiny_report(15840);
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);

    // `layout` injects the dependency-free ApproxLayout, so the one-shot divergence diagnostic fires.
    let doc = layout(&report, &ds, &formulas);
    assert!(
        doc.diagnostics
            .iter()
            .any(|d| d.message.contains("approximate text layout")),
        "ApproxLayout must emit the pagination-divergence diagnostic: {:?}",
        doc.diagnostics
    );
}

/// A metric-accurate layout (`is_approximate()` == false, the trait default) must NOT emit the
/// pagination-divergence diagnostic.
#[test]
fn exact_layout_emits_no_pagination_diagnostic() {
    #[derive(Debug)]
    struct ExactLayout;
    impl crate::TextLayout for ExactLayout {
        fn width_twips(&self, text: &str, font: &rpt_pages::FontSpec) -> f64 {
            text.chars().count() as f64 * font.size_pt as f64
        }
    }
    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["1"]]);
    let report = tiny_report(15840);
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);

    let doc = crate::layout_with(&report, &ds, &formulas, Box::new(ExactLayout));
    assert!(
        !doc.diagnostics
            .iter()
            .any(|d| d.message.contains("approximate text layout")),
        "a metric-accurate layout must not emit the approximate-layout diagnostic: {:?}",
        doc.diagnostics
    );
}

/// `PageNofM` must show the true final page count on every page: the layout resolves the total as a
/// forward reference once all pages exist, so page 1 of a 3-page report reads "Page 1 of 3" (not "Page 1 of 1").
#[test]
fn page_n_of_m_resolves_final_total() {
    let mut o = ReportObject::default();
    o.name = "PageNofM".into();
    o.bounds = Rect {
        left: Twips(0),
        top: Twips(0),
        width: Twips(3000),
        height: Twips(240),
    };
    let mut f = FieldObject::default();
    f.data_source = "PageNofM".into();
    f.ref_kind = FieldRefKind::Special;
    o.kind = ReportObjectKind::Field(Box::new(f));

    let mut report = tiny_report(1500); // small body → detail rows overflow to several pages
    report.report_definition.areas.push(area(
        AreaSectionKind::PageFooter,
        vec![section(
            AreaSectionKind::PageFooter,
            "PageFooter",
            300,
            vec![o],
        )],
    ));
    let doc = rendered(&report, &numeric_rows(6));
    let total = doc.pages.len();
    assert!(total >= 2, "small body must paginate: {total} page(s)");
    for (i, page) in doc.pages.iter().enumerate() {
        let feet: Vec<String> = page
            .ops
            .iter()
            .filter_map(|op| match op {
                DrawOp::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .filter(|s| s.contains(" of "))
            .collect();
        assert_eq!(
            feet,
            vec![format!("Page {} of {}", i + 1, total)],
            "page {} footer shows the final total",
            i + 1
        );
    }
}

/// A row-background box must (1) draw behind the row content even though it is stored after the
/// fields in object order, and (2) grow with a can-grow detail band so the shading covers the whole
/// rendered row rather than only its fixed-height top slice.
#[test]
fn section_box_underlays_and_grows_with_the_band() {
    // A section-spanning shaded box (top 0, height 284 of a 300-twip section).
    let mut boxo = ReportObject::default();
    boxo.name = "Zebra".into();
    boxo.bounds = Rect {
        left: Twips(0),
        top: Twips(0),
        width: Twips(9000),
        height: Twips(284),
    };
    boxo.kind = ReportObjectKind::Box(rpt_model::BoxShape::default());
    let shade = Color {
        a: 255,
        r: 0xf2,
        g: 0xf5,
        b: 0xf8,
    };
    boxo.border.background_color = Some(shade);

    // A can-grow field that wraps to several lines, growing the band past the box's design height.
    let mut txt = ReportObject::default();
    txt.name = "Wrap".into();
    txt.bounds = Rect {
        left: Twips(0),
        top: Twips(0),
        width: Twips(1000),
        height: Twips(240),
    };
    txt.format.can_grow = true;
    let mut t = TextObject::default();
    t.text = "alpha bravo charlie delta echo foxtrot golf hotel india".into();
    t.font_color.font.size_pt = 10.0;
    txt.kind = ReportObjectKind::Text(t);

    let mut report = tiny_report(15840);
    // Detail section holds the field first, then the box: the box draws underneath despite the
    // storage order, matching the engine's z-order.
    report.report_definition.areas[1].sections[0].objects = vec![txt, boxo];
    let doc = rendered(&report, &numeric_rows(1));

    let ops = &doc.pages[0].ops;
    let box_idx = ops.iter().position(|op| {
        matches!(op, DrawOp::Rect(r) if r.fill.as_ref().map(rpt_pages::Fill::representative_color) == Some(shade))
    });
    let text_idx = ops
        .iter()
        .position(|op| matches!(op, DrawOp::Text(t) if t.text.starts_with("alpha")));
    let (box_idx, text_idx) = (box_idx.expect("shaded box op"), text_idx.expect("text op"));
    assert!(
        box_idx < text_idx,
        "the box fill must precede (underlay) the row text: box@{box_idx} text@{text_idx}"
    );
    // The band grew (multi-line wrap): the box height tracks it, exceeding the 284-twip design box.
    let DrawOp::Rect(r) = &ops[box_idx] else {
        unreachable!()
    };
    assert!(
        r.bounds.height.0 > 284,
        "the box must grow with the band, got {} twips",
        r.bounds.height.0
    );
}

/// A box whose stored end-section index names a later section spans down to that section's bottom,
/// not only its own band.
#[test]
fn box_spans_to_end_section() {
    // Page header (300) then detail (500): a box in the page header (section 0) whose end-section
    // index is the detail section (1) must reach the detail bottom = 300 + 500 = 800 twips from the
    // box top.
    let mut boxo = ReportObject::default();
    boxo.name = "Frame".into();
    boxo.bounds = Rect {
        left: Twips(0),
        top: Twips(0),
        width: Twips(6000),
        height: Twips(200), // decoded height only within its own band
    };
    let mut bs = rpt_model::BoxShape::default();
    bs.shape.end_section_index = 1; // the Details section
    let border = Color {
        a: 255,
        r: 0x33,
        g: 0x33,
        b: 0x33,
    };
    boxo.kind = ReportObjectKind::Box(bs);
    boxo.border.top = LineStyle::SingleLine;
    boxo.border.border_color = Some(border);

    let mut report = tiny_report(15840);
    report.report_definition.areas[1].sections[0].height = Twips(500);
    report.report_definition.areas[0].sections[0]
        .objects
        .push(boxo);
    let doc = rendered(&report, &numeric_rows(1));
    let frame = doc.pages[0]
        .ops
        .iter()
        .find_map(|op| match op {
            DrawOp::Rect(r) if r.stroke.is_some() => Some(r),
            _ => None,
        })
        .expect("the spanning box rect");
    assert_eq!(
        frame.bounds.top,
        Twips(0),
        "box keeps its top in the header"
    );
    assert_eq!(
        frame.bounds.height,
        Twips(800),
        "box spans from its top to the detail section bottom (300 + 500)"
    );
}

/// A line whose stored end-section index names a later section extends its lower endpoint down to
/// that section's bottom.
#[test]
fn line_spans_to_end_section() {
    // A vertical line in the page header (section 0) ending in the detail section (1) reaches
    // y = 300 + 500 = 800.
    let mut lineo = ReportObject::default();
    lineo.name = "Rule".into();
    lineo.bounds = Rect {
        left: Twips(1000),
        top: Twips(0),
        width: Twips(0),
        height: Twips(200), // only within its own band before spanning
    };
    let mut ls = LineShape::default();
    ls.shape.end_section_index = 1; // the Details section
    ls.shape.line_thickness = Twips(10);
    lineo.kind = ReportObjectKind::Line(ls);
    lineo.border.left = LineStyle::SingleLine;
    lineo.border.border_color = Some(Color {
        a: 255,
        r: 0,
        g: 0,
        b: 0,
    });

    let mut report = tiny_report(15840);
    report.report_definition.areas[1].sections[0].height = Twips(500);
    report.report_definition.areas[0].sections[0]
        .objects
        .push(lineo);
    let doc = rendered(&report, &numeric_rows(1));
    let line = doc.pages[0]
        .ops
        .iter()
        .find_map(|op| match op {
            DrawOp::Line(l) => Some(l),
            _ => None,
        })
        .expect("the spanning line op");
    let bottom = line.from.y.0.max(line.to.y.0);
    assert_eq!(bottom, 800, "the line reaches the detail section bottom");
}

/// A `suppress-if-blank` detail section that resolves to no visible content is dropped and reserves
/// no vertical space, so following rows are not pushed onto extra pages. The same section with
/// non-empty text is kept and does occupy its height.
#[test]
fn suppress_if_blank_section_reserves_no_space_when_empty() {
    fn build(note_text: &str, rows: usize) -> crate::PagedDocument {
        let mut report = Report::default();
        report.print_options.content_width = Twips(12240);
        // Page body 1200 twips after the 300 header: 4 detail-only rows/page, but only 2 rows/page
        // once a 300-twip note band is also reserved per row.
        report.print_options.content_height = Twips(1500);
        // A note section with SuppressIfBlank set, carrying one text object.
        let mut notes = section(
            AreaSectionKind::Detail,
            "Notes",
            300,
            vec![text_object("Note", note_text, 0)],
        );
        notes.format.suppress_if_blank = true;
        report.report_definition.areas = vec![
            area(
                AreaSectionKind::PageHeader,
                vec![section(
                    AreaSectionKind::PageHeader,
                    "PageHeader",
                    300,
                    vec![text_object("Hdr", "REPORT", 0)],
                )],
            ),
            area(
                AreaSectionKind::Detail,
                vec![
                    section(
                        AreaSectionKind::Detail,
                        "Details",
                        300,
                        vec![text_object("Row", "line", 0)],
                    ),
                    notes,
                ],
            ),
        ];
        let saved = SavedData {
            record_count: rows as u32,
            columns: vec![SavedColumn {
                name: "t.x".into(),
                value_type: FieldValueType::Number,
            }],
            rows: (0..rows).map(|i| vec![Some(i.to_string())]).collect(),
        };
        let src = SavedDataSource::new(&saved);
        let ds = build_dataset(&src, &report.data_definition);
        let formulas = rpt_data::compile_formulas(&report.data_definition);
        layout(&report, &ds, &formulas)
    }

    // Empty note: the band is blank → suppressed, so 4 rows fit on a single page.
    let blank = build("", 4);
    assert_eq!(
        blank.pages.len(),
        1,
        "a blank suppress-if-blank band must reserve no space"
    );
    let note_runs = blank
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter(|op| matches!(op, DrawOp::Text(t) if t.source.as_ref().is_some_and(|s| s.object_name.as_deref() == Some("Note"))))
        .count();
    assert_eq!(note_runs, 0, "a suppressed blank band emits no runs");

    // Non-empty note: the band is kept and reserves 300 twips per row → only 2 rows/page → 2 pages.
    let kept = build("NOTE", 4);
    assert_eq!(
        kept.pages.len(),
        2,
        "a non-blank suppress-if-blank band still occupies its height"
    );
    let notes_shown = kept
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter(|op| matches!(op, DrawOp::Text(t) if t.text == "NOTE"))
        .count();
    assert_eq!(notes_shown, 4, "every non-blank note renders");
}

// --- Per-section paging limits ("Records per page" / "Groups per page"). ---

/// `groups` region groups of `rows_per_group` rows each, grouped by `t.region`, with a 300-twip
/// group header ("GH") and a 300-twip detail band ("line"). `records_per_page` sets the Detail
/// area's cap and `groups_per_page` the group-header area's; `0` is the stored "no limit".
fn paging_limit_report(
    page_height: i32,
    groups: usize,
    rows_per_group: usize,
    records_per_page: i32,
    groups_per_page: i32,
) -> (Report, SavedData) {
    use rpt_model::{Group, GroupAreaFormat, SortDirection};
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(page_height);
    let mut g = Group::default();
    g.condition_field = "t.region".into();
    g.sort.direction = SortDirection::AscendingOrder;
    report.data_definition.groups = vec![g];

    let mut gaf = GroupAreaFormat::default();
    gaf.visible_groups_per_page = groups_per_page;
    let mut gh = area(
        AreaSectionKind::GroupHeader,
        vec![section(
            AreaSectionKind::GroupHeader,
            "GH",
            300,
            vec![text_object("Hdr", "GH", 0)],
        )],
    );
    gh.format.group = Some(gaf);
    let mut detail = area(
        AreaSectionKind::Detail,
        vec![section(
            AreaSectionKind::Detail,
            "Details",
            300,
            vec![text_object("Row", "line", 0)],
        )],
    );
    detail.format.visible_records_per_page = records_per_page;
    report.report_definition.areas = vec![gh, detail];

    let rows: Vec<Vec<Option<String>>> = (0..groups)
        .flat_map(|gi| {
            (0..rows_per_group).map(move |ri| {
                vec![
                    Some(((b'A' + gi as u8) as char).to_string()),
                    Some((gi * rows_per_group + ri).to_string()),
                ]
            })
        })
        .collect();
    let saved = SavedData {
        record_count: rows.len() as u32,
        columns: vec![
            SavedColumn {
                name: "t.region".into(),
                value_type: FieldValueType::String,
            },
            SavedColumn {
                name: "t.x".into(),
                value_type: FieldValueType::Number,
            },
        ],
        rows,
    };
    (report, saved)
}

#[test]
fn records_per_page_breaks_at_the_cap_not_where_the_page_fills() {
    // 3 groups × 4 rows on a page tall enough for every band: without a cap the whole report is one
    // page. A cap of 5 breaks after the 5th record wherever it falls — the count runs across the
    // group boundary, so page 1 carries group A's 4 rows plus group B's first.
    let (uncapped, saved) = paging_limit_report(9000, 3, 4, 0, 0);
    let doc = rendered(&uncapped, &saved);
    assert_eq!(doc.pages.len(), 1, "height alone never breaks this report");

    let (capped, saved) = paging_limit_report(9000, 3, 4, 5, 0);
    let doc = rendered(&capped, &saved);
    let per_page: Vec<usize> = doc
        .pages
        .iter()
        .map(|p| page_text_tops(p, "line").len())
        .collect();
    assert_eq!(per_page, vec![5, 5, 2], "the break lands at the 5th record");
    // Group B opens on page 1 (records 5) and continues on page 2, so page 1 carries two headers.
    let headers: Vec<usize> = doc
        .pages
        .iter()
        .map(|p| page_text_tops(p, "GH").len())
        .collect();
    assert_eq!(headers, vec![2, 1, 0]);
}

#[test]
fn records_per_page_break_precedes_the_next_group_header() {
    // Group A's 5 rows exactly fill a cap of 5. The next group's header does not print in the space
    // left on page 1: a page with no record quota left starts the next group on a fresh page.
    let (report, saved) = paging_limit_report(9000, 2, 5, 5, 0);
    let doc = rendered(&report, &saved);
    assert_eq!(doc.pages.len(), 2);
    assert_eq!(page_text_tops(&doc.pages[0], "GH").len(), 1);
    assert_eq!(
        page_text_tops(&doc.pages[1], "GH"),
        vec![0],
        "group B's header opens page 2"
    );
}

#[test]
fn groups_per_page_breaks_at_the_cap_not_where_the_page_fills() {
    // 4 groups × 2 rows fit one page on height alone; a cap of 2 groups splits them 2 + 2.
    let (uncapped, saved) = paging_limit_report(9000, 4, 2, 0, 0);
    assert_eq!(rendered(&uncapped, &saved).pages.len(), 1);

    let (capped, saved) = paging_limit_report(9000, 4, 2, 0, 2);
    let doc = rendered(&capped, &saved);
    let headers: Vec<usize> = doc
        .pages
        .iter()
        .map(|p| page_text_tops(p, "GH").len())
        .collect();
    assert_eq!(headers, vec![2, 2]);
}

#[test]
fn a_group_carried_over_a_page_break_counts_against_the_next_page_cap() {
    // The page fits 8 bands (2400 / 300). Group A (header + 4 rows) plus group B's header + 2 rows
    // fill page 1 on height, so B carries onto page 2 — where it occupies one of the two group slots,
    // leaving room for group C alone. Counting only the headers that *start* on a page would fit D
    // there as well.
    let (report, saved) = paging_limit_report(2400, 4, 4, 0, 2);
    let doc = rendered(&report, &saved);
    let headers: Vec<usize> = doc
        .pages
        .iter()
        .map(|p| page_text_tops(p, "GH").len())
        .collect();
    assert_eq!(headers, vec![2, 1, 1]);
    assert_eq!(
        page_text_tops(&doc.pages[1], "line").len(),
        2 + 4,
        "group B's remaining rows and the whole of group C"
    );
}
