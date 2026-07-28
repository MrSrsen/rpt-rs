//! Saved-data (stored rows) decode.
//!
//! A report saved with data caches its rows across two streams, decoded by [`decode_saved_rows`]:
//! `SavedRecordsStream` holds the record-**index** batches (a fixed-width record per row: byte-0
//! present bitmap + inline integer/date fields) followed by the memo-**descriptor** batches; and
//! `MemoValuesStream` holds the memo-value heaps (`(u32 len)(utf16z)` entries). Each batch is
//! `zlib(records)` encrypted with the `Contents` modified-AES-CFB cipher.
//!
//! The module is split by responsibility: [`schema`] parses the `DataSourceManager` batch directory
//! and stored-record field catalog; [`crack`] is the cipher/IV/inflate layer;
//! [`packed`] decodes the inline-string (memo-less) packed rowset; [`memo`] resolves the memo heaps.
//!
//! There is **no delta / change mask** to reconstruct: each row's memo descriptor holds an explicit
//! per-cell `[u16 col][u16 flag][u32 heap_offset][u32 byte_length]` pointer straight into the memo
//! heap, so a repeated value simply points back at an earlier heap entry.

mod crack;
mod memo;
mod packed;
mod schema;

pub(crate) use crack::decode_index_stream;
pub(crate) use schema::{index_directory, saved_record_count, saved_schema, SavedFieldDesc};

use crate::codec::tree::parse_tree_qe;
use crate::model::FieldValueType;
use crate::records::RecordStream;
use crate::StreamId;

use crack::{batch_iv4, block0_is_zlib, decode_batch_at, inflate_zlib_counted};
use memo::{decode_memo_heaps, read_memo_cell};
use packed::decode_packed_index;
use schema::{
    persistent_item_size, saved_batch_dir_leaves, saved_batches, DESC_BATCH_BYTE_BUDGET,
    INDEX_BATCH_SIZE, MEMO_CELL_SIZE,
};

