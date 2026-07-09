//! Non-running-total summary definitions — the ordered `0x7e` list a Summary object indexes into.

use super::*;

/// One decoded non-running-total summary definition: `(operation, summarized-field operand, result
/// value type)`. A Summary object's opener `code` byte indexes into the ordered list of these.
pub(super) type SummaryDef = (
    crate::model::SummaryOperation,
    String,
    crate::model::FieldValueType,
);

/// Collect the ordered non-running-total summary definitions (`0x7e` not preceded by a `0x80`
/// reset). A localized field-object reference string fails the ASCII guard, so a Summary object's
/// `code` byte indexes into this list to recover its operation + summarized field.
pub(super) fn collect_summary_defs(tree: &[RecordNode], logical: &[u8]) -> Vec<SummaryDef> {
    let mut prev = 0u16;
    let mut out = Vec::new();
    for n in flatten(tree) {
        if n.rtype == SUMMARY_DEF && prev != RT_RESET {
            let lb = n.leaf_bytes(logical);
            let op = crate::model::SummaryOperation::from_code(i32::from(
                lb.first().copied().unwrap_or(0),
            ));
            let operand = lb
                .get(4..)
                .and_then(read_lp_string)
                .map(|(s, _)| s)
                .unwrap_or_default();
            // The `0x71` child carries the summary's result value type: unlike a running total's
            // child (which leads with the field name), a summary's child is a fixed header
            // `00 00 00 01 00 <vt> 00 <nbytes> …` — the value-type code sits at offset 5.
            let value_type = n
                .children
                .iter()
                .find(|c| c.rtype == NAMED_VALUE)
                .and_then(|child| child.leaf_bytes(logical).get(5).copied())
                .map(|code| crate::model::FieldValueType::from_code(i32::from(code)))
                .unwrap_or_default();
            out.push((op, operand, value_type));
        }
        prev = n.rtype;
    }
    out
}
