//! The document skeleton: the `report` node and its top-level children (info, page, options,
//! database, data-definition, layout, subreports), mirroring [`rpt_model::Report`]'s two halves.
//!
//! Every model struct on the report/definition/layout path is destructured **without `..`** and every
//! enum matched without a `_` wildcard, so a new field or variant in `rpt-model` fails to compile here
//! until it is either emitted or explicitly bound and skipped with a stated reason. This is what keeps
//! the export a lossless view of the model going forward.

use rpt_model::{
    Area, AreaFormat, CommandParameter, ConnectionInfo, CustomFunction, DataDefinition, Database,
    DbFieldDef, DynamicLovBinding, Embed, FieldDef, FieldKindData, FieldValueType, FormulaField,
    FormulaSyntax, FormulaVariable, Group, GroupAreaFormat, HierarchicalGroupValue, MultiColumn,
    PageMargins, ParameterField, ParameterRange, ParameterType, ParameterValue, PrintOptions,
    RangeBoundType, Report, ReportDefinition, ReportOptions, RunningTotalField, Section,
    SectionAreaFormatBase, SectionFormat, Sort, SortKind, SpecialField, SqlExpressionField,
    Subreport, SubreportLink, SummaryField, SummaryInfo, Table, TableLink, TopBottomNSort,
};

use crate::build::{color, int, twips, Node};
use crate::enums;
use crate::objects::object_node;

/// The root `report` node for a whole report (recursive: subreports embed their own `report` node).
pub(crate) fn report_node(report: &Report) -> Node {
    let mut node = Node::new("report").prop("version", int(report.version));
    // Only when the file states one — a report that records no authoring version says nothing here
    // rather than "0.0".
    if report.authoring_version != Default::default() {
        node = node.prop("authored-by", report.authoring_version.to_string());
    }
    node.flag("saved-data", report.has_saved_data)
        .children(report_children(report))
}

/// The ordered top-level children of a `report` node — the same set for the main report and every
/// subreport, so [`report_node`] recurses uniformly.
fn report_children(report: &Report) -> Vec<Node> {
    let Report {
        // `version`/`has_saved_data` are emitted on the report node itself.
        version: _,
        // The authoring version is emitted on the report node too.
        authoring_version: _,
        has_saved_data: _,
        summary_info,
        print_options,
        report_options,
        report_definition: ReportDefinition { kind, style, areas },
        data_definition,
        database,
        subreports,
        embeds,
        // A cached row payload (data, not report definition) — inspected via `rpt saved`.
        saved_data: _,
        // Save-time environment provenance and designer/IDE state — not report semantics.
        save_metadata: _,
        reimport: _,
        designer_state: _,
    } = report;

    let mut out = Vec::new();
    if let Some(n) = info_node(summary_info) {
        out.push(n);
    }
    out.push(page_node(print_options));
    if let Some(n) = report_options_node(report_options) {
        out.push(n);
    }
    if let Some(n) = database_node(database) {
        out.push(n);
    }
    out.extend(data_definition_nodes(data_definition));
    out.push(layout_node(*kind, *style, areas));
    out.extend(subreports.iter().map(subreport_node));
    if let Some(n) = embeds_node(embeds) {
        out.push(n);
    }
    out
}

/// OLE embeds carried by the report, listed by reference only (name/size/digest) — the model retains
/// no bytes for these, so there is nothing to write as a sidecar.
fn embeds_node(embeds: &[Embed]) -> Option<Node> {
    if embeds.is_empty() {
        return None;
    }
    Some(Node::new("embeds").children(embeds.iter().map(|e| {
        let Embed {
            name,
            size,
            md5_hash,
        } = e;
        Node::new("embed")
            .arg(name.as_str())
            .prop("size", int(*size as i64))
            .str_if("md5", md5_hash)
    })))
}

