//! Derived `<SummaryFields>` reconstruction: the deduplicated, ordered list of placed summary
//! field objects for one report level (the engine's `ISummaryFields`).

use super::formula;
use super::SummaryFieldDef;
use rpt::model::{FieldRefKind, FieldValueType, Report, ReportObjectKind};
use std::collections::{HashMap, HashSet};

/// The deduplicated, ordered `<SummaryFields>` list for one report level. Each entry is a placed
/// summary field object, identified by its summary-definition code. The order matches the engine's
/// `ISummaryFields` collection: scope blocks (the group owning the section, grand totals last) in
/// order of first appearance, and within a block by the object's `(Left, Top)`.
pub fn summary_fields(report: &Report) -> Vec<SummaryFieldDef> {
    // Placed summary objects in document (section) order.
    struct Placed<'a> {
        code: u16,
        scope: String, // the group-by argument; empty = grand total
        ds: &'a str,
        vt: FieldValueType,
        left: i32,
        top: i32,
        doc: usize,
        named: bool, // false for a cross-tab cell (no ObjectName) — counted for UseCount, not emitted
    }
    let mut placed: Vec<Placed> = Vec::new();
    let mut doc = 0usize;
    for area in &report.report_definition.areas {
        for sec in &area.sections {
            for obj in &sec.objects {
                if let ReportObjectKind::Field(f) = &obj.kind {
                    if f.ref_kind == FieldRefKind::Summary {
                        if let Some(code) = f.summary_code {
                            placed.push(Placed {
                                code,
                                scope: brace_groups(&f.data_source)
                                    .get(1)
                                    .map(|s| s.to_string())
                                    .unwrap_or_default(),
                                ds: &f.data_source,
                                vt: f.value_type,
                                left: obj.bounds.left.0,
                                top: obj.bounds.top.0,
                                doc,
                                // A cross-tab cell carries no ObjectName; it is counted for UseCount
                                // but the CrossTabObject owns it, so it is not emitted standalone.
                                named: !obj.name.is_empty(),
                            });
                        }
                    }
                }
                doc += 1;
            }
        }
    }

    // Deduplicate by summary code, keeping the first (document-order) placement.
    let mut seen: HashSet<u16> = HashSet::new();
    let mut uniq: Vec<&Placed> = placed.iter().filter(|p| seen.insert(p.code)).collect();

    // Block order = first appearance of each scope; within a block, sort by (Left, Top).
    let mut scope_first: HashMap<&str, usize> = HashMap::new();
    for p in &uniq {
        scope_first.entry(p.scope.as_str()).or_insert(p.doc);
    }
    uniq.sort_by(|a, b| {
        scope_first[a.scope.as_str()]
            .cmp(&scope_first[b.scope.as_str()])
            .then(a.left.cmp(&b.left))
            .then(a.top.cmp(&b.top))
    });

    uniq.into_iter()
        .map(|p| {
            let groups = brace_groups(p.ds);
            let summarized_field_type = if groups.first().is_some_and(|g| g.starts_with("{@")) {
                "CrystalDecisions.CrystalReports.Engine.FormulaFieldDefinition"
            } else {
                "CrystalDecisions.CrystalReports.Engine.DatabaseFieldDefinition"
            }
            .to_string();
            // The Operation is the base aggregation. A percentage summary leads with `PercentOf<Op>`
            // but its Operation is the underlying op (`Sum`) — percentage is a display mode, not a
            // distinct SummaryOperation.
            let op_token = p.ds.split(" (").next().unwrap_or("").trim();
            let operation = op_token
                .strip_prefix("PercentOf")
                .unwrap_or(op_token)
                .to_string();
            // Maximum / Minimum return the summarized field's own type (the result is one of the
            // input values), unlike Sum/Average which coerce to Number/Currency. Use the summarized
            // DB field's declared type (the placed object's value_type reports the coerced Number).
            let vt = match operation.as_str() {
                "Maximum" | "Minimum" => summarized_db_field_type(&groups, report).unwrap_or(p.vt),
                _ => p.vt,
            };
            SummaryFieldDef {
                formula_name: p.ds.to_string(),
                operation,
                value_type: format!("{vt:?}Field"),
                number_of_bytes: summary_number_of_bytes(vt, &groups, report),
                summarized_field_type,
                grouped: !p.scope.is_empty(),
                named: p.named,
            }
        })
        .collect()
}

/// The maximal `{…}` substrings of a summary expression, in order: `[0]` is the summarized field,
/// `[1]` (if present) the group-by argument.
pub(crate) fn brace_groups(s: &str) -> Vec<&str> {
    formula::brace_groups(s).collect()
}

/// The declared type of the summary's summarized field (`groups[0]`, e.g. `{Command.days}`) when it
/// is a database field; `None` for a formula operand or an unresolved field.
fn summarized_db_field_type(groups: &[&str], report: &Report) -> Option<FieldValueType> {
    let name = groups
        .first()?
        .trim_start_matches('{')
        .trim_end_matches('}');
    report
        .database
        .tables
        .iter()
        .flat_map(|t| &t.data_fields)
        .find(|f| f.long_name.as_deref().unwrap_or(&f.name) == name)
        .map(|f| f.value_type)
}

/// `NumberOfBytes` for a summary's result type: the intrinsic width, except a string result uses the
/// summarized field's declared length (the result record's length byte is unreliable for strings).
fn summary_number_of_bytes(vt: FieldValueType, groups: &[&str], report: &Report) -> i32 {
    use FieldValueType::*;
    match vt {
        Number | Currency | DateTime => 8,
        Date | Time | Int32s | Int32u => 4,
        Int16s | Boolean => 2,
        Int8s => 1,
        String => {
            let name = groups
                .first()
                .map(|g| g.trim_start_matches('{').trim_end_matches('}'));
            name.and_then(|name| {
                report
                    .database
                    .tables
                    .iter()
                    .flat_map(|t| &t.data_fields)
                    .find(|f| f.long_name.as_deref().unwrap_or(&f.name) == name)
                    .map(|f| f.length)
            })
            .unwrap_or(65534)
        }
        _ => 0,
    }
}
