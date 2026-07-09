//! Layout unit tests over a hand-built minimal report + dataset.
//!
//! Several `rpt` model structs (`ChartObject`, `CrossTabObject`, …) are cross-crate
//! `#[non_exhaustive]`, so the builders below construct them via `Default` + field assignment
//! (struct literals are disallowed cross-crate) — which `clippy::field_reassign_with_default` flags
//! but cannot offer a valid rewrite for, so the lint is allowed for this test module.
#![allow(clippy::field_reassign_with_default)]

use crate::layout;
use rpt_data::{build_dataset, SavedDataSource};
use rpt_model::{
    Area, AreaSectionKind, FieldValueType, Rect, Report, ReportObject, ReportObjectKind,
    SavedColumn, SavedData, Section, TextObject, Twips,
};
use rpt_pages::DrawOp;
use rpt_test_support::saved_data;

fn text_object(name: &str, text: &str, top: i32) -> ReportObject {
    let mut o = ReportObject::default();
    o.name = name.to_string();
    o.bounds = Rect {
        left: Twips(100),
        top: Twips(top),
        width: Twips(3000),
        height: Twips(240),
    };
    let mut t = TextObject::default();
    t.text = text.to_string();
    o.kind = ReportObjectKind::Text(t);
    o
}

fn section(kind: AreaSectionKind, name: &str, height: i32, objects: Vec<ReportObject>) -> Section {
    let mut s = Section::default();
    s.kind = kind;
    s.name = name.to_string();
    s.height = Twips(height);
    s.objects = objects;
    s
}

fn area(kind: AreaSectionKind, sections: Vec<Section>) -> Area {
    let mut a = Area::default();
    a.kind = kind;
    a.sections = sections;
    a
}

/// A report with a page header (one text) and a detail band (one text) and small page geometry so
/// pagination triggers.
fn tiny_report(page_height: i32) -> Report {
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(page_height);
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
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                300,
                vec![text_object("Row", "line", 0)],
            )],
        ),
    ];
    report
}

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
fn can_grow_text_wraps_into_multiple_lines_and_grows_band() {
    // A detail band with a narrow can-grow text object holding long text, followed by a second
    // detail row — the wrapped text must produce multiple runs and push the next row down.
    let mut wide = text_object(
        "Memo",
        "the quick brown fox jumps over the lazy dog again",
        0,
    );
    wide.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(1500),
        height: Twips(240),
    };
    if let ReportObjectKind::Text(t) = &mut wide.kind {
        t.text = "the quick brown fox jumps over the lazy dog again".into();
    }
    wide.format.can_grow = true;

    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(AreaSectionKind::Detail, "Details", 240, vec![wide])],
    )];
    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["1"], &["2"]]);
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    let runs: Vec<&rpt_pages::TextRun> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t),
            _ => None,
        })
        .collect();
    // Two rows, each wrapping to >1 line → more than 2 runs total.
    assert!(
        runs.len() > 2,
        "expected wrapped multi-line runs, got {}",
        runs.len()
    );
    // Row 1's runs stack vertically (increasing tops).
    let tops: Vec<i32> = runs.iter().map(|r| r.bounds.top.0).collect();
    assert!(tops.windows(2).any(|w| w[1] > w[0]), "lines should stack");
    // The band grew: the two rows are separated by more than the 240-twip design height.
    let distinct_tops: std::collections::BTreeSet<i32> = tops.iter().copied().collect();
    assert!(
        *distinct_tops.iter().last().unwrap() > 240,
        "band grew past design height"
    );
}

/// A narrow can-grow text object with text long enough to wrap in a 1500-twip box.
fn wrapping_can_grow(name: &str) -> ReportObject {
    let mut o = text_object(name, "the quick brown fox jumps over the lazy dog again", 0);
    o.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(1500),
        height: Twips(240),
    };
    o.format.can_grow = true;
    o
}

fn empty_saved() -> SavedData {
    saved_data(&[("t.x", FieldValueType::Number)], &[&["1"]])
}

fn text_runs(doc: &rpt_pages::PagedDocument) -> Vec<&rpt_pages::TextRun> {
    doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t),
            _ => None,
        })
        .collect()
}

#[test]
fn report_header_grows_with_can_grow_content() {
    // A Report Header is a flow section: a can-grow object wraps and the band grows, pushing the
    // detail below the header's designed height.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "RH",
                240,
                vec![wrapping_can_grow("Memo")],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                240,
                vec![text_object("MARK", "MARK", 0)],
            )],
        ),
    ];
    let saved = empty_saved();
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    let runs = text_runs(&doc);
    // The header's can-grow text wrapped to more than one line.
    let memo_runs = runs.iter().filter(|r| r.text != "MARK").count();
    assert!(
        memo_runs > 1,
        "report header can-grow should wrap, got {memo_runs} run(s)"
    );
    // The detail marker sits below the grown header (past its 240-twip design height).
    let mark_top = runs
        .iter()
        .find(|r| r.text == "MARK")
        .expect("MARK run")
        .bounds
        .top
        .0;
    assert!(
        mark_top > 240,
        "detail should follow the grown header, MARK at {mark_top}"
    );
}

#[test]
fn page_header_does_not_grow_can_grow_is_inert() {
    // A Page Header is a fixed repeating band: can-grow is inert (native behavior), so its long
    // text stays a single (clipped) run and the detail is not pushed down.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::PageHeader,
            vec![section(
                AreaSectionKind::PageHeader,
                "PH",
                240,
                vec![wrapping_can_grow("Memo")],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                240,
                vec![text_object("MARK", "MARK", 0)],
            )],
        ),
    ];
    let saved = empty_saved();
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    let runs = text_runs(&doc);
    // Can-grow is inert in a page header: the long text is a single (clipped) run, not wrapped.
    let memo_runs = runs.iter().filter(|r| r.text != "MARK").count();
    assert_eq!(
        memo_runs, 1,
        "page header can-grow must be inert (no wrapping)"
    );
    // The detail is not pushed past the fixed 240-twip header height.
    let mark_top = runs
        .iter()
        .find(|r| r.text == "MARK")
        .expect("MARK run")
        .bounds
        .top
        .0;
    assert!(
        mark_top <= 240,
        "fixed page header must not push the detail down, MARK at {mark_top}"
    );
}

