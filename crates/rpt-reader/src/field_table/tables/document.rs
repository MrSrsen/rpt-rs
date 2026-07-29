//! The report document itself: its root and options, the streams it embeds, and what it
//! remembers of the subreports imported into it.

use super::*;

/// A GUID, in the pieces the archive writes it as: `Data1`, `Data2`, `Data3`, then the eight bytes
/// of `Data4` one at a time. Sixteen bytes, and not one run — a fixed run would read the same bytes
/// but say nothing about what they are.
const GUID: &[Field] = &[
    Field::new("data1", Kind::U32Be),
    Field::new("data2", Kind::U16Be),
    Field::new("data3", Kind::U16Be),
    Field::new(
        "data4",
        Kind::Repeat {
            count: Count::Fixed(8),
            body: &[Field::new("byte", Kind::U8)],
        },
    ),
];

/// One GUID, as a field of the record that carries it.
const GUID_FIELD: Kind = Kind::Repeat {
    count: Count::Fixed(1),
    body: GUID,
};

/// One stream stored alongside the report: the mode it is opened in, and its name in the storage.
const EMBEDDED_STREAM: &[Field] = &[
    Field::new("mode", Kind::U32Be),
    Field::new("name", Kind::Str),
];

/// `0x0064 ReportRoot` — the report document's own record, the first in a `Contents` stream and the
/// one every other record follows.
///
/// It opens with the report's statement of which designer wrote it — major and minor version, and a
/// letter — and its document name, then a timestamp as a Julian day and a second of the day. The
/// nested `0x0000` is the document's own body, read mid-sequence, so every field after it is placed
/// by the record before the child and by the child's framed length, never by a byte offset.
///
/// `options` is a bitfield whose bit 0 is `EnableSaveDataWithReport`, and `save_preview_picture` a
/// word of its own further along whose bit 0 is `EnableVerifyOnEveryPrint`. `saved_data` decides
/// whether the record carries the saved-data handle after the child — the one data-driven arm here.
///
/// Everything past the handle is guarded on the record still having content rather than on its
/// version: two GUIDs, two words, a list of the streams stored beside the report, and then a run of
/// trailing fields a writer appends one at a time — the preview/verify word, a saved-data version,
/// the time zone the report was saved in, and the locale. A record that stops early simply carries
/// fewer of them.
///
/// The document name is what makes this record a sequence rather than a set of offsets: a main
/// report leaves it empty and a subreport carries its own name there, so every field after it —
/// the timestamp, the option word, the saved-data flag — moves by the length of the name. Read at
/// the offset the empty form puts them at, a subreport's option word is a letter of its file name.
///
/// A timestamp of `-1` in both halves is the engine's "no timestamp" sentinel rather than a date.
pub(crate) const REPORT_ROOT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0064,
    name: "ReportRoot",
    fields: &[
        Field::new("major_version", Kind::U16Be),
        Field::new("minor_version", Kind::U16Be),
        Field::new("version_letter", Kind::U8),
        Field::new("_u0", Kind::I16Be),
        Field::new("document_name", Kind::Str),
        Field::new("_u1", Kind::VarU16),
        Field::new("timestamp_julian_day", Kind::I32Be),
        Field::new("timestamp_seconds", Kind::I32Be),
        Field::new("_u2", Kind::I16Be),
        Field::new("options", Kind::I16Be),
        Field::new("saved_data", Kind::I16Be),
        Field::new("valid_formula_interface", Kind::I16Be),
        Field::new("document", Kind::Child(0x0000)),
        Field::when("saved_data_handle", Kind::U32Be, |c| {
            c.row.i("saved_data") != 0
        }),
        Field::optional("_guid0", GUID_FIELD),
        Field::new("_guid1", GUID_FIELD),
        Field::new("_u3", Kind::U32Be),
        Field::new("_u4", Kind::U32Be),
        Field::optional("_u5", Kind::U16Be),
        Field::optional("stream_count", Kind::U32Be),
        Field::new(
            "streams",
            Kind::Repeat {
                count: Count::FromField("stream_count"),
                body: EMBEDDED_STREAM,
            },
        ),
        Field::optional("save_preview_picture", Kind::I16Be),
        Field::optional("saved_data_version", Kind::U16Be),
        Field::optional("time_zone", Kind::Str),
        Field::optional("_u6", Kind::I16Be),
        Field::optional("locale", Kind::Str),
        Field::optional("_u7", Kind::I16Be),
    ],
};

