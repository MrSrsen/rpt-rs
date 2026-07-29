//! The saved-data catalog is a third vocabulary, written by several components at once. Its records
//! are QE-framed but its type numbers are neither the report definition's nor the session's.

use super::*;

/// `0x0041 SavedFieldHeader` — a `0x0040` descriptor, an ordinal, and the descriptor's offset within
/// the space its container addresses. Under a stored-field container that is the byte offset of the
/// field's slot in the fixed saved record.
///
/// The offset is the one field in the corpus whose **width** follows the record's schema rather than
/// its presence, and both forms are well represented — the writer picks the narrow word while the
/// value fits and the wide long otherwise. Read at one fixed width, one of the two groups is wrong
/// by two bytes and takes the trailing word with it.
pub(crate) const SAVED_FIELD_HEADER: Table = Table {
    dialect: Dialect::Catalog,
    rtype: 0x0041,
    name: "SavedFieldHeader",
    fields: &[
        Field::new("entry", Kind::Child(0x0040)),
        Field::new("_u0", Kind::U16Be),
        Field::new(
            "offset",
            Kind::WidensAt {
                at: 0x0702,
                narrow: Width::U16Be,
                wide: Width::U32Be,
            },
        ),
        Field::new("_u1", Kind::I16Be),
    ],
};

/// `0x0040 SavedFieldDescriptor` — which field a stored column holds.
///
/// The record is one field reference — the column's qualified name and the handle that resolves it
/// — then a word, and a trailing group the record carries only while it still has content: a word,
/// and the handle repeated as an index and a pool.
///
/// Under a stored-field container the word after the reference is the column's length marker:
/// `0xffff` for a variable-length (memo) column, whose value lives in `MemoValuesStream`, and `0`
/// for a fixed inline one. The other containers put their own quantity there.
pub(crate) const SAVED_FIELD_DESCRIPTOR: Table = Table {
    dialect: Dialect::Catalog,
    rtype: 0x0040,
    name: "SavedFieldDescriptor",
    fields: &[
        Field::new("field", Kind::FieldRef),
        Field::new("_u0", Kind::U16Be),
        Field::optional("_u1", Kind::I16Be),
        Field::optional("field_index", Kind::U16Be),
        Field::optional("field_kind", Kind::VarU16),
    ],
};

/// One physical stream the saved data occupies: the id the container names the stream by
/// (`<name> <id>l`), and the version it is written at.
const SAVED_STREAM: &[Field] = &[
    Field::new("stream_id", Kind::U32Be),
    Field::new("version", Kind::U16Be),
];

/// One byte span of a physical stream.
const SAVED_STREAM_SPAN: &[Field] = &[
    Field::new("_u0", Kind::U32Be),
    Field::new("byte_length", Kind::U32Be),
];

/// A stream the record names after its batch headers: the same id and version as [`SAVED_STREAM`],
/// with the span beside it rather than in a list of its own.
const SAVED_TRAILING_STREAM: &[Field] = &[
    Field::new("stream_id", Kind::U32Be),
    Field::new("version", Kind::U16Be),
    Field::new("_u0", Kind::U32Be),
    Field::new("byte_length", Kind::U32Be),
];

/// One entry of a batch-header list: a nested `0x006d`.
const SAVED_BATCH_HEADER: &[Field] = &[Field::new("header", Kind::Child(0x006d))];

