//! Document-level DTOs: summary info, print/report options, subreports.

use super::enums::{PaperOrientation, PaperSize, PaperSource, PrinterDuplex};
use super::primitives::Twips;

/// SDK: `ISummaryInfo` (XML `<Summaryinfo>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct SummaryInfo {
    pub title: String,
    pub subject: String,
    pub author: String,
    pub comments: String,
    pub keywords: String,
    pub save_with_preview: bool,
}

/// SDK: `IPrintOptions` (XML `<PrintOptions>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct PrintOptions {
    pub content_width: Twips,
    pub content_height: Twips,
    pub paper_orientation: PaperOrientation,
    pub paper_size: PaperSize,
    pub paper_source: PaperSource,
    pub printer_duplex: PrinterDuplex,
    pub printer_name: String,
    pub driver_name: Option<String>,
    pub port_name: Option<String>,
    pub margins: PageMargins,
}

/// SDK: `IPageMargins` (XML `<PageMargins>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PageMargins {
    pub left: Twips,
    pub right: Twips,
    pub top: Twips,
    pub bottom: Twips,
}

/// SDK: `IReportOptions` (XML `<ReportOptions>`) — saved-data / query behavior.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ReportOptions {
    pub save_data_with_report: bool,
    pub save_summaries_with_report: bool,
    pub save_preview_picture: bool,
    pub use_dummy_data: bool,
    pub initial_data_context: Option<String>,
    pub initial_report_part_name: Option<String>,
}

/// SDK: a subreport is a full nested report + its link wiring.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Subreport {
    pub name: String,
    /// The nested report (recursive).
    pub report: Box<super::Report>,
    pub links: Vec<SubreportLink>,
}

/// SDK: `ISubreportLink` (XML `<SubReportLink>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct SubreportLink {
    pub main_report_field: String,
    pub subreport_field: String,
    pub linked_parameter: Option<String>,
}
