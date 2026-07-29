//! Saved-data (stored rows) decode.
//!
//! A report saved with data caches its rows across two streams, decoded by [`decode_saved_rows`]:
//! `SavedRecordsStream` holds the record-**index** batches (a fixed-width record per row: a leading
//! present bitmap, then a fixed slot per inline field — scalars at their natural width, strings as a
//! NUL-terminated UTF-16LE run filling the slot) followed by the memo-**descriptor** batches; and
//! `MemoValuesStream` holds the memo-value heaps (`(u32 len)(utf16z)` entries). Each batch is
//! `zlib(records)` encrypted with the `Contents` modified-AES-CFB cipher.
//!
//! The module is split by responsibility: [`schema`] holds the catalog this layer is handed — the
//! batch directory and the stored-record field list — and the layout facts derived from it;
//! [`decrypt`] is the cipher/IV/inflate layer; [`packed`] decodes the inline-string (memo-less)
//! packed rowset; [`memo`] resolves the memo heaps; [`inline`] reads a row out of the fixed record
//! index; [`inspect`] is the byte-level view of the same batches.
//!
//! There is **no delta / change mask** to reconstruct: each row's memo descriptor holds an explicit
//! per-cell `[u16 col][u16 flag][u32 heap_offset][u32 byte_length]` pointer straight into the memo
//! heap, so a repeated value simply points back at an earlier heap entry.

mod decrypt;
mod inline;
mod inspect;
mod memo;
mod packed;
mod schema;

pub(crate) use inspect::inspect_saved_batches;
pub(crate) use schema::{BatchDesc, SavedBatch, SavedCatalog, SavedFieldDesc};

use crate::coverage::{BatchProblem, SavedDataStatus};
use crate::model::{FieldValueType, SavedBatchKind};

use decrypt::decode_batch_at;
use inline::RowReader;
use memo::decode_memo_heaps;
use packed::decode_packed_index;
use schema::{BatchShape, DESC_BATCH_BYTE_BUDGET, INDEX_BATCH_SIZE, MEMO_CELL_SIZE};

/// The stored rows a report's saved-data batches yielded, and why they are what they are.
///
/// The status is not an afterthought over `rows`: the reasons a rowset comes back empty — a
/// directory listing no record batch, a batch whose cipher key is wrong, a heap that will not
/// inflate — are indistinguishable once the reader has returned, and only the reader that gave up
/// knows which one it met.
#[derive(Debug, Clone, Default)]
pub(crate) struct SavedRowset {
    /// The decoded rows, in record order, each cell in schema order.
    pub rows: Vec<Vec<Option<String>>>,
    /// The row total the batch directory claims, independent of how many actually decoded.
    pub record_count: u32,
    /// What the decode made of the batches.
    pub status: SavedDataStatus,
}

impl SavedRowset {
    /// A rowset that decoded nothing, for the stated reason.
    fn nothing(status: SavedDataStatus) -> SavedRowset {
        SavedRowset {
            status,
            ..SavedRowset::default()
        }
    }

    /// The rowset a finished batch walk produced. The status is the first batch that failed, unless
    /// every row the directory claims decoded regardless — a failure beyond the claimed rows cost
    /// nothing.
    fn walked(
        rows: Vec<Vec<Option<String>>>,
        record_count: u32,
        first_problem: Option<SavedDataStatus>,
    ) -> SavedRowset {
        let decoded = if rows.is_empty() {
            SavedDataStatus::NoRows
        } else {
            SavedDataStatus::Decoded {
                rows: rows.len(),
                stored: record_count,
            }
        };
        SavedRowset {
            rows,
            record_count,
            status: match first_problem {
                Some(p) if !decoded.is_complete() => p,
                _ => decoded,
            },
        }
    }
}

