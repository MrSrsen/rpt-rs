use super::*;

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
    obj.kind = ReportObjectKind::Field(Box::new(field));

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

/// A formula field object leaves its own `value_type` as `Unknown` (the type lives on the formula
/// *definition*, not the placed object). The layout engine must resolve that effective value type
/// before picking the display format, so the field honours its stored **currency** format leaf
/// (symbol + 2 decimals) rather than falling through to bare number formatting.
#[test]
fn formula_field_honours_stored_currency_format() {
    use rpt_model::{
        CommonFieldFormat, CurrencySymbolFormat, FieldDef, FieldFormat, FieldKindData, FieldObject,
        FieldRefKind, Formula, FormulaField, NumericFieldFormat,
    };

    // Field object bound to `{@Amt}`; its own `value_type` stays `Unknown`, as `rpt` decodes it for
    // formula objects — the effective type comes from the formula definition. The `data_source` is
    // brace-wrapped exactly as `rpt` decodes it (the sigil resolution keys off the leading `{`).
    let mut field = FieldObject::default();
    field.data_source = "{@Amt}".into();
    field.ref_kind = FieldRefKind::Formula;
    field.value_type = FieldValueType::Unknown;
    // An explicit (non-system-default) currency leaf: a `$` symbol and 2 decimal places.
    field.format = Some(FieldFormat {
        common: CommonFieldFormat {
            use_system_defaults: false,
            ..Default::default()
        },
        numeric: NumericFieldFormat {
            decimal_places: 2,
            currency_symbol: CurrencySymbolFormat::FixedSymbol,
            currency_symbol_text: "$".into(),
            ..Default::default()
        },
        ..Default::default()
    });
    let mut obj = ReportObject::default();
    obj.name = "Amt".into();
    obj.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(3000),
        height: Twips(240),
    };
    obj.kind = ReportObjectKind::Field(Box::new(field));

    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(AreaSectionKind::Detail, "Details", 300, vec![obj])],
    )];
    // The formula definition carries the value type (Currency); the object above does not.
    let mut amt = FieldDef::default();
    amt.name = "Amt".into();
    amt.value_type = FieldValueType::Currency;
    amt.kind = FieldKindData::Formula(FormulaField {
        text: Formula("100.5".into()),
        ..FormulaField::default()
    });
    report.data_definition.field_definitions = vec![amt];

    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["1"]]);
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
    // With the effective type resolved to Currency, the stored `$`/2-decimal leaf applies; without
    // it the field would render as a bare "100.50".
    assert_eq!(texts, vec!["$100.50"]);
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
    obj.kind = ReportObjectKind::Field(Box::new(field));

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
    obj.kind = ReportObjectKind::Field(Box::new(field));

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
    // The suppress condition is stored under its reserved Crystal name, `Object_Visibility`.
    hide.format.condition_formulas = vec![("Object_Visibility".into(), "{t.x} > 5".into())];
    let mut red = text_object("Red", "SHOWN", 0);
    red.bounds.top = Twips(300);
    if let ReportObjectKind::Text(t) = &mut red.kind {
        // The font-color condition is stored under its reserved Crystal name, `Font_Color`.
        t.font_color.condition_formulas = vec![("Font_Color".into(), "Color(255, 0, 0)".into())];
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

#[test]
fn conditional_section_background_color() {
    use rpt_model::Color;

    // Two detail rows; the section's own BackgroundColor condition tints the row whose field is 1
    // (RGB(255,220,220)) and leaves the other white — the section-level conditional-format path.
    let mut detail = section(
        AreaSectionKind::Detail,
        "Details",
        300,
        vec![db_field_object("Cell", "t.x", 0)],
    );
    detail.condition_formulas = vec![(
        "Section_Back_Color".into(),
        "if {t.x} = 1 then RGB(255, 220, 220) else crWhite".into(),
    )];

    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![area(AreaSectionKind::Detail, vec![detail])];

    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["1"], &["2"]]);
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    let fills: Vec<Color> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Rect(r) => r.fill.as_ref().map(rpt_pages::Fill::representative_color),
            _ => None,
        })
        .collect();
    let tint = Color {
        a: 255,
        r: 255,
        g: 220,
        b: 220,
    };
    let white = Color {
        a: 255,
        r: 255,
        g: 255,
        b: 255,
    };
    // The first row resolves to the tint, the second to white (crWhite).
    assert!(
        fills.contains(&tint),
        "under-performer section tint must be emitted: {fills:?}"
    );
    assert!(
        fills.contains(&white),
        "non-tinted section resolves to crWhite: {fills:?}"
    );
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

/// A line object stores its stroke style and colour in the border record (the `0xec` leaf); only the
/// thickness lives on the shape. The renderer must read the border to emit a visible stroke.
#[test]
fn line_stroke_read_from_border() {
    let mut o = ReportObject::default();
    o.name = "Divider".into();
    o.bounds = Rect {
        left: Twips(0),
        top: Twips(50),
        width: Twips(3000),
        height: Twips(0),
    };
    let mut ls = LineShape::default();
    ls.shape.line_thickness = Twips(10); // thin hairline; style/colour come from the border
    o.kind = ReportObjectKind::Line(ls);
    o.border.top = LineStyle::SingleLine;
    let color = Color {
        a: 255,
        r: 0x6b,
        g: 0x76,
        b: 0x83,
    };
    o.border.border_color = Some(color);

    let mut report = tiny_report(15840);
    report.report_definition.areas[1].sections[0]
        .objects
        .push(o);
    let doc = rendered(&report, &numeric_rows(1));
    let lines: Vec<&rpt_pages::LineOp> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Line(l) => Some(l),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 1, "the line must emit one stroke: {lines:?}");
    assert_eq!(lines[0].stroke.color, color, "colour comes from the border");
    assert_eq!(lines[0].stroke.width, Twips(10), "thickness from the shape");
}

