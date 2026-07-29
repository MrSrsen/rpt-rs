//! Behavioural tests for the KDL exporter, built from synthetic [`rpt_model::Report`]s.
#![allow(missing_docs)]

use kdl::{KdlDocument, KdlValue};
use rpt_model::*;

/// A report with just a version — the smallest well-formed document.
fn empty_report() -> Report {
    Report {
        version: 700,
        ..Default::default()
    }
}

/// Parse the exported text back into a document (asserts it is valid KDL) and return it.
fn reparse(report: &Report) -> KdlDocument {
    let text = rpt_kdl::to_kdl_string(report);
    KdlDocument::parse(&text).unwrap_or_else(|e| panic!("exported KDL does not parse: {e}\n{text}"))
}

/// The single top-level `report` node of a document.
fn report_node(doc: &KdlDocument) -> &kdl::KdlNode {
    doc.nodes()
        .iter()
        .find(|n| n.name().value() == "report")
        .expect("a report node")
}

/// Depth-first search for the first descendant node with the given name.
fn find<'a>(node: &'a kdl::KdlNode, name: &str) -> Option<&'a kdl::KdlNode> {
    if let Some(children) = node.children() {
        for c in children.nodes() {
            if c.name().value() == name {
                return Some(c);
            }
            if let Some(found) = find(c, name) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn golden_minimal() {
    let out = rpt_kdl::to_kdl_string(&empty_report());
    let expected = "\
report version=700 {
    page
    layout
}
";
    assert_eq!(out, expected);
}

#[test]
fn golden_field_sparse() {
    // A plainly formatted database field: no format sub-nodes, no default-valued properties.
    let mut report = empty_report();
    let mut section = Section {
        name: "Details".into(),
        height: Twips(300),
        ..Default::default()
    };
    section.objects.push(ReportObject {
        name: "Field1".into(),
        bounds: Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(1440),
            height: Twips(240),
        },
        kind: ReportObjectKind::Field(Box::new(FieldObject {
            data_source: "{orders.id}".into(),
            ref_kind: FieldRefKind::DatabaseField,
            value_type: FieldValueType::Int32s,
            // A default field format must emit nothing.
            format: Some(FieldFormat::default()),
            ..Default::default()
        })),
        ..Default::default()
    });
    report.report_definition.areas.push(Area {
        kind: AreaSectionKind::Detail,
        name: "Details".into(),
        sections: vec![section],
        ..Default::default()
    });

    let out = rpt_kdl::to_kdl_string(&report);
    let expected = "\
report version=700 {
    page
    layout {
        area Details {
            section Details height=300 {
                field Field1 x=0 y=0 w=1440 h=240 src=\"{orders.id}\" value-type=int32
            }
        }
    }
}
";
    assert_eq!(out, expected);
}

#[test]
fn default_field_format_emits_no_children() {
    let mut report = empty_report();
    let mut section = Section::default();
    section.objects.push(ReportObject {
        name: "F".into(),
        kind: ReportObjectKind::Field(Box::new(FieldObject {
            data_source: "{t.f}".into(),
            format: Some(FieldFormat::default()),
            ..Default::default()
        })),
        ..Default::default()
    });
    report.report_definition.areas.push(Area {
        sections: vec![section],
        ..Default::default()
    });

    let doc = reparse(&report);
    let field = find(report_node(&doc), "field").expect("a field node");
    // No numeric/date/boolean/font children were emitted for a default format + default font.
    assert!(
        field.children().is_none(),
        "default format emits no children"
    );
}