#[test]
fn multi_column_flows_records_across_columns() {
    use rpt_model::MultiColumn;
    // A detail band with one object at left=100, laid out in 3 columns of pitch 3000.
    let obj = text_object("Cell", "X", 0);
    let mut report = Report::default();
    report.print_options.content_width = Twips(12000);
    report.print_options.content_height = Twips(20000);
    report.print_options.multi_column = Some(MultiColumn {
        columns: 3,
        column_width: Twips(3000),
        gap_h: Twips(0),
        gap_v: Twips(0),
        across_then_down: true,
    });
    report.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(AreaSectionKind::Detail, "Details", 300, vec![obj])],
    )];
    let saved = SavedData {
        record_count: 6,
        columns: vec![SavedColumn {
            name: "t.x".into(),
            value_type: FieldValueType::Number,
        }],
        rows: (0..6).map(|_| vec![Some("1".into())]).collect(),
    };
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    // Collect the 6 detail cells' (left, top).
    let cells: Vec<(i32, i32)> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) if t.text == "X" => Some((t.bounds.left.0, t.bounds.top.0)),
            _ => None,
        })
        .collect();
    assert_eq!(cells.len(), 6, "6 records rendered");
    // content_left = 0 (zero margins); object left = 100; pitch = 3000 → columns at 100/3100/6100.
    let lefts: Vec<i32> = cells.iter().map(|c| c.0).collect();
    assert_eq!(
        lefts,
        vec![100, 3100, 6100, 100, 3100, 6100],
        "3 columns, wrap after 3"
    );
    // Rows 0-2 share a top; rows 3-5 share a lower top.
    assert_eq!(cells[0].1, cells[1].1, "first row of columns aligned");
    assert!(cells[3].1 > cells[0].1, "second column-row is lower");
}

#[test]
fn multi_column_down_then_across_fills_columns_vertically() {
    use rpt_model::MultiColumn;
    // 2 columns, pitch 3000, one 300-twip object; a short body so only 2 records fit per column.
    // Down-then-across fills column 0 top-to-bottom, then column 1.
    let obj = text_object("Cell", "X", 0);
    let mut report = Report::default();
    report.print_options.content_width = Twips(12000);
    report.print_options.content_height = Twips(700);
    report.print_options.multi_column = Some(MultiColumn {
        columns: 2,
        column_width: Twips(3000),
        gap_h: Twips(0),
        gap_v: Twips(0),
        across_then_down: false,
    });
    report.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(AreaSectionKind::Detail, "Details", 300, vec![obj])],
    )];
    let saved = SavedData {
        record_count: 4,
        columns: vec![SavedColumn {
            name: "t.x".into(),
            value_type: FieldValueType::Number,
        }],
        rows: (0..4).map(|_| vec![Some("1".into())]).collect(),
    };
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    // (left, top) of every cell, in record (print) order — all should land on page 1.
    let cells: Vec<(i32, i32)> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) if t.text == "X" => Some((t.bounds.left.0, t.bounds.top.0)),
            _ => None,
        })
        .collect();
    assert_eq!(cells.len(), 4, "4 records on one page");
    // Records 0,1 stack down column 0 (same left); 2,3 stack down column 1 (left + pitch 3000).
    assert_eq!(cells[0].0, cells[1].0, "records 0,1 in the same column");
    assert_eq!(cells[2].0, cells[3].0, "records 2,3 in the same column");
    assert_eq!(
        cells[2].0 - cells[0].0,
        3000,
        "second column is one pitch to the right"
    );
    // Vertical stacking within a column, restarting at the top of the next column.
    assert!(cells[1].1 > cells[0].1, "record 1 sits below record 0");
    assert_eq!(cells[1].1 - cells[0].1, 300, "stacked by the detail height");
    assert_eq!(cells[2].1, cells[0].1, "column 1 restarts at the top");
    assert_eq!(
        cells[3].1, cells[1].1,
        "column 1 second record aligns with column 0's"
    );
}

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
fn running_total_global_accumulates_across_printed_rows() {
    use rpt_model::{FieldDef, FieldKindData, FieldObject, FieldRefKind, Formula, FormulaField};

    // A detail band with a field bound to `{@RunTotal}`, where the formula is a Global running sum:
    //   Global NumberVar t; t := t + {t.amt}; t
    // Across three printed rows (amounts 10, 20, 5) the field must show 10, 30, 35 — proving the
    // Global variable persists across the print pass.
    let mut field = FieldObject::default();
    field.data_source = "@RunTotal".into();
    field.ref_kind = FieldRefKind::Formula;
    field.value_type = FieldValueType::Number;
    let mut obj = ReportObject::default();
    obj.name = "Total".into();
    obj.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(3000),
        height: Twips(240),
    };
    obj.kind = ReportObjectKind::Field(field);

    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(AreaSectionKind::Detail, "Details", 300, vec![obj])],
    )];
    let mut run_total = FieldDef::default();
    run_total.name = "RunTotal".into();
    run_total.kind = FieldKindData::Formula(FormulaField {
        text: Formula("Global NumberVar t; t := t + {t.amt}; t".into()),
        ..FormulaField::default()
    });
    report.data_definition.field_definitions = vec![run_total];

    let saved = saved_data(
        &[("t.amt", FieldValueType::Number)],
        &[&["10"], &["20"], &["5"]],
    );
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    let totals: Vec<&str> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        totals,
        vec!["10.00", "30.00", "35.00"],
        "Global running total accumulates in print order"
    );
}

