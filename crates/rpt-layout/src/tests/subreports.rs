use super::*;

#[test]
fn subreport_renders_nested_content_into_its_box() {
    use rpt_model::{Subreport, SubreportObject};

    // Nested subreport: a report-header with a static label at left=100, top=50.
    let mut nested = Report::default();
    nested.report_definition.areas = vec![area(
        AreaSectionKind::ReportHeader,
        vec![section(
            AreaSectionKind::ReportHeader,
            "SubRH",
            400,
            vec![text_object("Lbl", "SUBTEXT", 50)],
        )],
    )];

    // Main report: a report-header holding a subreport object at box (left=1000, top=500).
    let mut sub_obj = ReportObject::default();
    sub_obj.name = "SubObj".into();
    sub_obj.bounds = Rect {
        left: Twips(1000),
        top: Twips(500),
        width: Twips(3000),
        height: Twips(2000),
    };
    let mut so = SubreportObject::default();
    so.subreport_name = "Sub".into();
    sub_obj.kind = ReportObjectKind::Subreport(so);

    let mut main = Report::default();
    main.report_definition.areas = vec![area(
        AreaSectionKind::ReportHeader,
        vec![section(
            AreaSectionKind::ReportHeader,
            "MainRH",
            3000,
            vec![sub_obj],
        )],
    )];
    let mut sr = Subreport::default();
    sr.name = "Sub".into();
    sr.report = Box::new(nested);
    main.subreports = vec![sr];

    let empty = SavedData::default();
    let ds = build_dataset(&SavedDataSource::new(&empty), &main.data_definition);
    let formulas = rpt_data::compile_formulas(&main.data_definition);
    let doc = layout(&main, &ds, &formulas);

    // The subreport's label renders, translated into the box: left = box.left(1000) + obj.left(100),
    // top = box.top(500) + obj.top(50).
    let hit = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .find_map(|op| match op {
            DrawOp::Text(t) if t.text == "SUBTEXT" => Some((t.bounds.left.0, t.bounds.top.0)),
            _ => None,
        });
    assert_eq!(
        hit,
        Some((1100, 550)),
        "subreport label placed into its box"
    );
}

#[test]
fn subreport_taller_than_its_box_grows_the_detail_band() {
    use rpt_model::{Subreport, SubreportObject};

    // A subreport whose content reaches ~1240 twips deep (a label at top=1000, height 240) but is
    // placed in a 240-twip box in a detail band. With inline growth the band grows to fit it, so the
    // deep label renders (not clipped) for every detail row and the next row is pushed below it.
    let mut nested = Report::default();
    nested.report_definition.areas = vec![area(
        AreaSectionKind::ReportHeader,
        vec![section(
            AreaSectionKind::ReportHeader,
            "SubRH",
            1400,
            vec![text_object("Deep", "DEEP", 1000)],
        )],
    )];

    let mut sub_obj = ReportObject::default();
    sub_obj.name = "SubObj".into();
    sub_obj.bounds = Rect {
        left: Twips(0),
        top: Twips(0),
        width: Twips(3000),
        height: Twips(240),
    };
    let mut so = SubreportObject::default();
    so.subreport_name = "Sub".into();
    sub_obj.kind = ReportObjectKind::Subreport(so);

    let mut main = Report::default();
    main.print_options.content_width = Twips(12240);
    main.print_options.content_height = Twips(15840);
    // Detail band the height of the box (240): the subreport, not the design height, drives growth.
    main.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(
            AreaSectionKind::Detail,
            "Details",
            240,
            vec![sub_obj],
        )],
    )];
    let mut sr = Subreport::default();
    sr.name = "Sub".into();
    sr.report = Box::new(nested);
    main.subreports = vec![sr];

    // Two detail rows -> two subreport instances.
    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["1"], &["2"]]);
    let ds = build_dataset(&SavedDataSource::new(&saved), &main.data_definition);
    let formulas = rpt_data::compile_formulas(&main.data_definition);
    let doc = layout(&main, &ds, &formulas);

    // Every DEEP label renders (would be clipped at box height 240 without growth), and the second
    // row's label sits a full grown-band height (~1240) below the first — the band grew to fit.
    let mut tops: Vec<i32> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) if t.text == "DEEP" => Some(t.bounds.top.0),
            _ => None,
        })
        .collect();
    tops.sort_unstable();
    assert_eq!(
        tops.len(),
        2,
        "both rows' subreport content rendered (unclipped)"
    );
    assert_eq!(
        tops[0], 1000,
        "row 1 subreport label at box-top + label-top"
    );
    assert_eq!(
        tops[1] - tops[0],
        1240,
        "row 2 pushed below row 1's grown band (band grew from 240 to 1240)"
    );
}

