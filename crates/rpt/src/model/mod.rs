//! The semantic DOM: a type-strict tree of domain DTOs named after the RAS/Engine SDK.
//!
//! [`Report`] is the root. Its typed members ([`ReportDefinition`], [`DataDefinition`],
//! [`Database`], [`PrintOptions`], …) are populated by [`crate::project::raise`]. The raw record
//! tree is also kept in [`Report::records`] as a tree of [`Node`]s (typed where decoded,
//! [`Node::Unknown`] otherwise), so every record is represented and any consumer can walk the
//! whole report. Byte-exact round-trip is guaranteed by the lossless record substrate.

mod data_def;
mod database;
mod document;
mod dom;
mod enums;
mod format;
mod objects;
mod primitives;
mod report_def;

pub use data_def::{
    DataDefinition, DbField, FieldDef, FieldKind, FieldKindData, FormulaField, Group,
    GroupNameField, GroupOptions, ParameterField, ParameterValue, RunningTotalField, Sort,
    SpecialField, SqlExpressionField, SummaryField,
};
pub use database::{ConnectionInfo, Database, DbFieldDef, Table, TableLink};
pub use document::{
    PageMargins, PrintOptions, ReportOptions, Subreport, SubreportLink, SummaryInfo,
};
pub use dom::{Node, Unknown, Value};
pub use enums::*;
pub use format::{
    BooleanFieldFormat, Border, CommonFieldFormat, DateFieldFormat, FieldFormat, Font, FontColor,
    Hyperlink, NumericFieldFormat, ObjectFormat, StringFieldFormat, TimeFieldFormat,
};
pub use objects::{
    BlobFieldObject, BoxShape, ChartObject, CrossTabObject, DrawingShape, FieldHeadingObject,
    FieldObject, FieldRefKind, LineShape, PictureObject, ReportObject, ReportObjectKind,
    SubreportObject, TextObject,
};
pub use primitives::{Color, Conditioned, Formula, RecordRef, Rect, Twips, Version};
pub use report_def::{
    Area, AreaFormat, GroupAreaFormat, ReportDefinition, Section, SectionAreaFormatBase,
    SectionFormat,
};

use crate::records::RecordTag;

/// The root of a decoded report (SDK: `IReportDocument`).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Report {
    /// Format version word from the `Contents` stream header.
    pub version: u16,
    /// SDK `ReportDocument.HasSavedData` — the file carries a saved result set (saved rows), not
    /// just the report definition. Detected from the saved-data block descriptor record
    /// (`0x0061`) in the `Contents` stream; the saved rows themselves are not decoded.
    pub has_saved_data: bool,
    /// SDK `SummaryInfo`.
    pub summary_info: SummaryInfo,
    /// SDK `PrintOptions`.
    pub print_options: PrintOptions,
    /// SDK `ReportOptions`.
    pub report_options: ReportOptions,
    /// SDK `ReportDefinition` — areas/sections/objects.
    pub report_definition: ReportDefinition,
    /// SDK `DataDefinition` — fields/groups/sorts/selection.
    pub data_definition: DataDefinition,
    /// SDK `Database` — tables/links.
    pub database: Database,
    /// SDK `Subreports` (physically `Subdocument N` streams).
    pub subreports: Vec<Subreport>,
    /// Embedded OLE objects (`Embedding N` storages), summarised by their `Ole` stream digest.
    pub embeds: Vec<Embed>,
    /// The raw record tree (typed [`Node`]s where decoded, [`Node::Unknown`] otherwise) — the
    /// total view that keeps every record represented.
    pub records: Vec<Node>,
    /// A structural summary of the decoded record stream: how many records of each type.
    pub record_inventory: Vec<RecordTypeCount>,
}

/// An embedded OLE object, summarised by the `Ole` stream's name, byte size, and the Base64 of
/// its MD5 digest.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Embed {
    /// The stream name (the `\x01Ole` control prefix stripped — always `Ole` in practice).
    pub name: String,
    /// The `Ole` stream's byte length.
    pub size: u64,
    /// Base64 of the MD5 digest of the `Ole` stream bytes.
    pub md5_hash: String,
}

/// One entry in the [`Report::record_inventory`]: a record type and how many of it occur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RecordTypeCount {
    /// The raw record type.
    pub tag: u16,
    /// The symbolic name, if this type has been identified (derived from `tag`).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub name: Option<&'static str>,
    /// Number of records of this type in the decoded stream.
    pub count: usize,
}

impl Report {
    /// Total number of records summarised in the inventory.
    pub fn record_count(&self) -> usize {
        self.record_inventory.iter().map(|e| e.count).sum()
    }

    /// Number of distinct record types present.
    pub fn distinct_record_types(&self) -> usize {
        self.record_inventory.len()
    }

    /// Look up the count for a specific record tag.
    pub fn count_of(&self, tag: RecordTag) -> usize {
        self.record_inventory
            .iter()
            .find(|e| e.tag == tag.value())
            .map_or(0, |e| e.count)
    }
}