fn info_node(info: &SummaryInfo) -> Option<Node> {
    if info == &SummaryInfo::default() {
        return None;
    }
    let SummaryInfo {
        title,
        subject,
        author,
        comments,
        keywords,
        // Authoring provenance (OLE property set); not projected to KDL. The timestamps join the
        // same group: they say when the file was written, not what the report is.
        revision_number: _,
        last_saved_by: _,
        created: _,
        last_saved: _,
        last_printed: _,
        save_with_preview,
    } = info;
    Some(
        Node::new("info")
            .str_if("title", title)
            .str_if("subject", subject)
            .str_if("author", author)
            .str_if("comments", comments)
            .str_if("keywords", keywords)
            .flag("save-with-preview", *save_with_preview),
    )
}

fn page_node(po: &PrintOptions) -> Node {
    let PrintOptions {
        content_width,
        content_height,
        paper_orientation,
        paper_size,
        paper_source,
        printer_duplex,
        printer_name,
        // Saved DEVMODE device name; not projected to KDL.
        saved_printer_name: _,
        driver_name,
        port_name,
        margins,
        multi_column,
    } = po;
    let mut n = Node::new("page").prop_if(
        *paper_orientation != rpt_model::PaperOrientation::default(),
        "orientation",
        enums::paper_orientation(*paper_orientation),
    );
    if let Some(paper) = enums::paper_size(*paper_size) {
        n = n.prop("paper", paper);
    }
    let PageMargins {
        left,
        right,
        top,
        bottom,
    } = margins;
    n = n
        .prop_if(
            *paper_source != rpt_model::PaperSource::default(),
            "source",
            enums::paper_source(*paper_source),
        )
        .prop_if(
            *printer_duplex != rpt_model::PrinterDuplex::default(),
            "duplex",
            enums::printer_duplex(*printer_duplex),
        )
        .prop_if(content_width.0 != 0, "content-width", twips(*content_width))
        .prop_if(
            content_height.0 != 0,
            "content-height",
            twips(*content_height),
        )
        .prop_if(left.0 != 0, "margin-left", twips(*left))
        .prop_if(right.0 != 0, "margin-right", twips(*right))
        .prop_if(top.0 != 0, "margin-top", twips(*top))
        .prop_if(bottom.0 != 0, "margin-bottom", twips(*bottom))
        .str_if("printer", printer_name)
        .opt_str("driver", driver_name.as_deref())
        .opt_str("port", port_name.as_deref());
    if let Some(MultiColumn {
        columns,
        column_width,
        gap_h,
        gap_v,
        across_then_down,
    }) = multi_column
    {
        n = n.child(
            Node::new("columns")
                .prop("count", int(*columns))
                .prop("width", twips(*column_width))
                .prop_if(gap_h.0 != 0, "gap-h", twips(*gap_h))
                .prop_if(gap_v.0 != 0, "gap-v", twips(*gap_v))
                .flag("across-then-down", *across_then_down),
        );
    }
    n
}

/// The `report-options` node (saved-data / query behaviour), or `None` when entirely default.
fn report_options_node(ro: &ReportOptions) -> Option<Node> {
    if ro == &ReportOptions::default() {
        return None;
    }
    let ReportOptions {
        save_data_with_report,
        save_summaries_with_report,
        save_preview_picture,
        use_dummy_data,
        enable_verify_on_every_print: _,
        convert_null_field_to_default,
        convert_other_nulls_to_default,
        initial_data_context,
        initial_report_part_name,
    } = ro;
    Some(
        Node::new("report-options")
            .flag("save-data", *save_data_with_report)
            .flag("save-summaries", *save_summaries_with_report)
            .flag("save-preview", *save_preview_picture)
            .flag("use-dummy-data", *use_dummy_data)
            .flag(
                "convert-null-field-to-default",
                *convert_null_field_to_default,
            )
            .flag(
                "convert-other-nulls-to-default",
                *convert_other_nulls_to_default,
            )
            .opt_str("initial-data-context", initial_data_context.as_deref())
            .opt_str("initial-report-part", initial_report_part_name.as_deref()),
    )
}

fn database_node(db: &Database) -> Option<Node> {
    let Database { tables, links } = db;
    if tables.is_empty() && links.is_empty() {
        return None;
    }
    Some(
        Node::new("database")
            .children(tables.iter().map(table_node))
            .children(links.iter().map(link_node)),
    )
}

