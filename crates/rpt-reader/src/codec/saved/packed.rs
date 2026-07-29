//! The packed-rowset decode: memo-less reports store their string columns inline, compacted per
//! batch, so each index batch is decoded with its own on-disk `item_size` and per-column slot
//! widths resolved from its directory entry's column table.

use crate::coverage::{BatchProblem, SavedDataStatus};
use crate::model::{FieldValueType, SavedBatchKind};

use super::decrypt::decode_batch_at;
use super::schema::{SavedCatalog, SavedFieldDesc, INDEX_BATCH_SIZE, MAX_ITEM_SIZE};
use super::{batch_failed, SavedRowset};

/// The widest inline buffer read as a scalar: a value type with no fixed native size is a scalar of
/// its buffer's width up to this, and a string beyond it.
const MAX_SCALAR_WIDTH: usize = 8;

/// A batch's column table holds three values per inline string column, of which the third is the
/// on-disk offset of the field that follows that column.
const COLUMN_TABLE_STRIDE: usize = 3;
const COLUMN_NEXT_FIELD_OFFSET: usize = 2;

/// How a saved inline field is stored on disk in a packed record, from its declared value type:
/// a variable UTF-16LE string, or a fixed-width little-endian scalar of a given byte size read as a
/// signed integer or an IEEE-754 double.
#[derive(Clone, Copy)]
enum PackedField {
    Str,
    Int(usize),
    Float(usize),
}

/// Resolve a field's on-disk packed encoding from its declared value type, falling back to the
/// in-memory buffer `width` for types with no fixed native size (an inline buffer `<= 8` bytes is a
/// scalar of that width, wider is a string). Scalars are stored at their natural size (`Int32s` = 4,
/// `Number`/`Currency` = an 8-byte double, `Date`/`Time` = a 4-byte day/second count).
fn packed_field(vt: FieldValueType, width: usize) -> PackedField {
    match vt {
        FieldValueType::String | FieldValueType::Blob | FieldValueType::PersistentMemo => {
            PackedField::Str
        }
        FieldValueType::Int8s => PackedField::Int(1),
        FieldValueType::Int16s => PackedField::Int(2),
        FieldValueType::Int32s | FieldValueType::Int32u => PackedField::Int(4),
        FieldValueType::Date | FieldValueType::Time => PackedField::Int(4),
        FieldValueType::DateTime => PackedField::Int(8),
        FieldValueType::Number | FieldValueType::Currency => PackedField::Float(8),
        // Boolean / Unknown / Other: use the in-memory buffer width to tell scalar from string.
        _ if (1..=MAX_SCALAR_WIDTH).contains(&width) => PackedField::Int(width),
        _ => PackedField::Str,
    }
}

/// Format a fixed-width little-endian signed-integer scalar cell as its stored decimal string.
pub(super) fn int_cell_to_string(b: &[u8]) -> String {
    let mut v = 0i64;
    for (i, &x) in b.iter().take(8).enumerate() {
        v |= (x as i64) << (8 * i);
    }
    // Sign-extend from the field's width.
    let bits = (b.len().min(8) * 8) as u32;
    if bits < 64 && v & (1 << (bits - 1)) != 0 {
        v |= -1i64 << bits;
    }
    v.to_string()
}

/// The factor a stored `Number`/`Currency` cell is scaled by on the way into the saved batch. The
/// engine multiplies the value by 100 as a plain `f64` — not an OLE `CY` integer scale — before
/// writing it, so the stored double is the value in hundredths, floating-point rounding included, and
/// reading it back means dividing by this same `f64` constant. The same constant already applies to
/// the `Number`/`Currency` parameter values decoded from the `Contents` stream.
pub(super) const SAVED_NUMERIC_SCALE: f64 = 100.0;

/// Format an 8-byte IEEE-754 double scalar cell (a stored `Number`/`Currency`) as its shortest
/// round-tripping decimal, undoing the [`SAVED_NUMERIC_SCALE`] the engine applied when it wrote the
/// batch.
///
/// The scaling belongs here rather than in a consumer: `"1025910"` is a perfectly valid unscaled
/// number, so nothing downstream can tell a scaled cell from an unscaled one without knowing which
/// `RowSource` it came from — which is exactly the knowledge that trait exists to
/// hide. A live-database row of the same column arrives already unscaled, and the two must agree.
pub(super) fn float_cell_to_string(b: &[u8]) -> String {
    if b.len() == 8 {
        let scaled = f64::from_le_bytes(b.try_into().unwrap_or_default());
        (scaled / SAVED_NUMERIC_SCALE).to_string()
    } else {
        // A slot too short to hold a double is a truncated record, not a scaled value; read what is
        // there rather than scaling a number that was never written whole.
        int_cell_to_string(b)
    }
}

