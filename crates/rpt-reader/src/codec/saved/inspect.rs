//! The byte-level view of a report's saved-data batches, behind `rpt dump --saved`: the decoded
//! schema, the batch directory, and — per batch — the decrypt IV the decoder *would* derive plus
//! whether it actually yields a zlib header. It surfaces the encrypted-batch layer even for batch
//! classes the row decode does not model.

use crate::codec::crypto::cfb_decrypt;
use crate::model::{SavedBatchInfo, SavedBatchInspection, SavedBatchKind, SavedFieldInfo};

use super::decrypt::{batch_iv4, block0_is_zlib, inflate_zlib_counted};
use super::schema::{
    srs_directory, BatchShape, SavedCatalog, DESC_BATCH_BYTE_BUDGET, INDEX_BATCH_SIZE,
    MEMO_CELL_SIZE,
};

/// Build the per-batch byte-level view of a report's saved data.
pub(crate) fn inspect_saved_batches(
    catalog: &SavedCatalog,
    srs_raw: &[u8],
    memo_raw: &[u8],
) -> SavedBatchInspection {
    let shape = BatchShape::of(catalog);
    let dir = catalog.descriptors();

    // Per-kind sequence counters and the running ciphertext cursor within each raw stream. Index and
    // descriptor batches sit back-to-back in `SavedRecordsStream`; memo-value batches in `MemoValuesStream`.
    let (mut srs_cursor, mut memo_cursor) = (0usize, 0usize);
    let (mut idx_seq, mut desc_seq, mut memo_seq) = (0u32, 0u32, 0u32);

    // Which physical stream each entry belongs to is decided by the byte chain, not by width: the
    // directory's `SavedRecordsStream` entries run contiguously from offset 0, and the first entry
    // that restarts the chain opens `MemoValuesStream`. A packed report's index batches each carry
    // their own compacted width, so classifying on `item_size` alone reports every batch after the
    // first as a memo-value batch — the misreading that hides a multi-batch index.
    let srs_run = srs_directory(&dir).len();

    let mut batches = Vec::with_capacity(dir.len());
    for (bi, entry) in catalog.batches.iter().enumerate() {
        let b = &entry.desc;
        let dir_entry = entry.bytes.clone();
        let in_srs = bi < srs_run;
        let is_desc = in_srs && shape.memo_cols > 0 && b.item_size == shape.desc_item;
        let (kind, batch_size, item_count, item_size, seq) = if in_srs && !is_desc {
            let s = idx_seq;
            idx_seq += 1;
            (
                SavedBatchKind::Index,
                INDEX_BATCH_SIZE,
                b.count,
                shape.index_iv_item,
                s,
            )
        } else if is_desc {
            let s = desc_seq;
            desc_seq += 1;
            let cap = DESC_BATCH_BYTE_BUDGET
                .checked_div(shape.desc_item)
                .unwrap_or(0);
            (SavedBatchKind::Descriptor, cap, b.count, shape.desc_item, s)
        } else {
            let s = memo_seq;
            memo_seq += 1;
            (
                SavedBatchKind::MemoValue,
                shape.memo_cols,
                shape.memo_cols.saturating_mul(MEMO_CELL_SIZE),
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
            dir_entry,
        });
    }

    SavedBatchInspection {
        schema: catalog
            .fields
            .iter()
            .map(|f| SavedFieldInfo {
                rec_offset: f.rec_offset,
                name: f.name.clone(),
                is_memo: f.is_memo,
            })
            .collect(),
        memo_cols: shape.memo_cols,
        srs_len: srs_raw.len(),
        memo_len: memo_raw.len(),
        batches,
    }
}