fn table_node(t: &Table) -> Node {
    let Table {
        name,
        alias,
        class_name,
        qualified_name,
        connection,
        data_fields,
        command_text,
        parameters,
    } = t;
    let mut n = Node::new("table")
        .arg(name.as_str())
        .prop_if(alias != name, "alias", alias.as_str())
        .opt_str("class", class_name.as_deref())
        .opt_str("qualified", qualified_name.as_deref())
        .child(connection_node(connection))
        .children(data_fields.iter().map(table_column_node));
    if let Some(cmd) = command_text {
        n = n.child(Node::new("command").arg(cmd.as_str()));
    }
    n.children(parameters.iter().map(command_parameter_node))
}

fn command_parameter_node(p: &CommandParameter) -> Node {
    let CommandParameter { name, value_type } = p;
    Node::new("parameter").arg(name.as_str()).prop_if(
        *value_type != FieldValueType::default(),
        "value-type",
        enums::field_value_type(*value_type),
    )
}

fn table_column_node(f: &DbFieldDef) -> Node {
    let DbFieldDef {
        name,
        value_type,
        length,
        short_name,
        long_name,
        description,
    } = f;
    Node::new("column")
        .arg(name.as_str())
        .prop_if(
            *value_type != FieldValueType::default(),
            "value-type",
            enums::field_value_type(*value_type),
        )
        .prop_if(*length != 0, "length", int(*length))
        .opt_str("short-name", short_name.as_deref())
        .opt_str("long-name", long_name.as_deref())
        .opt_str("description", description.as_deref())
}

fn connection_node(c: &ConnectionInfo) -> Node {
    let ConnectionInfo {
        user_name,
        kind,
        attributes,
    } = c;
    Node::new("connection")
        .prop_if(
            *kind != rpt_model::ConnectionInfoKind::default(),
            "kind",
            enums::connection_kind(*kind),
        )
        .opt_str("user", user_name.as_deref())
        .children(
            attributes
                .iter()
                .map(|(k, v)| Node::new("attr").arg(k.as_str()).arg(v.as_str())),
        )
}

fn link_node(l: &TableLink) -> Node {
    let TableLink {
        join_kind,
        operator,
        source_table_alias,
        target_table_alias,
        source_fields,
        target_fields,
    } = l;
    Node::new("link")
        .arg(source_table_alias.as_str())
        .arg(target_table_alias.as_str())
        .prop("join", enums::table_join_kind(*join_kind))
        .prop("op", enums::table_link_operator(*operator))
        .children(
            source_fields
                .iter()
                .zip(target_fields.iter())
                .map(|(s, t)| Node::new("on").arg(s.as_str()).arg(t.as_str())),
        )
}

/// Every emitted child of the data-definition half: the field pool, selection formulas, groups,
/// record sort, persisted formula variables, and running-total condition formulas.
fn data_definition_nodes(dd: &DataDefinition) -> Vec<Node> {
    let DataDefinition {
        record_selection,
        group_selection,
        saved_data_filter,
        groups,
        record_sorts,
        field_definitions,
        // Aggregate of the conditional formulas already emitted at their section/object/format
        // attachment sites — retained in the model only to feed the derived reference-count analytics.
        condition_formula_bodies: _,
        running_total_condition_formulas,
        // Derived reference-count inputs, redundant with the summary field definitions above.
        summary_binding_fields: _,
        formula_variables,
        // A redundant cross-check tally of `field_definitions`.
        field_manager_census: _,
        custom_functions,
    } = dd;

    let mut out = Vec::new();
    if let Some(n) = fields_node(field_definitions) {
        out.push(n);
    }
    out.extend(selection_nodes(
        record_selection,
        group_selection,
        saved_data_filter,
    ));
    out.extend(group_nodes(groups));
    out.extend(record_sort_nodes(record_sorts));
    out.extend(formula_variables.iter().map(formula_variable_node));
    out.extend(custom_functions.iter().map(custom_function_node));
    out.extend(
        running_total_condition_formulas
            .iter()
            .map(|body| Node::new("running-total-condition").arg(body.as_str())),
    );
    out
}

