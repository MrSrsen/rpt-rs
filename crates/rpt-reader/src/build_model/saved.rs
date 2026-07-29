//! The saved-data rows as part of the semantic model.
//!
//! [`crate::codec::saved`] decodes bytes to strings and asks to be told each column's value type;
//! this is where that type comes from. Resolving it means reading the report's own database field
//! catalog, so it is a model question, not a byte question, and it belongs above the codec layer.

use crate::codec::{decode_saved_rows, SavedFieldDesc};
use crate::coverage::SavedDataStatus;
use crate::model::{FieldValueType, Report, SavedColumn, SavedData};
use crate::records::RecordStream;
use crate::StreamId;
use std::collections::HashMap;

/// Decode a report's stored saved data from its `SavedRecordsStream` (record index) and
/// `MemoValuesStream` (variable-length values), with the account of what became of it.
///
/// Returns the stored records — not the engine's result rowset, which projects/reorders/groups/
/// formats them. The rows are `None` whenever nothing decoded, which several very different
/// situations produce; the [`SavedDataStatus`] is what tells them apart, and it is always returned.
pub(crate) fn decode_saved_data(
    streams: &[RecordStream],
    report: &Report,
) -> (Option<SavedData>, SavedDataStatus) {
    let find = |pred: fn(&StreamId) -> bool| streams.iter().find(|s| pred(s.id()));
    let nothing = |status| (None, status);
    // The top-level `DataSourceManager` variant is inherently non-subdocument (nested streams stay
    // `StreamId::Other`), so no explicit Subdocument exclusion is needed. Its logical payload is
    // decoded once at stream-decode time.
    let dsm =
        find(|id| matches!(id, StreamId::DataSourceManager(_))).map(RecordStream::logical_bytes);
    let Some(dsm) = dsm.filter(|d| !d.is_empty()) else {
        return nothing(SavedDataStatus::NoCatalog);
    };

    // Decodable only when the field values are in an external MemoValuesStream. Reports with no memo
    // columns (all-inline) still decode: the memo stream may be absent.
    let memo_raw = find(|id| matches!(id, StreamId::MemoValuesStream(_)))
        .map(|s| s.encode())
        .unwrap_or_default();

    // The catalog is read before the row stream is looked for, so a report that carries neither is
    // reported as storing no fields rather than as missing a stream it never needed.
    let catalog = super::read_catalog(dsm);
    let schema = &catalog.fields;
    if schema.is_empty() {
        return nothing(SavedDataStatus::NoStoredFields);
    }
    let Some(srs_raw) =
        find(|id| matches!(id, StreamId::SavedRecordsStream(_))).map(|s| s.encode())
    else {
        return nothing(SavedDataStatus::MissingRowStream);
    };
    // Each stored column's value type: a memo column is a `PersistentMemo`; every other column takes
    // its declared type from the report's database field of the same qualified name (the inline
    // packed reader keys the on-disk field width on this — a `Number` is an 8-byte double, an
    // `Int32s` is 4 bytes, a `String` is a NUL-terminated UTF-16 run). Unmatched fields fall back to
    // `Int32s`.
    let field_types = saved_field_types(schema, report);
    // The stored rows: index batches (inline fields, packed or fixed) + memo-descriptor batches whose
    // cells point into the memo-value heaps (no delta reconstruction needed).
    let rowset = decode_saved_rows(&catalog, &srs_raw, &memo_raw, &field_types);
    if rowset.rows.is_empty() {
        return nothing(rowset.status);
    }
    let columns = schema
        .iter()
        .zip(field_types)
        .map(|(f, value_type)| SavedColumn {
            name: f.name.clone(),
            value_type,
        })
        .collect();
    (
        Some(SavedData {
            record_count: rowset.record_count,
            columns,
            rows: rowset.rows,
        }),
        rowset.status,
    )
}

/// Resolve each saved column's value type (schema order): a memo column is a `PersistentMemo`; every
/// other column takes the declared type of the report database field with the same qualified name
/// (`Table.Field`, matched on both the table's stored name and its alias), defaulting to `Int32s`.
/// This is what tells the inline row reader a `Number` column is an 8-byte double vs an `Int32s`
/// 4-byte scalar vs a `String` — the DSM saved-field catalog itself carries no type code.
fn saved_field_types(schema: &[SavedFieldDesc], report: &Report) -> Vec<FieldValueType> {
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
