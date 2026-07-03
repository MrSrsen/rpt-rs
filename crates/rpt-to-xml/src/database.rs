//! The `<Database>` and parameter/formula `<DataDefinition>` sections.
//!
//! Turns the decoded [`rpt`] model (plus the derived analytics from [`rpt_engine`]) into XML
//! elements. Derived values — field `UseCount`, parameter usage flags — are computed by the
//! engine layer and passed in, never recomputed here.

use std::collections::HashMap;
use std::fmt::Write as _;

use rpt::model::{FieldDef, FieldKind, FieldKindData, Report};

use crate::util::{b, escape, escape_text, field_type_name, value_type_name};

/// Emit the `<Database>` section: table links, then each table with its connection info, SQL
/// command, and field schema (the data from the QESession stream). Field `UseCount` is the
/// engine-derived aggregation, passed in by the caller.
pub(crate) fn write_database(o: &mut String, report: &Report, use_counts: &HashMap<String, i32>) {
    let db = &report.database;
    o.push_str("  <Database>\n");

    // TableLinks.
    if db.links.is_empty() {
        o.push_str("    <TableLinks />\n");
    } else {
        o.push_str("    <TableLinks>\n");
        for link in &db.links {
            // The linked tables are implicit in each field's `{table.field}` FormulaName, so the
            // TableLink element carries only the join type.
            let _ = writeln!(o, "      <TableLink JoinType=\"{:?}\">", link.join_type);
            o.push_str("        <SourceFields>\n");
            for f in &link.source_fields {
                write_link_field(o, db, &link.source_table_alias, f);
            }
            o.push_str("        </SourceFields>\n        <DestinationFields>\n");
            for f in &link.target_fields {
                write_link_field(o, db, &link.target_table_alias, f);
            }
            o.push_str("        </DestinationFields>\n      </TableLink>\n");
        }
        o.push_str("    </TableLinks>\n");
    }

    // Tables.
    o.push_str("    <Tables>\n");
    for table in &db.tables {
        let _ = writeln!(
            o,
            "      <Table Alias=\"{}\" ClassName=\"{}\" Name=\"{}\">",
            escape(&table.alias),
            escape(table.class_name.as_deref().unwrap_or("")),
            escape(&table.name)
        );
        // ConnectionInfo — the QE_* / Database_DLL attributes (Password intentionally empty).
        o.push_str("        <ConnectionInfo");
        for (k, v) in &table.connection.attributes {
            let _ = write!(o, " {}=\"{}\"", k, escape(v));
        }
        // UserName + Password are the two trailing top-level properties (UserName empty when the
        // connection stores no user; Password intentionally empty).
        let user = table.connection.user_name.as_deref().unwrap_or("");
        let _ = write!(o, " UserName=\"{}\"", escape(user));
        o.push_str(" Password=\"\" />\n");
        if let Some(cmd) = &table.command_text {
            let _ = writeln!(o, "        <Command>{}</Command>", escape_text(cmd));
        }
        o.push_str("        <Fields>\n");
        for f in &table.data_fields {
            let long = f.long_name.as_deref().unwrap_or(&f.name);
            let short = f.short_name.as_deref().unwrap_or(&f.name);
            let use_count = use_counts.get(&format!("{{{long}}}")).copied().unwrap_or(0);
            // A table `<Field>` uses the qualified RAS form `crFieldKindDBField` for `Kind` (not
            // the SDK `FieldKind` name "DatabaseField" used elsewhere), so this one `Kind=` is a
            // literal, not driven by `FieldKind` — see [`write_link_field`].
            let _ = writeln!(
                o,
                "          <Field Description=\"{}\" FormulaForm=\"{{{}}}\" HeadingText=\"\" IsRecurring=\"True\" Kind=\"crFieldKindDBField\" Length=\"{}\" LongName=\"{}\" Name=\"{}\" ShortName=\"{}\" Type=\"{}\" UseCount=\"{}\" />",
                escape(f.description.as_deref().unwrap_or("")),
                escape(long),
                f.length,
                escape(long),
                escape(&f.name),
                escape(short),
                field_type_name(f.value_type),
                use_count,
            );
        }
        o.push_str("        </Fields>\n      </Table>\n");
    }
    o.push_str("    </Tables>\n  </Database>\n");
}

