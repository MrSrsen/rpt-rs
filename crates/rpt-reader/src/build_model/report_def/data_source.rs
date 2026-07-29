//! Field/object data-source references — the `{...}`/summary/special display text, and the field
//! object a `0x9f` opener builds around one.

use super::summary::SummaryDef;
use crate::build_model::record_values::summary_op_full;
use crate::field_table::table::{Cell, Row, UNSET_FIELD_INDEX};
use crate::model::{FieldObject, FieldRefKind, Group};

/// Build a field object from its `0x9f` opener row.
///
/// The opener's reference is one composite — the display text, the pool it names and the index
/// within that pool — so both halves of the handle are values of the reference rather than bytes at
/// a distance from it. The display text is the engine's own form; the pool selects how the
/// `DataSource` is rendered (a `{…}` reference, a summary, a special field, …).
///
/// `group_no` is the 1-based nesting level of the group whose section hosts the object, which scopes
/// a summary; `None` yields a grand total.
pub(super) fn field_object(
    row: &Row,
    groups: &[Group],
    sum_defs: &[SummaryDef],
    group_no: Option<usize>,
) -> FieldObject {
    let handle = row.get("data_source").and_then(Cell::handle);
    let mut raw = row.text("data_source").to_owned();
    let kind = handle
        .map(|(pool, _)| FieldRefKind::from_code(pool as u8))
        .unwrap_or_default();
    // A special field's specific kind is the low half of the reference's index (its display string
    // is localized, so the code — not the string — is authoritative). For a GroupName it is the
    // 0-based group index; for a Summary it is the index into `sum_defs`.
    let code = handle.map(|(_, index)| index.unwrap_or(UNSET_FIELD_INDEX) as u8);
    // The 1-based group number for a GroupName object. Its authoritative source is the reference
    // itself (`Group #N Name`): every locale embeds the group number as the sole ASCII digit run
    // there. The ObjectName *child* is user-renameable and the reference's index is not the group
    // number, so neither is reliable.
    let group_display = matches!(kind, FieldRefKind::GroupName)
        .then(|| group_display_number(&raw))
        .flatten();
    // A localized Summary reference can't be parsed from the (non-ASCII) display string; recover
    // its engine display form ("Op of field") from the indexed summary def.
    if matches!(kind, FieldRefKind::Summary) && !raw.contains(" of ") {
        if let Some(d) = code.and_then(|c| sum_defs.get(c as usize)) {
            raw = format!("{} of {}", d.operation.token(), d.operand);
        }
    }
    // For a summary object, carry its definition index (dedup identity for `<SummaryFields>`) and
    // result value type (from the indexed `0x7e` def's child).
    let summary_code = matches!(kind, FieldRefKind::Summary)
        .then(|| code.map(u16::from))
        .flatten();
    let def = summary_code.and_then(|c| sum_defs.get(c as usize));
    FieldObject {
        data_source: field_data_source(
            kind,
            &raw,
            groups,
            group_display,
            code,
            group_no,
            def.and_then(|d| d.percentage_base_group),
        ),
        ref_kind: kind,
        value_type: def.map(|d| d.value_type).unwrap_or_default(),
        summary_code,
        ..Default::default()
    }
}

/// The 1-based group number embedded in a GroupName object's display reference, the sole run of
/// ASCII digits in `Group #N Name` (locale-independent: only the digits are ASCII). `None` if the
/// string holds no digits.
pub(super) fn group_display_number(name: &str) -> Option<usize> {
    name.split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())?
        .parse()
        .ok()
}

/// A group as it appears in a summary/group-name reference: its condition field, brace-wrapped, plus
/// the grouping period as a second (lowercase, quoted) operand when the group is a date/time group —
/// `{Orders.Order Date}, "monthly"`. Only a date/time condition has a known operand; a boolean
/// condition has none and a discrete group none at all.
fn group_operand(g: &Group) -> String {
    match g
        .date_condition
        .filter(|c| c.date_time_operand().is_some())
        .map(|c| c.token())
    {
        Some(tok) => format!("{{{}}}, \"{tok}\"", g.condition_field),
        None => format!("{{{}}}", g.condition_field),
    }
}

