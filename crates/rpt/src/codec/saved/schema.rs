//! Saved-data catalog parsing: the `DataSourceManager` batch directory and the stored-record
//! field catalog — the stored facts about how the cached rows are laid out.

use crate::codec::tree::{parse_tree_qe, RecordNode};
use crate::records::rtype::{
    DSM_BATCH_ENTRY, DSM_FIELD_CONTAINER, DSM_FIELD_DESC, DSM_FIELD_HEADER, DSM_STRUCTURE,
};

/// Batch size (row capacity) of the record-index batch (`SavedRecordsStream`) — a fixed 1000-row cap.
pub(crate) const INDEX_BATCH_SIZE: u32 = 1000;

/// Fixed byte width of one memo-descriptor cell: `[u16 col][u16 flag][u32 heap_offset][u32 byte_length]`.
pub(crate) const MEMO_CELL_SIZE: u32 = 12;

/// The byte budget a memo-descriptor batch fills: its row capacity (the IV's first
/// word) is `DESC_BATCH_BYTE_BUDGET / item_size` (e.g. an item size of 72 gives a row capacity of
/// 142).
pub(crate) const DESC_BATCH_BYTE_BUDGET: u32 = 10224;

/// One saved-data batch descriptor from the `DataSourceManager` directory: row `count` and fixed
/// per-item byte width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BatchDesc {
    /// Number of items (records) in the batch.
    pub count: u32,
    /// Fixed per-item byte width.
    pub item_size: u32,
}

/// The leaf bytes of every batch-directory entry (`0x6d`) under a saved-records structure record
/// (`0x2d`) in a parsed `DataSourceManager` tree, in file order. The one traversal the batch-directory
/// readers share; callers parse the tree once and thread it in.
pub(crate) fn dsm_batch_leaves(tree: &[RecordNode], logical: &[u8]) -> Vec<Vec<u8>> {
    fn collect(n: &RecordNode, lg: &[u8], out: &mut Vec<Vec<u8>>) {
        if n.rtype == DSM_BATCH_ENTRY {
            out.push(n.leaf_bytes(lg));
        }
        for c in &n.children {
            collect(c, lg, out);
        }
    }
    // Batch entries live under the saved-records structure record (`0x2d`).
    fn under_structure(n: &RecordNode, lg: &[u8], out: &mut Vec<Vec<u8>>) {
        if n.rtype == DSM_STRUCTURE {
            collect(n, lg, out);
        } else {
            for c in &n.children {
                under_structure(c, lg, out);
            }
        }
    }
    let mut out = Vec::new();
    for r in tree {
        under_structure(r, logical, &mut out);
    }
    out
}

/// Read a batch entry leaf's `count` (big-endian u32 at `[0..4]`) and `item_size` (`[4..8]`).
fn batch_desc(leaf: &[u8]) -> Option<BatchDesc> {
    (leaf.len() >= 8).then(|| BatchDesc {
        count: u32::from_be_bytes([leaf[0], leaf[1], leaf[2], leaf[3]]),
        item_size: u32::from_be_bytes([leaf[4], leaf[5], leaf[6], leaf[7]]),
    })
}

/// The saved-data batch directory, guarded: every `0x6d` entry whose `item_size` is a plausible
/// positive width (dropping spurious matches). Returns the entries in file order.
pub(crate) fn batch_directory(tree: &[RecordNode], logical: &[u8]) -> Vec<BatchDesc> {
    dsm_batch_leaves(tree, logical)
        .into_iter()
        .filter_map(|leaf| batch_desc(&leaf))
        .filter(|b| b.item_size > 0 && b.item_size < 0x1_0000 && b.count < 0x0100_0000)
        .collect()
}

/// The full saved-data batch directory (unguarded): every `0x6d` entry in file order — index batches
/// (`item_size` = the fixed record width), then memo-descriptor batches (`item_size` =
/// `memo_cols * MEMO_CELL_SIZE`), then memo-value batches (`item_size` = 0).
pub(crate) fn saved_batches(tree: &[RecordNode], logical: &[u8]) -> Vec<BatchDesc> {
    dsm_batch_leaves(tree, logical)
        .into_iter()
        .filter_map(|leaf| batch_desc(&leaf))
        .collect()
}

/// The full `0x6d` directory-entry leaves (each `>= 8` bytes), in the same order as [`saved_batches`].
/// Surfaces the whole entry (including a packed index batch's column table) for byte-level inspection.
pub(crate) fn saved_batch_dir_leaves(tree: &[RecordNode], logical: &[u8]) -> Vec<Vec<u8>> {
    dsm_batch_leaves(tree, logical)
        .into_iter()
        .filter(|l| l.len() >= 8)
        .collect()
}