/// One custom function: `custom-function <name> syntax=… <body>`.
fn custom_function_node(cf: &CustomFunction) -> Node {
    Node::new("custom-function")
        .arg(cf.name.as_str())
        .prop_if(
            cf.syntax != FormulaSyntax::default(),
            "syntax",
            enums::formula_syntax(cf.syntax),
        )
        .arg(cf.text.as_str())
}

fn fields_node(field_definitions: &[FieldDef]) -> Option<Node> {
    let children: Vec<Node> = field_definitions
        .iter()
        .filter_map(field_pool_node)
        .collect();
    if children.is_empty() {
        return None;
    }
    Some(Node::new("fields").children(children))
}

/// One field-pool entry. Database and group-name fields are skipped — they are derivable from the
/// tables and the group list, respectively.
fn field_pool_node(f: &FieldDef) -> Option<Node> {
    let FieldDef {
        name,
        value_type,
        length,
        formula_form,
        heading_text,
        description,
        long_name,
        short_name,
        kind,
    } = f;
    let base = |node_name: &str| {
        Node::new(node_name)
            .arg(name.as_str())
            .prop_if(
                *value_type != FieldValueType::default(),
                "value-type",
                enums::field_value_type(*value_type),
            )
            .prop_if(*length != 0, "length", int(*length))
            .opt_str("formula-form", formula_form.as_deref())
            .opt_str("heading", heading_text.as_deref())
            .opt_str("description", description.as_deref())
            .opt_str("long-name", long_name.as_deref())
            .opt_str("short-name", short_name.as_deref())
    };
    match kind {
        FieldKindData::Formula(ff) => Some(formula_node(base("formula"), ff)),
        FieldKindData::Parameter(pf) => Some(parameter_node(base("parameter"), pf)),
        FieldKindData::Summary(sf) => Some(summary_node(base("summary"), sf)),
        FieldKindData::RunningTotal(rt) => Some(running_total_node(base("running-total"), rt)),
        FieldKindData::SqlExpression(sq) => Some(sql_expression_node(base("sql-expression"), sq)),
        FieldKindData::Special(sp) => Some(special_node(base("special"), sp)),
        // Database and group-name pool entries mirror the tables / group list; unknown kinds carry no
        // decoded payload.
        FieldKindData::Database(_) | FieldKindData::GroupName(_) | FieldKindData::Unknown => None,
    }
}

fn formula_node(n: Node, ff: &FormulaField) -> Node {
    let FormulaField {
        text,
        options,
        number_of_bytes,
        syntax,
        // Decoded formula editor setting; not projected to KDL (keeps the export surface unchanged).
        null_treatment: _,
    } = ff;
    n.prop_if(
        *syntax != FormulaSyntax::default(),
        "syntax",
        enums::formula_syntax(*syntax),
    )
    .prop_if(*options != 0, "options", int(*options))
    .prop_if(*number_of_bytes != 0, "bytes", int(*number_of_bytes))
    .arg(text.0.as_str())
}

