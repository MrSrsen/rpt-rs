//! Layout unit tests over a hand-built minimal report + dataset.
//!
//! Several `rpt` model structs (`ChartObject`, `CrossTabObject`, …) are cross-crate
//! `#[non_exhaustive]`, so the builders below construct them via `Default` + field assignment
//! (struct literals are disallowed cross-crate) — which `clippy::field_reassign_with_default` flags
//! but cannot offer a valid rewrite for, so the lint is allowed for this test module.
#![allow(clippy::field_reassign_with_default)]

pub(crate) use crate::layout;
pub(crate) use rpt_data::{build_dataset, SavedDataSource};
pub(crate) use rpt_model::{
    Area, AreaSectionKind, Color, FieldObject, FieldRefKind, FieldValueType, LineShape, LineStyle,
    Rect, Report, ReportObject, ReportObjectKind, SavedColumn, SavedData, Section, TextObject,
    Twips, VerticalAlignment,
};
pub(crate) use rpt_pages::DrawOp;
pub(crate) use rpt_test_support::saved_data;

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

fn field_heading_object(name: &str, text: &str, top: i32) -> ReportObject {
    let mut o = ReportObject::default();
    o.name = name.to_string();
    o.bounds = Rect {
        left: Twips(100),
        top: Twips(top),
        width: Twips(3000),
        height: Twips(240),
    };
    let mut h = rpt_model::FieldHeadingObject::default();
    h.text = text.to_string();
    o.kind = ReportObjectKind::FieldHeading(h);
    o
}

/// A field object bound to a database field reference (`t.name`), rendered as a string.
fn db_field_object(name: &str, reference: &str, top: i32) -> ReportObject {
    let mut o = ReportObject::default();
    o.name = name.to_string();
    o.bounds = Rect {
        left: Twips(100),
        top: Twips(top),
        width: Twips(3000),
        height: Twips(240),
    };
    let mut f = FieldObject::default();
    f.data_source = reference.to_string();
    f.ref_kind = FieldRefKind::DatabaseField;
    f.value_type = FieldValueType::String;
    o.kind = ReportObjectKind::Field(Box::new(f));
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

// --- Paragraph indentation + reading order (IndentAndSpacingFormat / ReadingOrder). ---

/// A single-paragraph text object at (left=100, width=3000) carrying the given per-paragraph indents
/// (twips). Stays one line unless `can_grow` is set by the caller.
fn indented_text(name: &str, text: &str, left: i32, right: i32, first: i32) -> ReportObject {
    use rpt_model::{IndentAndSpacingFormat, Paragraph};
    let mut o = text_object(name, text, 0);
    if let ReportObjectKind::Text(t) = &mut o.kind {
        let mut p = Paragraph::default();
        p.indent = IndentAndSpacingFormat {
            left_indent: Twips(left),
            right_indent: Twips(right),
            first_line_indent: Twips(first),
            ..Default::default()
        };
        t.paragraphs = vec![p];
    }
    o
}

/// Lay `obj` out alone in a report header (a flow band, so can-grow applies) and return its text runs.
fn runs_for_object(obj: ReportObject) -> Vec<rpt_pages::TextRun> {
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![area(
        AreaSectionKind::ReportHeader,
        vec![section(
            AreaSectionKind::ReportHeader,
            "RH",
            2000,
            vec![obj],
        )],
    )];
    let saved = empty_saved();
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);
    doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
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

/// An ungrouped dataset from a flat saved-data rowset (no report grouping), for testing the chart's
/// own category grouping directly.
fn ungrouped_dataset(saved: &SavedData) -> rpt_data::Dataset {
    let report = Report::default();
    build_dataset(&SavedDataSource::new(saved), &report.data_definition)
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

/// A footer whose sections carry a single named section, for asserting which footer landed at a level.
fn footer_section(name: &str) -> Section {
    let mut s = Section::default();
    s.kind = AreaSectionKind::GroupFooter;
    s.name = name.to_string();
    s
}

// --- Cross-page subreport flow (a subreport taller than one page splits across parent pages). ---

/// A subreport whose detail band renders one row per saved record (field `s.v`), `rows` rows tall at
/// `detail_h` twips each, driven by its own saved data (values `R0..R{rows}`).
fn subreport_rows(name: &str, rows: usize, detail_h: i32) -> rpt_model::Subreport {
    let mut sub = Report::default();
    sub.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(
            AreaSectionKind::Detail,
            "SubDetail",
            detail_h,
            vec![db_field_object("Cell", "s.v", 0)],
        )],
    )];
    sub.saved_data = Some(SavedData {
        record_count: rows as u32,
        columns: vec![SavedColumn {
            name: "s.v".into(),
            value_type: FieldValueType::String,
        }],
        rows: (0..rows).map(|i| vec![Some(format!("R{i}"))]).collect(),
    });
    let mut sr = rpt_model::Subreport::default();
    sr.name = name.to_string();
    sr.report = Box::new(sub);
    sr
}

/// A main report whose single detail band holds one subreport object (box `box_h` tall) over one
/// detail row, with a body `page_h` twips tall and no page footer.
fn main_with_subreport(sr: rpt_model::Subreport, page_h: i32, box_h: i32) -> Report {
    let mut sub_obj = ReportObject::default();
    sub_obj.name = "SubObj".into();
    sub_obj.bounds = Rect {
        left: Twips(0),
        top: Twips(0),
        width: Twips(6000),
        height: Twips(box_h),
    };
    let mut so = rpt_model::SubreportObject::default();
    so.subreport_name = sr.name.clone();
    sub_obj.kind = ReportObjectKind::Subreport(so);

    let mut main = Report::default();
    main.print_options.content_width = Twips(12240);
    main.print_options.content_height = Twips(page_h);
    main.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(
            AreaSectionKind::Detail,
            "MainDetail",
            box_h,
            vec![sub_obj],
        )],
    )];
    main.subreports = vec![sr];
    main
}

/// A multi-paragraph text object: one run per paragraph, each carrying its own font size and line
/// spacing, wide enough not to wrap. `display` joins the paragraphs so `text_plan` splits them back.
fn multi_para_object(name: &str, paras: &[(rpt_model::LineSpacing, f32, &str)]) -> ReportObject {
    use rpt_model::{Font, IndentAndSpacingFormat, Paragraph};
    let mut o = text_object(name, "", 0);
    o.bounds.width = Twips(12000);
    o.bounds.height = Twips(6000);
    if let ReportObjectKind::Text(t) = &mut o.kind {
        t.display = paras
            .iter()
            .map(|(_, _, s)| *s)
            .collect::<Vec<_>>()
            .join("\n");
        t.paragraphs = paras
            .iter()
            .map(|(ls, sz, s)| {
                let mut font = Font::default();
                font.size_pt = *sz;
                Paragraph {
                    runs: vec![rpt_model::TextRun {
                        text: s.to_string(),
                        field_ref: None,
                        font: Some(font),
                    }],
                    indent: IndentAndSpacingFormat {
                        line_spacing: *ls,
                        ..Default::default()
                    },
                }
            })
            .collect();
    }
    o
}

mod charts;
mod crosstab;
mod objects;
mod pagination;
mod subreports;
mod text;