/// The record-index batches from a `DataSourceManager` directory: the leading run of entries that
/// share the first entry's `item_size`. The directory lists all index batches first (the fixed-width
/// record index, physically stored in `SavedRecordsStream`), then the memo batches (`item_size` =
/// `memo_col_count * 12`, physically in `MemoValuesStream`). A large saved rowset splits the index
/// across several batches (`SavedRecordsStream` is itself multi-batch), so the record count is the
/// **sum** of these, not the max.
pub(crate) fn index_directory(dsm_logical: &[u8]) -> Vec<BatchDesc> {
    let tree = parse_tree_qe(dsm_logical);
    let dir = batch_directory(&tree, dsm_logical);
    let Some(first) = dir.first().copied() else {
        return Vec::new();
    };
    dir.into_iter()
        .take_while(|b| b.item_size == first.item_size)
        .collect()
}

/// The report's saved record count — the total across all record-index batches in the
/// `DataSourceManager` directory. `None` when there is no saved data.
pub(crate) fn saved_record_count(dsm_logical: &[u8]) -> Option<u32> {
    let total: u32 = index_directory(dsm_logical).iter().map(|b| b.count).sum();
    (total > 0).then_some(total)
}

/// The in-memory (persistent) record width from the `DataSourceManager` saved-records structure
/// record (`0x2d`): a big-endian u16 at the leaf's `[0..2]`. This is the record size the batch
/// cipher IV keys on (the persistent item size) — for a record whose string columns are stored
/// **inline** it is larger than the directory's on-disk `item_size` (the packed record width). It
/// equals the on-disk width when no columns are packed (e.g. the memo-heap reports). The value is
/// echoed in each decoded batch's own header (`[type u16][count u32][item_size u32][batch_size u32]`).
pub(crate) fn persistent_item_size(tree: &[RecordNode], logical: &[u8]) -> Option<u32> {
    fn find(n: &RecordNode, lg: &[u8]) -> Option<u32> {
        if n.rtype == DSM_STRUCTURE {
            let l = n.leaf_bytes(lg);
            if l.len() >= 2 {
                return Some(u16::from_be_bytes([l[0], l[1]]) as u32);
            }
        }
        n.children.iter().find_map(|c| find(c, lg))
    }
    tree.iter().find_map(|r| find(r, logical))
}

/// A stored saved-record field descriptor from the `DataSourceManager` catalog.
pub(crate) struct SavedFieldDesc {
    /// Byte offset of the field's slot within the fixed record.
    pub rec_offset: usize,
    /// The stored field name (e.g. `countries_all_iso.id`).
    pub name: String,
    /// Variable-length (memo/string) field — its value lives in `MemoValuesStream`, not inline.
    pub is_memo: bool,
}

/// Parse the stored-record field catalog (the database fields) out of a decoded `DataSourceManager`
/// stream. The catalog lives under `0x07` field containers: each `0x41` header is `00 idx 00 offset
/// 00 00` (the field's byte offset in the record), and its child `0x40` is `<u32BE nameLen><name>
/// <trailer>` where the trailer carries `ff ff` for a variable-length (memo) field. Fields are
/// returned in `0x41` (record-layout) order.
pub(crate) fn saved_schema(dsm_logical: &[u8]) -> Vec<SavedFieldDesc> {
    let tree = parse_tree_qe(dsm_logical);
    let mut out = Vec::new();
    // Only `0x41` headers directly under a `0x07` container describe stored database-field slots
    // (formula `0x08` / special `0x17` fields carry offsets in an unrelated space).
    fn walk(n: &RecordNode, lg: &[u8], parent: u16, out: &mut Vec<SavedFieldDesc>) {
        if n.rtype == DSM_FIELD_HEADER && parent == DSM_FIELD_CONTAINER {
            let hdr = n.leaf_bytes(lg);
            if hdr.len() >= 4 {
                // The field's byte offset in the (in-memory) record is a big-endian u16 at [2..4] —
                // wide records place fields past offset 255, so the low byte alone is not enough.
                let rec_offset = u16::from_be_bytes([hdr[2], hdr[3]]) as usize;
                if let Some(desc) = n.children.iter().find(|c| c.rtype == DSM_FIELD_DESC) {
                    let leaf = desc.leaf_bytes(lg);
                    if leaf.len() >= 4 {
                        let nl = u32::from_be_bytes([leaf[0], leaf[1], leaf[2], leaf[3]]) as usize;
                        if 4 + nl <= leaf.len() {
                            let name = String::from_utf8_lossy(&leaf[4..4 + nl])
                                .trim_end_matches('\0')
                                .to_owned();
                            let trailer = &leaf[4 + nl..];
                            let is_memo = trailer.windows(2).any(|w| w == [0xff, 0xff]);
                            out.push(SavedFieldDesc {
                                rec_offset,
                                name,
                                is_memo,
                            });
                        }
                    }
                }
            }
        }
        for c in &n.children {
            walk(c, lg, n.rtype, out);
        }
    }
    for r in &tree {
        walk(r, dsm_logical, 0, &mut out);
    }
    out
}