fn parameter_node(n: Node, pf: &ParameterField) -> Node {
    let ParameterField {
        parameter_type,
        value_kind,
        prompt_text,
        report_name,
        edit_mask,
        allow_custom_values,
        allow_editing_default_value,
        allow_multiple_values,
        allow_null_value,
        has_current_value,
        optional_prompt,
        show_on_panel,
        editable_on_panel,
        default_values,
        current_values,
        initial_values,
        default_value_display_type,
        default_value_sort_order,
        prompt_group,
        part_of_group,
        mutually_exclusive_group,
        dynamic_lov,
    } = pf;
    let mut n = n
        .prop("kind", enums::parameter_value_kind(*value_kind))
        .prop_if(
            *parameter_type != ParameterType::default(),
            "type",
            enums::parameter_type(*parameter_type),
        )
        .opt_str("prompt", prompt_text.as_deref())
        .opt_str("report", report_name.as_deref())
        .opt_str("edit-mask", edit_mask.as_deref())
        .flag("allow-multiple", *allow_multiple_values)
        .flag("allow-null", *allow_null_value)
        .flag("allow-custom", *allow_custom_values)
        .flag("allow-editing-default", *allow_editing_default_value)
        .flag("optional", *optional_prompt)
        .flag("has-current-value", *has_current_value)
        .flag("show-on-panel", *show_on_panel)
        .flag("editable-on-panel", *editable_on_panel)
        .prop_if(
            *default_value_display_type != rpt_model::ParameterDisplayType::default(),
            "display",
            enums::parameter_display_type(*default_value_display_type),
        )
        .prop_if(
            *default_value_sort_order != rpt_model::ParameterSortOrder::default(),
            "sort",
            enums::parameter_sort_order(*default_value_sort_order),
        )
        .opt_str("prompt-group", prompt_group.as_deref())
        .flag("part-of-group", *part_of_group)
        .flag("mutually-exclusive", *mutually_exclusive_group)
        .children(
            default_values
                .iter()
                .map(|v| param_value_node("default", v)),
        )
        .children(
            current_values
                .iter()
                .map(|v| param_value_node("current", v)),
        )
        .children(
            initial_values
                .iter()
                .map(|v| param_value_node("initial", v)),
        );
    if let Some(lov) = dynamic_lov {
        n = n.child(dynamic_lov_node(lov));
    }
    n
}

fn param_value_node(node_name: &str, v: &ParameterValue) -> Node {
    let ParameterValue {
        value,
        description,
        range,
    } = v;
    let mut d = Node::new(node_name).arg(value.as_str());
    if let Some(desc) = description {
        d = d.prop("description", desc.as_str());
    }
    if let Some(ParameterRange {
        end_value,
        lower_bound,
        upper_bound,
    }) = range
    {
        d = d
            .prop("end", end_value.as_str())
            .prop_if(
                *lower_bound != RangeBoundType::default(),
                "lower-bound",
                enums::range_bound_type(*lower_bound),
            )
            .prop_if(
                *upper_bound != RangeBoundType::default(),
                "upper-bound",
                enums::range_bound_type(*upper_bound),
            );
    }
    d
}

fn dynamic_lov_node(lov: &DynamicLovBinding) -> Node {
    let DynamicLovBinding {
        source_kind,
        source,
        value_field,
        description_field,
    } = lov;
    Node::new("dynamic-lov")
        .prop("source-kind", enums::lov_source_kind(*source_kind))
        .str_if("source", source)
        .str_if("value-field", value_field)
        .str_if("description-field", description_field)
}

fn summary_node(n: Node, sf: &SummaryField) -> Node {
    let SummaryField {
        operation,
        summarized_field,
        secondary_summarized_field,
        operation_parameter,
        group_index,
        is_percentage_summary,
        second_group_for_percentage,
    } = sf;
    n.prop("op", enums::summary_operation(*operation))
        .prop("of", summarized_field.as_str())
        .str_if("second", secondary_summarized_field)
        .prop_if(
            group_index.is_some(),
            "group",
            int(group_index.unwrap_or(0)),
        )
        .prop_if(*operation_parameter != 0, "n", int(*operation_parameter))
        .flag("percentage", *is_percentage_summary)
        .prop_if(
            second_group_for_percentage.is_some(),
            "percentage-of-group",
            int(second_group_for_percentage.unwrap_or(0)),
        )
}

fn running_total_node(n: Node, rt: &RunningTotalField) -> Node {
    let RunningTotalField {
        operation,
        summarized_field,
        secondary_summarized_field,
        operation_parameter,
        evaluation,
        reset,
        on_change_field,
    } = rt;
    n.prop("op", enums::summary_operation(*operation))
        .prop("of", summarized_field.as_str())
        .str_if("second", secondary_summarized_field)
        .prop_if(
            *evaluation != rpt_model::EvaluationConditionType::default(),
            "evaluate",
            enums::evaluation_condition(*evaluation),
        )
        .prop_if(
            *reset != rpt_model::ResetConditionType::default(),
            "reset",
            enums::reset_condition(*reset),
        )
        .str_if("on-change", on_change_field)
        .prop_if(*operation_parameter != 0, "n", int(*operation_parameter))
}