/// Emit a `<Field>` inside a table link's Source/DestinationFields, resolving the field's
/// value-type and byte length from its table via the `{table.field}` formula name.
pub(crate) fn write_link_field(
    o: &mut String,
    db: &rpt::model::Database,
    table_alias: &str,
    field: &str,
) {
    let resolved = db
        .tables
        .iter()
        .find(|t| t.alias == table_alias)
        .and_then(|t| t.data_fields.iter().find(|f| f.name == field));
    let (vt, len) = match resolved {
        Some(f) => (value_type_name(f.value_type), f.length),
        None => ("UnknownField".to_string(), 0),
    };
    let _ = writeln!(
        o,
        "          <Field FormulaName=\"{{{}.{}}}\" Kind=\"{}\" Name=\"{}\" NumberOfBytes=\"{}\" ValueType=\"{}\" />",
        escape(table_alias),
        escape(field),
        FieldKind::DatabaseField.name(),
        escape(field),
        len,
        vt,
    );
}

pub(crate) fn formula_fields(r: &Report) -> impl Iterator<Item = &FieldDef> {
    r.data_definition
        .field_definitions
        .iter()
        .filter(|f| matches!(f.kind, FieldKindData::Formula(_)))
}

pub(crate) fn parameter_fields(r: &Report) -> impl Iterator<Item = &FieldDef> {
    r.data_definition
        .field_definitions
        .iter()
        .filter(|f| matches!(f.kind, FieldKindData::Parameter(_)))
}

pub(crate) fn running_total_fields(r: &Report) -> impl Iterator<Item = &FieldDef> {
    r.data_definition
        .field_definitions
        .iter()
        .filter(|f| matches!(f.kind, FieldKindData::RunningTotal(_)))
}

/// Emit a `<RunningTotalFieldDefinition>` from the `0x80` (reset) + `0x7e` (operation) record
/// pair. The `Group` attribute is omitted (null here) and `EvaluationConditionType` is
/// `NoCondition`. `ValueType` uses the enum name, like the other field definitions.
pub(crate) fn write_running_total(
    o: &mut String,
    f: &FieldDef,
    rt: &rpt::model::RunningTotalField,
) {
    let summarized = if rt.summarized_field.is_empty() {
        String::new()
    } else {
        format!("{{{}}}", rt.summarized_field)
    };
    let _ = writeln!(
        o,
        "      <RunningTotalFieldDefinition EvaluationConditionType=\"{:?}\" FormulaName=\"{{#{}}}\" Kind=\"{}\" Name=\"{}\" NumberOfBytes=\"{}\" Operation=\"{:?}\" OperationParameter=\"{}\" ResetConditionType=\"{:?}\" SummarizedField=\"{}\" ValueType=\"{}\" />",
        rt.evaluation,
        escape(&f.name),
        f.kind.field_kind().name(),
        escape(&f.name),
        f.length,
        rt.operation,
        rt.operation_parameter,
        rt.reset,
        escape(&summarized),
        value_type_name(f.value_type),
    );
}

