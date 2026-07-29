//! Field tables: one per record type, stating that record's content as a sequence of named
//! typed fields.
//!
//! A `Skip` names nothing: it is an opaque run of bytes whose meaning is unknown, and the number in
//! it is a hand-derived length, not a decoded fact.
//!
//! Each module below states the tables of one domain. The registries here are the only place a
//! table is enrolled, so a dialect's set is read whole rather than assembled from the modules
//! that contribute to it.

mod bands;
mod catalog;
mod chart;
mod crosstab;
mod data_def;
mod document;
mod field_definitions;
mod format_conditions;
mod formats;
mod layout;
mod print_options;
mod qe_session;
mod report_parameters;

pub(crate) use bands::*;
pub(crate) use catalog::*;
pub(crate) use chart::*;
pub(crate) use crosstab::*;
pub(crate) use data_def::*;
pub(crate) use document::*;
pub(crate) use field_definitions::*;
pub(crate) use format_conditions::*;
pub(crate) use formats::*;
pub(crate) use layout::*;
pub(crate) use print_options::*;
pub(crate) use qe_session::*;
pub(crate) use report_parameters::*;

use super::table::{Count, Ctx, Field, Kind, Table, Width};
use crate::codec::Dialect;

/// Every table describing a **`Contents`** record.
pub(crate) const ALL: &[&Table] = &[
    &OBJECT_POSITION,
    &DRAWING_OBJECT,
    &GROUP_AREA_FORMAT,
    &OBJECT_NAME,
    &CHART_ANALYTIC,
    &CHART_DATA_VALUE,
    &CHART_DEFINITION2,
    &CHART_OBJECT,
    &CROSSTAB_WRAPPER,
    &CROSSTAB_OBJECT,
    &CROSSTAB_GRID_CELL,
    &CROSSTAB_DIM_FIELD,
    &CROSSTAB_COLUMN_AXIS,
    &CROSSTAB_ROW_AXIS,
    &CROSSTAB_GRID_FORMAT,
    &CROSSTAB_GRID_CELL_FORMAT,
    &CROSSTAB_CUSTOM_MEMBERS,
    &BOOLEAN_FIELD_FORMAT,
    &COMMON_FIELD_FORMAT,
    &DATE_TIME_FIELD_FORMAT,
    &DATE_FIELD_FORMAT,
    &TIME_FIELD_FORMAT,
    &NUMERIC_FIELD_FORMAT,
    &STRING_FIELD_FORMAT,
    &OBJECT_FORMAT,
    &AREA_SECTION_FORMAT,
    &BORDER,
    &FONT,
    &FONT_COLOR,
    &BOOLEAN_FIELD_FORMAT_WRAPPER,
    &COMMON_FIELD_FORMAT_WRAPPER,
    &DATE_TIME_FIELD_FORMAT_WRAPPER,
    &TIME_FIELD_FORMAT_WRAPPER,
    &STRING_FIELD_FORMAT_WRAPPER,
    &NUMERIC_FIELD_FORMAT_WRAPPER,
    &DATE_FIELD_FORMAT_WRAPPER,
    &BORDER_WRAPPER,
    &OBJECT_FORMAT_WRAPPER,
    &SECTION_FORMAT_WRAPPER,
    &FONT_CONDITION_FORMAT,
    &SECTION_CODE_AREA_TYPE,
    &SECTION_CODE_HEADER_FOOTER,
    &OBJECT_MARKER,
    &XML_DEFINITION,
    &SAVE_METADATA,
    &FIELD_OBJECT,
    &PICTURE_OBJECT,
    &BLOB_FIELD_WRAPPER,
    &OLE_OBJECT_ITEM,
    &TEXT_OBJECT_CONTAINER,
    &TEXT_OBJECT_FORMAT,
    &TEXT_OBJECT,
    &TEXT_EMBEDDED_FIELD,
    &AREA,
    &SECTION,
    &REPORT_HEADER_BAND,
    &REPORT_FOOTER_BAND,
    &PAGE_HEADER_BAND,
    &PAGE_FOOTER_BAND,
    &DETAIL_BAND,
    &GROUP_HEADER_BAND,
    &GROUP_FOOTER_BAND,
    &SUBREPORT_OBJECT,
    &SUBREPORT_LINK,
    &SUMMARY_FIELD_DEFINITION,
    &RUNNING_TOTAL_FIELD,
    &SQL_EXPRESSION_FIELD,
    &SUMMARY_FIELD_WRAPPER,
    &NAMED_VALUE_WRAPPER,
    &FIELD_DEFINITION,
    &FIELD_DEFINITION2,
    &PARAMETER_RECORD,
    &FORMULA,
    &FORMULA_FIELD_WRAPPER,
    &REPORT_PROPERTY,
    &GROUP,
    &HIERARCHICAL_GROUP_VALUE,
    &RECORD_SORT_FIELD,
    &FIELD_MANAGER_ENTRY,
    &GUIDELINE_ENTRY,
    &FORMULA_VARIABLE,
    &PAGE_SETUP,
    &PAPER_RECT,
    &MULTI_COLUMN,
    &NAMED_VALUE,
    &PAGE_DEVMODE,
    &PRINTER,
    &OBJECT_CONNECTION,
    &REPORT_ROOT,
    &FIELD_HEADING_LINK,
    &SUBREPORT_REIMPORT_INFO,
    &SAVED_DATA,
    &REPORT_OPTIONS,
];