/// A `{#name}` field in a detail band shows the running total accumulated up to and including the
/// current record, resetting on each group change. Grouped by region, a Sum running
/// total that resets on group change shows each group's partial sums in print order.
#[test]
fn per_record_running_total_accumulates_and_resets_on_group() {
    use rpt_model::{
        FieldDef, FieldKindData, FieldObject, FieldRefKind, Group, ResetConditionType,
        RunningTotalField, SortDirection, SummaryOperation,
    };

    // Detail band with a field bound to the running total {#RT}.
    let mut field = FieldObject::default();
    field.data_source = "#RT".into();
    field.ref_kind = FieldRefKind::RunningTotal;
    field.value_type = FieldValueType::Number;
    let mut obj = ReportObject::default();
    obj.name = "RT".into();
    obj.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(3000),
        height: Twips(240),
    };
    obj.kind = ReportObjectKind::Field(field);

    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(AreaSectionKind::Detail, "Details", 300, vec![obj])],
    )];
    // Group by region; running total RT = Sum(amt), reset on each group change.
    let mut g = Group::default();
    g.condition_field = "t.region".into();
    g.sort.direction = SortDirection::AscendingOrder;
    report.data_definition.groups = vec![g];
    let rt = RunningTotalField {
        operation: SummaryOperation::Sum,
        summarized_field: "t.amt".into(),
        reset: ResetConditionType::OnChangeOfGroup,
        ..Default::default()
    };
    let mut rt_def = FieldDef::default();
    rt_def.name = "RT".into();
    rt_def.kind = FieldKindData::RunningTotal(rt);
    report.data_definition.field_definitions = vec![rt_def];

    let saved = saved_data(
        &[
            ("t.region", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[
            &["West", "10"],
            &["East", "5"],
            &["West", "20"],
            &["East", "15"],
        ],
    );
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    let totals: Vec<&str> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    // Groups ascending: East [5,15] then West [10,20]. The running sum resets on each group change:
    // East → 5, 20; West → 10, 30.
    assert_eq!(totals, vec!["5.00", "20.00", "10.00", "30.00"]);
}

/// A `WhileReadingRecords` Global accumulator fires in **read order**, not print order:
/// with a record sort that reorders the rows, the value each printed record shows is the one it got
/// when read, so the printed sequence follows the source order, not the sorted order.
#[test]
fn while_reading_formula_accumulates_in_read_order_not_print_order() {
    use rpt_model::{
        FieldDef, FieldKindData, FieldObject, FieldRefKind, Formula, FormulaField, Sort,
        SortDirection,
    };

    // Detail field bound to {@Accum}, a Global running sum over {t.n} (references a field only →
    // classified WhileReadingRecords, so it is pre-evaluated in read order).
    let mut field = FieldObject::default();
    field.data_source = "@Accum".into();
    field.ref_kind = FieldRefKind::Formula;
    field.value_type = FieldValueType::Number;
    let mut obj = ReportObject::default();
    obj.name = "A".into();
    obj.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(3000),
        height: Twips(240),
    };
    obj.kind = ReportObjectKind::Field(field);

    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(AreaSectionKind::Detail, "Details", 300, vec![obj])],
    )];
    // Sort records ascending on t.n so read order (3,1,2) differs from print order (1,2,3).
    let mut s = Sort::default();
    s.field = "t.n".into();
    s.direction = SortDirection::AscendingOrder;
    report.data_definition.record_sorts = vec![s];
    let mut accum = FieldDef::default();
    accum.name = "Accum".into();
    accum.kind = FieldKindData::Formula(FormulaField {
        text: Formula("Global NumberVar n; n := n + {t.n}; n".into()),
        ..FormulaField::default()
    });
    report.data_definition.field_definitions = vec![accum];

    let saved = saved_data(
        &[("t.n", FieldValueType::Number)],
        &[&["3"], &["1"], &["2"]],
    );
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    let vals: Vec<&str> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    // Read order 3,1,2 → running sums 3,4,6 recorded per record. Printed in sorted order 1,2,3, each
    // record shows its read-order value: n=1→4, n=2→6, n=3→3.
    assert_eq!(vals, vec!["4.00", "6.00", "3.00"]);
}

#[test]
fn conditional_suppress_and_font_color() {
    use rpt_model::Color;

    // A field suppressed by a per-row formula, and one whose font color is set by a formula.
    let mut hide = text_object("Hide", "SECRET", 0);
    hide.format.condition_formulas = vec![("EnableSuppress".into(), "{t.x} > 5".into())];
    let mut red = text_object("Red", "SHOWN", 0);
    red.bounds.top = Twips(300);
    if let ReportObjectKind::Text(t) = &mut red.kind {
        t.font_color.condition_formulas = vec![("Color".into(), "Color(255, 0, 0)".into())];
    }

    let mut report = Report::default();
    report.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(
            AreaSectionKind::Detail,
            "Details",
            600,
            vec![hide, red],
        )],
    )];
    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["10"]]);
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    let texts: Vec<(String, Color)> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some((t.text.clone(), t.color)),
            _ => None,
        })
        .collect();
    // {t.x}=10 > 5 → the field is conditionally suppressed.
    assert!(
        !texts.iter().any(|(s, _)| s == "SECRET"),
        "conditionally suppressed object must be hidden: {texts:?}"
    );
    // The other field's color comes from Color(255,0,0) = red.
    let shown = texts
        .iter()
        .find(|(s, _)| s == "SHOWN")
        .expect("SHOWN present");
    assert_eq!(
        shown.1,
        Color {
            a: 255,
            r: 255,
            g: 0,
            b: 0
        },
        "conditional font Color applied"
    );
}

fn chart_object(name: &str, top: i32) -> ReportObject {
    let mut o = ReportObject::default();
    o.name = name.to_string();
    o.bounds = Rect {
        left: Twips(100),
        top: Twips(top),
        width: Twips(3000),
        height: Twips(2000),
    };
    o.kind = ReportObjectKind::Chart(Box::default());
    o
}

/// A chart object in the detail band over multiple rows produces exactly ONE unsupported-object
/// diagnostic (deduped across records), tagged with the object name.
#[test]
fn chart_emits_one_unsupported_diagnostic_deduped() {
    use crate::layout;
    use rpt_pages::DiagnosticKind;

    let saved = saved_data(
        &[("t.x", FieldValueType::Number)],
        &[&["1"], &["2"], &["3"]],
    );
    let mut report = tiny_report(15840);
    // Add a chart to the detail section (over 3 rows → 3 placements, one diagnostic after dedup).
    report.report_definition.areas[1].sections[0]
        .objects
        .push(chart_object("Graph1", 0));
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);

    let doc = layout(&report, &ds, &formulas);
    let charts: Vec<_> = doc
        .diagnostics
        .iter()
        .filter(|d| d.kind == DiagnosticKind::UnsupportedObject)
        .collect();
    assert_eq!(
        charts.len(),
        1,
        "one deduped diagnostic: {:?}",
        doc.diagnostics
    );
    assert_eq!(charts[0].source.as_deref(), Some("Graph1"));
}

#[test]
fn push_diag_dedups_identical() {
    use crate::{push_diag, DiagSink};
    use rpt_pages::{Diagnostic, DiagnosticKind};
    let sink: DiagSink = std::cell::RefCell::new(Vec::new());
    let d = || Diagnostic::warn(DiagnosticKind::FormulaError, "boom").with_source("f1");
    push_diag(&sink, d());
    push_diag(&sink, d()); // identical → dropped
    push_diag(
        &sink,
        Diagnostic::warn(DiagnosticKind::FormulaError, "boom").with_source("f2"),
    ); // different source → kept
    assert_eq!(sink.into_inner().len(), 2);
}

/// A raw SQL Command table emits a "bound to <DB>" diagnostic naming the driver.
#[test]
fn command_table_emits_bound_diagnostic() {
    use rpt_model::Table;
    use rpt_pages::DiagnosticKind;

    let mut report = tiny_report(15840);
    let mut t = Table::default();
    t.alias = "Cmd".into();
    t.command_text = Some("SELECT 1 FROM dual".into());
    t.connection
        .attributes
        .push(("Database_DLL".into(), "crdb_odbc.dll".into()));
    report.database.tables.push(t);

    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[]);
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    let d = doc
        .diagnostics
        .iter()
        .find(|d| d.kind == DiagnosticKind::Other)
        .expect("command-bound diagnostic present");
    // One aggregated diagnostic (no per-table source): it names the table and the driver, and
    // makes clear the report renders with live data only against that specific database.
    assert!(d.message.contains("Cmd"), "names the table: {}", d.message);
    assert!(
        d.message.contains("crdb_odbc.dll"),
        "names driver: {}",
        d.message
    );
    assert!(
        d.message.contains("only against that database"),
        "{}",
        d.message
    );
}