fn sql_expression_node(n: Node, sq: &SqlExpressionField) -> Node {
    let SqlExpressionField { text } = sq;
    n.arg(text.as_str())
}

fn special_node(n: Node, sp: &SpecialField) -> Node {
    let SpecialField { special_type } = sp;
    n.prop("type", enums::special_field_type(*special_type))
}

fn formula_variable_node(v: &FormulaVariable) -> Node {
    let FormulaVariable {
        name,
        value_type,
        scope,
    } = v;
    Node::new("variable")
        .arg(name.as_str())
        .prop("scope", enums::formula_variable_scope(*scope))
        .prop_if(
            *value_type != FieldValueType::default(),
            "value-type",
            enums::field_value_type(*value_type),
        )
}

fn selection_nodes(
    record_selection: &Option<rpt_model::Formula>,
    group_selection: &Option<rpt_model::Formula>,
    saved_data_filter: &Option<rpt_model::Formula>,
) -> Vec<Node> {
    // A stored-but-empty selection formula carries no filter; skip it rather than emit `""`.
    let non_empty =
        |f: &Option<rpt_model::Formula>| f.as_ref().filter(|f| !f.0.is_empty()).cloned();
    let mut out = Vec::new();
    if let Some(f) = non_empty(record_selection) {
        out.push(Node::new("select-records").arg(f.0));
    }
    if let Some(f) = non_empty(group_selection) {
        out.push(Node::new("select-groups").arg(f.0));
    }
    if let Some(f) = non_empty(saved_data_filter) {
        out.push(Node::new("select-saved").arg(f.0));
    }
    out
}

fn group_nodes(groups: &[Group]) -> Vec<Node> {
    groups
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let Group {
                condition_field,
                sort,
                // No decoded members.
                options: _,
                date_condition,
                area_format,
                hierarchical,
                // Hierarchical-grouping options have no KDL surface.
                hierarchical_options: _,
            } = g;
            let Sort {
                field,
                direction,
                // A group's sort kind is `GroupSortField` by context.
                kind: _,
                topn,
            } = sort;
            let GroupAreaFormat {
                keep_group_together,
                repeat_group_header,
                visible_groups_per_page,
            } = area_format;
            let mut n = Node::new("group")
                .arg(int((i + 1) as i64))
                .prop("on", condition_field.as_str())
                .prop_if(
                    *direction != rpt_model::SortDirection::default(),
                    "dir",
                    enums::sort_direction(*direction),
                )
                // The sort field only differs from the group's break field for a summary-based sort.
                .prop_if(field != condition_field, "sort-field", field.as_str())
                .opt_str("date", date_condition.as_ref().map(|c| c.token()))
                .flag("keep-together", *keep_group_together)
                .flag("repeat-header", *repeat_group_header)
                .prop_if(
                    *visible_groups_per_page != 0,
                    "visible-per-page",
                    int(*visible_groups_per_page),
                );
            if let Some(TopBottomNSort {
                number_of_groups,
                discard_others,
                not_in_topn_name,
                with_ties,
            }) = topn
            {
                n = n.child(
                    Node::new("top-n")
                        .prop("number", int(*number_of_groups))
                        .flag("discard-others", *discard_others)
                        .flag("with-ties", *with_ties)
                        .str_if("others", not_in_topn_name),
                );
            }
            n = n.children(hierarchical.iter().map(|h| {
                let HierarchicalGroupValue {
                    value_name,
                    condition,
                } = h;
                Node::new("specified-value")
                    .arg(value_name.as_str())
                    .arg(condition.as_str())
            }));
            n
        })
        .collect()
}

/// The detail-record sort order (SDK `SortFields`, record-level entries). Group-sort entries are
/// surfaced per-group by [`group_nodes`], so only `RecordSortField` entries are emitted here.
fn record_sort_nodes(sorts: &[Sort]) -> Vec<Node> {
    sorts
        .iter()
        .filter(|s| s.kind == SortKind::RecordSortField)
        .map(|s| {
            let Sort {
                field,
                direction,
                // Filtered to record sorts above; Top N options are a group-sort concern.
                kind: _,
                topn: _,
            } = s;
            Node::new("record-sort").arg(field.as_str()).prop_if(
                *direction != rpt_model::SortDirection::default(),
                "dir",
                enums::sort_direction(*direction),
            )
        })
        .collect()
}

