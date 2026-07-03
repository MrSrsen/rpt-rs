//! Saved-data (stored rows) decode.
//!
//! A report saved with data caches its rows in `SavedRecordsStream` (the fixed-width record index)
//! and `MemoValuesStream` (variable-length field values). Each stream is a batch: `zlib(records)`
//! encrypted with the `Contents` modified-AES-CFB cipher. The batch IV is
//! `[batch_size u32 LE | item_count u32 LE | item_size u32 LE | 0 u16]`; `item_count` and `item_size`
//! come from the `DataSourceManager` `0x6d` batch headers, and `batch_size` is a per-batch-type
//! default (record index = 1000).

use super::crypto::{cfb_decrypt, encrypt_block};
use super::tree::{parse_tree_qe, RecordNode};

/// Batch size of the record-index batch (`SavedRecordsStream`).
pub(crate) const INDEX_BATCH_SIZE: u32 = 1000;

/// One saved-data batch descriptor from the `DataSourceManager` directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BatchDesc {
    /// Number of items (records) in the batch.
    pub count: u32,
    /// Fixed per-item byte width.
    pub item_size: u32,
}

/// Parse the saved-data batch directory from a decoded `DataSourceManager` stream. Returns the batch
/// headers in file order, or empty when the stream carries no saved-data structure.
pub(crate) fn batch_directory(dsm_logical: &[u8]) -> Vec<BatchDesc> {
    let tree = parse_tree_qe(dsm_logical);
    let mut out = Vec::new();
    fn walk(n: &RecordNode, lg: &[u8], out: &mut Vec<BatchDesc>) {
        // Batch headers are `0x6d` records: `count` (big-endian u32) at [0..4], `item_size` at [4..8].
        if n.rtype == 0x6d {
            let leaf = n.leaf_bytes(lg);
            if leaf.len() >= 8 {
                let count = u32::from_be_bytes([leaf[0], leaf[1], leaf[2], leaf[3]]);
                let item_size = u32::from_be_bytes([leaf[4], leaf[5], leaf[6], leaf[7]]);
                // A batch header has a positive item width; guard against spurious `0x6d` matches.
                if item_size > 0 && item_size < 0x1_0000 && count < 0x0100_0000 {
                    out.push(BatchDesc { count, item_size });
                }
            }
        }
        for c in &n.children {
            walk(c, lg, out);
        }
    }
    // Batch headers live under the saved-records structure record (`0x2d`).
    fn under_2d(n: &RecordNode, lg: &[u8], out: &mut Vec<BatchDesc>) {
        if n.rtype == 0x2d {
            walk(n, lg, out);
        } else {
            for c in &n.children {
                under_2d(c, lg, out);
            }
        }
    }
    for r in &tree {
        under_2d(r, dsm_logical, &mut out);
    }
    out
}

/// The report's saved record count — the item count of the primary (largest) batch in the
/// `DataSourceManager` directory. `None` when there is no saved data.
pub(crate) fn saved_record_count(dsm_logical: &[u8]) -> Option<u32> {
    batch_directory(dsm_logical)
        .into_iter()
        .map(|b| b.count)
        .max()
        .filter(|&c| c > 0)
}

fn batch_iv(batch_size: u32, item_count: u32, item_size: u32) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[0..4].copy_from_slice(&batch_size.to_le_bytes());
    iv[4..8].copy_from_slice(&item_count.to_le_bytes());
    iv[8..12].copy_from_slice(&item_size.to_le_bytes());
    iv
}

