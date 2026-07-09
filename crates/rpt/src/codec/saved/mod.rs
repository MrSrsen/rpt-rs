//! Saved-data (stored rows) decode.
//!
//! A report saved with data caches its rows across two streams, decoded by [`decode_saved_rows`]:
//! `SavedRecordsStream` holds the record-**index** batches (a fixed-width record per row: byte-0
//! present bitmap + inline integer/date fields) followed by the memo-**descriptor** batches; and
//! `MemoValuesStream` holds the memo-value heaps (`(u32 len)(utf16z)` entries). Each batch is
//! `zlib(records)` encrypted with the `Contents` modified-AES-CFB cipher.
//!
//! The module is split by responsibility: [`schema`] parses the `DataSourceManager` batch directory
//! and stored-record field catalog; [`crack`] is the cipher/IV/inflate layer and the IV brute-force;
//! [`packed`] decodes the inline-string (memo-less) packed rowset; [`memo`] resolves the memo heaps.
//!
//! There is **no delta / change mask** to reconstruct: each row's memo descriptor holds an explicit
//! per-cell `[u16 col][u16 flag][u32 heap_offset][u32 byte_length]` pointer straight into the memo
//! heap, so a repeated value simply points back at an earlier heap entry.

mod crack;
mod memo;
mod packed;
mod schema;

pub(crate) use crack::{decode_index_stream, saved_iv_search};
pub(crate) use schema::{index_directory, saved_record_count, saved_schema, SavedFieldDesc};

use crate::model::FieldValueType;

use crack::{batch_iv4, block0_is_zlib, decode_batch_at, inflate_zlib_counted};
use memo::{decode_memo_heaps, read_memo_cell};
use packed::decode_packed_index;
use schema::{
    persistent_item_size, saved_batch_dir_leaves, saved_batches, DESC_BATCH_BYTE_BUDGET,
    INDEX_BATCH_SIZE,
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
    let batches = saved_batches(dsm_logical);
    let idx_item = batches.first()?.item_size;
    if idx_item == 0 {
        return None;
    }
    let memo_cols = schema.iter().filter(|f| f.is_memo).count() as u32;
    let desc_is = memo_cols * 12;

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
    let persistent = persistent_item_size(dsm_logical).unwrap_or(idx_item);

    // Packed, memo-less records store string columns inline, compacted per batch to that batch's
    // per-column maximum. Each index batch carries its own on-disk `item_size` and its per-column
    // on-disk slot boundaries in its `0x6d` directory entry, so it must be decoded with a per-batch
    // layout (batches of one report can differ in `item_size`).
    if memo_cols == 0 && persistent > idx_item {
        return decode_packed_index(
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
            let cells: Vec<&[u8]> = (0..memo_cols as usize)
                .map(|ci| &drec[ci * 12..ci * 12 + 12])
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

/// Build the reverse-engineering view of a report's saved-data batch substrate: the decoded schema,
/// the batch directory, and — per batch — the decrypt IV the decoder *would* derive plus whether it
/// actually yields a zlib header (and, on success, the inflated record region). This is the data
/// behind `rpt dump --saved`; it surfaces the encrypted-batch layer for cracking a new class.
pub(crate) fn inspect_saved_batches(
    dsm_logical: &[u8],
    srs_raw: &[u8],
    memo_raw: &[u8],
    schema: &[SavedFieldDesc],
) -> crate::model::SavedBatchInspection {
    use crate::codec::crypto::cfb_decrypt;
    use crate::model::{SavedBatchInfo, SavedBatchInspection, SavedBatchKind, SavedFieldInfo};

    let dir = saved_batches(dsm_logical);
    let memo_cols = schema.iter().filter(|f| f.is_memo).count() as u32;
    let desc_is = memo_cols * 12;
    let index_item = dir.first().map(|b| b.item_size).unwrap_or(0);
    // The record-index IV keys on the in-memory (persistent) width for a memo-less packed record;
    // it equals the on-disk width otherwise.
    let persistent = persistent_item_size(dsm_logical).unwrap_or(index_item);
    let idx_iv_item = if memo_cols == 0 {
        persistent
    } else {
        index_item
    };

    // Per-kind sequence counters and the running ciphertext cursor within each raw stream. Index and
    // descriptor batches sit back-to-back in `SavedRecordsStream`; memo-value batches in `MemoValuesStream`.
    let (mut srs_cursor, mut memo_cursor) = (0usize, 0usize);
    let (mut idx_seq, mut desc_seq, mut memo_seq) = (0u32, 0u32, 0u32);

    let dir_leaves = saved_batch_dir_leaves(dsm_logical);
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
                memo_cols.saturating_mul(12),
                12,
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