/// Reconstruct the stored saved-data rows from the raw `SavedRecordsStream` (`srs_raw`, holding the
/// record-index batches then the memo-descriptor batches) and `MemoValuesStream` (`memo_raw`, the
/// memo-value heaps).
///
/// Each row's **inline** fields (integers, dates as day counts, …) are read from the fixed record
/// index at the schema `rec_offset`. Each row's **memo** fields are read via the memo descriptor: a
/// per-row `memo_cols × 12` record whose 12-byte cells are `[u16 col][u16 flag][u32 heap_offset]
/// [u32 byte_length]` pointing directly into the corresponding memo-value batch heap. This is an
/// explicit per-cell pointer — there is no delta/change-mask to reconstruct: an
/// "unchanged" cell simply points back at an earlier heap entry.
pub(crate) fn decode_saved_rows(
    catalog: &SavedCatalog,
    srs_raw: &[u8],
    memo_raw: &[u8],
    field_types: &[FieldValueType],
) -> SavedRowset {
    let shape = BatchShape::of(catalog);
    // A directory with no entry at all describes no batch, let alone a record index.
    if catalog.batches.is_empty() {
        return SavedRowset::nothing(SavedDataStatus::NoRecordBatches);
    }
    // A memo-value heap is the one batch class with no per-item width, so a directory opening with
    // one carries no record index at all.
    if shape.index_item == 0 {
        return SavedRowset::nothing(SavedDataStatus::MemoValuesOnly);
    }
    // The record index is the leading run of batches sharing the index width.
    let index_counts: Vec<u32> = catalog
        .descriptors()
        .iter()
        .take_while(|b| b.item_size == shape.index_item)
        .map(|b| b.count)
        .collect();
    if index_counts.iter().all(|&c| c == 0) {
        return SavedRowset::nothing(SavedDataStatus::NoRecordBatches);
    }
    // A packed record needs a per-batch layout: its string columns are stored inline, compacted to
    // each batch's own per-column maxima, so the on-disk widths live in the batch's directory entry
    // and batches of one report differ in `item_size`.
    if shape.is_packed() {
        return decode_packed_index(catalog, srs_raw, field_types, shape.persistent as usize);
    }

    let index = read_index_records(srs_raw, &index_counts, &shape);
    let reader = RowReader::new(&catalog.fields, field_types, shape.index_item as usize);

    // No memo column and not packed → every field is a fixed-offset slot in the index record, so
    // the rows are the records.
    if shape.memo_cols == 0 {
        let rows = index
            .records
            .chunks_exact(shape.index_item as usize)
            .map(|rec| reader.row(rec, &[], &[]))
            .collect();
        return SavedRowset::walked(rows, index.record_count, index.problem);
    }
    read_memo_rows(catalog, srs_raw, memo_raw, &shape, &index, &reader)
}

/// What the record-index batch run yielded.
#[derive(Debug, Default)]
struct IndexRecords {
    /// The row total the run's directory entries claim, independent of how many decoded.
    record_count: u32,
    /// The inflated fixed-width records of every batch that decoded, back to back.
    records: Vec<u8>,
    /// Where the run left the ciphertext cursor — the offset the memo-descriptor batches begin at.
    cursor: usize,
    /// The first batch that would not decode.
    problem: Option<SavedDataStatus>,
}

/// Decode the record-index batches — the leading run of `SavedRecordsStream`, one per entry of
/// `counts` — into their concatenated fixed-width records.
///
/// A batch that will not decode ends the walk: batches of one kind sit back to back and the next
/// one's offset is this one's consumed length, so everything past it is unreachable and that first
/// failure is the whole account of what was lost. A batch that decodes but carries fewer bytes than
/// its record count claims is only recorded — its consumed length still locates the next.
fn read_index_records(srs_raw: &[u8], counts: &[u32], shape: &BatchShape) -> IndexRecords {
    let mut index = IndexRecords {
        record_count: counts.iter().sum(),
        ..IndexRecords::default()
    };
    for (k, &c) in counts.iter().enumerate() {
        let (inf, consumed) = match decode_batch_at(
            srs_raw,
            index.cursor,
            INDEX_BATCH_SIZE,
            c,
            shape.index_iv_item,
            k as u32,
        ) {
            Ok(batch) => batch,
            Err(problem) => {
                index
                    .problem
                    .get_or_insert(batch_failed(SavedBatchKind::Index, k, problem));
                break;
            }
        };
        // The records sit at the tail of the inflated batch, after its header and allocation region.
        let need = c as usize * shape.index_item as usize;
        match inf.len().checked_sub(need) {
            Some(start) => index.records.extend_from_slice(&inf[start..]),
            None => {
                index.problem.get_or_insert(batch_failed(
                    SavedBatchKind::Index,
                    k,
                    BatchProblem::Short,
                ));
            }
        }
        index.cursor += consumed;
    }
    index
}