fn crosstab_object(name: &str) -> ReportObject {
    use rpt_model::{CrossTabDimension, CrossTabMeasure, CrossTabObject, SummaryOperation};
    let mut o = ReportObject::default();
    o.name = name.to_string();
    o.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(8000),
        height: Twips(4000),
    };
    let dim = |f: &str| {
        let mut d = CrossTabDimension::default();
        d.field_ref = f.to_string();
        d
    };
    let mut ct = CrossTabObject::default();
    ct.rows = vec![dim("t.region")];
    ct.columns = vec![dim("t.quarter")];
    let mut m = CrossTabMeasure::default();
    m.operation = SummaryOperation::Sum;
    m.field = "t.amt".to_string();
    ct.measures = vec![m];
    o.kind = ReportObjectKind::CrossTab(ct);
    o
}

/// A cross-tab pivots the detail rows into a grid: region (rows) × quarter (cols), Sum(amt) cells,
/// rendered as native draw-ops.
#[test]
fn crosstab_renders_a_grid_from_data() {
    use rpt_pages::{DrawOp, ObjectKind};

    let saved = saved_data(
        &[
            ("t.region", FieldValueType::String),
            ("t.quarter", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[
            &["East", "Q1", "10"],
            &["West", "Q1", "20"],
            &["East", "Q2", "30"],
            &["West", "Q2", "40"],
        ],
    );
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "RH",
                4200,
                vec![crosstab_object("CT1")],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "D",
                200,
                vec![text_object("row", "x", 0)],
            )],
        ),
    ];
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    // Collect the cross-tab-sourced draw-ops and their text.
    let texts: Vec<String> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t)
                if t.source
                    .as_ref()
                    .is_some_and(|s| s.kind == ObjectKind::CrossTab) =>
            {
                Some(t.text.clone())
            }
            _ => None,
        })
        .collect();
    // Headers (East/West rows, Q1/Q2 cols) + the four cell sums (2-decimal formatted) must be present.
    for expected in [
        "East", "West", "Q1", "Q2", "10.00", "20.00", "30.00", "40.00",
    ] {
        assert!(
            texts.iter().any(|t| t == expected),
            "cross-tab grid missing {expected:?}: {texts:?}"
        );
    }
}

// --- Pagination edge cases: frozen Page-IR behavior for the riskiest branches, so the later
// Formatter decomposition is guarded. ---

/// Build `SavedData` of `n` single-column numeric rows keyed `t.x`.
fn numeric_rows(n: usize) -> SavedData {
    SavedData {
        record_count: n as u32,
        columns: vec![SavedColumn {
            name: "t.x".into(),
            value_type: FieldValueType::Number,
        }],
        rows: (0..n).map(|i| vec![Some(i.to_string())]).collect(),
    }
}

fn rendered(report: &Report, saved: &SavedData) -> rpt_pages::PagedDocument {
    let ds = build_dataset(&SavedDataSource::new(saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    layout(report, &ds, &formulas)
}

fn page_text_tops(page: &rpt_pages::Page, text: &str) -> Vec<i32> {
    page.ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) if t.text == text => Some(t.bounds.top.0),
            _ => None,
        })
        .collect()
}

#[test]
fn can_grow_band_taller_than_body_gets_one_page_each_never_stalls() {
    // A can-grow detail that wraps taller than the whole body must still emit (at content_top) and
    // then move the next record to a fresh page — the `cursor_y > content_top` guard is the only
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
fn chart_summary_op_parses_the_axis_title_operation() {
    use crate::aggregate::chart_summary_op;
    use rpt_model::SummaryOperation as Op;
    assert_eq!(chart_summary_op("Sum of id"), Some(Op::Sum));
    assert_eq!(
        chart_summary_op("Count of Command.some_field"),
        Some(Op::Count)
    );
    assert_eq!(
        chart_summary_op("Distinct Count of x"),
        Some(Op::DistinctCount)
    );
    assert_eq!(chart_summary_op("Average of amt"), Some(Op::Average));
    assert_eq!(chart_summary_op("Maximum of d"), Some(Op::Maximum));
    // No "<op> of …" prefix → no operation.
    assert_eq!(chart_summary_op("created_at"), None);
    assert_eq!(chart_summary_op(""), None);
}

#[test]
fn chart_computes_group_aggregation_from_axis_title() {
    use rpt_model::{ChartObject, Group};
    // A grouped report (by region) with a chart whose value binding is "Sum of amt". The chart's
    // aggregation is not a declared summary field, so the layout must compute it per group from the
    // axis title rather than render an empty placeholder.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    let mut g = Group::default();
    g.condition_field = "t.region".into();
    report.data_definition.groups = vec![g];

    let mut cdef = ChartObject::default();
    cdef.data_refs = vec!["t.amt".into()];
    cdef.category_refs = vec!["t.region".into()];
    cdef.definition.data_axis_title = "Sum of amt".into();
    let mut chart = ReportObject::default();
    chart.name = "Graph1".into();
    chart.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(6000),
        height: Twips(4000),
    };
    chart.kind = ReportObjectKind::Chart(Box::new(cdef));
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "RH",
                4000,
                vec![chart],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(AreaSectionKind::Detail, "D", 240, vec![])],
        ),
    ];
    let saved = saved_data(
        &[
            ("t.region", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[&["A", "10"], &["A", "20"], &["B", "100"]],
    );
    let doc = rendered(&report, &saved);

    // The series was computed (Sum of amt per region: A=30, B=100), so no empty-placeholder warning.
    assert!(
        !doc.diagnostics
            .iter()
            .any(|d| d.message.contains("no group series")),
        "chart should compute a series, diagnostics: {:?}",
        doc.diagnostics
    );
    // Both category bars are labelled on the page.
    let texts: Vec<&str> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        texts.contains(&"A") && texts.contains(&"B"),
        "category labels: {texts:?}"
    );
}

/// An ungrouped dataset from a flat saved-data rowset (no report grouping), for testing the chart's
/// own category grouping directly.
fn ungrouped_dataset(saved: &SavedData) -> rpt_data::Dataset {
    let report = Report::default();
    build_dataset(&SavedDataSource::new(saved), &report.data_definition)
}

