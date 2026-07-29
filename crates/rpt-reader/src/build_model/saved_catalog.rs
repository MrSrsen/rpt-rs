//! Read the saved-data catalog out of a `DataSourceManager` stream.
//!
//! The stream is a tree of records like any other, so it is read here, through the same field
//! tables as everything else: the batch directory from the saved-records structure record and its
//! nested batch headers, the stored fields from the field headers under each field container. What
//! the byte layer gets is the reading — [`SavedCatalog`] — never the records.

use super::row_of;
use crate::codec::{
    parse_tree_catalog, BatchDesc, RecordNode, SavedBatch, SavedCatalog, SavedFieldDesc,
};
use crate::field_table::declared_children;
use crate::field_table::table::Cell;
use crate::field_table::tables as ft;
use crate::records::rtype::{
    SAVED_BATCH_ENTRY, SAVED_FIELD_CONTAINER, SAVED_FIELD_DESCRIPTOR, SAVED_FIELD_HEADER,
    SAVED_RECORDS_STRUCTURE,
};

/// The marker a field descriptor's word carries for a variable-length (memo) column, whose value
/// lives in `MemoValuesStream` rather than inline in the fixed record.
const VARIABLE_LENGTH: u32 = 0xffff;

/// Read the catalog out of a decoded `DataSourceManager` stream.
pub(crate) fn read_catalog(dsm_logical: &[u8]) -> SavedCatalog {
    let tree = parse_tree_catalog(dsm_logical, Some(declared_children));
    let mut catalog = SavedCatalog::default();
    for root in &tree {
        collect(root, dsm_logical, 0, &mut catalog);
    }
    catalog
}

/// Walk one record, taking the catalog's two shapes wherever they appear.
fn collect(node: &RecordNode, logical: &[u8], parent: u16, out: &mut SavedCatalog) {
    if node.rtype == SAVED_RECORDS_STRUCTURE {
        read_structure(node, logical, out);
    }
    // Only a field header directly under a field container describes a stored database-field slot;
    // the headers under the other containers carry offsets in an unrelated space.
    if node.rtype == SAVED_FIELD_HEADER && parent == SAVED_FIELD_CONTAINER {
        if let Some(field) = read_field(node, logical) {
            out.fields.push(field);
        }
    }
    for child in &node.children {
        collect(child, logical, node.rtype, out);
    }
}

/// The in-memory record width and the batch directory, from the saved-records structure record.
fn read_structure(node: &RecordNode, logical: &[u8], out: &mut SavedCatalog) {
    let row = row_of(node, logical, &ft::SAVED_RECORDS_STRUCTURE);
    out.item_size = row.get("item_size").and_then(Cell::u);
    for child in &node.children {
        if child.rtype != SAVED_BATCH_ENTRY {
            continue;
        }
        let entry = row_of(child, logical, &ft::SAVED_BATCH_ENTRY);
        out.batches.push(SavedBatch {
            desc: BatchDesc {
                count: entry.u("count"),
                item_size: entry.u("item_size"),
                stream_off: entry.u("stream_offset"),
                stream_len: entry.u("stream_length"),
            },
            columns: entry.seq("columns").iter().map(|c| c.u("value")).collect(),
            bytes: child.joined_runs(logical),
        });
    }
}

/// One stored field: where its slot sits in the fixed record, what it is called, and whether its
/// value is inline or in the memo heap.
fn read_field(node: &RecordNode, logical: &[u8]) -> Option<SavedFieldDesc> {
    let header = row_of(node, logical, &ft::SAVED_FIELD_HEADER);
    let descriptor = node
        .children
        .iter()
        .find(|c| c.rtype == SAVED_FIELD_DESCRIPTOR)
        .map(|c| row_of(c, logical, &ft::SAVED_FIELD_DESCRIPTOR))?;
    Some(SavedFieldDesc {
        rec_offset: header.u("offset") as usize,
        name: descriptor.text("field").to_owned(),
        is_memo: descriptor.u("_u0") == VARIABLE_LENGTH,
    })
}