/// Read the rows of a record with memo columns: walk the memo-descriptor batches — the run that
/// follows the record index in `SavedRecordsStream` — each paired with the memo-value heap at the
/// same position, and join each descriptor record to its index record.
fn read_memo_rows(
    catalog: &SavedCatalog,
    srs_raw: &[u8],
    memo_raw: &[u8],
    shape: &BatchShape,
    index: &IndexRecords,
    reader: &RowReader,
) -> SavedRowset {
    let mut first_problem = index.problem;
    let desc_counts: Vec<u32> = catalog
        .descriptors()
        .iter()
        .filter(|b| b.item_size == shape.desc_item)
        .map(|b| b.count)
        .collect();
    if desc_counts.is_empty() {
        // The record has memo columns, so every row's variable-length cells live behind a
        // descriptor batch that the directory does not list.
        first_problem.get_or_insert(batch_failed(
            SavedBatchKind::Descriptor,
            0,
            BatchProblem::Absent,
        ));
        return SavedRowset::walked(Vec::new(), index.record_count, first_problem);
    }
    // Descriptor batches share one capacity — the rows that fit a fixed byte budget
    // (`DESC_BATCH_BYTE_BUDGET / item_size`), which is the IV's first word. A batch's own row count
    // equals it only when the batch is full.
    let desc_cap = DESC_BATCH_BYTE_BUDGET / shape.desc_item;
    let heaps = decode_memo_heaps(memo_raw, shape.memo_cols);
    let desc_item = shape.desc_item as usize;
    let index_item = shape.index_item as usize;

    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    let mut cursor = index.cursor;
    let mut global = 0usize;
    for (k, &c) in desc_counts.iter().enumerate() {
        let (inf, consumed) =
            match decode_batch_at(srs_raw, cursor, desc_cap, c, shape.desc_item, k as u32) {
                Ok(batch) => batch,
                Err(problem) => {
                    first_problem.get_or_insert(batch_failed(
                        SavedBatchKind::Descriptor,
                        k,
                        problem,
                    ));
                    break;
                }
            };
        cursor += consumed;
        let count = c as usize;
        let need = count * desc_item;
        let Some(hdr) = inf.len().checked_sub(need) else {
            first_problem.get_or_insert(batch_failed(
                SavedBatchKind::Descriptor,
                k,
                BatchProblem::Short,
            ));
            break;
        };
        // The k-th heap pairs 1:1 with the k-th descriptor batch, so a heap the memo stream did not
        // yield leaves this batch's cells pointing nowhere.
        let Some(heap) = heaps.get(k) else {
            first_problem.get_or_insert(batch_failed(
                SavedBatchKind::MemoValue,
                k,
                BatchProblem::Absent,
            ));
            break;
        };
        for r in 0..count {
            let drec = &inf[hdr + r * desc_item..hdr + (r + 1) * desc_item];
            let cell = MEMO_CELL_SIZE as usize;
            let cells: Vec<&[u8]> = (0..shape.memo_cols as usize)
                .map(|ci| &drec[ci * cell..ci * cell + cell])
                .collect();
            let idx_rec = index
                .records
                .get(global * index_item..(global + 1) * index_item)
                .unwrap_or(&[]);
            rows.push(reader.row(idx_rec, &cells, heap));
            global += 1;
        }
    }
    SavedRowset::walked(rows, index.record_count, first_problem)
}