/// `0x002d SavedRecordsStructure` — how a report's stored rows are laid out and where they live.
///
/// The record states the in-memory width of one stored record, the number of records, the physical
/// streams the batches occupy, and then the batch headers themselves in four lists. Two fields
/// widen from a word to a long at schema `0x0702`; the wide form rests on the record's own reader,
/// not on an observed example.
///
/// Everything from the stream spans on is carried only while the record still has content, which is
/// how a report whose saved data occupies fewer streams simply ends earlier.
pub(crate) const SAVED_RECORDS_STRUCTURE: Table = Table {
    dialect: Dialect::Catalog,
    rtype: 0x002d,
    name: "SavedRecordsStructure",
    fields: &[
        Field::new(
            "item_size",
            Kind::WidensAt {
                at: 0x0702,
                narrow: Width::U16Be,
                wide: Width::U32Be,
            },
        ),
        Field::new("_u0", Kind::U16Be),
        Field::new(
            "_u1",
            Kind::WidensAt {
                at: 0x0702,
                narrow: Width::U16Be,
                wide: Width::U32Be,
            },
        ),
        Field::new("record_count", Kind::U32Be),
        Field::new("_u2", Kind::I16Be),
        Field::new("_u3", Kind::I16Be),
        Field::new("_u4", Kind::I16Be),
        Field::new("_u5", Kind::I16Be),
        Field::new("_u6", Kind::I16Be),
        Field::new("_u7", Kind::U16Be),
        Field::new(
            "streams",
            Kind::Repeat {
                count: Count::Fixed(4),
                body: SAVED_STREAM,
            },
        ),
        Field::new("_u8", Kind::I16Be),
        Field::optional(
            "stream_spans",
            Kind::Repeat {
                count: Count::Fixed(4),
                body: SAVED_STREAM_SPAN,
            },
        ),
        Field::optional("_u9", Kind::U32Be),
        Field::optional("_u10", Kind::U32Be),
        // Four batch-header lists back to back: each count decides where the next list starts, so a
        // header read into the wrong list would take the following list's headers with it.
        Field::optional("batch_count_0", Kind::U16Be),
        Field::optional("batch_count_1", Kind::U16Be),
        Field::optional("batch_count_2", Kind::U16Be),
        Field::optional("batch_count_3", Kind::U16Be),
        Field::new(
            "batches_0",
            Kind::Repeat {
                count: Count::FromField("batch_count_0"),
                body: SAVED_BATCH_HEADER,
            },
        ),
        Field::new(
            "batches_1",
            Kind::Repeat {
                count: Count::FromField("batch_count_1"),
                body: SAVED_BATCH_HEADER,
            },
        ),
        Field::new(
            "batches_2",
            Kind::Repeat {
                count: Count::FromField("batch_count_2"),
                body: SAVED_BATCH_HEADER,
            },
        ),
        Field::new(
            "batches_3",
            Kind::Repeat {
                count: Count::FromField("batch_count_3"),
                body: SAVED_BATCH_HEADER,
            },
        ),
        Field::optional(
            "trailing_streams",
            Kind::Repeat {
                count: Count::Fixed(2),
                body: SAVED_TRAILING_STREAM,
            },
        ),
    ],
};

/// `0x006d SavedBatchEntry` — one batch of stored records: how many items it holds, how wide each
/// one is on disk, and the byte span it occupies within its physical stream.
///
/// The column table after it is the part that varies per batch: a record whose string columns are
/// stored inline is compacted to that batch's own per-column maxima, so the boundaries between its
/// columns are stored here rather than derived from the record layout. The three trailing fields
/// are carried only while the record still has content.
pub(crate) const SAVED_BATCH_ENTRY: Table = Table {
    dialect: Dialect::Catalog,
    rtype: 0x006d,
    name: "SavedBatchEntry",
    fields: &[
        Field::new("count", Kind::U32Be),
        Field::new("item_size", Kind::U32Be),
        Field::new("stream_offset", Kind::U32Be),
        Field::new("stream_length", Kind::U32Be),
        Field::new("column_count", Kind::U16Be),
        Field::new(
            "columns",
            Kind::Repeat {
                count: Count::FromField("column_count"),
                body: &[Field::new("value", Kind::U32Be)],
            },
        ),
        Field::optional("_u0", Kind::U32Be),
        Field::optional("_u1", Kind::U32Be),
        Field::optional("_u2", Kind::I16Be),
    ],
};