/// The per-field in-memory buffer widths (schema order) and the present-bitmap byte width for a
/// saved record. A field's width is the gap to the next-higher field offset (the last field runs to
/// `persistent`); the bitmap width is the smallest field offset (values begin right after the
/// bitmap). On the packed path the width is only a fallback for typing — the declared value type is
/// authoritative; on the fixed (memo-heap) path it is the field's actual slot width, which is what a
/// variable-length inline column needs.
pub(super) fn record_slot_layout(
    schema: &[SavedFieldDesc],
    persistent: usize,
) -> (Vec<usize>, usize) {
    let mut sorted: Vec<usize> = schema.iter().map(|f| f.rec_offset).collect();
    sorted.sort_unstable();
    let width_of = |o: usize| -> usize {
        sorted
            .iter()
            .copied()
            .find(|&x| x > o)
            .unwrap_or(persistent)
            .saturating_sub(o)
    };
    let widths = schema.iter().map(|f| width_of(f.rec_offset)).collect();
    let bitmap_w = sorted.first().copied().unwrap_or(0);
    (widths, bitmap_w)
}

/// Decode one **packed** saved record: a leading present-bitmap of `bitmap_w` bytes, then a
/// **fixed-width** slot per field in ascending-offset order. The record is fixed length
/// (`item_size`), so *every* field — present or absent — occupies its slot; the walk always advances
/// by `widths[k]` (the field's on-disk slot width, from [`packed_ondisk_layout`]). A present field is
/// read per its precomputed on-disk `kinds[k]`: a fixed-width little-endian scalar, or a UTF-16LE
/// string up to the first NUL within its slot (the rest of the slot is zero padding). A bit-clear
/// (absent) field is `None` but still consumes its slot. Cells are returned in `schema` order.
fn decode_packed_record(
    rec: &[u8],
    schema: &[SavedFieldDesc],
    kinds: &[PackedField],
    widths: &[usize],
    bitmap_w: usize,
) -> Vec<Option<String>> {
    let n = schema.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&k| schema[k].rec_offset);
    let present = |bit: usize| rec.get(bit / 8).map(|b| b & (1 << (bit % 8)) != 0) == Some(true);

    let mut cells = vec![None; n];
    let mut cur = bitmap_w;
    for (pos, &k) in order.iter().enumerate() {
        let width = widths.get(k).copied().unwrap_or(0);
        let end = (cur + width).min(rec.len());
        let slot = rec.get(cur..end).unwrap_or(&[]);
        cur = end;
        if !present(pos) {
            continue;
        }
        cells[k] = Some(match kinds.get(k).copied().unwrap_or(PackedField::Str) {
            PackedField::Int(_) => int_cell_to_string(slot),
            PackedField::Float(_) => float_cell_to_string(slot),
            PackedField::Str => decode_utf16z(slot),
        });
    }
    cells
}

/// Decode a UTF-16LE run up to the first NUL word (the rest of the on-disk slot is zero padding).
pub(super) fn decode_utf16z(slot: &[u8]) -> String {
    let end = slot
        .chunks_exact(2)
        .position(|c| c == [0, 0])
        .map(|w| w * 2)
        .unwrap_or(slot.len());
    char::decode_utf16(
        slot.get(..end)
            .unwrap_or(&[])
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]])),
    )
    .map(|r| r.unwrap_or('\u{FFFD}'))
    .collect()
}

/// One packed index batch's on-disk layout, read from its `0x6d` directory entry: the row `count`,
/// the fixed on-disk record width (`item_size`), and — per inline string column, in ascending record
/// offset — the on-disk byte offset of the field that immediately follows it. String columns are
/// compacted per batch (slot width = `(batch_max_char + 1) * 2`), so these boundaries are the only
/// non-derivable part of the layout and vary per batch.
struct PackedBatchLayout {
    count: u32,
    item_size: u32,
    /// On-disk offset of the field after each string column (the k-th string's slot ends here). The
    /// last field, if a string, ends at `item_size` instead — its entry here is not used.
    next_offsets: Vec<usize>,
}