#[test]
fn non_default_numeric_format_emits_child() {
    let mut report = empty_report();
    let mut section = Section::default();
    let numeric = NumericFieldFormat {
        decimal_places: 0,
        negative: NegativeFormat::Bracketed,
        ..NumericFieldFormat::default()
    };
    section.objects.push(ReportObject {
        name: "F".into(),
        kind: ReportObjectKind::Field(Box::new(FieldObject {
            data_source: "{t.amt}".into(),
            value_type: FieldValueType::Number,
            format: Some(FieldFormat {
                numeric,
                ..Default::default()
            }),
            ..Default::default()
        })),
        ..Default::default()
    });
    report.report_definition.areas.push(Area {
        sections: vec![section],
        ..Default::default()
    });

    let doc = reparse(&report);
    let field = find(report_node(&doc), "field").expect("a field");
    let num = find(field, "numeric").expect("a numeric format child");
    assert_eq!(num.get("decimals"), Some(&KdlValue::Integer(0)));
    assert_eq!(
        num.get("negative"),
        Some(&KdlValue::String("parentheses".into()))
    );
    // The at-default rounding place is not emitted.
    assert!(num.get("rounding").is_none());
}

#[test]
fn enum_kebab_case_and_other_fallback() {
    let mut report = empty_report();
    // A known join renders as a kebab token; an unmapped ordinal falls back to a bare integer.
    report.database.tables.push(Table {
        name: "a".into(),
        alias: "a".into(),
        ..Default::default()
    });
    report.database.links.push(TableLink {
        join_kind: TableJoinKind::LeftOuter,
        operator: TableLinkOperator::GreaterOrEqual,
        source_table_alias: "a".into(),
        target_table_alias: "b".into(),
        ..Default::default()
    });
    report.database.links.push(TableLink {
        join_kind: TableJoinKind::Other(42),
        source_table_alias: "b".into(),
        target_table_alias: "c".into(),
        ..Default::default()
    });

    let doc = reparse(&report);
    let db = find(report_node(&doc), "database").expect("a database node");
    let links: Vec<_> = db
        .children()
        .unwrap()
        .nodes()
        .iter()
        .filter(|n| n.name().value() == "link")
        .collect();
    assert_eq!(
        links[0].get("join"),
        Some(&KdlValue::String("left-outer".into()))
    );
    assert_eq!(
        links[0].get("op"),
        Some(&KdlValue::String("greater-or-equal".into()))
    );
    assert_eq!(links[1].get("join"), Some(&KdlValue::Integer(42)));
}

#[test]
fn multiline_formula_round_trips() {
    let mut report = empty_report();
    let body = "Sum({o.amt})\nWhilePrintingRecords\n  indented";
    report.data_definition.field_definitions.push(FieldDef {
        name: "Total".into(),
        kind: FieldKindData::Formula(FormulaField {
            text: Formula(body.into()),
            ..Default::default()
        }),
        ..Default::default()
    });

    let text = rpt_kdl::to_kdl_string(&report);
    // Emitted as a triple-quoted block, not an escaped one-liner.
    assert!(
        text.contains("\"\"\""),
        "multi-line formula uses triple quotes"
    );
    assert!(
        !text.contains("\\n"),
        "no escaped newline in a multi-line block"
    );

    // The value parses back to exactly the original body (indentation preserved).
    let doc = KdlDocument::parse(&text).expect("valid KDL");
    let formula = find(report_node(&doc), "formula").expect("a formula node");
    let arg = formula
        .entries()
        .iter()
        .rfind(|e| e.name().is_none())
        .expect("a positional body argument");
    assert_eq!(arg.value(), &KdlValue::String(body.into()));
}

