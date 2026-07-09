//! Document-level DTOs: summary info, print/report options, subreports.

use super::enums::{PaperOrientation, PaperSize, PaperSource, PrinterDuplex};
use super::primitives::Twips;

/// SDK: `ISummaryInfo` (XML `<Summaryinfo>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SummaryInfo {
    /// The report title (SDK `SummaryInfo.ReportTitle`).
    pub title: String,
    /// The report subject (SDK `SummaryInfo.ReportSubject`).
    pub subject: String,
    /// The report author (SDK `SummaryInfo.ReportAuthor`).
    pub author: String,
    /// Free-form comments (SDK `SummaryInfo.ReportComments`).
    pub comments: String,
    /// Keywords associated with the report (SDK `SummaryInfo.KeywordsInReport`).
    pub keywords: String,
    /// Whether the report is saved with a preview thumbnail (SDK `SavePreviewPicture`).
    pub save_with_preview: bool,
}

/// SDK: `IPrintOptions` (XML `<PrintOptions>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PrintOptions {
    /// Printable page width (paper width minus left/right margins), in twips.
    pub content_width: Twips,
    /// Printable page height (paper height minus top/bottom margins), in twips.
    pub content_height: Twips,
    /// Portrait or landscape paper orientation.
    pub paper_orientation: PaperOrientation,
    /// The selected paper size (Letter / A4 / …).
    pub paper_size: PaperSize,
    /// The selected printer paper source (tray).
    pub paper_source: PaperSource,
    /// The printer's duplex (two-sided printing) mode.
    pub printer_duplex: PrinterDuplex,
    /// The saved printer name.
    pub printer_name: String,
    /// The saved printer driver name, when recorded.
    pub driver_name: Option<String>,
    /// The saved printer port name, when recorded.
    pub port_name: Option<String>,
    /// The page margins.
    pub margins: PageMargins,
    /// Multi-column detail layout ("Format with Multiple Columns"), or `None` for a single column.
    pub multi_column: Option<MultiColumn>,
}

/// Multi-column detail layout: detail records flow into several columns across the page (label /
/// phone-book style — Crystal's "Format with Multiple Columns"). Stored in the bytes and used by the
/// layout engine, but **not** exported (render-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MultiColumn {
    /// Number of columns across the page.
    pub columns: u16,
    /// Width of one column's detail region.
    pub column_width: Twips,
    /// Horizontal gap between adjacent columns.
    pub gap_h: Twips,
    /// Vertical gap between records within a column (usually 0).
    pub gap_v: Twips,
    /// Flow direction: `true` = fill across a row of columns then move down ("across then down");
    /// `false` = fill a column top-to-bottom then move to the next ("down then across").
    pub across_then_down: bool,
}

/// SDK: `IPageMargins` (XML `<PageMargins>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PageMargins {
    /// Left page margin, in twips.
    pub left: Twips,
    /// Right page margin, in twips.
    pub right: Twips,
    /// Top page margin, in twips.
    pub top: Twips,
    /// Bottom page margin, in twips.
    pub bottom: Twips,
}

/// SDK: `IReportOptions` (XML `<ReportOptions>`) — saved-data / query behavior.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReportOptions {
    /// Whether the report saves its query's data rows (SDK `EnableSaveDataWithReport`).
    pub save_data_with_report: bool,
    /// Whether saved summaries are stored alongside the data (SDK `EnableSaveSummariesWithReport`).
    pub save_summaries_with_report: bool,
    /// Whether a preview thumbnail is saved (SDK `EnableSavePreviewPicture`).
    pub save_preview_picture: bool,
    /// Whether the report renders against generated dummy data (SDK `EnableUseDummyData`).
    pub use_dummy_data: bool,
    /// The initial data context (report-part navigation entry point), when set.
    pub initial_data_context: Option<String>,
    /// The initial report-part name to display, when set.
    pub initial_report_part_name: Option<String>,
}

/// One save-time environment metadata entry: a key/value string pair. The engine writes a group of
/// these on every save — `Saved Date`, `Build Version`, `Print Engine`, `OS`, `Architecture` — so a
/// report saved N times carries N such groups in stream order. Authoring/environment provenance,
/// not a report-semantic value: stored in the model for completeness but **not** exported.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SaveMetadataEntry {
    /// The metadata key, e.g. `OS`, `Build Version`, `Saved Date`.
    pub key: String,
    /// The metadata value, e.g. `Windows XP`, `12.2.0.290`.
    pub value: String,
}

/// Where a report/subreport was last imported from, for the designer's "re-import subreport when
/// opening" feature. STRUCTURAL: the RAS `SubreportController` exposes no re-import accessor, so it
/// is internal (not exported); decoded for completeness.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SubreportReimportInfo {
    /// The source `.rpt` path the subreport was imported from (empty when none was recorded).
    pub source_path: String,
    /// When the subreport was imported into this report.
    pub imported_at: ReimportTimestamp,
    /// The re-import policy enum byte (the designer's "re-import when opening" setting). Kept raw —
    /// no SDK reader pins its value set.
    pub reimport_when_opening: u8,
    /// A second `(Julian-day, time-fraction)` timestamp (the source's own save time); zero across
    /// the corpus.
    pub source_saved_at: ReimportTimestamp,
}

/// A compound Crystal date-time as stored in a [`SubreportReimportInfo`]: a Julian day number and a
/// same-day time fraction. Kept in raw stored form (no calendar conversion) — it is provenance
/// metadata, not exported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReimportTimestamp {
    /// Julian day number component.
    pub julian_day: u32,
    /// Same-day time-fraction component.
    pub time_fraction: u32,
}

/// The report designer's on-canvas editing geometry — snap guidelines and object-connection edges.
/// This is pure IDE state: it positions the designer's rulers/guides and records which layout nodes
/// are connected, and has no effect on rendering. STRUCTURAL — no SDK reader, not exported.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DesignerState {
    /// The design-surface snap guidelines.
    pub guidelines: Vec<Guideline>,
    /// The object-connection edges between layout nodes.
    pub connections: Vec<ObjectConnection>,
}

/// One designer snap guideline. Its position is a twip coordinate on the design canvas (horizontal
/// and vertical guides share the same shape; the axis is implied by the parent collection).
/// Designer-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Guideline {
    /// The guideline's position on the canvas, in twips.
    pub position: Twips,
    /// The guideline's flag word (raw; snap/lock state — not separately decoded).
    pub flags: u16,
}

/// One designer object-connection edge. `source`/`destination` are small layout-object node
/// indices; the edge `kind` word is `0x0002` for every real edge (a degenerate `0 → 0` root edge
/// stores it shifted). Designer-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ObjectConnection {
    /// The source layout-object node index.
    pub source: u16,
    /// The destination layout-object node index.
    pub destination: u16,
    /// The connection kind word (`0x0002` for every real edge).
    pub kind: u16,
}

/// SDK: a subreport is a full nested report + its link wiring.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Subreport {
    /// The subreport's name (SDK `SubreportObject.SubreportName`).
    pub name: String,
    /// The nested report (recursive).
    pub report: Box<super::Report>,
    /// The links binding main-report fields to this subreport's parameters.
    pub links: Vec<SubreportLink>,
}

/// SDK: `ISubreportLink` (XML `<SubReportLink>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SubreportLink {
    /// The main-report field whose value is passed into the subreport.
    pub main_report_field: String,
    /// The subreport field the value is linked to.
    pub subreport_field: String,
    /// The subreport parameter the linked value is bound to, when the link goes through one.
    pub linked_parameter: Option<String>,
}