/// The packed index batches of a saved-data catalog, each with the per-string boundaries its own
/// column table states (see [`COLUMN_TABLE_STRIDE`]).
///
/// Restricted to the entries that chain contiguously through `SavedRecordsStream`. A memo-less
/// report can still carry a trailing directory entry addressing `MemoValuesStream` — vestigial when
/// that stream is absent — and it is not an index batch: taking it would add its `count` to the
/// report's record total for rows that do not exist.
fn packed_index_layouts(catalog: &SavedCatalog) -> Vec<PackedBatchLayout> {
    let mut cursor = 0u32;
    catalog
        .batches
        .iter()
        // The `SavedRecordsStream` run: entries whose byte spans chain contiguously from offset 0.
        // The first that does not continue the chain belongs to the next physical stream.
        .take_while(|b| {
            let chains = b.desc.stream_off == cursor && b.desc.stream_len > 0;
            cursor = cursor.saturating_add(b.desc.stream_len);
            chains
        })
        .filter(|b| b.desc.item_size > 0 && b.desc.item_size < MAX_ITEM_SIZE)
        .map(|b| PackedBatchLayout {
            count: b.desc.count,
            item_size: b.desc.item_size,
            next_offsets: b
                .columns
                .chunks_exact(COLUMN_TABLE_STRIDE)
                .map(|triple| triple[COLUMN_NEXT_FIELD_OFFSET] as usize)
                .collect(),
        })
        .collect()
}

/// Resolve one packed batch's on-disk layout: per-field slot widths, per-field on-disk read kind
/// (schema order), and the present-bitmap width. Walks fields in ascending record offset carrying an
/// on-disk cursor `cur`; each field's in-memory (persistent) span width is `pw`.
///
/// A field is a **compacted string** iff the next unconsumed column-table boundary falls within its
/// persistent span (`next_offsets[si] <= cur + pw`) — a scalar or a fixed short string keeps its
/// persistent width on disk, so the next boundary (belonging to a later, actually-compacted string)
/// always lies strictly beyond `cur + pw`. This identifies the compacted columns **structurally from
/// the table**, so it needs neither the value type nor a matching string count, and — because each
/// boundary is an absolute on-disk offset — a mistyped scalar width cannot drift past the next
/// string. The last field always ends at `item_size`. The value type only selects how a
/// **non-compacted** field is read (scalar vs. fixed short string); a compacted field is always a
/// string.
fn packed_ondisk_layout(
    schema: &[SavedFieldDesc],
    field_types: &[FieldValueType],
    persistent: usize,
    item_size: usize,
    next_offsets: &[usize],
) -> (Vec<usize>, Vec<PackedField>, usize) {
    let n = schema.len();
    let (pwidths, bitmap_w) = record_slot_layout(schema, persistent);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&k| schema[k].rec_offset);

    let mut widths = vec![0usize; n];
    let mut kinds = vec![PackedField::Str; n];
    let mut cur = bitmap_w;
    let mut si = 0usize;
    for (idx, &k) in order.iter().enumerate() {
        let pw = pwidths.get(k).copied().unwrap_or(0);
        let last = idx + 1 == order.len();
        let boundary = next_offsets.get(si).copied();
        let compacted =
            si < next_offsets.len() && (last || boundary.unwrap_or(usize::MAX) <= cur + pw);
        let (w, kind) = if compacted {
            si += 1;
            let end = if last {
                item_size
            } else {
                boundary.unwrap_or(item_size)
            };
            (end.saturating_sub(cur), PackedField::Str)
        } else {
            // Non-compacted: a scalar occupies its natural on-disk size (`Int`/`Float` payload — the
            // record may carry trailing padding a `pw`-wide slot would wrongly swallow); a fixed
            // short string occupies its small persistent buffer.
            let vt = field_types
                .get(k)
                .copied()
                .unwrap_or(FieldValueType::Int32s);
            let kind = packed_field(vt, pw);
            let w = match kind {
                PackedField::Int(sz) | PackedField::Float(sz) => sz,
                PackedField::Str => pw,
            };
            (w, kind)
        };
        widths[k] = w;
        kinds[k] = kind;
        cur += w;
    }
    (widths, kinds, bitmap_w)
}