/// Reconstruct the stored saved-data rows from the raw `SavedRecordsStream` (`srs_raw`, holding the
/// record-index batches then the memo-descriptor batches) and `MemoValuesStream` (`memo_raw`, the
/// memo-value heaps). Returns `(rows, record_count)`; `None` when there is no saved data.
///
/// Each row's **inline** fields (integers, dates as day counts, …) are read from the fixed record
/// index at the schema `rec_offset`. Each row's **memo** fields are read via the memo descriptor: a
/// per-row `memo_cols × 12` record whose 12-byte cells are `[u16 col][u16 flag][u32 heap_offset]
/// [u32 byte_length]` pointing directly into the corresponding memo-value batch heap. This is an
/// explicit per-cell pointer — there is no delta/change-mask to reconstruct: an
/// "unchanged" cell simply points back at an earlier heap entry.
pub(crate) fn decode_saved_rows(
    dsm_logical: &[u8],
    srs_raw: &[u8],
    memo_raw: &[u8],
    schema: &[SavedFieldDesc],
    field_types: &[FieldValueType],
) -> Option<(Vec<Vec<Option<String>>>, u32)> {
    // Parse the DSM tree once and thread it to every batch-directory reader below.
    let tree = parse_tree_qe(dsm_logical);
    let batches = saved_batches(&tree, dsm_logical);
    let idx_item = batches.first()?.item_size;
    if idx_item == 0 {
        return None;
    }
    let memo_cols = schema.iter().filter(|f| f.is_memo).count() as u32;
    let desc_is = memo_cols * MEMO_CELL_SIZE;

    // Read one row over the schema: inline fields from `idx_rec`, memo fields from `cells`+`heap`.
    let build_row = |idx_rec: &[u8], cells: &[&[u8]], heap: &[u8]| -> Vec<Option<String>> {
        let mut memo_i = 0usize;
        schema
            .iter()
            .map(|f| {
                if f.is_memo {
                    let cell = cells.get(memo_i).copied();
                    memo_i += 1;
                    cell.and_then(|c| {
                        let o = u32::from_le_bytes([c[4], c[5], c[6], c[7]]) as usize;
                        let l = u32::from_le_bytes([c[8], c[9], c[10], c[11]]) as usize;
                        read_memo_cell(heap, o, l)
                    })
                } else if f.rec_offset + 4 <= idx_rec.len() {
                    Some(
                        i32::from_le_bytes([
                            idx_rec[f.rec_offset],
                            idx_rec[f.rec_offset + 1],
                            idx_rec[f.rec_offset + 2],
                            idx_rec[f.rec_offset + 3],
                        ])
                        .to_string(),
                    )
                } else {
                    None
                }
            })
            .collect()
    };

    // Decode the record-index batches (leading run sharing the index width) → flat inline records.
    let idx_counts: Vec<u32> = batches
        .iter()
        .take_while(|b| b.item_size == idx_item)
        .map(|b| b.count)
        .collect();
    let record_count: u32 = idx_counts.iter().sum();
    if record_count == 0 {
        return None;
    }
    // The record-index batch cipher IV keys on the in-memory (persistent) record width, not the
    // on-disk `item_size`. They differ only when string columns are stored **inline** (a packed
    // record, memo-less reports); otherwise they are equal, so this is a no-op for the memo-heap
    // reports. The memo-heap path keeps keying on the on-disk width (unchanged).
    let persistent = persistent_item_size(&tree, dsm_logical).unwrap_or(idx_item);

    // Packed, memo-less records store string columns inline, compacted per batch to that batch's
    // per-column maximum. Each index batch carries its own on-disk `item_size` and its per-column
    // on-disk slot boundaries in its `0x6d` directory entry, so it must be decoded with a per-batch
    // layout (batches of one report can differ in `item_size`).
    if memo_cols == 0 && persistent > idx_item {
        return decode_packed_index(
            &tree,
            dsm_logical,
            srs_raw,
            schema,
            field_types,
            persistent as usize,
        );
    }
    let iv_item = if memo_cols == 0 { persistent } else { idx_item };

    let mut idx_recs: Vec<u8> = Vec::new();
    let mut cursor = 0usize;
    for (k, &c) in idx_counts.iter().enumerate() {
        let Some((inf, consumed)) =
            decode_batch_at(srs_raw, cursor, INDEX_BATCH_SIZE, c, iv_item, k as u32)
        else {
            break;
        };
        let need = c as usize * idx_item as usize;
        if let Some(start) = inf.len().checked_sub(need) {
            idx_recs.extend_from_slice(&inf[start..]);
        }
        cursor += consumed;
    }
    let idx_item = idx_item as usize;

    // No memo columns and not packed (persistent == on-disk) → every field is a fixed-offset scalar
    // slot; emit straight from the index records.
    if memo_cols == 0 {
        let n = idx_recs.len() / idx_item;
        let rows = (0..n)
            .map(|i| build_row(&idx_recs[i * idx_item..(i + 1) * idx_item], &[], &[]))
            .collect();
        return Some((rows, record_count));
    }

    // Decode the memo-descriptor batches (next run) paired with the memo-value heaps.
    let desc_counts: Vec<u32> = batches
        .iter()
        .filter(|b| b.item_size == desc_is)
        .map(|b| b.count)
        .collect();
    if desc_counts.is_empty() {
        return None;
    }
    // Descriptor batches share one capacity (the IV's first word) = the rows that fit a fixed byte budget
    // (`DESC_BATCH_BYTE_BUDGET / item_size`); the first batch's row count only equals it when the
    // batch is full. This capacity is the IV's first word.
    let desc_cap = DESC_BATCH_BYTE_BUDGET / desc_is;
    let heaps = decode_memo_heaps(memo_raw, memo_cols);
    let desc_is_u = desc_is as usize;

    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    let mut global = 0usize;
    for (k, &c) in desc_counts.iter().enumerate() {
        let Some((inf, consumed)) =
            decode_batch_at(srs_raw, cursor, desc_cap, c, desc_is, k as u32)
        else {
            break;
        };
        cursor += consumed;
        let count = c as usize;
        let need = count * desc_is_u;
        let Some(hdr) = inf.len().checked_sub(need) else {
            break;
        };
        let Some(heap) = heaps.get(k) else { break };
        for r in 0..count {
            let drec = &inf[hdr + r * desc_is_u..hdr + (r + 1) * desc_is_u];
            let cell = MEMO_CELL_SIZE as usize;
            let cells: Vec<&[u8]> = (0..memo_cols as usize)
                .map(|ci| &drec[ci * cell..ci * cell + cell])
                .collect();
            let idx_rec = idx_recs
                .get(global * idx_item..(global + 1) * idx_item)
                .unwrap_or(&[]);
            rows.push(build_row(idx_rec, &cells, heap));
            global += 1;
        }
    }
    (!rows.is_empty()).then_some((rows, record_count))
}