/// `0x0142 SubreportReimportInfo` — where a re-imported subreport came from, and when.
///
/// Every report carries one, whether or not it holds a subreport: a report that imported none
/// stores the empty path and a zero source timestamp, which is what makes a stored path the
/// evidence that a subreport was imported at all.
///
/// The path is a length-prefixed string and the first field, so the five fields after it are placed
/// by its length. `reimport_when_opening` is a narrowing enum between the two timestamps, one byte
/// while its value fits below `0x80`, and each timestamp is a pair of longs — a Julian day and a
/// same-day time fraction — kept in stored form rather than converted to a calendar date.
pub(crate) const SUBREPORT_REIMPORT_INFO: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0142,
    name: "SubreportReimportInfo",
    fields: &[
        Field::new("source_path", Kind::Str),
        Field::new("imported_at_julian_day", Kind::I32Be),
        Field::new("imported_at_time_fraction", Kind::I32Be),
        Field::new("reimport_when_opening", Kind::VarU16),
        Field::new("source_saved_at_julian_day", Kind::I32Be),
        Field::new("source_saved_at_time_fraction", Kind::I32Be),
    ],
};

/// `0x0061 SavedData` — the descriptor for the data the report was saved with.
///
/// Two words, on every record of the type, and nothing after them. The first is an invariant the
/// writer emits and a reader takes and discards. The second is a **stream id**: the compound file
/// names each of its streams `<name> <id>l`, and this id is the suffix of the `AnalysisGridsStream`
/// holding the saved instance state that belongs to this descriptor.
///
/// The id is drawn from the same document-wide sequence as the report root's `saved_data_handle`
/// but is a different id — each stream is assigned one as it is written — so the two sit near each
/// other and never coincide.
///
/// The record has no children; an empty `0x0062` follows it and closes the block.
pub(crate) const SAVED_DATA: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0061,
    name: "SavedData",
    fields: &[
        Field::new("_u0", Kind::I32Be),
        Field::new("stream_id", Kind::U32Be),
    ],
};

/// `0x0160 ReportOptions` — the document's report-wide option bag, one per `Contents` stream and
/// so one per report and per subreport. It closes the report-definition container along with the
/// paper rectangle and the saved-data selection formula, carries no nested record and no string,
/// and is nothing but scalars: a narrowing enum and a run of option words.
///
/// Two of those words are the null-conversion options, `convert_null_field_to_default` and
/// `convert_other_nulls_to_default`; the rest are named by position alone. Each is a whole word
/// rather than a bit, and a boolean one stores its truth in the word's low half — which is why
/// reading such a flag as a single byte only works while every field ahead of it keeps its width.
///
/// One value is written into **two consecutive words** rather than one, so `_u2` and `_u3` are the
/// same number twice. Reading them as a single wide integer, or as one word, would put every field
/// after them two bytes out.
///
/// Everything from `convert_other_nulls_to_default` on is guarded on the record still having
/// content: a writer appends these one at a time and a record written before a word existed simply
/// ends short of it. `_u18` is a reserved word the writer emits as zero and its reader discards.
pub(crate) const REPORT_OPTIONS: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0160,
    name: "ReportOptions",
    fields: &[
        Field::new("_u0", Kind::I16Be),
        Field::new("_u1", Kind::VarU16),
        Field::new("convert_null_field_to_default", Kind::I16Be),
        Field::new("_u2", Kind::I16Be),
        Field::new("_u3", Kind::I16Be),
        Field::new("_u4", Kind::I16Be),
        Field::new("_u5", Kind::I16Be),
        Field::new("_u6", Kind::I16Be),
        Field::new("_u7", Kind::I16Be),
        Field::new("_u8", Kind::I16Be),
        Field::new("_u9", Kind::I16Be),
        Field::new("_u10", Kind::I16Be),
        Field::new("_u11", Kind::I16Be),
        Field::new("_u12", Kind::I16Be),
        Field::optional("convert_other_nulls_to_default", Kind::I16Be),
        Field::optional("_u13", Kind::I16Be),
        Field::optional("_u14", Kind::I16Be),
        Field::optional("_u15", Kind::I16Be),
        Field::optional("_u16", Kind::I16Be),
        Field::optional("_u17", Kind::I16Be),
        Field::optional("_u18", Kind::U32Be),
        Field::optional("_u19", Kind::I16Be),
    ],
};