/// Decode the memo-less **packed** index — its string columns are stored inline, compacted per batch.
/// Each `0x6d` index batch is decoded with its own on-disk `item_size` and per-column slot widths
/// (from its column table), keyed on the shared in-memory `persistent` record width for the cipher
/// IV. Batches sit back-to-back in `SavedRecordsStream` (walked by consumed length, IV tail = the
/// 0-based batch sequence).
///
/// `record_count` is the **stored** total over every directory entry, not the number of rows that
/// decoded. Accumulating it per successful batch instead would make it agree with `rows.len()` by
/// construction, so a batch that failed to decrypt would shrink both together and drop rows with no
/// trace. Keeping the stored count independent leaves `rows.len() < record_count` as the signal, and
/// the returned status names the batch that failed.
pub(crate) fn decode_packed_index(
    catalog: &SavedCatalog,
    srs_raw: &[u8],
    field_types: &[FieldValueType],
    persistent: usize,
) -> SavedRowset {
    let schema = &catalog.fields;
    let layouts = packed_index_layouts(catalog);
    let record_count: u32 = layouts.iter().map(|l| l.count).sum();
    let mut first_problem: Option<SavedDataStatus> = None;
    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    let mut cursor = 0usize;
    for (k, lay) in layouts.iter().enumerate() {
        let (inf, consumed) = match decode_batch_at(
            srs_raw,
            cursor,
            INDEX_BATCH_SIZE,
            lay.count,
            persistent as u32,
            k as u32,
        ) {
            Ok(batch) => batch,
            Err(problem) => {
                first_problem.get_or_insert(batch_failed(SavedBatchKind::Index, k, problem));
                break;
            }
        };
        cursor += consumed;
        let is = lay.item_size as usize;
        let need = lay.count as usize * is;
        let Some(start) = inf.len().checked_sub(need) else {
            first_problem.get_or_insert(batch_failed(
                SavedBatchKind::Index,
                k,
                BatchProblem::Short,
            ));
            break;
        };
        // The batch's own column table gives the compacted-string boundaries; the layout is resolved
        // structurally from it (no dependence on a resolved string count).
        let (widths, kinds, bitmap_w) =
            packed_ondisk_layout(schema, field_types, persistent, is, &lay.next_offsets);
        for i in 0..lay.count as usize {
            let rec = &inf[start + i * is..start + (i + 1) * is];
            rows.push(decode_packed_record(rec, schema, &kinds, &widths, bitmap_w));
        }
    }
    SavedRowset::walked(rows, record_count, first_problem)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(name: &str, off: usize) -> SavedFieldDesc {
        SavedFieldDesc {
            rec_offset: off,
            name: name.to_string(),
            is_memo: false,
        }
    }

    #[test]
    fn packed_field_types_map_to_on_disk_encodings() {
        assert!(matches!(
            packed_field(FieldValueType::String, 200),
            PackedField::Str
        ));
        assert!(matches!(
            packed_field(FieldValueType::Int32s, 4),
            PackedField::Int(4)
        ));
        assert!(matches!(
            packed_field(FieldValueType::Number, 8),
            PackedField::Float(8)
        ));
        assert!(matches!(
            packed_field(FieldValueType::Date, 4),
            PackedField::Int(4)
        ));
        // Unknown type falls back to the in-memory buffer width to tell scalar from string.
        assert!(matches!(
            packed_field(FieldValueType::Unknown, 4),
            PackedField::Int(4)
        ));
        assert!(matches!(
            packed_field(FieldValueType::Unknown, 120),
            PackedField::Str
        ));
    }

    #[test]
    fn packed_record_reads_fixed_width_slots() {
        // A packed record is FIXED-width: a 2-byte present-bitmap, then a fixed on-disk slot per
        // field (present or not). Here a 6-byte string slot (off 2), a 4-byte int slot (off 8), an
        // 8-byte double slot (off 12). Bitmap bits 0 and 2 set → the string and the double are
        // present; the int is absent but STILL occupies its zero-filled 4-byte slot.
        let schema = [desc("T.name", 2), desc("T.count", 8), desc("T.amount", 12)];
        let _field_types = [
            FieldValueType::String,
            FieldValueType::Int32s,
            FieldValueType::Number,
        ];
        // On-disk slot widths (the string is compacted to hold "Hi" + NUL = 6 bytes).
        let widths = vec![6usize, 4, 8];
        let bitmap_w = 2usize;

        let mut rec = Vec::new();
        rec.extend_from_slice(&0b101u16.to_le_bytes()); // bits 0 and 2 present (name + amount)
        for c in "Hi".encode_utf16() {
            rec.extend_from_slice(&c.to_le_bytes());
        }
        rec.extend_from_slice(&[0, 0]); // string NUL (fills the 6-byte slot)
        rec.extend_from_slice(&0u32.to_le_bytes()); // absent int slot, zero-filled
                                                    // The amount as the engine stores it: scaled by SAVED_NUMERIC_SCALE, so 12.5 is written as
                                                    // 1250.0 and must read back as "12.5".
        rec.extend_from_slice(&1250.0f64.to_le_bytes()); // amount

        let kinds = [PackedField::Str, PackedField::Int(4), PackedField::Float(8)];
        let cells = decode_packed_record(&rec, &schema, &kinds, &widths, bitmap_w);
        assert_eq!(cells[0].as_deref(), Some("Hi"));
        assert_eq!(cells[1], None); // count absent (bit clear)
        assert_eq!(cells[2].as_deref(), Some("12.5"));
    }

    #[test]
    fn packed_ondisk_layout_uses_string_boundaries() {
        // 4 fields in record-offset order: a Date scalar (off 2), a String (off 6), a Number scalar
        // (off 208), a String (off 216, the last field). Persistent width 300. The batch column
        // table gives the on-disk offset of the field AFTER the first string (= 30). The trailing
        // string runs to `item_size`.
        let schema = [
            desc("T.d", 2),
            desc("T.s1", 6),
            desc("T.n", 208),
            desc("T.s2", 216),
        ];
        let field_types = [
            FieldValueType::Date,
            FieldValueType::String,
            FieldValueType::Number,
            FieldValueType::String,
        ];
        let item_size = 60usize;
        let next_offsets = vec![30usize, 42]; // s1→n at 30; s2 is last so 42 is unused (item_size wins)
        let (widths, kinds, bitmap_w) =
            packed_ondisk_layout(&schema, &field_types, 300, item_size, &next_offsets);
        assert_eq!(bitmap_w, 2);
        // d: [2..6]=4 (Date); s1: [6..30]=24; n: [30..38]=8 (Number); s2: [38..60]=22 (to item_size)
        assert_eq!(widths, vec![4, 24, 8, 22]);
        assert!(matches!(kinds[0], PackedField::Int(4)));
        assert!(matches!(kinds[1], PackedField::Str));
        assert!(matches!(kinds[2], PackedField::Float(8)));
        assert!(matches!(kinds[3], PackedField::Str));
    }

    #[test]
    fn packed_ondisk_layout_leaves_short_fixed_string_uncompacted() {
        // A short (1-char) String field is stored FIXED, not compacted, so the column table lists
        // fewer boundaries than there are String columns. Layout: a big String (off 2, compacted),
        // a 4-byte fixed String (off 200), a Number (off 204). Only ONE boundary (for the big
        // string); the fixed short string must be detected structurally as non-compacted.
        let schema = [desc("T.big", 2), desc("T.code", 200), desc("T.n", 204)];
        let field_types = [
            FieldValueType::String,
            FieldValueType::String,
            FieldValueType::Number,
        ];
        let item_size = 26usize; // bitmap 2 + big 12 + code 4 + number 8
        let next_offsets = vec![14usize]; // big ends at on-disk 14; code + number are fixed
        let (widths, kinds, bitmap_w) =
            packed_ondisk_layout(&schema, &field_types, 212, item_size, &next_offsets);
        assert_eq!(bitmap_w, 2);
        // big: [2..14]=12 (compacted); code: fixed persistent width 4; number: last → item_size-cur.
        assert_eq!(widths[0], 12);
        assert_eq!(widths[1], 4);
        assert!(matches!(kinds[0], PackedField::Str)); // compacted
        assert!(matches!(kinds[1], PackedField::Str)); // fixed short string, still read as string
        assert!(matches!(kinds[2], PackedField::Float(8)));
    }

    #[test]
    fn int_cell_sign_extends() {
        assert_eq!(int_cell_to_string(&(-5i32).to_le_bytes()), "-5");
        assert_eq!(int_cell_to_string(&7i32.to_le_bytes()), "7");
    }
}