#[test]
fn chart_series_ungrouped_buckets_string_category_and_sums() {
    use crate::aggregate::chart_series_ungrouped;
    use crate::Locale;
    use rpt_model::SummaryOperation;
    // The report has no grouping, so the chart builds its own category grouping from the detail rows:
    // Sum of amt per region — A=30, B=100 — in first-seen order.
    let saved = saved_data(
        &[
            ("t.region", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[&["A", "10"], &["A", "20"], &["B", "100"]],
    );
    let ds = ungrouped_dataset(&saved);
    let series = chart_series_ungrouped(
        &ds,
        &Locale::default(),
        "t.region",
        Some("t.amt"),
        Some(SummaryOperation::Sum),
        "monthly",
    );
    assert_eq!(
        series,
        vec![("A".to_string(), 30.0), ("B".to_string(), 100.0)]
    );
}

#[test]
fn chart_series_ungrouped_buckets_temporal_category_by_month_ascending() {
    use crate::aggregate::chart_series_ungrouped;
    use crate::Locale;
    use rpt_model::SummaryOperation;
    // A date category buckets by calendar month (rows fed out of order); the buckets come back
    // temporally ascending: Jan (5+15=20) then Feb (100).
    let saved = saved_data(
        &[
            ("t.d", FieldValueType::Date),
            ("t.amt", FieldValueType::Number),
        ],
        &[
            &["2024-02-05", "100"],
            &["2024-01-10", "5"],
            &["2024-01-20", "15"],
        ],
    );
    let ds = ungrouped_dataset(&saved);
    let series = chart_series_ungrouped(
        &ds,
        &Locale::default(),
        "t.d",
        Some("t.amt"),
        Some(SummaryOperation::Sum),
        "monthly",
    );
    let values: Vec<f64> = series.iter().map(|(_, v)| *v).collect();
    assert_eq!(values, vec![20.0, 100.0], "two monthly buckets, ascending");
    // Monthly buckets read as M/YYYY (no leading zero), matching the engine, not a full localized date.
    let labels: Vec<&str> = series.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels, vec!["1/2024", "2/2024"], "monthly bucket labels");
}

#[test]
fn chart_series_ungrouped_honours_weekly_period() {
    use crate::aggregate::chart_series_ungrouped;
    use crate::Locale;
    use rpt_model::SummaryOperation;
    // The same three January dates that collapse to one MONTHLY bucket fall into three distinct
    // WEEKLY buckets, keyed by the Sunday week-start and labelled M/d/yyyy (matching the engine's
    // weekly category axis). 2024-01-03 is a Wednesday (week of 2023-12-31), 2024-01-10 the next
    // week (2024-01-07), 2024-01-20 the week of 2024-01-14.
    let saved = saved_data(
        &[
            ("t.d", FieldValueType::Date),
            ("t.amt", FieldValueType::Number),
        ],
        &[
            &["2024-01-20", "15"],
            &["2024-01-03", "5"],
            &["2024-01-10", "10"],
        ],
    );
    let ds = ungrouped_dataset(&saved);
    let series = chart_series_ungrouped(
        &ds,
        &Locale::default(),
        "t.d",
        Some("t.amt"),
        Some(SummaryOperation::Sum),
        "weekly",
    );
    let labels: Vec<&str> = series.iter().map(|(l, _)| l.as_str()).collect();
    let values: Vec<f64> = series.iter().map(|(_, v)| *v).collect();
    assert_eq!(
        labels,
        vec!["12/31/2023", "1/7/2024", "1/14/2024"],
        "three weekly buckets, week-start labelled M/d/yyyy, ascending"
    );
    assert_eq!(values, vec![5.0, 10.0, 15.0], "one row per weekly bucket");
}

/// Render a legend-visible single-series chart of `graph_type` over three string categories and
/// return every text drawn. A per-category-coloured family (bar) repeats each category label in its
/// legend (axis + legend); a single-colour family (area/line) draws no legend, so each category
/// appears once (axis only).
#[cfg(test)]
fn chart_render_texts(graph_type: rpt_model::ChartGraphType) -> Vec<String> {
    use rpt_model::ReportObjectKind;
    let saved = saved_data(
        &[
            ("t.cat", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[&["Alpha", "10"], &["Beta", "20"], &["Gamma", "30"]],
    );
    let mut chart = chart_object("Graph1", 0);
    if let ReportObjectKind::Chart(def) = &mut chart.kind {
        def.definition.graph_type = graph_type;
        def.definition.legend_visible = true;
        def.data_refs = vec!["t.amt".into()];
        def.category_refs = vec!["t.cat".into()];
    }
    // A tall report-header section holding just the chart, so it renders once over the whole dataset.
    let mut report = tiny_report(15840);
    report.report_definition.areas[0].sections[0] =
        section(AreaSectionKind::PageHeader, "PageHeader", 6000, vec![chart]);
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);
    doc.pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|o| match o {
            DrawOp::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

/// A per-category-coloured bar chart draws a per-category legend (each category label appears twice:
/// once on the X axis, once in the legend); a single-colour area/line chart draws no legend, so each
/// category appears once.
#[test]
fn area_and_line_suppress_per_category_legend_bar_keeps_it() {
    use rpt_model::ChartGraphType as Gt;
    let count = |texts: &[String], want: &str| texts.iter().filter(|t| *t == want).count();

    let bar = chart_render_texts(Gt::Bar);
    assert_eq!(
        count(&bar, "Alpha"),
        2,
        "bar legend repeats the category: {bar:?}"
    );
    assert_eq!(
        count(&bar, "Gamma"),
        2,
        "bar legend repeats the category: {bar:?}"
    );

    let area = chart_render_texts(Gt::Area);
    assert_eq!(
        count(&area, "Alpha"),
        1,
        "area draws no per-category legend: {area:?}"
    );

    let line = chart_render_texts(Gt::Line);
    assert_eq!(
        count(&line, "Alpha"),
        1,
        "line draws no per-category legend: {line:?}"
    );
}

#[test]
fn chart_series_multi_binds_one_series_per_second_group_value() {
    use crate::aggregate::chart_series_multi;
    use crate::Locale;
    use rpt_model::ChartObject;
    // A chart bound to a SECOND category dimension (created_at × lot) with a single value field draws
    // one series per distinct secondary value — not one series per value field. Primary categories are
    // the monthly buckets of created_at; the two lots each become a series carrying Sum of amt.
    let saved = saved_data(
        &[
            ("t.d", FieldValueType::Date),
            ("t.lot", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[
            &["2024-01-05", "L1", "10"],
            &["2024-01-20", "L1", "5"],
            &["2024-02-10", "L1", "100"],
            &["2024-01-15", "L2", "7"],
            &["2024-02-01", "L2", "3"],
            &["2024-02-25", "L2", "40"],
        ],
    );
    let ds = ungrouped_dataset(&saved);
    let mut cdef = ChartObject::default();
    cdef.data_refs = vec!["Sum of t.amt".into()];
    cdef.category_refs = vec!["t.d".into(), "t.lot".into()];
    cdef.definition.data_axis_title = "Sum of amt".into();

    let (categories, series) = chart_series_multi(&ds, &Locale::default(), &cdef);
    // Two monthly primary categories, ascending.
    assert_eq!(categories, vec!["1/2024".to_string(), "2/2024".to_string()]);
    // One series per distinct lot (first-seen order), each a per-month Sum of amt.
    assert_eq!(series.len(), 2, "one series per second-group value");
    assert_eq!(series[0], ("L1".to_string(), vec![15.0, 100.0]));
    assert_eq!(series[1], ("L2".to_string(), vec![7.0, 43.0]));
}

#[test]
fn chart_series_multi_single_dimension_keeps_value_field_series() {
    use crate::aggregate::chart_series_multi;
    use crate::Locale;
    use rpt_model::ChartObject;
    // With a single category dimension, the multi-series path is unchanged: one series per value field
    // over the chart's own category buckets (here a single value field → a single series).
    let saved = saved_data(
        &[
            ("t.region", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[&["A", "10"], &["A", "20"], &["B", "100"]],
    );
    let ds = ungrouped_dataset(&saved);
    let mut cdef = ChartObject::default();
    cdef.data_refs = vec!["Sum of t.amt".into()];
    cdef.category_refs = vec!["t.region".into()];
    cdef.definition.data_axis_title = "Sum of amt".into();

    let (categories, series) = chart_series_multi(&ds, &Locale::default(), &cdef);
    assert_eq!(categories, vec!["A".to_string(), "B".to_string()]);
    assert_eq!(series.len(), 1, "single value field → single series");
    assert_eq!(series[0], ("t.amt".to_string(), vec![30.0, 100.0]));
}

#[test]
fn chart_ungrouped_report_renders_non_empty_series() {
    use rpt_model::{ChartGraphType, ChartObject};
    // Reproduces the funnel/radar case: the chart groups only inside itself, the report body is not
    // grouped. The chart must still plot a series (no empty-placeholder warning).
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    // Deliberately no report groups.

    let mut cdef = ChartObject::default();
    cdef.data_refs = vec!["t.amt".into()];
    cdef.category_refs = vec!["t.region".into()];
    cdef.definition.data_axis_title = "Sum of amt".into();
    cdef.definition.graph_type = ChartGraphType::Funnel;
    let mut chart = ReportObject::default();
    chart.name = "Graph1".into();
    chart.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(6000),
        height: Twips(4000),
    };
    chart.kind = ReportObjectKind::Chart(Box::new(cdef));
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "RH",
                4000,
                vec![chart],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(AreaSectionKind::Detail, "D", 240, vec![])],
        ),
    ];
    let saved = saved_data(
        &[
            ("t.region", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[&["A", "10"], &["A", "20"], &["B", "100"]],
    );
    let doc = rendered(&report, &saved);
    assert!(
        !doc.diagnostics
            .iter()
            .any(|d| d.message.contains("no group series")),
        "ungrouped chart should build its own series, diagnostics: {:?}",
        doc.diagnostics
    );
    // The funnel renderer draws its category labels from the chart's own grouping.
    let texts: Vec<&str> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        texts.contains(&"A") && texts.contains(&"B"),
        "category labels: {texts:?}"
    );
}

#[test]
fn chart_grouped_report_prefers_fast_path_over_own_grouping() {
    use rpt_model::{ChartObject, Group};
    // The report IS grouped (by region), but the chart's category binding names a different field.
    // The fast path (report groups) must win, so the series is the two region groups — not buckets
    // built from the category field. If the fallback ran on the (unresolvable) category field, the
    // series would be empty and a "no group series" warning would fire.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    let mut g = Group::default();
    g.condition_field = "t.region".into();
    report.data_definition.groups = vec![g];

    let mut cdef = ChartObject::default();
    cdef.data_refs = vec!["t.amt".into()];
    cdef.category_refs = vec!["t.absent".into()];
    cdef.definition.data_axis_title = "Sum of amt".into();
    let mut chart = ReportObject::default();
    chart.name = "Graph1".into();
    chart.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(6000),
        height: Twips(4000),
    };
    chart.kind = ReportObjectKind::Chart(Box::new(cdef));
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "RH",
                4000,
                vec![chart],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(AreaSectionKind::Detail, "D", 240, vec![])],
        ),
    ];
    let saved = saved_data(
        &[
            ("t.region", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[&["A", "10"], &["A", "20"], &["B", "100"]],
    );
    let doc = rendered(&report, &saved);
    assert!(
        !doc.diagnostics
            .iter()
            .any(|d| d.message.contains("no group series")),
        "grouped chart should use the fast path, diagnostics: {:?}",
        doc.diagnostics
    );
    let texts: Vec<&str> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    // The category labels are the report group keys (region), proving the fast path was taken.
    assert!(
        texts.contains(&"A") && texts.contains(&"B"),
        "fast-path category labels: {texts:?}"
    );
}

#[test]
fn chart_multi_series_ungrouped_report_builds_own_categories() {
    use rpt_model::{ChartGraphType, ChartObject};
    // A 2-D multi-series bar chart (two data bindings) over an ungrouped report: the multi-series path
    // must build the chart's own category buckets from the detail rows rather than the report groups.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);

    let mut cdef = ChartObject::default();
    cdef.data_refs = vec!["t.a".into(), "t.b".into()];
    cdef.category_refs = vec!["t.region".into()];
    cdef.definition.graph_type = ChartGraphType::Bar;
    cdef.definition.data_axis_title = "Sum of a".into();
    let mut chart = ReportObject::default();
    chart.name = "Graph1".into();
    chart.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(6000),
        height: Twips(4000),
    };
    chart.kind = ReportObjectKind::Chart(Box::new(cdef));
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "RH",
                4000,
                vec![chart],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(AreaSectionKind::Detail, "D", 240, vec![])],
        ),
    ];
    let saved = saved_data(
        &[
            ("t.region", FieldValueType::String),
            ("t.a", FieldValueType::Number),
            ("t.b", FieldValueType::Number),
        ],
        &[&["A", "10", "1"], &["B", "20", "2"]],
    );
    let doc = rendered(&report, &saved);
    assert!(
        !doc.diagnostics
            .iter()
            .any(|d| d.message.contains("no group series")),
        "ungrouped multi-series chart should build its own series, diagnostics: {:?}",
        doc.diagnostics
    );
    let texts: Vec<&str> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        texts.contains(&"A") && texts.contains(&"B"),
        "multi-series category labels: {texts:?}"
    );
}

