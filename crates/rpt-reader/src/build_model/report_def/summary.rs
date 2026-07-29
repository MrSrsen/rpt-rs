//! Non-running-total summary definitions — the ordered `0x7e` list a Summary object indexes into.

use crate::build_model::data_def::named_value;
use crate::build_model::row_of;
use crate::build_model::tree_search::summary_def_nodes;
use crate::codec::RecordNode;
use crate::field_table::tables as ft;
use crate::records::rtype::*;

/// One decoded non-running-total summary definition. A Summary object's opener `code` byte indexes
/// into the ordered list of these.
pub(super) struct SummaryDef {
    /// The aggregate operation.
    pub operation: crate::model::SummaryOperation,
    /// The summarized-field operand (`Table.field` or `@formula`).
    pub operand: String,
    /// The result value type.
    pub value_type: crate::model::FieldValueType,
    /// For a percentage summary, the 1-based number of the group whose total is the percentage's
    /// base; `None` for the grand total and for a non-percentage summary.
    pub percentage_base_group: Option<i32>,
}

/// Collect the ordered non-running-total summary definitions (`0x7e` no `0x80` running total
/// contains). A localized field-object reference string fails the ASCII guard, so a Summary object's
/// `code` byte indexes into this list to recover its operation + summarized field.
pub(super) fn collect_summary_defs(tree: &[RecordNode], logical: &[u8]) -> Vec<SummaryDef> {
    let mut out = Vec::new();
    for n in summary_def_nodes(tree, false) {
        let row = row_of(n, logical, &ft::SUMMARY_FIELD_DEFINITION);
        let operation = crate::model::SummaryOperation::from_code(row.u("operation") as i32);
        let operand = row.text("operand").to_owned();
        // The base group is only stored for a percentage summary; `0` there names the report grand
        // total, which has no group.
        let percentage_base_group = (row.i("is_percentage") != 0)
            .then(|| row.i("percentage_base_group"))
            .filter(|&g| g != 0);
        // The `0x71` child carries the summary's result value type. A summary has no name of its
        // own, so that child's name is stored empty.
        let value_type = n
            .children
            .iter()
            .find(|c| c.rtype == NAMED_VALUE)
            .map(|child| named_value(child, logical).value_type)
            .unwrap_or_default();
        out.push(SummaryDef {
            operation,
            operand,
            value_type,
            percentage_base_group,
        });
    }
    out
}
