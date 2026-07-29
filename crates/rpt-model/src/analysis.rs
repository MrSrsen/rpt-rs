//! Pure, derived helpers over the semantic model that both the export and render layers need.
//!
//! Nothing here is a stored fact — these functions *infer* a value from the decoded model, so they
//! carry no state on any model struct. They are dependency-free (no formula engine, no decoder), so
//! every consumer of [`crate::Report`] can reach them.

use crate::{
    CrossTabMeasure, FieldDef, FieldKind, FieldKindData, FieldObject, FieldRefKind, FieldValueType,
    Report, SpecialFieldType, SummaryOperation,
};

/// The **measures** a report's cross-tabs aggregate into their data cells — the operation and
/// summarized field of each, in stacking order.
///
/// Attributed rather than read: a cross-tab's measures are the report's own summary definitions, and
/// the file records no link from a summary back to the cross-tab that uses it. So the attribution is
/// report-wide, and this takes no cross-tab: a report with one cross-tab and no unrelated group
/// summary gets exactly its measures, while a report with a second cross-tab or a group summary of
/// its own cannot be told apart and gets the same list for each. That inexactness is why this is
/// derived on demand and stored nowhere.
///
/// A measure summarizes a database field or a formula; a summary over anything else names no field,
/// and its measure carries an empty reference rather than a name that resolves to nothing.
pub fn crosstab_measures(report: &Report) -> Vec<CrossTabMeasure> {
    report
        .data_definition
        .summary_fields()
        .map(|(_, s)| CrossTabMeasure {
            operation: s.operation,
            field: match is_field_ref(&s.summarized_field) {
                true => s.summarized_field.clone(),
                false => String::new(),
            },
        })
        .collect()
}

/// Whether a summarized-field reference names a field: a formula (`@name`) or a table-qualified
/// column (`table.field`). Anything else is not a reference to a field at all.
fn is_field_ref(s: &str) -> bool {
    s.starts_with('@') || s.contains('.')
}

/// Resolve a placed field object's **effective value type** — the type used to pick its display
/// format.
///
/// Only a *summary* object carries a bound `value_type` on the object itself (the summary
/// definition's stored result type, which nothing else in the model records). Every other kind —
/// database field / formula / parameter / running total / SQL expression / group name / special —
/// leaves it [`FieldValueType::Unknown`] and is resolved here from the object's `data_source`
/// reference: the sigil selects which field *definition* (or database column) to consult, whose
/// declared `value_type` is the answer. A bare spaceless name is a special field, typed by its kind.
pub fn field_object_value_type(report: &Report, field: &FieldObject) -> FieldValueType {
    // A summary object is already bound; trust it.
    if field.value_type != FieldValueType::Unknown {
        return field.value_type;
    }
    let ds = field.data_source.trim();
    // A group-name object: `GroupName ({group field}, ["token"])` renders as its grouped field's
    // type. One that names no field — a cross-tab row/column label (`Row #1 Name`) — is a string.
    if field.ref_kind == FieldRefKind::GroupName || ds.starts_with("GroupName") {
        return match first_brace_ref(ds) {
            Some(inner) => lookup_value_type(report, inner, None),
            None => FieldValueType::String,
        };
    }
    // A braced reference — the sigil selects which definition kind to prefer on a name collision.
    if let Some(inner) = ds.strip_prefix('{') {
        let (name, prefer) = match inner.as_bytes().first().copied() {
            Some(b'@') => (ref_name(ds), Some(FieldKind::FormulaField)),
            Some(b'?') => (ref_name(ds), Some(FieldKind::ParameterField)),
            // A running total's own declared type is generic `Number`; the engine promotes a
            // value-preserving aggregate to the summarized field's type (a `Sum` of a Currency
            // column is Currency). Resolve that first, and fall back to the declared type.
            Some(b'#') => {
                return running_total_value_type(report, ref_name(ds));
            }
            Some(b'%') => (ref_name(ds), Some(FieldKind::SqlExpressionField)),
            // A DB field `{table.field}` — the database schema is its declaration.
            _ => {
                let name = ref_name(ds);
                let vt = db_field_value_type(report, name);
                if vt != FieldValueType::Unknown {
                    return vt;
                }
                (name, None)
            }
        };
        return lookup_value_type(report, name, prefer);
    }
    // A bare spaceless kind name is a special field (`PrintDate`, `PageNofM`, …).
    special_value_type(ds)
}