fn layout_node(kind: rpt_model::ReportKind, style: rpt_model::ReportStyle, areas: &[Area]) -> Node {
    Node::new("layout")
        .prop_if(
            kind != rpt_model::ReportKind::default(),
            "kind",
            enums::report_kind(kind),
        )
        .prop_if(
            style != rpt_model::ReportStyle::default(),
            "style",
            enums::report_style(style),
        )
        .children(areas.iter().map(area_node))
}

/// Append the [`SectionAreaFormatBase`] flags shared by areas and sections.
fn base_format_flags(n: Node, base: &SectionAreaFormatBase) -> Node {
    let SectionAreaFormatBase {
        keep_together,
        new_page_before,
        new_page_after,
        print_at_bottom_of_page,
        reset_page_number_after,
        suppress,
    } = base;
    n.flag("suppress", *suppress)
        .flag("keep-together", *keep_together)
        .flag("new-page-before", *new_page_before)
        .flag("new-page-after", *new_page_after)
        .flag("print-at-bottom", *print_at_bottom_of_page)
        .flag("reset-page-number-after", *reset_page_number_after)
}

fn area_node(a: &Area) -> Node {
    let Area {
        kind,
        name,
        // The group nesting level is used by the decoder to order group bands; it carries no KDL
        // surface of its own (the `group` node under the section already conveys grouping).
        group_level: _,
        format,
        sections,
    } = a;
    let AreaFormat {
        base,
        hide_for_drill_down,
        visible_records_per_page,
        clamp_page_footer,
        // Group header/footer formatting is emitted on the `group` node (from `Group.area_format`).
        group: _,
    } = format;
    let display_name: kdl::KdlValue = if name.is_empty() {
        enums::area_section_kind(*kind)
    } else {
        kdl::KdlValue::from(name.as_str())
    };
    let n = Node::new("area").arg(display_name).prop_if(
        name.is_empty(),
        "kind",
        enums::area_section_kind(*kind),
    );
    base_format_flags(n, base)
        .flag("hide-for-drill-down", *hide_for_drill_down)
        .prop_if(
            *visible_records_per_page != 0,
            "visible-records-per-page",
            int(*visible_records_per_page),
        )
        .flag("clamp-page-footer", *clamp_page_footer)
        .children(sections.iter().map(section_node))
}

fn section_node(s: &Section) -> Node {
    let Section {
        // The section's band mirrors its owning area's kind.
        kind: _,
        name,
        height,
        width,
        format,
        objects,
        condition_formulas,
    } = s;
    let SectionFormat {
        base,
        suppress_if_blank,
        underlay_section,
        css_class,
        page_orientation,
        background_color,
    } = format;
    let mut n = Node::new("section")
        .arg(name.as_str())
        .prop_if(height.0 != 0, "height", twips(*height))
        .prop_if(width.0 != 0, "width", twips(*width));
    n = base_format_flags(n, base)
        .flag("suppress-if-blank", *suppress_if_blank)
        .flag("underlay", *underlay_section)
        .opt_str("css-class", css_class.as_deref());
    if let Some(o) = page_orientation {
        n = n.prop("orientation", enums::paper_orientation(*o));
    }
    if let Some(c) = background_color {
        n = n.prop("background", color(*c));
    }
    n.children(objects.iter().map(object_node))
        .children(crate::format::condition_formula_nodes(condition_formulas))
}

fn subreport_node(sub: &Subreport) -> Node {
    let Subreport {
        name,
        report,
        links,
    } = sub;
    Node::new("subreport")
        .arg(name.as_str())
        .children(links.iter().map(|l| {
            let SubreportLink {
                main_report_field,
                subreport_field,
                linked_parameter,
            } = l;
            Node::new("link")
                .prop("main", main_report_field.as_str())
                .prop("sub", subreport_field.as_str())
                .opt_str("param", linked_parameter.as_deref())
        }))
        .child(report_node(report))
}
