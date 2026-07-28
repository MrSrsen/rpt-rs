//! Field/object data-source reference text (the `{...}`/summary/special display refs).

use super::*;

/// An object's data string — the text of a text object or the data-source reference of a field
/// object — read from the object record's **own** leaf bytes (its `ObjectName` child holds the
/// object name, not the data, so it must be excluded).
pub(super) fn object_data_string(node: &RecordNode, logical: &[u8]) -> Option<String> {
    first_lp(&node.leaf_bytes(logical))
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
        // operand in a field object's DataSource: `GroupName ({fld}, "monthly")` (the same lowercase
        // token used for a group-scoped summary's period operand). A boolean/discrete group has no
        // known operand, so the bare `GroupName ({fld})` form is emitted.
        FieldRefKind::GroupName => group_display
            .and_then(|n| groups.get(n.wrapping_sub(1)))
            .map(|g| {
                match g
                    .date_condition
                    .filter(|c| c.date_time_operand().is_some())
                    .map(|c| c.token())
                {
                    Some(op) => format!("GroupName ({{{}}}, \"{op}\")", g.condition_field),
                    None => format!("GroupName ({{{}}})", g.condition_field),
                }
            })
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
                match group_no.and_then(|n| groups.get(n.wrapping_sub(1))) {
                    // A summary scoped to a date/time group gets the group's grouping period as a
                    // (lowercase) third operand: `DistinctCount ({op}, {g}, "daily")`. Emitted only
                    // for a date/time condition (the operand check gates it); a boolean condition has
                    // no known operand and a discrete group none at all.
                    Some(g) => match g
                        .date_condition
                        .filter(|c| c.date_time_operand().is_some())
                        .map(|c| c.token())
                    {
                        Some(tok) => {
                            format!("{op} ({{{operand}}}, {{{}}}, \"{tok}\")", g.condition_field)
                        }
                        None => format!("{op} ({{{operand}}}, {{{}}})", g.condition_field),
                    },
                    None => format!("{op} ({{{operand}}})"),
                }
            }
            None => raw.to_string(),
        },
        // A `?`-prefixed reference is a parameter field; brace-wrap it like other references.
        FieldRefKind::Unknown if raw.starts_with('?') => format!("{{{raw}}}"),
        FieldRefKind::Unknown => raw.to_string(),
    }
}

/// The engine's display token for a summary operation, as it appears in a field object's reference
/// string ("Sum of {…}"). `Max`/`Min` are abbreviated there (remapped to Maximum/Minimum by
/// `field_data_source`); the rest match the full enum name.
pub(super) fn summary_op_token(op: crate::model::SummaryOperation) -> &'static str {
    use crate::model::SummaryOperation::*;
    match op {
        Sum => "Sum",
        Count => "Count",
        DistinctCount => "DistinctCount",
        Maximum => "Max",
        Minimum => "Min",
        Average => "Average",
        SampleVariance => "SampleVariance",
        SampleStandardDeviation => "SampleStandardDeviation",
        PopVariance => "PopVariance",
        PopStandardDeviation => "PopStandardDeviation",
        Correlation => "Correlation",
        Covariance => "Covariance",
        WeightedAvg => "WeightedAvg",
        Median => "Median",
        Percentile => "Percentile",
        NthLargest => "NthLargest",
        NthSmallest => "NthSmallest",
        Mode => "Mode",
        NthMostFrequent => "NthMostFrequent",
        Other(_) => "Sum",
    }
}

/// The canonical name for a special field's type code (the byte at `p+2` of its opener). Maps the
/// established codes to their engine `DataSource` string; for any other code the caller keeps the
/// (English) display string with spaces removed, which yields the same name.
pub(in crate::project::raise) fn special_field_name(code: u8) -> Option<&'static str> {
    crate::model::SpecialFieldType::from_code(code).map(|k| k.name())
}

#[cfg(test)]
mod tests {
    use super::summary_op_token;
    use crate::model::SummaryOperation;

    #[test]
    fn summary_op_token_abbreviates_min_max() {
        // Maximum/Minimum abbreviate; `summary_op_full` expands them back for the rendered form.
        assert_eq!(summary_op_token(SummaryOperation::Maximum), "Max");
        assert_eq!(summary_op_token(SummaryOperation::Minimum), "Min");
        assert_eq!(summary_op_token(SummaryOperation::Sum), "Sum");
        assert_eq!(
            summary_op_token(SummaryOperation::DistinctCount),
            "DistinctCount"
        );
        // Unmapped operations fall back to "Sum".
        assert_eq!(summary_op_token(SummaryOperation::Other(999)), "Sum");
    }
}