/// One batch's failure, as the status names it.
fn batch_failed(kind: SavedBatchKind, index: usize, problem: BatchProblem) -> SavedDataStatus {
    SavedDataStatus::BatchUndecodable {
        kind,
        index: index as u32,
        problem,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::crypto::cfb_encrypt;
    use decrypt::{batch_iv, batch_iv4};

    /// A catalog of one index batch and one memo-descriptor batch over `fields`, as the record
    /// layer would have read it.
    fn catalog(item_size: u32, fields: &[SavedFieldDesc], batches: &[(u32, u32)]) -> SavedCatalog {
        SavedCatalog {
            item_size: Some(item_size),
            batches: batches
                .iter()
                .map(|&(count, item_size)| SavedBatch {
                    desc: BatchDesc {
                        count,
                        item_size,
                        stream_off: 0,
                        stream_len: 0,
                    },
                    ..SavedBatch::default()
                })
                .collect(),
            fields: fields.to_vec(),
        }
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

    /// A one-column schema and its zlib-then-CFB index batch of `count` little-endian ints, written
    /// with the IV the directory entry `(count, 4)` derives.
    fn one_column_index(count: u32) -> ([SavedFieldDesc; 1], [FieldValueType; 1], Vec<u8>) {
        let schema = [SavedFieldDesc {
            rec_offset: 0,
            name: "T.id".into(),
            is_memo: false,
        }];
        let records: Vec<u8> = (0..count).flat_map(|i| (i as i32).to_le_bytes()).collect();
        let srs = cfb_encrypt(
            &batch_iv4(INDEX_BATCH_SIZE, count, 4, 0),
            &miniz_oxide::deflate::compress_to_vec_zlib(&records, 6),
        );
        (schema, [FieldValueType::Int32s], srs)
    }

    #[test]
    fn a_batch_the_directory_keys_wrongly_names_itself_rather_than_going_quiet() {
        // The failure that hides best: the ciphertext is present and intact, but the directory
        // figures the IV is built from are not the ones the batch was written with, so nothing
        // decrypts. Read as a bare `None` this is indistinguishable from a report saved with no data.
        let (schema, field_types, srs) = one_column_index(2);

        let misread = catalog(4, &schema, &[(3, 4)]);
        let decoded = decode_saved_rows(&misread, &srs, &[], &field_types);
        assert!(decoded.rows.is_empty());
        assert_eq!(
            decoded.status,
            SavedDataStatus::BatchUndecodable {
                kind: SavedBatchKind::Index,
                index: 0,
                problem: BatchProblem::NotDecrypted,
            }
        );

        // The same bytes under the directory they were written for.
        let decoded = decode_saved_rows(&catalog(4, &schema, &[(2, 4)]), &srs, &[], &field_types);
        assert_eq!(decoded.rows.len(), 2);
        assert!(decoded.status.is_complete());
    }

    #[test]
    fn an_empty_directory_and_a_heap_only_directory_are_told_apart() {
        let (schema, field_types, _) = one_column_index(2);
        assert_eq!(
            decode_saved_rows(&catalog(4, &schema, &[]), &[], &[], &field_types).status,
            SavedDataStatus::NoRecordBatches
        );
        // A memo-value heap is the one class with no per-item width.
        assert_eq!(
            decode_saved_rows(&catalog(4, &schema, &[(2, 0)]), &[], &[], &field_types).status,
            SavedDataStatus::MemoValuesOnly
        );
        // Both are outcomes of a file that stores no rows, not of a decoder that lost them.
        assert!(SavedDataStatus::NoRecordBatches.is_complete());
        assert!(SavedDataStatus::MemoValuesOnly.is_complete());
    }

    #[test]
    fn a_memo_column_with_no_descriptor_batch_names_the_missing_class() {
        // Every memo cell lives behind a descriptor batch, so a directory that lists none leaves the
        // decoded index unreadable — and the class that is missing is the actionable part.
        let (mut schema, field_types, srs) = one_column_index(2);
        schema[0].is_memo = true;
        let decoded = decode_saved_rows(&catalog(4, &schema, &[(2, 4)]), &srs, &[], &field_types);
        assert!(decoded.rows.is_empty());
        assert_eq!(
            decoded.status,
            SavedDataStatus::BatchUndecodable {
                kind: SavedBatchKind::Descriptor,
                index: 0,
                problem: BatchProblem::Absent,
            }
        );
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

        // The catalog: the persistent record width, then an index batch and a memo-descriptor batch.
        let catalog = catalog(idx_item, &schema, &[(count, idx_item), (count, desc_is)]);

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

        let decoded = decode_saved_rows(&catalog, &srs, &memo, &field_types);
        let (rows, record_count) = (decoded.rows, decoded.record_count);
        assert_eq!(record_count, 2);
        assert_eq!(
            decoded.status,
            SavedDataStatus::Decoded { rows: 2, stored: 2 }
        );
        assert_eq!(
            rows,
            vec![
                vec![Some("10".to_string()), Some("Ann".to_string())],
                vec![Some("20".to_string()), Some("Bob".to_string())],
            ]
        );
    }

    #[test]
    fn inline_string_column_reads_its_whole_slot_not_four_bytes() {
        // A `String` column can live INLINE in the fixed index record (not in the memo heap): it
        // occupies its whole record slot — the gap to the next field's offset — as a NUL-terminated
        // UTF-16LE run. Reading the leading 4 bytes as an integer instead returns the first two code
        // units as a number, which collapses every value sharing a two-character prefix into one.
        //
        // Layout of the 34-byte record: [0..2] present bitmap, [2..26] the 24-byte string slot,
        // [26..30] the memo field's slot, [30..34] a 4-byte int.
        let schema = [
            SavedFieldDesc {
                rec_offset: 2,
                name: "T.name".into(),
                is_memo: false,
            },
            SavedFieldDesc {
                rec_offset: 26,
                name: "T.province".into(),
                is_memo: true,
            },
            SavedFieldDesc {
                rec_offset: 30,
                name: "T.id".into(),
                is_memo: false,
            },
        ];
        let field_types = [
            FieldValueType::String,
            FieldValueType::PersistentMemo,
            FieldValueType::Int32s,
        ];
        let idx_item = 34u32;
        let count = 2u32;
        let desc_is = MEMO_CELL_SIZE;

        let catalog = catalog(idx_item, &schema, &[(count, idx_item), (count, desc_is)]);

        // Two index records. Both strings share their first two characters ("Ch"/"Ch"), which is
        // exactly what a 4-byte read cannot tell apart, and both are longer than 4 bytes.
        let idx_rec = |name: &str, id: i32| -> Vec<u8> {
            let mut r = vec![0u8; 34];
            r[0] = 0b101; // bitmap: the two non-memo fields present
            let mut w = 2usize;
            for c in name.encode_utf16() {
                r[w..w + 2].copy_from_slice(&c.to_le_bytes());
                w += 2;
            }
            r[30..34].copy_from_slice(&id.to_le_bytes());
            r
        };
        let mut idx_records = idx_rec("Chestermere", 6);
        idx_records.extend(idx_rec("Chelsea", 7));

        let alberta = memo_entry("Alberta");
        let quebec_off = alberta.len() as u32;
        let mut heap = alberta;
        heap.extend(memo_entry("Quebec"));

        let desc_rec = |heap_off: u32, byte_len: u32| -> Vec<u8> {
            let mut v = vec![0u8, 0, 0, 0];
            v.extend_from_slice(&heap_off.to_le_bytes());
            v.extend_from_slice(&byte_len.to_le_bytes());
            v
        };
        let mut desc_records = desc_rec(0, 16);
        desc_records.extend(desc_rec(quebec_off, 14));

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

        let decoded = decode_saved_rows(&catalog, &srs, &memo, &field_types);
        let (rows, record_count) = (decoded.rows, decoded.record_count);
        assert_eq!(record_count, 2);
        assert_eq!(
            decoded.status,
            SavedDataStatus::Decoded { rows: 2, stored: 2 }
        );
        assert_eq!(
            rows,
            vec![
                vec![
                    Some("Chestermere".to_string()),
                    Some("Alberta".to_string()),
                    Some("6".to_string()),
                ],
                vec![
                    Some("Chelsea".to_string()),
                    Some("Quebec".to_string()),
                    Some("7".to_string()),
                ],
            ]
        );
    }
}
