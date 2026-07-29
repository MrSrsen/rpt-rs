//! Seven record types, one shape. Each brackets one section of an area and is closed by the even
//! number above it, and the *record type* is the band's kind: the area and section names are
//! user-renameable — a group band is commonly renamed after its group field — so a name cannot
//! classify a band and the number is the only authority.

use super::*;

/// A band marker's whole content: the `0x008c` section it brackets, and nothing else.
///
/// The section comes **first**, as it does for every object opener — it is read from content offset
/// zero, and whatever a band carries past it is the remainder. So a field declared ahead of the
/// section is blocked by it rather than read — the order is load-bearing even though no band
/// carries a field for it to move.
///
/// A band record stores the section alone, with no field bytes on either side of it. The seven
/// share this declaration because they share it in the format, not by coincidence of the files at
/// hand.
const BAND_SECTION: &[Field] = &[Field::new("section", Kind::Child(0x008c))];

/// `0x008d ReportHeaderBand` — the report-header band, closed by `0x008e`.
pub(crate) const REPORT_HEADER_BAND: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x008d,
    name: "ReportHeaderBand",
    fields: BAND_SECTION,
};

/// `0x008f ReportFooterBand` — the report-footer band, closed by `0x0090`.
pub(crate) const REPORT_FOOTER_BAND: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x008f,
    name: "ReportFooterBand",
    fields: BAND_SECTION,
};

/// `0x0091 PageHeaderBand` — the page-header band, closed by `0x0092`.
pub(crate) const PAGE_HEADER_BAND: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0091,
    name: "PageHeaderBand",
    fields: BAND_SECTION,
};

/// `0x0093 PageFooterBand` — the page-footer band, closed by `0x0094`.
pub(crate) const PAGE_FOOTER_BAND: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0093,
    name: "PageFooterBand",
    fields: BAND_SECTION,
};

/// `0x0095 DetailBand` — the detail band, closed by `0x0096`.
pub(crate) const DETAIL_BAND: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0095,
    name: "DetailBand",
    fields: BAND_SECTION,
};

/// `0x0097 GroupHeaderBand` — a group-header band, closed by `0x0098`. Written only by a report
/// that has groups, one per group level.
pub(crate) const GROUP_HEADER_BAND: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0097,
    name: "GroupHeaderBand",
    fields: BAND_SECTION,
};

/// `0x0099 GroupFooterBand` — a group-footer band, closed by `0x009a`. Written only by a report
/// that has groups, one per group level.
pub(crate) const GROUP_FOOTER_BAND: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0099,
    name: "GroupFooterBand",
    fields: BAND_SECTION,
};