/// Build the byte-level view of a report's saved-data batch substrate: the decoded schema, the
/// batch directory, and — per batch — the decrypt IV the decoder *would* derive plus whether it
/// actually yields a zlib header (and, on success, the inflated record region). This is the data
/// behind `rpt dump --saved`; it surfaces the encrypted-batch layer even for batch classes the
/// decoder does not model.
pub(crate) fn inspect_saved_batches(
    dsm_logical: &[u8],
    srs_raw: &[u8],
    memo_raw: &[u8],
    schema: &[SavedFieldDesc],
) -> crate::model::SavedBatchInspection {
    use crate::codec::crypto::cfb_decrypt;
    use crate::model::{SavedBatchInfo, SavedBatchInspection, SavedBatchKind, SavedFieldInfo};

    let tree = parse_tree_qe(dsm_logical);
    let dir = saved_batches(&tree, dsm_logical);
    let memo_cols = schema.iter().filter(|f| f.is_memo).count() as u32;
    let desc_is = memo_cols * MEMO_CELL_SIZE;
    let index_item = dir.first().map(|b| b.item_size).unwrap_or(0);
    // The record-index IV keys on the in-memory (persistent) width for a memo-less packed record;
    // it equals the on-disk width otherwise.
    let persistent = persistent_item_size(&tree, dsm_logical).unwrap_or(index_item);
    let idx_iv_item = if memo_cols == 0 {
        persistent
    } else {
        index_item
    };

    // Per-kind sequence counters and the running ciphertext cursor within each raw stream. Index and
    // descriptor batches sit back-to-back in `SavedRecordsStream`; memo-value batches in `MemoValuesStream`.
    let (mut srs_cursor, mut memo_cursor) = (0usize, 0usize);
    let (mut idx_seq, mut desc_seq, mut memo_seq) = (0u32, 0u32, 0u32);

    let dir_leaves = saved_batch_dir_leaves(&tree, dsm_logical);
    let mut batches = Vec::with_capacity(dir.len());
    for (bi, b) in dir.iter().enumerate() {
        let dir_leaf = dir_leaves.get(bi).cloned().unwrap_or_default();
        let (kind, batch_size, item_count, item_size, seq) = if b.item_size == index_item {
            let s = idx_seq;
            idx_seq += 1;
            (
                SavedBatchKind::Index,
                INDEX_BATCH_SIZE,
                b.count,
                idx_iv_item,
                s,
            )
        } else if memo_cols > 0 && b.item_size == desc_is {
            let s = desc_seq;
            desc_seq += 1;
            let cap = DESC_BATCH_BYTE_BUDGET.checked_div(desc_is).unwrap_or(0);
            (SavedBatchKind::Descriptor, cap, b.count, desc_is, s)
        } else {
            let s = memo_seq;
            memo_seq += 1;
            (
                SavedBatchKind::MemoValue,
                memo_cols,
                memo_cols.saturating_mul(MEMO_CELL_SIZE),
                MEMO_CELL_SIZE,
                s,
            )
        };

        let in_memo = matches!(kind, SavedBatchKind::MemoValue);
        let (raw, cursor) = if in_memo {
            (memo_raw, memo_cursor)
        } else {
            (srs_raw, srs_cursor)
        };
        let iv = batch_iv4(batch_size, item_count, item_size, seq);
        let ct = raw.get(cursor..).unwrap_or(&[]);
        let ct_head = ct.get(..32.min(ct.len())).unwrap_or(&[]).to_vec();

        let mut decrypts_zlib = false;
        let mut inflated_len = None;
        let mut consumed = None;
        let mut first_record = Vec::new();
        if block0_is_zlib(&iv, ct) {
            decrypts_zlib = true;
            let plain = cfb_decrypt(&iv, ct);
            if let Some((inflated, used)) = inflate_zlib_counted(&plain) {
                inflated_len = Some(inflated.len());
                consumed = Some(used);
                // Records sit at the tail of the batch, after the header + allocation region.
                let need = (b.count as usize).saturating_mul(b.item_size as usize);
                if let Some(start) = inflated.len().checked_sub(need) {
                    let rec_end = (start + b.item_size as usize).min(inflated.len());
                    first_record = inflated.get(start..rec_end).unwrap_or(&[]).to_vec();
                }
                // Advance the cursor of this batch's stream so the next batch is located.
                if in_memo {
                    memo_cursor += used;
                } else {
                    srs_cursor += used;
                }
            }
        }

        batches.push(SavedBatchInfo {
            kind,
            dir_count: b.count,
            dir_item_size: b.item_size,
            seq,
            iv_batch_size: batch_size,
            iv_item_count: item_count,
            iv_item_size: item_size,
            iv: iv.to_vec(),
            in_memo_stream: in_memo,
            cursor,
            decrypts_zlib,
            inflated_len,
            consumed,
            first_record,
            ct_head,
            dir_leaf,
        });
    }

    SavedBatchInspection {
        schema: schema
            .iter()
            .map(|f| SavedFieldInfo {
                rec_offset: f.rec_offset,
                name: f.name.clone(),
                is_memo: f.is_memo,
            })
            .collect(),
        memo_cols,
        srs_len: srs_raw.len(),
        memo_len: memo_raw.len(),
        batches,
    }
}