/// A picture's raster is drawn at its authored scaled size (`original × scaling`) placed at the box
/// top-left, not stretched to fill the object box.
#[test]
fn picture_drawn_at_scaled_size() {
    let mut o = ReportObject::default();
    o.name = "Logo".into();
    // A box deliberately larger than the authored scaled size (1000×500 × 2.0 = 2000×1000).
    o.bounds = Rect {
        left: Twips(120),
        top: Twips(60),
        width: Twips(9999),
        height: Twips(8888),
    };
    let mut p = rpt_model::PictureObject::default();
    // A PNG signature so the format sniffs as a browser-renderable raster.
    p.data = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    p.original_width = Twips(1000);
    p.original_height = Twips(500);
    p.x_scaling = 2.0;
    p.y_scaling = 2.0;
    o.kind = ReportObjectKind::Picture(p);

    let mut report = tiny_report(15840);
    report.report_definition.areas[1].sections[0].objects = vec![o];
    let doc = rendered(&report, &numeric_rows(1));
    let img = doc.pages[0]
        .ops
        .iter()
        .find_map(|op| match op {
            DrawOp::Image(i) => Some(i),
            _ => None,
        })
        .expect("the picture image op");
    assert_eq!(
        img.bounds.width,
        Twips(2000),
        "width = original × x_scaling"
    );
    assert_eq!(
        img.bounds.height,
        Twips(1000),
        "height = original × y_scaling"
    );
    // Placed at the box top-left (the printable-relative box left/top, below the page header).
    assert_eq!(img.bounds.left, Twips(120), "placed at the box left");
    assert_eq!(
        img.bounds.top,
        Twips(360),
        "placed at the box top (300 header + 60)"
    );
}

#[test]
fn group_footers_pair_to_header_levels_by_name_not_position() {
    // Header levels (outermost first): region → level 0, order_date → level 1.
    let header_keys = ["nameHeader".to_string(), "orderdateHeader".to_string()]
        .iter()
        .map(|n| crate::group_area_key(n, "Header"))
        .collect::<Vec<_>>();

    // Footers are stored OUTERMOST-first.
    let secs = [
        footer_section("nameFooter"),
        footer_section("orderdateFooter"),
    ];
    let footer_entries = vec![
        (
            crate::group_area_key("nameFooter", "Footer"),
            vec![&secs[0]],
        ),
        (
            crate::group_area_key("orderdateFooter", "Footer"),
            vec![&secs[1]],
        ),
    ];
    let ordered = crate::order_group_footers(&header_keys, footer_entries);
    assert_eq!(ordered[0][0].name, "nameFooter", "level 0 = region footer");
    assert_eq!(
        ordered[1][0].name, "orderdateFooter",
        "level 1 = month footer"
    );
}

#[test]
fn group_footers_pair_across_digit_suffixes_regardless_of_order() {
    // Header levels 0..2 sharing a prefix, disambiguated only by digit suffix.
    let header_keys = ["nameHeader", "nameHeader1", "nameHeader2"]
        .iter()
        .map(|n| crate::group_area_key(n, "Header"))
        .collect::<Vec<_>>();

    // Footers stored INNERMOST-first (the canonical designer order).
    let secs = [
        footer_section("nameFooter2"),
        footer_section("nameFooter1"),
        footer_section("nameFooter"),
    ];
    let footer_entries = vec![
        (
            crate::group_area_key("nameFooter2", "Footer"),
            vec![&secs[0]],
        ),
        (
            crate::group_area_key("nameFooter1", "Footer"),
            vec![&secs[1]],
        ),
        (
            crate::group_area_key("nameFooter", "Footer"),
            vec![&secs[2]],
        ),
    ];
    let ordered = crate::order_group_footers(&header_keys, footer_entries);
    assert_eq!(ordered[0][0].name, "nameFooter");
    assert_eq!(ordered[1][0].name, "nameFooter1");
    assert_eq!(ordered[2][0].name, "nameFooter2");
}

#[test]
fn group_footers_fall_back_to_reverse_when_names_do_not_pair() {
    // Two headers but a footer whose key matches neither: fall back to innermost-first reverse.
    let header_keys = ["aHeader", "bHeader"]
        .iter()
        .map(|n| crate::group_area_key(n, "Header"))
        .collect::<Vec<_>>();
    let secs = [footer_section("xFooter"), footer_section("yFooter")];
    let footer_entries = vec![
        (crate::group_area_key("xFooter", "Footer"), vec![&secs[0]]),
        (crate::group_area_key("yFooter", "Footer"), vec![&secs[1]]),
    ];
    let ordered = crate::order_group_footers(&header_keys, footer_entries);
    // Reversed input order.
    assert_eq!(ordered[0][0].name, "yFooter");
    assert_eq!(ordered[1][0].name, "xFooter");
}