#[test]
fn report_header_subreport_taller_than_box_grows_the_header_band() {
    use rpt_model::{Subreport, SubreportObject};

    // A report-header subreport whose content reaches ~1240 twips deep (label at top=1000) but is
    // placed in a 240-twip box, itself well inside one page. The header band must grow to the
    // subreport's content height (rendering the deep label unclipped) and the page header below must
    // sit past the grown height — not at the box bottom (240), which clipping would have produced.
    let mut nested = Report::default();
    nested.report_definition.areas = vec![area(
        AreaSectionKind::ReportHeader,
        vec![section(
            AreaSectionKind::ReportHeader,
            "SubRH",
            1400,
            vec![text_object("Deep", "DEEP", 1000)],
        )],
    )];

    let mut sub_obj = ReportObject::default();
    sub_obj.name = "SubObj".into();
    sub_obj.bounds = Rect {
        left: Twips(0),
        top: Twips(0),
        width: Twips(3000),
        height: Twips(240),
    };
    let mut so = SubreportObject::default();
    so.subreport_name = "Sub".into();
    sub_obj.kind = ReportObjectKind::Subreport(so);

    let mut main = Report::default();
    main.print_options.content_width = Twips(12240);
    main.print_options.content_height = Twips(15840);
    main.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "MainRH",
                240,
                vec![sub_obj],
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
    ];
    let mut sr = Subreport::default();
    sr.name = "Sub".into();
    sr.report = Box::new(nested);
    main.subreports = vec![sr];

    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["1"]]);
    let ds = build_dataset(&SavedDataSource::new(&saved), &main.data_definition);
    let formulas = rpt_data::compile_formulas(&main.data_definition);
    let doc = layout(&main, &ds, &formulas);

    let top_of = |text: &str| -> Option<i32> {
        doc.pages
            .iter()
            .flat_map(|p| &p.ops)
            .find_map(|op| match op {
                DrawOp::Text(t) if t.text == text => Some(t.bounds.top.0),
                _ => None,
            })
    };
    // The deep label renders (clipping to the 240 box would have dropped it entirely).
    assert_eq!(
        top_of("DEEP"),
        Some(1000),
        "report-header subreport content must render unclipped"
    );
    // The page header sits below the grown header, not just past the 240-twip box.
    let head_y = top_of("COLHEAD").expect("page header rendered");
    assert!(
        head_y >= 1240,
        "page header ({head_y}) must sit below the grown report header, not the clipped box (240)"
    );
}

#[test]
fn tall_subreport_splits_across_parent_pages_at_row_boundaries() {
    // A 20-row × 300-twip subreport (6000 twips) placed in a report with a 3000-twip body must flow
    // across several parent pages, splitting between whole rows — every row rendered exactly once.
    let sr = subreport_rows("Sub", 20, 300);
    let main = main_with_subreport(sr, 3000, 300);
    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["1"]]);
    let ds = build_dataset(&SavedDataSource::new(&saved), &main.data_definition);
    let formulas = rpt_data::compile_formulas(&main.data_definition);
    let doc = layout(&main, &ds, &formulas);

    assert!(
        doc.pages.len() > 1,
        "a subreport taller than the body must span multiple pages, got {}",
        doc.pages.len()
    );
    // Every row's text lands on exactly one page (no loss, no duplication from the split).
    let mut seen = std::collections::BTreeSet::new();
    for i in 0..20 {
        let want = format!("R{i}");
        let count = doc
            .pages
            .iter()
            .flat_map(|p| &p.ops)
            .filter(|op| matches!(op, DrawOp::Text(t) if t.text == want))
            .count();
        assert_eq!(
            count, 1,
            "row {want} should render exactly once, got {count}"
        );
        seen.insert(want);
    }
    assert_eq!(seen.len(), 20, "all 20 rows rendered");
    // The split actually crossed a page: the first and last row are on different pages.
    let page_of = |txt: &str| {
        doc.pages.iter().position(|p| {
            p.ops
                .iter()
                .any(|op| matches!(op, DrawOp::Text(t) if t.text == txt))
        })
    };
    assert!(
        page_of("R0") < page_of("R19"),
        "later rows flow onto later pages"
    );
}