#[test]
fn line_chart_draws_a_connecting_polyline() {
    use rpt_model::{ChartGraphType, ChartObject, Group};
    // Same grouped setup as the aggregation test, but a Line chart: it must draw a polyline (segments
    // beyond the two axes) and emit no "not yet supported" diagnostic.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    let mut g = Group::default();
    g.condition_field = "t.region".into();
    report.data_definition.groups = vec![g];

    let mut cdef = ChartObject::default();
    cdef.data_refs = vec!["t.amt".into()];
    cdef.category_refs = vec!["t.region".into()];
    cdef.definition.data_axis_title = "Sum of amt".into();
    cdef.definition.graph_type = ChartGraphType::Line;
    let mut chart = ReportObject::default();
    chart.name = "Graph1".into();
    chart.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(6000),
        height: Twips(4000),
    };
    chart.kind = ReportObjectKind::Chart(Box::new(cdef));
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "RH",
                4000,
                vec![chart],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(AreaSectionKind::Detail, "D", 240, vec![])],
        ),
    ];
    let saved = saved_data(
        &[
            ("t.region", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[&["A", "10"], &["A", "20"], &["B", "100"]],
    );
    let doc = rendered(&report, &saved);

    // A Line chart is supported → no fallback diagnostic.
    assert!(
        !doc.diagnostics
            .iter()
            .any(|d| d.message.contains("not yet supported")),
        "line chart should be supported: {:?}",
        doc.diagnostics
    );
    // Two axes + at least one polyline segment (2 categories → 1 segment).
    let lines = doc.pages[0]
        .ops
        .iter()
        .filter(|op| matches!(op, DrawOp::Line(_)))
        .count();
    assert!(
        lines >= 3,
        "expected axes + a polyline segment, got {lines} line ops"
    );
}

#[test]
fn pie_chart_draws_filled_polygon_slices() {
    use rpt_model::{ChartGraphType, ChartObject, Group};
    // A Pie chart over the grouped data: one filled polygon wedge per category, no axes, and no
    // "not yet supported" diagnostic.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    let mut g = Group::default();
    g.condition_field = "t.region".into();
    report.data_definition.groups = vec![g];

    let mut cdef = ChartObject::default();
    cdef.data_refs = vec!["t.amt".into()];
    cdef.category_refs = vec!["t.region".into()];
    cdef.definition.data_axis_title = "Sum of amt".into();
    cdef.definition.graph_type = ChartGraphType::Pie;
    // Turn the legend on so this also proves a non-bar type composes with the legend band (the legend
    // is reserved before the type dispatch, so pie draws into the reduced body while the swatches sit
    // in the band).
    cdef.definition.legend_visible = true;
    cdef.definition.legend_position = rpt_model::ChartLegendPosition::Right;
    let mut chart = ReportObject::default();
    chart.name = "Graph1".into();
    chart.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(6000),
        height: Twips(6000),
    };
    chart.kind = ReportObjectKind::Chart(Box::new(cdef));
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "RH",
                6000,
                vec![chart],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(AreaSectionKind::Detail, "D", 240, vec![])],
        ),
    ];
    let saved = saved_data(
        &[
            ("t.region", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[&["A", "10"], &["A", "20"], &["B", "100"]],
    );
    let doc = rendered(&report, &saved);

    assert!(
        !doc.diagnostics
            .iter()
            .any(|d| d.message.contains("not yet supported")),
        "pie chart should be supported: {:?}",
        doc.diagnostics
    );
    // Two categories → two filled polygon slices.
    let polys = doc.pages[0]
        .ops
        .iter()
        .filter(|op| matches!(op, DrawOp::Polygon(p) if p.closed && p.fill.is_some()))
        .count();
    assert_eq!(polys, 2, "expected 2 pie slices, got {polys}");
    // The legend composed with the pie: one 150-twip swatch square per category.
    let swatches = doc.pages[0]
        .ops
        .iter()
        .filter(|op| matches!(op, DrawOp::Rect(r) if r.bounds.width.0 == 150 && r.bounds.height.0 == 150))
        .count();
    assert_eq!(swatches, 2, "expected 2 legend swatches, got {swatches}");
}

#[test]
fn riser_3d_chart_dispatches_to_the_perspective_path() {
    use rpt_model::{ChartGraphType, ChartObject, Group};
    // A 3-D riser chart over the grouped data: it routes to the perspective renderer (filled polygon
    // faces, no bar rects), records the view-angle-approximation diagnostic, and does NOT record the
    // generic "not yet supported" fallback diagnostic.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    let mut g = Group::default();
    g.condition_field = "t.region".into();
    report.data_definition.groups = vec![g];

    let mut cdef = ChartObject::default();
    cdef.data_refs = vec!["t.amt".into()];
    cdef.category_refs = vec!["t.region".into()];
    cdef.definition.data_axis_title = "Sum of amt".into();
    cdef.definition.graph_type = ChartGraphType::Riser3D;
    assert!(cdef.definition.is_3d(), "Riser3D is a 3-D family");
    let mut chart = ReportObject::default();
    chart.name = "Graph1".into();
    chart.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(6000),
        height: Twips(4000),
    };
    chart.kind = ReportObjectKind::Chart(Box::new(cdef));
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "RH",
                4000,
                vec![chart],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(AreaSectionKind::Detail, "D", 240, vec![])],
        ),
    ];
    let saved = saved_data(
        &[
            ("t.region", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[&["A", "10"], &["A", "20"], &["B", "100"]],
    );
    let doc = rendered(&report, &saved);

    // The 3-D path draws filled polygon faces (3 scenery planes + 3 per riser), no bar rects.
    let polys = doc.pages[0]
        .ops
        .iter()
        .filter(|op| matches!(op, DrawOp::Polygon(p) if p.closed && p.fill.is_some()))
        .count();
    // 2 categories (single series) → 3 scenery planes + 3 faces × 2 risers = 9.
    assert_eq!(polys, 9, "expected 3 planes + 3 faces/riser, got {polys}");
    // The default (Standard) view angle is decoded and rendered at its native angle, so no
    // approximation diagnostic fires; nor does the generic bar-fallback one.
    assert!(
        !doc.diagnostics
            .iter()
            .any(|d| d.message.contains("view-angle preset")),
        "default (Standard) 3-D chart records no view-angle diagnostic: {:?}",
        doc.diagnostics
    );
    assert!(
        !doc.diagnostics
            .iter()
            .any(|d| d.message.contains("not yet supported")),
        "3-D chart must not record the bar-fallback diagnostic: {:?}",
        doc.diagnostics
    );

    // A non-default preset is rendered at an approximated angle (its disk byte→preset mapping is not
    // currently decoded), so it DOES record the approximation diagnostic.
    let mut report2 = report.clone();
    if let Some(obj) = report2.report_definition.areas[0].sections[0]
        .objects
        .first_mut()
    {
        if let ReportObjectKind::Chart(c) = &mut obj.kind {
            c.definition.view_angle = rpt_model::ChartViewAngle::BirdsEyeView;
        }
    }
    let doc2 = rendered(&report2, &saved);
    assert!(
        doc2.diagnostics
            .iter()
            .any(|d| d.message.contains("non-default view-angle preset")),
        "non-default 3-D view angle records the approximation diagnostic: {:?}",
        doc2.diagnostics
    );
}

#[test]
fn gantt_chart_draws_one_horizontal_bar_per_record() {
    use rpt_model::{ChartGraphType, ChartObject};
    // A Gantt chart binds two date fields (start, end) and draws one horizontal bar per DETAIL record
    // spanning [start..end] on a date axis — not a group summary. The report is ungrouped, so this
    // also proves the per-record path bypasses the (empty) group-series path.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);

    let mut cdef = ChartObject::default();
    cdef.data_refs = vec!["t.start".into(), "t.finish".into()];
    cdef.definition.graph_type = ChartGraphType::Gantt;
    let mut chart = ReportObject::default();
    chart.name = "Graph1".into();
    chart.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(8000),
        height: Twips(4000),
    };
    chart.kind = ReportObjectKind::Chart(Box::new(cdef));
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "RH",
                4000,
                vec![chart],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(AreaSectionKind::Detail, "D", 240, vec![])],
        ),
    ];
    let saved = saved_data(
        &[
            ("t.start", FieldValueType::Date),
            ("t.finish", FieldValueType::Date),
        ],
        &[
            &["2024-01-01", "2024-01-10"],
            &["2024-01-05", "2024-01-08"],
            &["2024-01-20", "2024-01-31"],
        ],
    );
    let doc = rendered(&report, &saved);

    // Not the empty-placeholder path: the per-record series is built, no "no group series" warning.
    assert!(
        !doc.diagnostics
            .iter()
            .any(|d| d.message.contains("no group series") || d.message.contains("no datable")),
        "gantt should build a per-record series: {:?}",
        doc.diagnostics
    );
    // One horizontal bar per detail record (three), stacked top-to-bottom on the date axis.
    let bars: Vec<(i32, i32, i32)> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Rect(r) => Some((r.bounds.left.0, r.bounds.top.0, r.bounds.width.0)),
            _ => None,
        })
        .collect();
    assert_eq!(bars.len(), 3, "one bar per record: {bars:?}");
    assert!(
        bars[0].1 < bars[1].1 && bars[1].1 < bars[2].1,
        "records stack top-to-bottom: {bars:?}"
    );
    // Record 3 starts on 1/20, right of record 1's 1/1; record 1's 9-day span is wider than
    // record 2's 3-day span.
    assert!(bars[2].0 > bars[0].0, "later start further right: {bars:?}");
    assert!(bars[0].2 > bars[1].2, "longer span is wider: {bars:?}");
}