/// Emit a `<ParameterFieldDefinition>` — the parameter decoded from the PromptManager stream.
/// `EnableAllowMultipleValue` (flag at `ff_block_end+6`), `AllowCustomCurrentValues` and
/// `EnableAllowEditingDefaultValue` (both driven by the dynamic-param flag at `ff_block_end+4`)
/// come from the `0x007a` record; the remaining flag attributes carry the standard defaults for a
/// report parameter (those booleans are not yet decoded). `in_use` / `data_fetching` are the
/// engine-derived `ParameterFieldUsage` flags, passed in.
pub(crate) fn write_parameter(
    o: &mut String,
    name: &str,
    p: &rpt::model::ParameterField,
    in_use: bool,
    data_fetching: bool,
    report_name: &str,
) {
    use rpt::model::ParameterValueKind as Vk;
    // ValueType as the engine's CrFieldValueType name (e.g. StringField / NumberField).
    let value_type = match p.value_kind {
        Vk::NumberParameter => "NumberField",
        Vk::CurrencyParameter => "CurrencyField",
        Vk::BooleanParameter => "BooleanField",
        Vk::DateParameter => "DateField",
        Vk::TimeParameter => "TimeField",
        Vk::DateTimeParameter => "DateTimeField",
        _ => "StringField",
    };
    // NumberOfBytes is the engine's intrinsic byte size for the value kind (RAS `IField.Length`):
    // a string parameter is the 65534 unbounded sentinel; fixed types use their stored width.
    let number_of_bytes = match p.value_kind {
        Vk::NumberParameter | Vk::CurrencyParameter | Vk::DateTimeParameter => 8,
        Vk::DateParameter | Vk::TimeParameter => 4,
        Vk::BooleanParameter => 2,
        _ => 65534,
    };
    // ParameterFieldUsage flags: InUse/NotInUse by whether the parameter is referenced anywhere;
    // ShowOnPanel+EditableOnPanel when shown on the viewer panel; DataFetching when it feeds the query.
    let mut usage = vec![if in_use { "InUse" } else { "NotInUse" }];
    if p.show_on_panel {
        usage.push("ShowOnPanel");
        usage.push("EditableOnPanel");
    }
    if data_fetching {
        usage.push("DataFetching");
    }
    let usage = usage.join(", ");
    let _ = writeln!(
        o,
        "      <ParameterFieldDefinition AllowCustomCurrentValues=\"{accv}\" EditMask=\"\" EnableAllowEditingDefaultValue=\"{eaedv}\" EnableAllowMultipleValue=\"{eamv}\" EnableNullValue=\"False\" FormulaName=\"{{?{n}}}\" HasCurrentValue=\"{hcv}\" IsOptionalPrompt=\"{opt}\" Kind=\"{kind}\" Name=\"{n}\" NumberOfBytes=\"{nb}\" ParameterFieldName=\"{n}\" ParameterFieldUsage=\"{usage}\" ParameterType=\"{pt:?}\" ParameterValueKind=\"{vk:?}\" PromptText=\"{prompt}\" ReportName=\"{rn}\" ValueType=\"{vt}\">",
        kind = FieldKind::ParameterField.name(),
        n = escape(name),
        rn = escape(report_name),
        nb = number_of_bytes,
        accv = b(p.allow_custom_values),
        hcv = b(p.has_current_value),
        eaedv = b(p.allow_editing_default_value),
        eamv = b(p.allow_multiple_values),
        opt = b(p.optional_prompt),
        pt = p.parameter_type,
        vk = p.value_kind,
        prompt = escape(p.prompt_text.as_deref().unwrap_or("")),
        vt = value_type,
    );
    write_value_list(
        o,
        "ParameterDefaultValues",
        "ParameterDefaultValue",
        &p.default_values,
        true,
    );
    write_value_list(
        o,
        "ParameterInitialValues",
        "ParameterInitialValue",
        &p.initial_values,
        false,
    );
    write_value_list(
        o,
        "ParameterCurrentValues",
        "ParameterCurrentValue",
        &p.current_values,
        true,
    );
    o.push_str("      </ParameterFieldDefinition>\n");
}

/// Emit one `<Parameter{Default,Initial,Current}Values>` collection. Each entry is a self-closing
/// `<Parameter…Value>` with `Value` (always) and `Description` (default/current lists only;
/// `<ParameterInitialValue>` omits it). An empty collection is written self-closing.
fn write_value_list(
    o: &mut String,
    container: &str,
    item: &str,
    values: &[rpt::model::ParameterValue],
    with_description: bool,
) {
    if values.is_empty() {
        let _ = writeln!(o, "        <{container} />");
        return;
    }
    let _ = writeln!(o, "        <{container}>");
    for v in values {
        if with_description {
            let _ = writeln!(
                o,
                "          <{item} Description=\"{}\" Value=\"{}\" />",
                escape(v.description.as_deref().unwrap_or("")),
                escape(&v.value),
            );
        } else {
            let _ = writeln!(o, "          <{item} Value=\"{}\" />", escape(&v.value));
        }
    }
    let _ = writeln!(o, "        </{container}>");
}