#[test]
fn tall_subreport_in_report_header_flows_across_pages() {
    // A tall subreport in the main report's REPORT HEADER (not a detail band) must flow across
    // continuation pages the same way a detail-band subreport does — every row rendered exactly once,
    // spanning multiple pages, above the main detail rows that follow.
    let sr = subreport_rows("Sub", 20, 300);
    let mut sub_obj = ReportObject::default();
    sub_obj.name = "SubObj".into();
    sub_obj.bounds = Rect {
        left: Twips(0),
        top: Twips(0),
        width: Twips(6000),
        height: Twips(300),
    };
    let mut so = rpt_model::SubreportObject::default();
    so.subreport_name = "Sub".into();
    sub_obj.kind = ReportObjectKind::Subreport(so);

    let mut main = Report::default();
    main.print_options.content_width = Twips(12240);
    main.print_options.content_height = Twips(3000); // 3000-twip body; subreport is 6000 twips
    main.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "RH",
                300,
                vec![sub_obj],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "MainDetail",
                200,
                vec![db_field_object("Main", "t.x", 0)],
            )],
        ),
    ];
    main.subreports = vec![sr];

    let saved = saved_data(&[("t.x", FieldValueType::String)], &[&["M1"], &["M2"]]);
    let ds = build_dataset(&SavedDataSource::new(&saved), &main.data_definition);
    let formulas = rpt_data::compile_formulas(&main.data_definition);
    let doc = layout(&main, &ds, &formulas);

    assert!(
        doc.pages.len() > 1,
        "a report-header subreport taller than the body must span multiple pages, got {}",
        doc.pages.len()
    );
    // Every subreport row lands on exactly one page (no loss, no duplication).
    for i in 0..20 {
        let want = format!("R{i}");
        let count = doc
            .pages
            .iter()
            .flat_map(|p| &p.ops)
            .filter(|op| matches!(op, DrawOp::Text(t) if t.text == want))
            .count();
        assert_eq!(
            count, 1,
            "row {want} should render exactly once, got {count}"
        );
    }
    // The main detail rows still render, below the flowed header.
    for want in ["M1", "M2"] {
        let present = doc
            .pages
            .iter()
            .flat_map(|p| &p.ops)
            .any(|op| matches!(op, DrawOp::Text(t) if t.text == want));
        assert!(present, "main detail row {want} must still render");
    }
}

#[test]
fn page_header_waits_out_a_flowing_report_header() {
    // The page header sits BELOW the report header, so it prints on none of the pages a report
    // header occupies — including the continuation pages a tall subreport inside it flows onto. It
    // first appears on the page the body reaches, and repeats from there (native behavior). The
    // subreport is deliberately several pages tall, so pages exist that carry header content alone.
    let sr = subreport_rows("Sub", 60, 300);
    let mut sub_obj = ReportObject::default();
    sub_obj.name = "SubObj".into();
    sub_obj.bounds = Rect {
        left: Twips(0),
        top: Twips(0),
        width: Twips(6000),
        height: Twips(300),
    };
    let mut so = rpt_model::SubreportObject::default();
    so.subreport_name = "Sub".into();
    sub_obj.kind = ReportObjectKind::Subreport(so);

    let mut main = Report::default();
    main.print_options.content_width = Twips(12240);
    main.print_options.content_height = Twips(3000); // 3000-twip body; subreport is 6000 twips
    main.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "RH",
                300,
                vec![sub_obj],
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
                "MainDetail",
                200,
                vec![db_field_object("Main", "t.x", 0)],
            )],
        ),
    ];
    main.subreports = vec![sr];

    let saved = saved_data(&[("t.x", FieldValueType::String)], &[&["M1"], &["M2"]]);
    let ds = build_dataset(&SavedDataSource::new(&saved), &main.data_definition);
    let formulas = rpt_data::compile_formulas(&main.data_definition);
    let doc = layout(&main, &ds, &formulas);

    let pages_with = |text: &str| -> Vec<usize> {
        doc.pages
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.ops
                    .iter()
                    .any(|op| matches!(op, DrawOp::Text(t) if t.text == text))
            })
            .map(|(i, _)| i + 1)
            .collect()
    };
    let header_pages = pages_with("COLHEAD");
    assert!(
        doc.pages.len() > 1 && !pages_with("M1").is_empty(),
        "the subreport must flow and the body must still render, got {} page(s)",
        doc.pages.len()
    );
    // The report header ends on the page carrying the subreport's last row; the page header prints
    // below it there, and repeats to the end of the report.
    let rh_end = *pages_with("R59").last().expect("last subreport row placed");
    assert!(
        rh_end > 2,
        "this geometry must leave pages carrying header content alone"
    );
    assert_eq!(
        header_pages,
        (rh_end..=doc.pages.len()).collect::<Vec<_>>(),
        "page header must first print where the flowing report header ends (page {rh_end}) and \
         repeat from there, never on the pages the header alone occupies"
    );
}

