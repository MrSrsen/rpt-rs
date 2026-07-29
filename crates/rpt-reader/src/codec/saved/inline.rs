//! Reading a row out of the fixed record index: the inline (non-memo) cells, each from its slot in
//! the index record, and the memo cells, each from the heap its descriptor points into.
//!
//! An inline cell's width and its interpretation both come from the field's declared value type,
//! exactly as on the packed path — the record layout only supplies the slot a variable-length
//! column fills.

use crate::model::FieldValueType;

use super::memo::read_memo_cell;
use super::packed::{decode_utf16z, float_cell_to_string, int_cell_to_string, record_slot_layout};
use super::schema::SavedFieldDesc;

/// Reads one saved row over the stored schema.
#[derive(Debug)]
pub(super) struct RowReader<'a> {
    /// The stored fields, in record-layout order; a row's cells come back in this order.
    schema: &'a [SavedFieldDesc],
    /// The declared value types, in schema order.
    field_types: &'a [FieldValueType],
    /// Per-field slot width within the fixed index record: a field runs to the next-higher field
    /// offset, the last to the record width. Only a variable-length inline column needs this — a
    /// scalar's width comes from its type.
    slot_widths: Vec<usize>,
}

impl<'a> RowReader<'a> {
    /// A reader over `schema` for index records of `record_width` bytes.
    pub(super) fn new(
        schema: &'a [SavedFieldDesc],
        field_types: &'a [FieldValueType],
        record_width: usize,
    ) -> RowReader<'a> {
        let (slot_widths, _) = record_slot_layout(schema, record_width);
        RowReader {
            schema,
            field_types,
            slot_widths,
        }
    }

    /// One row: its inline fields out of the fixed index record `idx_rec`, its memo fields out of
    /// `heap` via `cells` — the row's descriptor cells in memo-column order, each
    /// `[u16 col][u16 flag][u32 heap_offset][u32 byte_length]`.
    pub(super) fn row(&self, idx_rec: &[u8], cells: &[&[u8]], heap: &[u8]) -> Vec<Option<String>> {
        let mut memo_i = 0usize;
        self.schema
            .iter()
            .enumerate()
            .map(|(k, f)| {
                if f.is_memo {
                    let cell = cells.get(memo_i).copied();
                    memo_i += 1;
                    cell.and_then(|c| {
                        let o = u32::from_le_bytes([c[4], c[5], c[6], c[7]]) as usize;
                        let l = u32::from_le_bytes([c[8], c[9], c[10], c[11]]) as usize;
                        read_memo_cell(heap, o, l)
                    })
                } else {
                    inline_cell(
                        idx_rec,
                        f.rec_offset,
                        self.field_types.get(k).copied(),
                        self.slot_widths.get(k).copied().unwrap_or(0),
                    )
                }
            })
            .collect()
    }
}

/// The on-disk width of an inline saved scalar, from its declared value type. `None` for a type
/// with no fixed inline width — a `String` occupies its whole declared slot instead (see
/// [`inline_cell`]), so its width comes from the record layout rather than from the type.
fn inline_width(vt: FieldValueType) -> Option<usize> {
    match vt {
        FieldValueType::Int8s => Some(1),
        FieldValueType::Int16s => Some(2),
        FieldValueType::Int32s | FieldValueType::Int32u => Some(4),
        // A day count and a second-of-day count respectively; a DateTime is the two side by side.
        FieldValueType::Date | FieldValueType::Time => Some(4),
        FieldValueType::DateTime => Some(8),
        FieldValueType::Number | FieldValueType::Currency => Some(8),
        _ => None,
    }
}

/// Read one inline (non-memo) cell from an index record at `off`, per its declared value type.
///
/// A `String` has no width of its own: it occupies the whole of its record slot (`slot_width`, the
/// gap to the next field) as a NUL-terminated UTF-16LE run.
///
/// A `DateTime` is `[u32 day-serial][u32 second-of-day]`, so only its low half is the date, and the
/// day serial is what a consumer parses. The time component is dropped: carrying it would mean
/// emitting an ISO datetime, which needs the Julian-to-calendar arithmetic that lives in
/// `rpt-format-value` — a crate the reader deliberately does not depend on.
fn inline_cell(
    rec: &[u8],
    off: usize,
    vt: Option<FieldValueType>,
    slot_width: usize,
) -> Option<String> {
    let Some(vt) = vt else {
        return untyped_cell(rec, off);
    };
    if vt == FieldValueType::String {
        // A slot too narrow to hold even one code unit means the record layout was not resolved;
        // fall back rather than invent an empty string.
        let Some(slot) = slot_width
            .checked_add(off)
            .filter(|_| slot_width >= 2)
            .and_then(|end| rec.get(off..end))
        else {
            return untyped_cell(rec, off);
        };
        return Some(decode_utf16z(slot));
    }
    let Some(width) = inline_width(vt) else {
        return untyped_cell(rec, off);
    };
    let Some(slot) = rec.get(off..off.checked_add(width)?) else {
        return untyped_cell(rec, off);
    };
    Some(match vt {
        FieldValueType::Number | FieldValueType::Currency => float_cell_to_string(slot),
        FieldValueType::DateTime => int_cell_to_string(&slot[..4]),
        _ => int_cell_to_string(slot),
    })
}

/// Read a cell whose inline encoding is not established — a type with no measured width, or a slot
/// the record layout could not resolve — as a 4-byte little-endian integer.
///
/// This is the reading of last resort, deliberately not widened to the types [`inline_cell`]
/// handles: an unproven answer is not interchangeable with an absent one, and blanking a column is
/// not a correction.
fn untyped_cell(rec: &[u8], off: usize) -> Option<String> {
    let s = rec.get(off..off.checked_add(4)?)?;
    Some(i32::from_le_bytes([s[0], s[1], s[2], s[3]]).to_string())
}