#[test]
fn legend_reserves_a_band_on_each_side() {
    use crate::chart::{legend, LegendPosition as LP};
    let rect = Rect {
        left: Twips(0),
        top: Twips(0),
        width: Twips(6000),
        height: Twips(4000),
    };
    let series = vec![
        ("A".to_string(), 1.0),
        ("B".to_string(), 2.0),
        ("C".to_string(), 3.0),
    ];

    // Each position shrinks the body rect on the correct side and draws swatch + label ops.
    let (ops_r, body_r) = legend(rect, LP::Right, &series, false, "S", "G");
    assert!(
        body_r.width.0 < rect.width.0 && body_r.left.0 == rect.left.0,
        "Right shrinks width, keeps left"
    );
    let (_ops_l, body_l) = legend(rect, LP::Left, &series, false, "S", "G");
    assert!(
        body_l.width.0 < rect.width.0 && body_l.left.0 > rect.left.0,
        "Left shrinks width, pushes left in"
    );
    let (_ops_t, body_t) = legend(rect, LP::Top, &series, false, "S", "G");
    assert!(
        body_t.height.0 < rect.height.0 && body_t.top.0 > rect.top.0,
        "Top shrinks height, pushes top down"
    );
    let (_ops_b, body_b) = legend(rect, LP::Bottom, &series, false, "S", "G");
    assert!(
        body_b.height.0 < rect.height.0 && body_b.top.0 == rect.top.0,
        "Bottom shrinks height, keeps top"
    );

    // One colour swatch (Rect) + one label (Text) per series entry.
    let swatches = ops_r
        .iter()
        .filter(|op| matches!(op, DrawOp::Rect(_)))
        .count();
    let labels = ops_r
        .iter()
        .filter(|op| matches!(op, DrawOp::Text(_)))
        .count();
    assert_eq!(swatches, 3, "one swatch per entry");
    assert_eq!(labels, 3, "one label per entry");
}