#[test]
fn oversized_single_row_force_advances_without_spinning() {
    // A subreport whose single object is taller than a whole page must still terminate: the flow
    // splitter force-advances past a row taller than the available height rather than looping forever.
    // A single object 5000 twips tall — larger than the 3000-twip body, so no page can hold it whole.
    let mut tall = text_object("TALL", "TALL", 0);
    tall.bounds.height = Twips(5000);
    let mut sub = Report::default();
    sub.report_definition.areas = vec![area(
        AreaSectionKind::ReportHeader,
        vec![section(
            AreaSectionKind::ReportHeader,
            "SubRH",
            5000,
            vec![tall],
        )],
    )];
    let mut sr = rpt_model::Subreport::default();
    sr.name = "Sub".into();
    sr.report = Box::new(sub);
    let main = main_with_subreport(sr, 3000, 300);
    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["1"]]);
    let ds = build_dataset(&SavedDataSource::new(&saved), &main.data_definition);
    let formulas = rpt_data::compile_formulas(&main.data_definition);

    // Reaching this assertion at all proves the splitter did not spin (the test would hang otherwise).
    let doc = layout(&main, &ds, &formulas);
    assert!(
        doc.pages.len() >= 2,
        "an oversized row still forces a break"
    );
    let tall = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter(|op| matches!(op, DrawOp::Text(t) if t.text == "TALL"))
        .count();
    assert_eq!(tall, 1, "the oversized object renders once");
}

#[test]
fn flowed_subreport_shared_total_runs_once() {
    // A subreport that accumulates a Shared variable across its rows, tall enough to flow across
    // parent pages, must still run exactly once: the main report reads back the single-pass sum, not
    // a doubled value (a second format at emit would re-fire the accumulation).
    use rpt_model::{FieldDef, FieldKindData, Formula, FormulaField};

    // Subreport detail field bound to {@Acc}: Shared NumberVar acc; acc := acc + {s.v}; acc.
    let mut acc_field = FieldObject::default();
    acc_field.data_source = "@Acc".into();
    acc_field.ref_kind = FieldRefKind::Formula;
    acc_field.value_type = FieldValueType::Number;
    let mut acc_obj = ReportObject::default();
    acc_obj.name = "Acc".into();
    acc_obj.bounds = Rect {
        left: Twips(0),
        top: Twips(0),
        width: Twips(3000),
        height: Twips(240),
    };
    acc_obj.kind = ReportObjectKind::Field(Box::new(acc_field));

    let mut sub = Report::default();
    sub.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(
            AreaSectionKind::Detail,
            "SubDetail",
            300,
            vec![acc_obj],
        )],
    )];
    let mut acc_def = FieldDef::default();
    acc_def.name = "Acc".into();
    acc_def.kind = FieldKindData::Formula(FormulaField {
        text: Formula("Shared NumberVar acc; acc := acc + {s.v}; acc".into()),
        ..FormulaField::default()
    });
    sub.data_definition.field_definitions = vec![acc_def];
    // 20 rows of 5 → single-pass Shared total is 100.
    sub.saved_data = Some(SavedData {
        record_count: 20,
        columns: vec![SavedColumn {
            name: "s.v".into(),
            value_type: FieldValueType::Number,
        }],
        rows: (0..20).map(|_| vec![Some("5".into())]).collect(),
    });
    let mut sr = rpt_model::Subreport::default();
    sr.name = "Sub".into();
    sr.report = Box::new(sub);

    // Main: a 3000-twip body (the 20×300 subreport flows across pages) plus a report footer reading
    // back the Shared total.
    let mut main = main_with_subreport(sr, 3000, 300);
    let mut read_field = FieldObject::default();
    read_field.data_source = "@Read".into();
    read_field.ref_kind = FieldRefKind::Formula;
    read_field.value_type = FieldValueType::Number;
    let mut read_obj = ReportObject::default();
    read_obj.name = "Read".into();
    read_obj.bounds = Rect {
        left: Twips(0),
        top: Twips(0),
        width: Twips(3000),
        height: Twips(240),
    };
    read_obj.kind = ReportObjectKind::Field(Box::new(read_field));
    main.report_definition.areas.push(area(
        AreaSectionKind::ReportFooter,
        vec![section(
            AreaSectionKind::ReportFooter,
            "MainRF",
            300,
            vec![read_obj],
        )],
    ));
    let mut read_def = FieldDef::default();
    read_def.name = "Read".into();
    read_def.kind = FieldKindData::Formula(FormulaField {
        text: Formula("Shared NumberVar acc; acc".into()),
        ..FormulaField::default()
    });
    main.data_definition.field_definitions = vec![read_def];

    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["1"]]);
    let ds = build_dataset(&SavedDataSource::new(&saved), &main.data_definition);
    let formulas = rpt_data::compile_formulas(&main.data_definition);
    let doc = layout(&main, &ds, &formulas);

    // The subreport flowed (its content exceeds the body).
    assert!(
        doc.pages.len() > 1,
        "the subreport should flow across pages"
    );
    // The main report's read-back equals the single-pass total (100.00), proving one run.
    let texts: Vec<String> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    assert!(
        texts.iter().any(|t| t == " 100.00"),
        "main reads the single-pass Shared total (100.00), got {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t == "200.00"),
        "the subreport must not run twice (would double the Shared total): {texts:?}"
    );
}
