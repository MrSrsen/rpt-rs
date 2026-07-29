//! The record-type-generic views of a stream: the typed record tree and the record-type inventory.
//!
//! Everything here works off the record type alone, so it covers records this reader does not
//! model. The per-domain decoders that read a record's field bytes are the sibling modules.

use super::data_def::build_field;
use crate::codec::{PieceSpan, RecordNode};
use crate::records::rtype::*;
use crate::records::{Node, Part, RecordStream, RecordTypeCount, Unknown};
use std::collections::BTreeMap;

/// Build the typed record tree: dispatch each record to its domain struct
/// [`Node`], falling through to [`Node::Unknown`] for unmodelled types. Built on demand from
/// the decoded records (see [`crate::Rpt::typed_record_tree`]).
pub(crate) fn build_typed_record_tree(stream: &RecordStream) -> Vec<Node> {
    let logical = stream.logical_bytes();
    stream
        .record_tree()
        .iter()
        .map(|n| build_node(n, logical))
        .collect()
}

pub(super) fn build_node(node: &RecordNode, logical: &[u8]) -> Node {
    match node.rtype {
        FIELD_DEFINITION => {
            if let Some(field) = build_field(node, logical) {
                return Node::FieldDef(Box::new(field));
            }
            unknown_node(node, logical)
        }
        _ => unknown_node(node, logical),
    }
}

/// Project a record into the typed tree as its content in wire order: each run of its own field
/// bytes, demasked, and each nested record where it sits between them.
pub(super) fn unknown_node(node: &RecordNode, logical: &[u8]) -> Node {
    Node::Unknown(Unknown {
        rtype: node.rtype,
        schema: node.schema,
        parts: node
            .pieces()
            .map(|piece| match piece {
                PieceSpan::Run { start, end } => Part::Run(node.demasked(logical, start, end)),
                PieceSpan::Child(child) => Part::Child {
                    framed_len: child.framed_len(),
                    node: build_node(child, logical),
                },
            })
            .collect(),
    })
}

/// Build the typed record inventory: count every record in the **full nested tree** (not just
/// the top-level tiling) per type, sorted by descending frequency then type, attaching the
/// symbolic name where the type is identified.
pub(crate) fn build_inventory(stream: &RecordStream) -> Vec<RecordTypeCount> {
    let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
    for root in stream.record_tree() {
        root.walk(&mut |node| {
            *counts.entry(node.rtype).or_default() += 1;
        });
    }
    let mut out: Vec<RecordTypeCount> = counts
        .into_iter()
        .map(|(tag, count)| RecordTypeCount { tag, count })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.tag.cmp(&b.tag)));
    out
}
