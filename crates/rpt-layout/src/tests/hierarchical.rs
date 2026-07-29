//! Hierarchical grouping: the parent/child tree walk and the per-level "Group Indent".
//!
//! The engine places every object of a group instance at `x + depth * GroupIndent` — group header,
//! detail and group footer alike, text, images and full-width lines, a pure translation — and closes
//! a group's footer only after that instance's whole subtree.

use super::*;

/// A report grouping on `t.id`, hierarchically sorted by `t.parent`, with one field in the group
/// header, one in the detail band and one in the group footer, each at the same design x.
fn hierarchical_report(indent: i32) -> Report {
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    let mut group = rpt_model::Group::default();
    group.condition_field = "t.id".to_string();
    group.hierarchical_options = Some(rpt_model::HierarchicalGroupOptions {
        enabled: true,
        parent_id_field: "t.parent".to_string(),
        instance_id_field: "t.id".to_string(),
        group_indent: Twips(indent),
    });
    report.data_definition.groups = vec![group];
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::GroupHeader,
            vec![section(
                AreaSectionKind::GroupHeader,
                "GroupHeaderArea1",
                300,
                vec![db_field_object("GhVal", "t.id", 0)],
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
    report
}

/// Lay out `rows` — `(id, parent, name)` — through `report` and return every text op's
/// `(text, left)` in print order.
fn placed_text(report: &Report, rows: &[(&str, &str, &str)]) -> Vec<(String, i32)> {
    let cells: Vec<Vec<&str>> = rows
        .iter()
        .map(|(id, parent, name)| vec![*id, *parent, *name])
        .collect();
    let refs: Vec<&[&str]> = cells.iter().map(|r| r.as_slice()).collect();
    let saved = saved_data(
        &[
            ("t.id", FieldValueType::String),
            ("t.parent", FieldValueType::String),
            ("t.name", FieldValueType::String),
        ],
        &refs,
    );
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    layout(report, &ds, &formulas)
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some((t.text.clone(), t.bounds.left.0)),
            _ => None,
        })
        .collect()
}

/// Every band of a hierarchically grouped instance — header, detail and footer — is shifted right by
/// the instance's depth in the tree times the group's stored indent, and its subtree prints between
/// its own header and footer.
#[test]
fn a_hierarchical_instance_indents_every_band_by_depth() {
    // 1 ─ 2 ─ 3, then a second root.
    let report = hierarchical_report(300);
    let placed = placed_text(
        &report,
        &[
            ("1", "", "one"),
            ("2", "1", "two"),
            ("3", "2", "three"),
            ("9", "", "nine"),
        ],
    );
    // Design x is 100 twips throughout; depth 0/1/2 adds 0/300/600 to all three bands. The whole
    // subtree also prints inside the root's header/footer bracket, so the footers of 3, 2 and 1
    // close in that order before the second root opens.
    assert_eq!(
        placed,
        vec![
            ("1".to_string(), 100),     // root header
            ("one".to_string(), 100),   // root detail
            ("2".to_string(), 400),     // child header
            ("two".to_string(), 400),   // child detail
            ("3".to_string(), 700),     // grandchild header
            ("three".to_string(), 700), // grandchild detail
            ("three".to_string(), 700), // grandchild footer
            ("two".to_string(), 400),   // child footer
            ("one".to_string(), 100),   // root footer
            ("9".to_string(), 100),     // second root
            ("nine".to_string(), 100),  //
            ("nine".to_string(), 100),  //
        ]
    );
}

/// The indent scales with the group's stored `GroupIndent`, and a zero indent (the designer's
/// unset state) leaves the tree flush left while still nesting it.
#[test]
fn a_zero_indent_still_nests_but_does_not_shift() {
    let report = hierarchical_report(0);
    let placed = placed_text(&report, &[("1", "", "one"), ("2", "1", "two")]);
    let headers: Vec<(String, i32)> = placed
        .iter()
        .filter(|(t, _)| matches!(t.as_str(), "1" | "2"))
        .cloned()
        .collect();
    assert_eq!(
        headers,
        vec![("1".to_string(), 100), ("2".to_string(), 100)]
    );
    let names: Vec<&str> = placed.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(names, vec!["1", "one", "2", "two", "two", "one"]);
}

/// A group without hierarchical options is untouched: instances stay flat and flush left.
#[test]
fn a_plain_group_is_not_indented() {
    let mut report = hierarchical_report(300);
    report.data_definition.groups[0].hierarchical_options = None;
    let placed = placed_text(&report, &[("1", "", "one"), ("2", "1", "two")]);
    let headers: Vec<(String, i32)> = placed
        .iter()
        .filter(|(t, _)| matches!(t.as_str(), "1" | "2"))
        .cloned()
        .collect();
    assert_eq!(
        headers,
        vec![("1".to_string(), 100), ("2".to_string(), 100)]
    );
}

/// A parent cycle lays out — every instance printing exactly once — rather than recursing forever.
#[test]
fn a_parent_cycle_lays_out_without_recursing_forever() {
    let report = hierarchical_report(300);
    let placed = placed_text(
        &report,
        &[("1", "3", "one"), ("2", "1", "two"), ("3", "2", "three")],
    );
    let mut ids: Vec<&str> = placed
        .iter()
        .map(|(t, _)| t.as_str())
        .filter(|t| matches!(*t, "1" | "2" | "3"))
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["1", "2", "3"]);
}