/// Every table describing a **`QESession`** record. Kept apart from [`ALL`] because a record type
/// number is per stream: the report definition and the query-engine session both use `0x0003` and
/// `0x0007`, for unrelated records.
pub(crate) const QE_ALL: &[&Table] = &[
    &QE_CONNECTION,
    &QE_TABLE,
    &QE_FIELD,
    &QE_COMMAND_PARAMETER,
    &QE_INDEX,
    &QE_LOGON_PROPERTY,
    &QE_TABLE_LINK,
];

/// Every table describing a **`DataSourceManager`** catalog record.
pub(crate) const CATALOG_ALL: &[&Table] = &[
    &SAVED_RECORDS_STRUCTURE,
    &SAVED_FIELD_DESCRIPTOR,
    &SAVED_FIELD_HEADER,
    &SAVED_BATCH_ENTRY,
];

/// Every table describing a **`ReportParametersStream`** record — a fourth vocabulary. Its records
/// are framed like the report definition's and numbered like nothing else: `0x0030`, `0x0031` and
/// `0x003b` are the records of a parameter's saved entry here and unrelated report-definition
/// records in [`ALL`], so a table for one may not be reached from the other.
pub(crate) const REPORT_PARAMETERS_ALL: &[&Table] = &[&CURRENT_VALUE_RECORD];

/// The newest schema each `Contents` record type is known to have, by type number.
///
/// A schema word is a version, ordered, and a record newer than the newest layout a reader knows
/// cannot be decoded by it — the fields may have been widened or reordered since. Each entry is the
/// value the record type's own reader declares as its maximum. Most types have never changed shape,
/// which is why one value covers all but ten of them.
///
/// The list is the report definition's alone. A type number is per dialect, so a maximum recorded
/// here says nothing about the unrelated record another stream writes under the same number, and
/// borrowing one across would refuse that record outright: each stream numbers its versions in its
/// own series, and the query engine's begin above everything here.
///
/// [`max_supported_schema`] returns `None` for a type with no entry, and a `None` is not a licence
/// to decode anything: it means no maximum has been established for that type, so the check cannot
/// speak.
const CONTENTS_SCHEMA_EXCEPTIONS: &[(u16, u16)] = &[
    (0x0061, 0x0701),
    (0x0064, 0x1400),
    (0x0065, 0x0701),
    (0x0067, 0x0701),
    (0x0069, 0x0701),
    (0x006c, 0x0701),
    (0x006e, 0x0950),
    (0x007a, 0x0900),
    (0x007b, 0x0701),
    (0x007c, 0x0701),
];