/// A Report Header (marked underlay or not) carrying one text, followed by a Detail band. Small
/// `rows` count keeps everything on one page.
fn underlay_report(underlay: bool) -> Report {
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    let mut rh = section(
        AreaSectionKind::ReportHeader,
        "ReportHeader",
        600,
        vec![text_object("Mark", "WMARK", 0)],
    );
    rh.format.underlay_section = underlay;
    report.report_definition.areas = vec![
        area(AreaSectionKind::ReportHeader, vec![rh]),
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                300,
                vec![text_object("Row", "line", 0)],
            )],
        ),
    ];
    report
}

fn underlay_saved(rows: usize) -> SavedData {
    numeric_rows(rows)
}

/// The op-list index and `top` twip of the first `DrawOp::Text` with the given text on `page`.
fn text_op(page: &rpt_pages::Page, text: &str) -> Option<(usize, i32)> {
    page.ops.iter().enumerate().find_map(|(i, op)| match op {
        DrawOp::Text(t) if t.text == text => Some((i, t.bounds.top.0)),
        _ => None,
    })
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

/// Two region groups (two detail rows each), grouped by `t.region`, with a group header ("GH") and a
/// detail band ("line"). `keep` sets `GroupAreaFormat.keep_group_together` on the group-header area.
fn keep_together_report(page_height: i32, keep: bool) -> (Report, SavedData) {
    use rpt_model::{Group, GroupAreaFormat, SortDirection};
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(page_height);
    let mut g = Group::default();
    g.condition_field = "t.region".into();
    g.sort.direction = SortDirection::AscendingOrder;
    report.data_definition.groups = vec![g];

    let mut gaf = GroupAreaFormat::default();
    gaf.keep_group_together = keep;
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
    report.report_definition.areas = vec![
        gh,
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                300,
                vec![text_object("Row", "line", 0)],
            )],
        ),
    ];
    let saved = saved_data(
        &[
            ("t.region", FieldValueType::String),
            ("t.x", FieldValueType::Number),
        ],
        &[&["A", "1"], &["A", "2"], &["B", "3"], &["B", "4"]],
    );
    (report, saved)
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
