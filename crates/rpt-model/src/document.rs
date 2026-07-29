//! Document-level DTOs: summary info, print/report options, subreports.

use super::enums::{PaperOrientation, PaperSize, PaperSource, PrinterDuplex};
use super::primitives::Twips;

/// The Crystal Reports version that wrote a report, as the report itself records it.
///
/// Stored in the first field of the report's root record, ahead of everything else. `letter` is a
/// third component the engine renders as a character; every report seen stores `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AuthoringVersion {
    /// The release's major number — `12` for Crystal Reports 2008, `14` for 2011 and later.
    pub major: u16,
    /// The release's minor number.
    pub minor: u16,
    /// A third component the engine renders as a character; `0` on every report seen.
    pub letter: u8,
}

impl std::fmt::Display for AuthoringVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)?;
        if self.letter != 0 {
            write!(f, ".{}", self.letter as char)?;
        }
        Ok(())
    }
}

/// SDK: `ISummaryInfo`.
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
    /// The report revision number (SDK `SummaryInfo.RevisionNumber`) — sourced from the OLE
    /// `\x05SummaryInformation` property set (`PIDSI_REVNUMBER` = 0x09), kept in the engine's stored
    /// string form (e.g. `"128"`). Empty when the property is absent.
    pub revision_number: String,
    /// The user who last saved the report (SDK `SummaryInfo.LastSavedBy`) — from the OLE
    /// `\x05SummaryInformation` property set (`PIDSI_LASTAUTHOR` = 0x08). May carry a login name;
    /// empty when the property is absent.
    pub last_saved_by: String,
    /// Whether the report is saved with a preview thumbnail (SDK `SavePreviewPicture`).
    pub save_with_preview: bool,
    /// When the report was created — OLE `\x05SummaryInformation` `PIDSI_CREATE_DTM` (0x0C), as a
    /// raw Windows `FILETIME`: 100-nanosecond intervals since 1601-01-01 UTC.
    pub created: Option<u64>,
    /// When the report was last saved — `PIDSI_LASTSAVE_DTM` (0x0D), as a raw Windows `FILETIME`.
    ///
    /// This is the fact behind the engine's `ModificationDate` / `ModificationTime` special fields.
    pub last_saved: Option<u64>,
    /// When the report was last printed — `PIDSI_LASTPRINTED` (0x0B), as a raw Windows `FILETIME`.
    pub last_printed: Option<u64>,
}

/// SDK: `IPrintOptions`.
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
    /// The live/current printer name (SDK `PrinterName`); the engine reports this empty.
    pub printer_name: String,
    /// The saved printer's device name (SDK `SavedPrinterName`) — the DEVMODE device string, e.g. a
    /// network printer path. Distinct from the empty live `printer_name`; empty when no printer
    /// record was saved.
    pub saved_printer_name: String,
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

/// SDK: `IPageMargins`.
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

/// SDK: `IReportOptions` — saved-data / query behavior.
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
    /// Whether the engine re-verifies the database schema on every print (SDK
    /// `EnableVerifyOnEveryPrint`). Stored in the report-header record's option flag byte.
    pub enable_verify_on_every_print: bool,
    /// Whether database `NULL` field values are converted to their type's default value (SDK
    /// `ConvertNullFieldToDefault`). Stored in the report's data-source options record.
    pub convert_null_field_to_default: bool,
    /// Whether other `NULL` values are converted to their type's default value (SDK
    /// `ConvertOtherNullsToDefault`). Stored in the report's data-source options record.
    pub convert_other_nulls_to_default: bool,
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
    /// The "re-import subreport when opening" policy — a multi-valued enum, evaluated at load: value
    /// `1` skips the source-file check, other values compare the source `.rpt`'s modification time to
    /// decide whether to re-import. Kept raw: it is invariant at `1` (so there is no non-default
    /// example) and the RAS `SubreportController` exposes no re-import accessor, so the value→name
    /// mapping is unknown and left undecoded rather than fabricated.
    pub reimport_when_opening: u8,
    /// A second `(Julian-day, time-fraction)` timestamp (the source's own save time); normally zero.
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

/// One designer snap guideline. Its position is a twip coordinate on the design canvas. Horizontal
/// and vertical guides share this shape; the axis is carried by the **parent** list record, not by
/// this entry — the two guideline lists (`0x010d` and `0x010f`) partition the guides into the two
/// axes (which list is horizontal vs vertical is unknown, having no reader surface). This flat
/// collection is not split by axis, since nothing consumes the orientation. Designer-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Guideline {
    /// The guideline's position on the canvas, in twips (leaf `[0..4]`, big-endian).
    pub position: Twips,
    /// How many objects are attached to this guide: the number of object-connection records the
    /// guideline's own collection carries after it. Zero for a guide nothing is snapped to. It is
    /// a count and not a bit field, and it is not the orientation — that is the parent list, above.
    pub flags: u16,
}

/// One designer object-connection edge (record `0x0111`): the layout object a guideline is attached
/// to, and the state of the attachment. Designer-only.
///
/// The record names one object, not two. Its two leading words are a single identifier — a kind and
/// an index — followed by a pair of longs, the attachment state, and four qualifier words that name
/// a sub-object within it. The two member names here predate that reading and are kept for the
/// export surface's sake, not because the record has two endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ObjectConnection {
    /// The connected object's kind — the first word of its identifier. A kind and an index of `-1`
    /// together are the identifier for no object at all.
    pub source: u16,
    /// The connected object's index within its kind — the second word of its identifier.
    pub destination: u16,
    /// The attachment state, as one word: two codes stored side by side, the high byte first. Each
    /// is a narrowing value of its own, so a code past a byte's range would widen and this word
    /// would not hold the pair. Their meaning is unresolved — no reader surface exposes them, and
    /// these records are interactive-designer state that programmatic authoring never emits — so
    /// they are preserved raw, with no enum.
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

/// SDK: `ISubreportLink`.
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