/// Decode a report's stored saved data from its `SavedRecordsStream` (record index) and
/// `MemoValuesStream` (variable-length values). Returns the stored records — not the engine's
/// result rowset, which projects/reorders/groups/formats them. `None` when there is no saved data,
/// no `MemoValuesStream`, or the streams do not decode.
pub(crate) fn decode_saved_data(
    streams: &[RecordStream],
    report: &crate::model::Report,
) -> Option<crate::model::SavedData> {
    use crate::model::{SavedColumn, SavedData};

    let find = |pred: fn(&StreamId) -> bool| streams.iter().find(|s| pred(s.id()));
    // The top-level `DataSourceManager` variant is inherently non-subdocument (nested streams stay
    // `StreamId::Other`), so no explicit Subdocument exclusion is needed. Its logical payload is
    // decoded once at stream-decode time.
    let dsm = find(|id| matches!(id, StreamId::DataSourceManager(_)))?.logical_bytes();
    if dsm.is_empty() {
        return None;
    }

    // Decodable only when the field values are in an external MemoValuesStream. Reports with no memo
    // columns (all-inline) still decode: the memo stream may be absent.
    let memo_raw = find(|id| matches!(id, StreamId::MemoValuesStream(_)))
        .map(|s| s.encode())
        .unwrap_or_default();
    let srs_raw = find(|id| matches!(id, StreamId::SavedRecordsStream(_)))?.encode();

    let schema = saved_schema(dsm);
    if schema.is_empty() {
        return None;
    }
    // Each stored column's value type: a memo column is a `PersistentMemo`; every other column takes
    // its declared type from the report's database field of the same qualified name (the inline
    // packed reader keys the on-disk field width on this — a `Number` is an 8-byte double, an
    // `Int32s` is 4 bytes, a `String` is a NUL-terminated UTF-16 run). Unmatched fields fall back to
    // `Int32s`.
    let field_types = saved_field_types(&schema, report);
    // The stored rows: index batches (inline fields, packed or fixed) + memo-descriptor batches whose
    // cells point into the memo-value heaps (no delta reconstruction needed).
    let (rows, record_count) = decode_saved_rows(dsm, &srs_raw, &memo_raw, &schema, &field_types)?;
    if rows.is_empty() {
        return None;
    }
    let columns = schema
        .iter()
        .zip(field_types)
        .map(|(f, value_type)| SavedColumn {
            name: f.name.clone(),
            value_type,
        })
        .collect();
    Some(SavedData {
        record_count,
        columns,
        rows,
    })
}

