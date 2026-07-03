//! Saved (stored) data — the cached rows a report carries when saved with data.
//!
//! The stored records decoded from `SavedRecordsStream` + `MemoValuesStream`, as they sit in the
//! bytes — not the engine's result rowset (which projects, reorders, groups and formats them).

use super::enums::FieldValueType;

/// A report's stored saved data: the record count and the cached rows in record order.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SavedData {
    /// The stored record count.
    pub record_count: u32,
    /// The stored columns, in record order.
    pub columns: Vec<SavedColumn>,
    /// Row-major cell values in their stored string form; `None` = a null cell.
    pub rows: Vec<Vec<Option<String>>>,
}

/// One stored saved-data column.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SavedColumn {
    /// The stored field name (e.g. `countries_all_iso.id`).
    pub name: String,
    /// The stored value type.
    pub value_type: FieldValueType,
}
