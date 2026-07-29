//! The recursive record tree.
//!
//! A report record's content is itself a sequence of **nested** TSLV records and runs of the
//! record's own field data. Content is read under a **stack-XOR mask**: the per-byte XOR mask is
//! the XOR of the record types currently on the parse stack (it un-XORs on pop).
//!
//! Three concerns, kept apart because each changes for its own reason: the node and the runs it
//! exposes ([`node`]), finding the records in the bytes ([`scan`]), and putting a tree back on the
//! wire ([`serialize`]). Nothing here changes a byte — rewriting a record's field bytes is
//! [`super::edit`], and it is the only part of the tree's machinery that can fail.

mod node;
mod scan;
mod serialize;

pub(crate) use node::PieceSpan;
pub use node::RecordNode;
pub(crate) use scan::{
    parse_tree, parse_tree_catalog, parse_tree_qe_session, parse_tree_report_parameters,
};
pub(crate) use serialize::serialize_tree;

/// The scan takes its child rule as a parameter; these are its entry points wired the way
/// [`crate::records`] wires them, with the report definition's own declarations. A test that
/// passes no rule is asking what the bare scan does, and calls the scan directly.
#[cfg(test)]
mod wired {
    use super::RecordNode;

    pub(super) fn parse_tree(logical: &[u8]) -> Vec<RecordNode> {
        super::scan::parse_tree(logical, Some(crate::field_table::declared_children))
    }

    pub(super) fn parse_tree_qe_session(logical: &[u8]) -> Vec<RecordNode> {
        super::scan::parse_tree_qe_session(logical, Some(crate::field_table::declared_children))
    }
}