#[test]
fn group_and_layout_nesting() {
    let mut report = empty_report();
    report.data_definition.groups.push(Group {
        condition_field: "{o.region}".into(),
        sort: Sort {
            direction: SortDirection::DescendingOrder,
            ..Default::default()
        },
        date_condition: Some(rpt_model::GroupCondition::Monthly),
        ..Default::default()
    });
    let mut section = Section {
        name: "GH".into(),
        height: Twips(200),
        ..Default::default()
    };
    section.objects.push(ReportObject {
        name: "L".into(),
        kind: ReportObjectKind::Line(LineShape {
            shape: DrawingShape {
                right: Twips(5000),
                bottom: Twips(0),
                line_thickness: Twips(15),
                ..Default::default()
            },
        }),
        ..Default::default()
    });
    report.report_definition.areas.push(Area {
        kind: AreaSectionKind::GroupHeader,
        name: "Group Header".into(),
        sections: vec![section],
        ..Default::default()
    });

    let doc = reparse(&report);
    let root = report_node(&doc);
    let group = find(root, "group").expect("a group node");
    // 1-based level as the positional argument, condition + direction + date period.
    assert_eq!(
        group.entries().first().map(|e| e.value()),
        Some(&KdlValue::Integer(1))
    );
    assert_eq!(
        group.get("on"),
        Some(&KdlValue::String("{o.region}".into()))
    );
    assert_eq!(
        group.get("dir"),
        Some(&KdlValue::String("descending".into()))
    );
    assert_eq!(group.get("date"), Some(&KdlValue::String("monthly".into())));

    // layout → area → section → line nesting is preserved.
    let line = find(find(root, "layout").unwrap(), "line").expect("a line under layout");
    assert_eq!(line.get("x2"), Some(&KdlValue::Integer(5000)));
    assert_eq!(line.get("thickness"), Some(&KdlValue::Integer(15)));
}

#[test]
fn subreport_recursion() {
    let mut report = empty_report();
    let mut sub = empty_report();
    sub.version = 800;
    sub.summary_info.title = "Inner".into();
    report.subreports.push(Subreport {
        name: "Sub1".into(),
        report: Box::new(sub),
        links: vec![SubreportLink {
            main_report_field: "{o.id}".into(),
            subreport_field: "{s.oid}".into(),
            linked_parameter: Some("?pid".into()),
        }],
    });

    let doc = reparse(&report);
    let subreport = find(report_node(&doc), "subreport").expect("a subreport node");
    assert_eq!(
        subreport.entries().first().map(|e| e.value()),
        Some(&KdlValue::String("Sub1".into()))
    );
    // A nested report node with its own version, recursively built.
    let inner = find(subreport, "report").expect("a nested report node");
    assert_eq!(inner.get("version"), Some(&KdlValue::Integer(800)));
    let link = find(subreport, "link").expect("a subreport link");
    assert_eq!(link.get("main"), Some(&KdlValue::String("{o.id}".into())));
    assert_eq!(link.get("param"), Some(&KdlValue::String("?pid".into())));
}

