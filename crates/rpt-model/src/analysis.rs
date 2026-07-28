//! Pure, derived helpers over the semantic model that both the export and render layers need.
//!
//! Nothing here is a stored fact — these functions *infer* a value from the decoded model, so they
//! carry no state on any model struct. They are dependency-free (no formula engine, no decoder), so
//! every consumer of [`crate::Report`] can reach them.

use crate::{FieldDef, FieldKind, FieldObject, FieldValueType, Report};

/// Resolve a placed field object's **effective value type** — the type used to pick its display
/// format. Database and summary field objects already carry a bound `value_type` on the object;
/// every other kind (formula / parameter / running-total / SQL-expression / group-name / special)
/// leaves the object's `value_type` as [`FieldValueType::Unknown`] and instead resolves from its
/// `data_source` reference: the sigil selects which field *definition* to consult, whose declared
/// `value_type` is the answer. A bare spaceless name is a special field, typed by its kind.
pub fn field_object_value_type(report: &Report, field: &FieldObject) -> FieldValueType {
    // Database / summary field objects are already bound (their `value_type` is set); trust it.
    if field.value_type != FieldValueType::Unknown {
        return field.value_type;
    }
    let ds = field.data_source.trim();
    // `GroupName ({group field}, ["token"])` renders as its grouped field's type.
    if let Some(rest) = ds.strip_prefix("GroupName") {
        return match first_brace_ref(rest) {
            Some(inner) => lookup_value_type(report, inner, None),
            None => FieldValueType::String,
        };
    }
    // A braced reference — the sigil selects which definition kind to prefer on a name collision.
    if let Some(inner) = ds.strip_prefix('{') {
        let (name, prefer) = match inner.as_bytes().first().copied() {
            Some(b'@') => (ref_name(ds), Some(FieldKind::FormulaField)),
            Some(b'?') => (ref_name(ds), Some(FieldKind::ParameterField)),
            Some(b'#') => (ref_name(ds), Some(FieldKind::RunningTotalField)),
            Some(b'%') => (ref_name(ds), Some(FieldKind::SqlExpressionField)),
            _ => (ref_name(ds), None), // a DB field `{table.field}`
        };
        return lookup_value_type(report, name, prefer);
    }
    // A bare spaceless kind name is a special field (`PrintDate`, `PageNofM`, …).
    special_value_type(ds)
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

/// The value type a **special** field (`PrintDate`, `PageNofM`, …) renders as. The date/time
/// specials carry a real Date/Time value; page/record/group counters are Number; everything else
/// (page-N-of-M, titles, paths) renders as a String.
fn special_value_type(name: &str) -> FieldValueType {
    use FieldValueType as V;
    match name {
        "PrintDate" | "ModificationDate" | "DataDate" => V::Date,
        "PrintTime" | "ModificationTime" | "DataTime" => V::Time,
        "PageNumber" | "RecordNumber" | "GroupNumber" | "TotalPageCount" => V::Number,
        _ => V::String,
    }
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
fn first_brace_ref(s: &str) -> Option<&str> {
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
        assert_eq!(special_value_type("PageNumber"), V::Number);
        assert_eq!(special_value_type("PageNofM"), V::String);
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
