use super::*;

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

/// A cross-tab's decomposed cell objects (the decoder surfaces them flat in the section, inside the
/// cross-tab box) must not be drawn on top of the native grid; a real object outside the box still is.
#[test]
fn crosstab_decomposed_cell_objects_are_not_double_drawn() {
    let ct = crosstab_object("CT1"); // box: left 100, top 0, 8000 × 4000
    let inside = text_object("", "INSIDE_CELL", 500); // top-left (100, 500): within the cross-tab box
    let mut outside = text_object("Title", "OUTSIDE_TITLE", 0);
    outside.bounds.top = Twips(4500); // below the cross-tab box bottom (4000)

    let saved = saved_data(
        &[
            ("t.region", FieldValueType::String),
            ("t.quarter", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[&["East", "Q1", "10"]],
    );
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![area(
        AreaSectionKind::ReportHeader,
        vec![section(
            AreaSectionKind::ReportHeader,
            "RH",
            5000,
            vec![ct, inside, outside],
        )],
    )];
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
        !texts.contains(&"INSIDE_CELL"),
        "a cell object inside the cross-tab box must be suppressed (drawn by the grid): {texts:?}"
    );
    assert!(
        texts.contains(&"OUTSIDE_TITLE"),
        "an object outside the cross-tab box must still render: {texts:?}"
    );
}
