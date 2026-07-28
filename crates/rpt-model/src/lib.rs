//! The rpt-rs semantic report model — the format-neutral IR.
//!
//! Pure data; no I/O. Produced by the `rpt` binary decoder today, and by future readers (serde,
//! DSL); consumed by the render pipeline, the exporter, and future writers.
//!
//! [`Report`] is the root, a type-strict tree of domain DTOs named after the RAS/Engine SDK. Its
//! typed members ([`ReportDefinition`], [`DataDefinition`], [`Database`], [`PrintOptions`], …) are
//! populated by whichever reader produced it. The raw record substrate (the per-record DOM and its
//! inventory) is *not* part of this format-neutral model — the `rpt` reader projects it on demand
//! from the bytes (`Rpt::record_dom`/`Rpt::inventory`), so pipeline consumers link no format types.
//!
//! These types describe *what* a report contains, not *where* it lives in the `.rpt` bytes. The
//! binary-format provenance of each field — its source `Contents` record and leaf layout — is
//! documented in the reader crate's `rpt::provenance` module.

#![forbid(unsafe_code)]

mod analysis;
mod data_def;
mod database;
mod document;
mod enums;
mod fit;
mod format;
mod objects;
mod primitives;
mod report_def;
mod saved;

pub use analysis::field_object_value_type;
pub use data_def::{
    CustomFunction, DataDefinition, DbField, DynamicLovBinding, FieldDef, FieldKind, FieldKindData,
    FieldManagerCensus, FormulaField, FormulaSyntax, FormulaVariable, Group, GroupCondition,
    GroupNameField, GroupOptions, HierarchicalGroupOptions, HierarchicalGroupValue, ParameterField,
    ParameterRange, ParameterValue, RunningTotalField, Sort, SpecialField, SqlExpressionField,
    SummaryField, TopBottomNSort,
};
pub use database::{CommandParameter, ConnectionInfo, Database, DbFieldDef, Table, TableLink};
pub use document::{
    DesignerState, Guideline, MultiColumn, ObjectConnection, PageMargins, PrintOptions,
    ReimportTimestamp, ReportOptions, SaveMetadataEntry, Subreport, SubreportLink,
    SubreportReimportInfo, SummaryInfo,
};
pub use enums::*;
pub use format::{
    BooleanFieldFormat, Border, CommonFieldFormat, DateFieldFormat, DateTimeFieldFormat,
    FieldFormat, Font, FontColor, Hyperlink, NumericFieldFormat, ObjectFormat, StringFieldFormat,
    TimeFieldFormat,
};
pub use objects::{
    BlobFieldObject, BoxShape, ChartArrangement, ChartCategoryPeriod, ChartDefinition,
    ChartElementFont, ChartGraphType, ChartGridType, ChartLegendPosition, ChartObject,
    ChartViewAngle, CrossTabCellFormat, CrossTabDimension, CrossTabGridFormat, CrossTabGridOptions,
    CrossTabMeasure, CrossTabObject, DrawingShape, FieldHeadingObject, FieldObject, FieldRefKind,
    IndentAndSpacingFormat, LineShape, LineSpacing, Paragraph, PictureObject, ReportObject,
    ReportObjectKind, SubreportObject, TextObject, TextRun,
};
pub use primitives::{Color, Conditioned, Formula, RecordRef, Rect, Twips, Version};
pub use report_def::{area_objects, area_objects_mut};
pub use report_def::{
    Area, AreaFormat, GroupAreaFormat, ReportDefinition, Section, SectionAreaFormatBase,
    SectionFormat,
};
pub use saved::{
    SavedBatchInfo, SavedBatchInspection, SavedBatchKind, SavedColumn, SavedData, SavedFieldInfo,
};

/// The root of a decoded report (SDK: `IReportDocument`).
///
/// Walk it top-down — areas → sections → objects — via [`Report::objects`] (a flat layout-order
/// iterator over every [`ReportObject`]), or reach the typed members directly
/// ([`Report::data_definition`](Report#structfield.data_definition),
/// [`Report::database`](Report#structfield.database), …).
///
/// ```no_run
/// # fn use_report(report: &rpt_model::Report) {
/// use rpt_model::ReportObjectKind;
///
/// println!("format version {}", report.version);
/// for object in report.objects() {
///     let kind = match &object.kind {
///         ReportObjectKind::Field(_) => "field",
///         ReportObjectKind::Text(_) => "text",
///         _ => "other",
///     };
///     println!("{kind} object {:?} at {:?}", object.name, object.bounds);
/// }
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Report {
    /// Format version word from the `Contents` stream header.
    pub version: u16,
    /// SDK `ReportDocument.HasSavedData` — the file carries a saved result set (saved rows), not
    /// just the report definition. The saved rows themselves are decoded separately (see
    /// [`Report::saved_data`]).
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
    /// The report's stored saved data (cached rows), decoded when present and decodable; `None`
    /// otherwise. The stored records, not the engine's result rowset — see [`SavedData`].
    pub saved_data: Option<SavedData>,
    /// Save-time environment metadata, in stream order — one key/value entry per save event (see
    /// [`SaveMetadataEntry`]). Authoring provenance, not exported.
    pub save_metadata: Vec<SaveMetadataEntry>,
    /// Subreport re-import provenance: where this report/subreport was imported from and when.
    /// `None` if not recorded. STRUCTURAL — not exported.
    pub reimport: Option<SubreportReimportInfo>,
    /// The report designer's on-canvas editing geometry — snap guidelines and object-connection
    /// edges. Designer/IDE state, not exported.
    pub designer_state: DesignerState,
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

impl Report {
    /// Iterate every report object in layout order (area → section → object), across all areas.
    pub fn objects(&self) -> impl Iterator<Item = &ReportObject> {
        report_def::area_objects(&self.report_definition.areas)
    }

    /// Mutable [`Report::objects`].
    pub fn objects_mut(&mut self) -> impl Iterator<Item = &mut ReportObject> {
        report_def::area_objects_mut(&mut self.report_definition.areas)
    }
}