/// Count the top-level `report` children with the given node name.
fn count_children(doc: &KdlDocument, name: &str) -> usize {
    report_node(doc)
        .children()
        .map(|c| {
            c.nodes()
                .iter()
                .filter(|n| n.name().value() == name)
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn record_sort_field_is_emitted_and_group_sort_is_not() {
    // A record-level sort (Record Sort Expert) must surface as a `record-sort` node; a group-sort
    // entry in the same collection must not (group sorts are surfaced per-group).
    let mut report = empty_report();
    report.data_definition.record_sorts = vec![
        Sort {
            field: "product.name".into(),
            direction: SortDirection::DescendingOrder,
            kind: SortKind::RecordSortField,
            topn: None,
        },
        Sort {
            field: "cat.name".into(),
            direction: SortDirection::AscendingOrder,
            kind: SortKind::GroupSortField,
            topn: None,
        },
    ];

    let doc = reparse(&report);
    assert_eq!(
        count_children(&doc, "record-sort"),
        1,
        "exactly one RecordSortField is emitted; the GroupSortField is filtered out"
    );
    let sort = find(report_node(&doc), "record-sort").expect("a record-sort node");
    assert_eq!(
        sort.entries().first().map(|e| e.value()),
        Some(&KdlValue::String("product.name".into()))
    );
    assert_eq!(
        sort.get("dir"),
        Some(&KdlValue::String("descending".into()))
    );
}

#[test]
fn ascending_record_sort_omits_default_direction() {
    // Ascending is the default sort direction, so it is not emitted (sparse convention).
    let mut report = empty_report();
    report.data_definition.record_sorts = vec![Sort {
        field: "product.name".into(),
        direction: SortDirection::AscendingOrder,
        kind: SortKind::RecordSortField,
        topn: None,
    }];

    let doc = reparse(&report);
    let sort = find(report_node(&doc), "record-sort").expect("a record-sort node");
    assert_eq!(
        sort.entries().first().map(|e| e.value()),
        Some(&KdlValue::String("product.name".into()))
    );
    assert!(sort.get("dir").is_none(), "default ascending is omitted");
}

#[test]
fn report_options_are_emitted() {
    let mut report = empty_report();
    report.report_options = ReportOptions {
        save_data_with_report: true,
        use_dummy_data: true,
        ..Default::default()
    };

    let doc = reparse(&report);
    let opts = find(report_node(&doc), "report-options").expect("a report-options node");
    assert_eq!(opts.get("save-data"), Some(&KdlValue::Bool(true)));
    assert_eq!(opts.get("use-dummy-data"), Some(&KdlValue::Bool(true)));
    assert!(opts.get("save-summaries").is_none(), "default flag omitted");
}

#[test]
fn parameter_details_beyond_the_basics_are_emitted() {
    let mut report = empty_report();
    let mut pf = ParameterField {
        value_kind: ParameterValueKind::StringParameter,
        edit_mask: Some("AAAA".into()),
        report_name: Some("Sub".into()),
        has_current_value: true,
        ..Default::default()
    };
    pf.current_values.push(ParameterValue {
        value: "42".into(),
        ..Default::default()
    });
    report.data_definition.field_definitions.push(FieldDef {
        name: "?p".into(),
        kind: FieldKindData::Parameter(Box::new(pf)),
        ..Default::default()
    });

    let doc = reparse(&report);
    let param = find(report_node(&doc), "parameter").expect("a parameter node");
    assert_eq!(
        param.get("edit-mask"),
        Some(&KdlValue::String("AAAA".into()))
    );
    assert_eq!(param.get("report"), Some(&KdlValue::String("Sub".into())));
    assert_eq!(param.get("has-current-value"), Some(&KdlValue::Bool(true)));
    let current = find(param, "current").expect("a current-value node");
    assert_eq!(
        current.entries().first().map(|e| e.value()),
        Some(&KdlValue::String("42".into()))
    );
}

#[test]
fn table_columns_are_emitted() {
    let mut report = empty_report();
    report.database.tables.push(Table {
        name: "product".into(),
        alias: "product".into(),
        data_fields: vec![DbFieldDef {
            name: "sku".into(),
            value_type: FieldValueType::String,
            length: 82,
            ..Default::default()
        }],
        ..Default::default()
    });

    let doc = reparse(&report);
    let column = find(report_node(&doc), "column").expect("a table column node");
    assert_eq!(
        column.entries().first().map(|e| e.value()),
        Some(&KdlValue::String("sku".into()))
    );
    assert_eq!(column.get("length"), Some(&KdlValue::Integer(82)));
}

#[test]
fn color_and_twips_rendering() {
    let mut report = empty_report();
    let mut section = Section::default();
    section.objects.push(ReportObject {
        name: "B".into(),
        bounds: Rect {
            left: Twips(100),
            top: Twips(200),
            ..Default::default()
        },
        // A box's fill is its border's background colour.
        border: rpt_model::Border {
            background_color: Some(Color {
                a: 255,
                r: 0x12,
                g: 0x34,
                b: 0x56,
            }),
            ..Default::default()
        },
        kind: ReportObjectKind::Box(BoxShape {
            shape: DrawingShape {
                right: Twips(900),
                bottom: Twips(700),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    });
    report.report_definition.areas.push(Area {
        sections: vec![section],
        ..Default::default()
    });

    let doc = reparse(&report);
    let boxn = find(report_node(&doc), "box").expect("a box node");
    // Raw twips as bare integers; opaque color as a #rrggbb string.
    assert_eq!(boxn.get("x"), Some(&KdlValue::Integer(100)));
    assert_eq!(boxn.get("y2"), Some(&KdlValue::Integer(700)));
    let border = find(boxn, "border").expect("a border node");
    assert_eq!(
        border.get("background"),
        Some(&KdlValue::String("#123456".into()))
    );
}

#[test]
fn non_opaque_color_is_packed_argb_integer() {
    let mut report = empty_report();
    let mut section = Section::default();
    section.objects.push(ReportObject {
        name: "B".into(),
        border: rpt_model::Border {
            background_color: Some(Color {
                a: 0x80,
                r: 0x10,
                g: 0x20,
                b: 0x30,
            }),
            ..Default::default()
        },
        kind: ReportObjectKind::Box(BoxShape::default()),
        ..Default::default()
    });
    report.report_definition.areas.push(Area {
        sections: vec![section],
        ..Default::default()
    });

    let doc = reparse(&report);
    let boxn = find(report_node(&doc), "box").expect("a box node");
    let border = find(boxn, "border").expect("a border node");
    assert_eq!(
        border.get("background"),
        Some(&KdlValue::Integer(0x8010_2030)),
        "non-opaque color packs to 0xAARRGGBB"
    );
}

#[test]
fn picture_bytes_go_to_a_sidecar_asset_not_the_kdl() {
    // A minimal 2x2 BMP file header + payload (magic "BM"); the exact bytes are opaque to the test.
    let mut bmp = vec![0x42, 0x4d];
    bmp.extend(std::iter::repeat_n(0u8, 60));

    let mut report = empty_report();
    let mut section = Section::default();
    section.objects.push(ReportObject {
        name: "Logo".into(),
        kind: ReportObjectKind::Picture(PictureObject {
            picture_type: PictureType::Bitmap,
            data: bmp.clone(),
            ole_ordinal: Some(1),
            ..Default::default()
        }),
        ..Default::default()
    });
    report.report_definition.areas.push(Area {
        sections: vec![section],
        ..Default::default()
    });

    let text = rpt_kdl::to_kdl_string(&report);
    // The KDL references the sidecar file; it does not contain the binary.
    assert!(
        text.contains("source=embed-1.bmp"),
        "picture references a sidecar:\n{text}"
    );

    let doc = KdlDocument::parse(&text).expect("valid KDL");
    let pic = find(report_node(&doc), "picture").expect("a picture node");
    assert_eq!(
        pic.get("source"),
        Some(&KdlValue::String("embed-1.bmp".into()))
    );

    // The bytes are recoverable as an asset under the same reference.
    let assets = rpt_kdl::assets(&report);
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].path, "embed-1.bmp");
    assert_eq!(assets[0].bytes, bmp);
}

#[test]
fn subreport_picture_assets_are_scope_qualified() {
    let mut bmp = vec![0x42, 0x4d];
    bmp.extend(std::iter::repeat_n(0u8, 20));

    let mut report = empty_report();
    let mut sub = empty_report();
    let mut section = Section::default();
    section.objects.push(ReportObject {
        name: "Pic".into(),
        kind: ReportObjectKind::Picture(PictureObject {
            data: bmp.clone(),
            ole_ordinal: Some(1),
            ..Default::default()
        }),
        ..Default::default()
    });
    sub.report_definition.areas.push(Area {
        sections: vec![section],
        ..Default::default()
    });
    report.subreports.push(Subreport {
        name: "Sub1".into(),
        report: Box::new(sub),
        links: vec![],
    });

    let assets = rpt_kdl::assets(&report);
    assert_eq!(assets.len(), 1);
    // The subreport's asset is namespaced so it never collides with a main-report embed-1.
    assert_eq!(assets[0].path, "sub-1/embed-1.bmp");
}

#[test]
fn document_and_string_agree() {
    let report = empty_report();
    let doc = rpt_kdl::to_document(&report);
    assert_eq!(doc.to_string(), rpt_kdl::to_kdl_string(&report));
}