/// Decode one saved-data batch to its inflated record bytes: CFB-decrypt with the IV built from the
/// batch metadata, then zlib-inflate. `None` if the metadata is wrong (decrypted block 0 is not a
/// zlib header) or the stream is not a saved batch.
pub(crate) fn decode_saved_batch(
    ciphertext: &[u8],
    batch_size: u32,
    item_count: u32,
    item_size: u32,
) -> Option<Vec<u8>> {
    if ciphertext.len() < 16 {
        return None;
    }
    let iv = batch_iv(batch_size, item_count, item_size);
    // Cheap block-0 zlib-magic check before the full decrypt + inflate.
    let ks = encrypt_block(&iv);
    if ciphertext[0] ^ ks[0] != 0x78 {
        return None;
    }
    let plain = cfb_decrypt(&iv, ciphertext);
    miniz_oxide::inflate::decompress_to_vec_zlib(&plain).ok()
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
        if n.rtype == 0x41 && parent == 0x07 {
            let hdr = n.leaf_bytes(lg);
            if hdr.len() >= 4 {
                let rec_offset = hdr[3] as usize;
                if let Some(desc) = n.children.iter().find(|c| c.rtype == 0x40) {
                    let leaf = desc.leaf_bytes(lg);
                    if leaf.len() >= 4 {
                        let nl = u32::from_be_bytes([leaf[0], leaf[1], leaf[2], leaf[3]]) as usize;
                        if 4 + nl <= leaf.len() {
                            let name = String::from_utf8_lossy(&leaf[4..4 + nl])
                                .trim_end_matches('\0')
                                .to_owned();
                            let trailer = &leaf[4 + nl..];
                            let is_memo = trailer.windows(2).any(|w| w == [0xff, 0xff]);
                            out.push(SavedFieldDesc { rec_offset, name, is_memo });
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

/// The memo batch size `bs` for `MemoValuesStream`: big-endian `u16` at `DataSourceManager` record
/// `0x05`, leaf offset 4 (the memo/string column count). The memo batch IV is `(bs, bs*12, 12)`.
pub(crate) fn memo_batch_size(dsm_logical: &[u8]) -> Option<u32> {
    let tree = parse_tree_qe(dsm_logical);
    fn find(n: &RecordNode, lg: &[u8]) -> Option<u32> {
        if n.rtype == 0x05 {
            let leaf = n.leaf_bytes(lg);
            if leaf.len() >= 6 {
                return Some(u16::from_be_bytes([leaf[4], leaf[5]]) as u32);
            }
        }
        n.children.iter().find_map(|c| find(c, lg))
    }
    tree.iter().find_map(|r| find(r, dsm_logical))
}

/// Parse `MemoValuesStream` plaintext into its sequence of values: each entry is a `u32` LE byte
/// length (including the trailing UTF-16 NUL) followed by that many UTF-16LE bytes.
pub(crate) fn parse_memo_values(memo_plain: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut o = 0usize;
    while o + 4 <= memo_plain.len() {
        let len = u32::from_le_bytes([
            memo_plain[o],
            memo_plain[o + 1],
            memo_plain[o + 2],
            memo_plain[o + 3],
        ]) as usize;
        let start = o + 4;
        if start + len > memo_plain.len() {
            break;
        }
        if len == 0 {
            // Empty value; keep the slot so sequential consumption stays column-aligned.
            out.push(String::new());
            o = start;
            continue;
        }
        let units: Vec<u16> = memo_plain[start..start + len]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        // Drop the trailing NUL unit.
        let s: String = char::decode_utf16(units.into_iter().take_while(|&u| u != 0))
            .map(|r| r.unwrap_or('\u{FFFD}'))
            .collect();
        out.push(s);
        o = start + len;
    }
    out
}

/// Reconstruct stored rows from a decoded `SavedRecordsStream` (`index_plain`) and the parsed
/// `MemoValuesStream` values (`memos`). Each record is `item_size` bytes at
/// `indexLen - count*item_size + i*item_size`; inline fields are 4-byte LE integers at their
/// `rec_offset`, and memo fields take the next value from `memos` in record order.
///
/// Only rows fully covered by `memos` are emitted; a split memo stream may hold fewer values than
/// `count` rows, and trailing uncovered rows are dropped.
pub(crate) fn decode_worrall_rows(
    index_plain: &[u8],
    memos: &[String],
    schema: &[SavedFieldDesc],
    count: u32,
    item_size: u32,
) -> Vec<Vec<Option<String>>> {
    let count = count as usize;
    let item_size = item_size as usize;
    let need = count * item_size;
    if item_size == 0 || need > index_plain.len() {
        return Vec::new();
    }
    let memo_cols = schema.iter().filter(|f| f.is_memo).count();
    let data_start = index_plain.len() - need;
    let mut rows = Vec::with_capacity(count);
    let mut memo_i = 0usize;
    for i in 0..count {
        // Stop once the remaining memo values cannot cover a full row.
        if memo_cols > 0 && memo_i + memo_cols > memos.len() {
            break;
        }
        let rec = &index_plain[data_start + i * item_size..data_start + (i + 1) * item_size];
        let mut row = Vec::with_capacity(schema.len());
        for f in schema {
            if f.is_memo {
                let v = memos.get(memo_i).cloned().filter(|s| !s.is_empty());
                row.push(v);
                memo_i += 1;
            } else if f.rec_offset + 4 <= rec.len() {
                let v = i32::from_le_bytes([
                    rec[f.rec_offset],
                    rec[f.rec_offset + 1],
                    rec[f.rec_offset + 2],
                    rec[f.rec_offset + 3],
                ]);
                row.push(Some(v.to_string()));
            } else {
                row.push(None);
            }
        }
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_batch_with_known_metadata() {
        let original: Vec<u8> = (0..2000u32).flat_map(|i| (i as u16).to_le_bytes()).collect();
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&original, 6);
        let ct = cfb_encrypt(&batch_iv(1000, 249, 30), &z);
        let decoded = decode_saved_batch(&ct, 1000, 249, 30).expect("decode");
        assert_eq!(decoded, original);
    }

    #[test]
    fn wrong_metadata_fails() {
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&[1u8; 500], 6);
        let ct = cfb_encrypt(&batch_iv(1000, 249, 30), &z);
        assert!(decode_saved_batch(&ct, 1000, 248, 30).is_none());
    }

    // CFB-encrypt mirror of `cfb_decrypt` (feedback = ciphertext), for the round-trip test only.
    fn cfb_encrypt(iv: &[u8; 16], plaintext: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(plaintext.len());
        let mut feedback = *iv;
        for chunk in plaintext.chunks(16) {
            let ks = encrypt_block(&feedback);
            let mut block = [0u8; 16];
            for (i, &p) in chunk.iter().enumerate() {
                let c = p ^ ks[i];
                out.push(c);
                block[i] = c;
            }
            if chunk.len() == 16 {
                feedback = block;
            }
        }
        out
    }
}