/// Render a field object's `DataSource` the way the engine does, from its kind and raw reference.
/// Plain references (database/formula/running-total/parameter fields) are wrapped in `{…}`; the
/// computed kinds (summary, special, group-name) get their own surface form.
pub(super) fn field_data_source(
    kind: FieldRefKind,
    raw: &str,
    groups: &[Group],
    group_display: Option<usize>,
    code: Option<u8>,
    group_no: Option<usize>,
    percentage_base_group: Option<i32>,
) -> String {
    match kind {
        FieldRefKind::DatabaseField
        | FieldRefKind::Formula
        | FieldRefKind::RunningTotal
        | FieldRefKind::Parameter
        | FieldRefKind::SqlExpression => format!("{{{raw}}}"),
        // A special field renders as its canonical kind name, from the type code (its display
        // string is localized). Fall back to the spaceless display string for unmapped codes.
        FieldRefKind::Special => code
            .and_then(special_field_name)
            .map(String::from)
            .unwrap_or_else(|| raw.replace(' ', "")),
        // `GroupName ({the Nth group's condition field})`. The 1-based `group_display` number,
        // read from the opener's own display reference, is authoritative (see `group_display_number`
        // and the call site). The opener's `code` byte is NOT the group index, and the ObjectName
        // (`raw`) is user-renameable, so neither is used here.
        // A date/time group carries a grouping condition the engine appends as a lowercase string
        // operand in a field object's DataSource: `GroupName ({fld}, "monthly")`. See
        // [`group_operand`].
        FieldRefKind::GroupName => group_display
            .and_then(|n| groups.get(n.wrapping_sub(1)))
            .map(|g| format!("GroupName ({})", group_operand(g)))
            .unwrap_or_else(|| raw.to_string()),
        // `Sum of {operand}` -> `Sum ({operand})`. A summary placed in a group's header/footer is
        // scoped to that group, which the engine appends as a second operand: the group's condition
        // field (`Sum ({operand}, {group field})`). The group is not in the raw string — it is the
        // group owning the hosting section (`group_no`). Report/page-band summaries are grand totals
        // (one operand).
        FieldRefKind::Summary => match raw.split_once(" of ") {
            Some((op0, operand0)) => {
                let remap = |o: &str| summary_op_full(o).to_string();
                // A percentage summary collapses `Percentage of <InnerOp> of {field}` to
                // `PercentOf<InnerOp> ({field}, {group})` (e.g. `Percentage of Sum of X` →
                // `PercentOfSum (…)`), dropping the inner `… of` level rather than nesting it.
                let (op, operand) = match (op0, operand0.split_once(" of ")) {
                    ("Percentage", Some((inner, field))) => {
                        (format!("PercentOf{}", remap(inner)), field.to_string())
                    }
                    _ => (remap(op0), operand0.to_string()),
                };
                let op = op.as_str();
                let operand = operand.as_str();
                // A percentage summary's *denominator* scope is stored on the definition (0 = the
                // report grand total, which needs no operand); it follows the summary's own group,
                // matching the engine's `PercentOf<Op> (fld, group, baseGroup)` form.
                let base = percentage_base_group
                    .and_then(|n| usize::try_from(n).ok())
                    .and_then(|n| groups.get(n.wrapping_sub(1)))
                    .map(|g| format!(", {}", group_operand(g)))
                    .unwrap_or_default();
                match group_no.and_then(|n| groups.get(n.wrapping_sub(1))) {
                    Some(g) => format!("{op} ({{{operand}}}, {}{base})", group_operand(g)),
                    None => format!("{op} ({{{operand}}}{base})"),
                }
            }
            None => raw.to_string(),
        },
        // A `?`-prefixed reference is a parameter field; brace-wrap it like other references.
        FieldRefKind::Unknown if raw.starts_with('?') => format!("{{{raw}}}"),
        FieldRefKind::Unknown => raw.to_string(),
    }
}

/// The canonical name for a special field's type code (the byte at `p+2` of its opener). Maps the
/// established codes to their engine `DataSource` string; for any other code the caller keeps the
/// (English) display string with spaces removed, which yields the same name.
pub(crate) fn special_field_name(code: u8) -> Option<&'static str> {
    crate::model::SpecialFieldType::from_code(code).map(|k| k.name())
}

#[cfg(test)]
mod tests {
    use super::field_data_source;
    use crate::model::{FieldRefKind, Group};

    fn groups(fields: &[&str]) -> Vec<Group> {
        fields
            .iter()
            .map(|f| Group {
                condition_field: (*f).to_string(),
                ..Group::default()
            })
            .collect()
    }

    /// A summary's group operands: its own scope (the group owning the hosting section) and, for a
    /// percentage summary, the group whose total is the percentage's base — the engine's
    /// `PercentOf<Op> (fld, group, baseGroup)` form.
    #[test]
    fn percentage_summary_appends_its_base_group() {
        let g = groups(&["region.name", "country.name", "city.name"]);
        let render = |base| {
            field_data_source(
                FieldRefKind::Summary,
                "Percentage of Sum of sales_order.total_amount",
                &g,
                None,
                None,
                Some(3),
                base,
            )
        };
        // No base group: the denominator is the report grand total, which has no operand.
        assert_eq!(
            render(None),
            "PercentOfSum ({sales_order.total_amount}, {city.name})"
        );
        // A base group is 1-based, so group 2 of three is the middle one.
        assert_eq!(
            render(Some(2)),
            "PercentOfSum ({sales_order.total_amount}, {city.name}, {country.name})"
        );
        assert_eq!(
            render(Some(1)),
            "PercentOfSum ({sales_order.total_amount}, {city.name}, {region.name})"
        );
        // A number naming no group is dropped rather than rendered as a dangling operand.
        assert_eq!(
            render(Some(9)),
            "PercentOfSum ({sales_order.total_amount}, {city.name})"
        );
    }

    /// A plain summary is unaffected, and a percentage summary placed outside any group keeps its
    /// base group as the sole group operand.
    #[test]
    fn base_group_is_independent_of_the_summarys_own_scope() {
        let g = groups(&["country.name", "city.name"]);
        assert_eq!(
            field_data_source(
                FieldRefKind::Summary,
                "Sum of sales_order.total_amount",
                &g,
                None,
                None,
                Some(2),
                None,
            ),
            "Sum ({sales_order.total_amount}, {city.name})"
        );
        assert_eq!(
            field_data_source(
                FieldRefKind::Summary,
                "Percentage of Sum of sales_order.total_amount",
                &g,
                None,
                None,
                None,
                Some(1),
            ),
            "PercentOfSum ({sales_order.total_amount}, {country.name})"
        );
    }
}