/// The declared type of a `alias.field` database column, from the report's decoded schema.
/// [`FieldValueType::Unknown`] when no table/column matches (the reference names a definition
/// rather than a column, or the schema was not decoded).
fn db_field_value_type(report: &Report, reference: &str) -> FieldValueType {
    let Some((alias, name)) = reference.rsplit_once('.') else {
        return FieldValueType::Unknown;
    };
    report
        .database
        .tables
        .iter()
        .filter(|t| t.alias == alias || t.name == alias)
        .flat_map(|t| &t.data_fields)
        .find(|f| f.name == name)
        .map_or(FieldValueType::Unknown, |f| f.value_type)
}

/// The effective type of a `{#name}` running-total object.
///
/// A counting operation (`Count`/`DistinctCount`) yields an integer count, which the running
/// total's own declared type already reflects. A value-preserving aggregate (`Sum`/`Maximum`/
/// `Minimum`) instead takes the **summarized** field's type — the engine promotes e.g. a `Sum` over
/// a Currency column to Currency, which the generic declared `Number` does not capture.
fn running_total_value_type(report: &Report, name: &str) -> FieldValueType {
    let Some(def) = report
        .data_definition
        .field_definitions
        .iter()
        .find(|d| d.name == name && d.kind.field_kind() == FieldKind::RunningTotalField)
    else {
        return lookup_value_type(report, name, Some(FieldKind::RunningTotalField));
    };
    let FieldKindData::RunningTotal(rt) = &def.kind else {
        return def.value_type;
    };
    if !matches!(
        rt.operation,
        SummaryOperation::Sum | SummaryOperation::Maximum | SummaryOperation::Minimum
    ) {
        return def.value_type;
    }
    // The summarized field is stored unwrapped (`Orders.Order Amount`, `@Line_Sum`).
    let summarized = rt.summarized_field.trim();
    let vt = match summarized.strip_prefix('@') {
        Some(formula) => lookup_value_type(report, formula, Some(FieldKind::FormulaField)),
        None => db_field_value_type(report, summarized),
    };
    if vt == FieldValueType::Unknown {
        def.value_type
    } else {
        vt
    }
}

/// Look up a field definition's value type by name, preferring a definition of `prefer` kind when
/// several defs share the name (e.g. a `String` DB field and a `Date` formula both named
/// `reportDate` — a `{@reportDate}` reference means the formula). Matches the reference name exactly,
/// then falls back to its last `.`-separated segment (`{Command.some_field}` →
/// `some_field`), so a table-qualified DB reference still resolves to its bare-named def.
fn lookup_value_type(report: &Report, name: &str, prefer: Option<FieldKind>) -> FieldValueType {
    let defs = &report.data_definition.field_definitions;
    let by = |n: &str| -> Vec<&FieldDef> { defs.iter().filter(|d| d.name == n).collect() };
    let mut matches = by(name);
    if matches.is_empty() {
        if let Some(short) = name.rsplit('.').next() {
            matches = by(short);
        }
    }
    if let Some(k) = prefer {
        if let Some(d) = matches.iter().find(|d| d.kind.field_kind() == k) {
            return d.value_type;
        }
    }
    matches
        .iter()
        .map(|d| d.value_type)
        .find(|v| *v != FieldValueType::Unknown)
        .or_else(|| matches.first().map(|d| d.value_type))
        .unwrap_or(FieldValueType::Unknown)
}

/// The value type a **special** field (`PrintDate`, `PageNofM`, …) renders as, from the one table
/// that maps a special-field kind to its type ([`SpecialFieldType::value_type`]). An unrecognised
/// name is [`FieldValueType::Unknown`], which formats as a string.
fn special_value_type(name: &str) -> FieldValueType {
    SpecialFieldType::from_name(name).map_or(FieldValueType::Unknown, SpecialFieldType::value_type)
}

