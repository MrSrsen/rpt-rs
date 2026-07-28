use super::*;

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

#[test]
fn chart_series_ungrouped_buckets_string_category_and_sums() {
    use crate::aggregate::{chart_series_ungrouped, LabelPeriod};
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
        LabelPeriod::Monthly,
    );
    assert_eq!(
        series,
        vec![("A".to_string(), 30.0), ("B".to_string(), 100.0)]
    );
}

#[test]
fn chart_series_ungrouped_buckets_temporal_category_by_month_ascending() {
    use crate::aggregate::{chart_series_ungrouped, LabelPeriod};
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
        LabelPeriod::Monthly,
    );
    let values: Vec<f64> = series.iter().map(|(_, v)| *v).collect();
    assert_eq!(values, vec![20.0, 100.0], "two monthly buckets, ascending");
    // Monthly buckets read as M/YYYY (no leading zero), matching the engine, not a full localized date.
    let labels: Vec<&str> = series.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels, vec!["1/2024", "2/2024"], "monthly bucket labels");
}

#[test]
fn chart_series_ungrouped_honours_weekly_period() {
    use crate::aggregate::{chart_series_ungrouped, LabelPeriod};
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
        LabelPeriod::Weekly,
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