/// The `Contents` record types covered by the `0x0700` default — the ones whose layout has not
/// changed since that version. Together with [`CONTENTS_SCHEMA_EXCEPTIONS`] this is every
/// `Contents` type whose maximum is established.
const CONTENTS_SCHEMA_0700: &[u16] = &[
    0x0008, 0x0066, 0x0068, 0x006a, 0x006b, 0x006f, 0x0070, 0x0071, 0x0072, 0x0073, 0x0076, 0x0077,
    0x0078, 0x0079, 0x007d, 0x007e, 0x007f, 0x0080, 0x0081, 0x0083, 0x0085, 0x0087, 0x0089, 0x008a,
    0x008b, 0x008c, 0x008e, 0x0090, 0x0092, 0x0094, 0x0096, 0x0098, 0x009a, 0x009b, 0x009c, 0x009d,
    0x009e, 0x009f, 0x00a0, 0x00a1, 0x00a2, 0x00a4, 0x00a5, 0x00a6, 0x00a7, 0x00a8, 0x00a9, 0x00ab,
    0x00ad, 0x00ae, 0x00b0, 0x00b2, 0x00b3, 0x00b5, 0x00b7, 0x00b8, 0x00ba, 0x00bd, 0x00bf, 0x00c0,
    0x00c1, 0x00c3, 0x00c5, 0x00ca, 0x00cb, 0x00cc, 0x00cd, 0x00ce, 0x00cf, 0x00d0, 0x00d1, 0x00d2,
    0x00d3, 0x00d4, 0x00d5, 0x00d6, 0x00d7, 0x00d8, 0x00d9, 0x00da, 0x00db, 0x00dc, 0x00dd, 0x00de,
    0x00df, 0x00e0, 0x00e1, 0x00e2, 0x00e3, 0x00e4, 0x00e5, 0x00e6, 0x00e7, 0x00e8, 0x00e9, 0x00ea,
    0x00eb, 0x00ec, 0x00ed, 0x00ee, 0x00ef, 0x00f0, 0x00f1, 0x00f2, 0x00f3, 0x00f4, 0x00f5, 0x00f6,
    0x00f7, 0x00f8, 0x00f9, 0x00fa, 0x00fb, 0x00fc, 0x00fd, 0x00fe, 0x00ff, 0x0100, 0x0101, 0x0102,
    0x0103, 0x0104, 0x0105, 0x0106, 0x0107, 0x0108, 0x0109, 0x010a, 0x010b, 0x010c, 0x010d, 0x010e,
    0x010f, 0x0110, 0x0111, 0x0112, 0x0114, 0x0115, 0x0116, 0x0117, 0x0118, 0x0142, 0x014f, 0x0151,
    0x0152, 0x0153, 0x0157, 0x015f, 0x0160, 0x0163, 0x0164, 0x0165, 0x016c, 0x017b, 0x017c, 0x0183,
];

/// The newest schema this reader can decode for a record type, or `None` when no maximum has been
/// established for it.
///
/// Keyed the way [`for_record`] routes, by dialect and type: only the report definition's
/// vocabulary has its ceilings recorded, and reading them for another dialect would refuse records
/// that merely share a number with a `Contents` type.
pub(crate) fn max_supported_schema(rtype: u16, dialect: Dialect) -> Option<u16> {
    match dialect {
        Dialect::Contents => {
            if let Some((_, max)) = CONTENTS_SCHEMA_EXCEPTIONS.iter().find(|(t, _)| *t == rtype) {
                return Some(*max);
            }
            CONTENTS_SCHEMA_0700.contains(&rtype).then_some(0x0700)
        }
        // No maximum has been established for these vocabularies, and the report definition's is
        // not one to borrow.
        Dialect::QeSession | Dialect::Catalog | Dialect::ReportParameters => None,
    }
}

/// The schema word a table's record carries, where the stream reuses its type number for a
/// structurally different record at another schema version. `None` when the type is unambiguous.
pub(crate) fn tabled_schema(rtype: u16, dialect: Dialect) -> Option<u16> {
    match (dialect, rtype) {
        // `Contents` writes a second, unrelated record at both numbers under schema `0x0701`: a
        // two-byte one wrapping a `0x0041` child, and the six-byte "no saved printer" form.
        (Dialect::Contents, 0x0007 | 0x0003) => Some(0x0700),
        _ => None,
    }
}

/// Every table describing a record of `dialect`.
pub(crate) fn set(dialect: Dialect) -> &'static [&'static Table] {
    match dialect {
        Dialect::Contents => ALL,
        Dialect::QeSession => QE_ALL,
        Dialect::Catalog => CATALOG_ALL,
        Dialect::ReportParameters => REPORT_PARAMETERS_ALL,
    }
}

/// The field table for a record of type `rtype` and version `schema` in `dialect`, if it has one.
///
/// The schema is part of the key: where a stream writes two structurally unrelated records under
/// one number, a table describes one of them and must not be applied to the other.
pub(crate) fn for_record(rtype: u16, schema: u16, dialect: Dialect) -> Option<&'static Table> {
    if tabled_schema(rtype, dialect).is_some_and(|only| only != schema) {
        return None;
    }
    set(dialect).iter().copied().find(|t| t.rtype == rtype)
}