/// Strip the display braces/prefix of a field reference to the bare definition name:
/// `{@From Date}` → `From Date`, `{?p}` → `p`, `{#rt}` → `rt`, `{%sql}` → `sql`.
fn ref_name(ds: &str) -> &str {
    let inner = strip_braces(ds);
    match inner.as_bytes().first() {
        Some(b'?') | Some(b'@') | Some(b'#') | Some(b'%') => &inner[1..],
        _ => inner,
    }
}

/// The bare definition name inside the first `{…}` of `s` (`" ({Command.d}, "Daily")"` →
/// `Command.d`), or `None` if there is no brace pair.
///
/// This is how a `GroupName (…)`-style data source names its field, so both the value-type
/// resolution here and the render pipeline's group-level lookup read a reference through it.
pub fn first_brace_ref(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let end = start + s[start..].find('}')?;
    Some(s[start + 1..end].trim())
}

/// Trim the `{ }` braces and surrounding/inner-adjacent whitespace from a display reference:
/// `" { Table.field } "` → `"Table.field"`, so a sigil sits at index 0 even with brace padding.
fn strip_braces(s: &str) -> &str {
    s.trim().trim_matches(['{', '}']).trim()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DataDefinition, FieldKindData, FieldRefKind, FormulaField};
    use FieldValueType as V;

    fn ref_name_of(ds: &str) -> &str {
        ref_name(ds)
    }

    #[test]
    fn ref_name_strips_sigils() {
        assert_eq!(ref_name_of("{@From Date}"), "From Date");
        assert_eq!(ref_name_of("{?param}"), "param");
        assert_eq!(ref_name_of("{#rt}"), "rt");
        assert_eq!(ref_name_of("{%sql}"), "sql");
        assert_eq!(ref_name_of("{Table.field}"), "Table.field");
    }

    #[test]
    fn special_field_value_types() {
        assert_eq!(special_value_type("PrintDate"), V::Date);
        assert_eq!(special_value_type("ModificationDate"), V::Date);
        assert_eq!(special_value_type("PrintTime"), V::Time);
        // The page/record/group counters are unsigned integers — the engine prints them flush, with
        // no cell reserved for a sign.
        assert_eq!(special_value_type("PageNumber"), V::Int32u);
        assert_eq!(special_value_type("RecordNumber"), V::Int32u);
        assert_eq!(special_value_type("PageNofM"), V::String);
        // An unrecognised name has no kind and so no type.
        assert_eq!(special_value_type("NotASpecialField"), V::Unknown);
    }

    #[test]
    fn first_brace_ref_extracts_inner() {
        assert_eq!(
            first_brace_ref(" ({Command.d}, \"Daily\")"),
            Some("Command.d")
        );
        assert_eq!(first_brace_ref("no braces"), None);
    }

    fn report_with_formula(name: &str, vt: FieldValueType) -> Report {
        let def = FieldDef {
            name: name.to_string(),
            value_type: vt,
            kind: FieldKindData::Formula(FormulaField::default()),
            ..Default::default()
        };
        Report {
            data_definition: DataDefinition {
                field_definitions: vec![def],
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// A formula field object leaves its own `value_type` as `Unknown`; the effective type is
    /// resolved from the referenced formula *definition*.
    #[test]
    fn formula_object_resolves_type_from_definition() {
        let report = report_with_formula("Total", V::Currency);
        let obj = FieldObject {
            data_source: "{@Total}".to_string(),
            ref_kind: FieldRefKind::Formula,
            value_type: V::Unknown,
            ..Default::default()
        };
        assert_eq!(field_object_value_type(&report, &obj), V::Currency);
    }

    /// A database field object declares no type of its own; the report's decoded schema does.
    #[test]
    fn database_object_resolves_type_from_the_schema() {
        use crate::{Database, DbFieldDef, Table};
        let report = Report {
            database: Database {
                tables: vec![Table {
                    alias: "Orders".into(),
                    data_fields: vec![DbFieldDef {
                        name: "Order Amount".into(),
                        value_type: V::Currency,
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let obj = FieldObject {
            data_source: "{Orders.Order Amount}".to_string(),
            ref_kind: FieldRefKind::DatabaseField,
            value_type: V::Unknown,
            ..Default::default()
        };
        assert_eq!(field_object_value_type(&report, &obj), V::Currency);
    }

    /// A running total declares the generic `Number`; a value-preserving aggregate takes the
    /// summarized column's type instead, and a counting one keeps the declared type.
    #[test]
    fn running_total_promotes_only_value_preserving_aggregates() {
        use crate::{Database, DbFieldDef, RunningTotalField, SummaryOperation, Table};
        let report_for = |op: SummaryOperation| Report {
            database: Database {
                tables: vec![Table {
                    alias: "Orders".into(),
                    data_fields: vec![DbFieldDef {
                        name: "Amount".into(),
                        value_type: V::Currency,
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            data_definition: DataDefinition {
                field_definitions: vec![FieldDef {
                    name: "RT".into(),
                    value_type: V::Number,
                    kind: FieldKindData::RunningTotal(RunningTotalField {
                        operation: op,
                        summarized_field: "Orders.Amount".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let obj = FieldObject {
            data_source: "{#RT}".to_string(),
            ref_kind: FieldRefKind::RunningTotal,
            value_type: V::Unknown,
            ..Default::default()
        };
        for op in [
            SummaryOperation::Sum,
            SummaryOperation::Maximum,
            SummaryOperation::Minimum,
        ] {
            assert_eq!(
                field_object_value_type(&report_for(op), &obj),
                V::Currency,
                "{op:?} preserves the summarized column's type"
            );
        }
        assert_eq!(
            field_object_value_type(&report_for(SummaryOperation::Count), &obj),
            V::Number,
            "a count keeps its own declared type"
        );
    }

    /// A cross-tab's measures are the report's summary definitions, in definition order: the
    /// operation as declared, and the summarized field only when it names one — a summary over a
    /// literal or a blank reference names no field, and its measure says so rather than carrying a
    /// name that resolves to nothing.
    #[test]
    fn measures_are_the_reports_summaries_and_only_the_field_shaped_ones_name_a_field() {
        use crate::{SummaryField, SummaryOperation as Op};
        let summary = |op: Op, field: &str| FieldDef {
            kind: FieldKindData::Summary(SummaryField {
                operation: op,
                summarized_field: field.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let report = Report {
            data_definition: DataDefinition {
                field_definitions: vec![
                    summary(Op::Sum, "Orders.Amount"),
                    // A formula reference is field-shaped too.
                    summary(Op::Maximum, "@Line Total"),
                    // Neither of these names a field.
                    summary(Op::Count, "unqualified"),
                    summary(Op::DistinctCount, ""),
                    // Not a summary at all, so not a measure.
                    FieldDef {
                        kind: FieldKindData::Formula(FormulaField::default()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        let measures = crosstab_measures(&report);
        assert_eq!(
            measures
                .iter()
                .map(|m| (m.operation, m.field.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (Op::Sum, "Orders.Amount"),
                (Op::Maximum, "@Line Total"),
                (Op::Count, ""),
                (Op::DistinctCount, ""),
            ]
        );
    }

    /// A report with no summary definitions has no measures, whatever its cross-tabs.
    #[test]
    fn a_report_without_summaries_has_no_measures() {
        assert!(crosstab_measures(&report_with_formula("Total", V::Currency)).is_empty());
    }

    /// A database/summary object already carries a bound `value_type`; it is trusted as-is.
    #[test]
    fn bound_object_type_is_trusted() {
        let report = report_with_formula("Total", V::Currency);
        let obj = FieldObject {
            data_source: "{Command.qty}".to_string(),
            ref_kind: FieldRefKind::DatabaseField,
            value_type: V::Int32s,
            ..Default::default()
        };
        assert_eq!(field_object_value_type(&report, &obj), V::Int32s);
    }
}