/// Resolve each saved column's value type (schema order): a memo column is a `PersistentMemo`; every
/// other column takes the declared type of the report database field with the same qualified name
/// (`Table.Field`, matched on both the table's stored name and its alias), defaulting to `Int32s`.
/// This is what tells the inline row reader a `Number` column is an 8-byte double vs an `Int32s`
/// 4-byte scalar vs a `String` — the DSM saved-field catalog itself carries no type code.
fn saved_field_types(
    schema: &[SavedFieldDesc],
    report: &crate::model::Report,
) -> Vec<FieldValueType> {
    use std::collections::HashMap;
    let mut by_name: HashMap<String, FieldValueType> = HashMap::new();
    for t in &report.database.tables {
        for f in &t.data_fields {
            by_name
                .entry(format!("{}.{}", t.name, f.name))
                .or_insert(f.value_type);
            if !t.alias.is_empty() {
                by_name
                    .entry(format!("{}.{}", t.alias, f.name))
                    .or_insert(f.value_type);
            }
        }
    }
    schema
        .iter()
        .map(|f| {
            if f.is_memo {
                FieldValueType::PersistentMemo
            } else {
                by_name
                    .get(&f.name)
                    .copied()
                    .unwrap_or(FieldValueType::Int32s)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crack::{batch_iv, batch_iv4, cfb_encrypt};

    /// A minimal DSM node for the synthetic-tree builder below.
    struct Node {
        rtype: u16,
        leaf: Vec<u8>,
        children: Vec<Node>,
    }

    /// Emit a QESession-dialect record tree to its masked logical bytes (the exact framing
    /// `parse_tree_qe` reads): an 8-byte header (`0xf8`, type low byte, zero subtype word, 4-byte
    /// big-endian content length), then the leaf bytes followed by each child, all XOR-masked by the
    /// running stack mask (`header_mask ^ type` for the content).
    fn emit(node: &Node, header_mask: u8) -> Vec<u8> {
        let child_mask = header_mask ^ node.rtype as u8;
        let mut content: Vec<u8> = node.leaf.iter().map(|&b| b ^ child_mask).collect();
        for c in &node.children {
            content.extend(emit(c, child_mask));
        }
        let mut hdr = vec![0xf8u8, node.rtype as u8, 0x00, 0x00];
        hdr.extend_from_slice(&(content.len() as u32).to_be_bytes());
        let mut out: Vec<u8> = hdr.iter().map(|&b| b ^ header_mask).collect();
        out.extend(content);
        out
    }

    /// `[count u32 BE][item_size u32 BE]` — one `0x6d` batch directory entry leaf.
    fn batch_leaf(count: u32, item_size: u32) -> Vec<u8> {
        let mut v = count.to_be_bytes().to_vec();
        v.extend_from_slice(&item_size.to_be_bytes());
        v
    }

    /// A memo heap entry: `[u32 byte_len][utf16le value][00 00]`, byte_len including the trailing NUL.
    fn memo_entry(s: &str) -> Vec<u8> {
        let mut v = (s.len() as u32 * 2 + 2).to_le_bytes().to_vec();
        for c in s.encode_utf16() {
            v.extend_from_slice(&c.to_le_bytes());
        }
        v.extend_from_slice(&[0, 0]);
        v
    }

    #[test]
    fn decode_saved_rows_joins_index_descriptor_and_memo_batches() {
        // Two rows over a schema of one inline int column and one memo (string) column. The DSM
        // directory lists an index batch (item_size 4) then a memo-descriptor batch (item_size 12);
        // the record index lives in `SavedRecordsStream`, the memo values in `MemoValuesStream`.
        let schema = [
            SavedFieldDesc {
                rec_offset: 0,
                name: "T.id".into(),
                is_memo: false,
            },
            SavedFieldDesc {
                rec_offset: 0,
                name: "T.name".into(),
                is_memo: true,
            },
        ];
        let field_types = [FieldValueType::Int32s, FieldValueType::PersistentMemo];
        let idx_item = 4u32;
        let count = 2u32;
        let desc_is = MEMO_CELL_SIZE; // one memo column

        // DSM: a `0x2d` structure record whose leaf carries the persistent width (BE u16) and whose
        // children are the two `0x6d` directory entries.
        let dsm = emit(
            &Node {
                rtype: 0x2d,
                leaf: (idx_item as u16).to_be_bytes().to_vec(),
                children: vec![
                    Node {
                        rtype: 0x6d,
                        leaf: batch_leaf(count, idx_item),
                        children: vec![],
                    },
                    Node {
                        rtype: 0x6d,
                        leaf: batch_leaf(count, desc_is),
                        children: vec![],
                    },
                ],
            },
            0,
        );

        // Index records: one 4-byte little-endian int per row, at the tail of the inflated batch.
        let mut idx_records = Vec::new();
        idx_records.extend_from_slice(&10i32.to_le_bytes());
        idx_records.extend_from_slice(&20i32.to_le_bytes());

        // Memo heap: "Ann" at offset 0, "Bob" at offset 12.
        let ann = memo_entry("Ann");
        let bob_off = ann.len() as u32;
        let mut heap = ann;
        heap.extend(memo_entry("Bob"));

        // Descriptor records: `[u16 col][u16 flag][u32 heap_offset LE][u32 byte_len LE]` per memo cell.
        let desc_rec = |heap_off: u32, byte_len: u32| -> Vec<u8> {
            let mut v = vec![0u8, 0, 0, 0]; // col 0, flag 0
            v.extend_from_slice(&heap_off.to_le_bytes());
            v.extend_from_slice(&byte_len.to_le_bytes());
            v
        };
        let mut desc_records = desc_rec(0, 8);
        desc_records.extend(desc_rec(bob_off, 8));

        // Encrypt each batch with the IV the decoder derives (see `batch_iv4`/`batch_iv`).
        let zlib = |b: &[u8]| miniz_oxide::deflate::compress_to_vec_zlib(b, 6);
        let mut srs = cfb_encrypt(
            &batch_iv4(INDEX_BATCH_SIZE, count, idx_item, 0),
            &zlib(&idx_records),
        );
        let desc_cap = DESC_BATCH_BYTE_BUDGET / desc_is;
        srs.extend(cfb_encrypt(
            &batch_iv4(desc_cap, count, desc_is, 0),
            &zlib(&desc_records),
        ));
        let memo = cfb_encrypt(&batch_iv(1, desc_is, MEMO_CELL_SIZE), &zlib(&heap));

        let (rows, record_count) =
            decode_saved_rows(&dsm, &srs, &memo, &schema, &field_types).expect("decode");
        assert_eq!(record_count, 2);
        assert_eq!(
            rows,
            vec![
                vec![Some("10".to_string()), Some("Ann".to_string())],
                vec![Some("20".to_string()), Some("Bob".to_string())],
            ]
        );
    }
}
