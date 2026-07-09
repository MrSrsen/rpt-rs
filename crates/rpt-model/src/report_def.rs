//! Report definition — areas/sections/objects (SDK: `IReportDefinition`).

use super::enums::{AreaSectionKind, PaperOrientation};
use super::objects::ReportObject;
use super::primitives::{Color, Twips};

/// SDK: `IReportDefinition` — the layout half of the report.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReportDefinition {
    /// The report's areas, in top-to-bottom layout order (report/page header, group headers,
    /// details, group footers, report/page footer).
    pub areas: Vec<Area>,
}

/// Iterate every report object across `areas` in layout order (area → section → object) — the one
/// traversal the projection and orchestration layers share instead of hand-rolling the nesting.
pub fn area_objects(areas: &[Area]) -> impl Iterator<Item = &ReportObject> {
    areas
        .iter()
        .flat_map(|a| &a.sections)
        .flat_map(|s| &s.objects)
}

/// Mutable [`area_objects`].
pub fn area_objects_mut(areas: &mut [Area]) -> impl Iterator<Item = &mut ReportObject> {
    areas
        .iter_mut()
        .flat_map(|a| &mut a.sections)
        .flat_map(|s| &mut s.objects)
}

/// SDK: `IArea` — a group of like sections.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Area {
    /// Which band this area is (report header, details, group footer, …).
    pub kind: AreaSectionKind,
    /// The area's name (SDK `Area.Name`).
    pub name: String,
    /// Formatting shared by all sections in the area.
    pub format: AreaFormat,
    /// The sections that make up this area (usually one; groups repeat per instance).
    pub sections: Vec<Section>,
}

/// SDK: `ISection`.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Section {
    /// Which band this section belongs to (mirrors its area's kind).
    pub kind: AreaSectionKind,
    /// The section's name (SDK `Section.Name`).
    pub name: String,
    /// The section's design height, in twips.
    pub height: Twips,
    /// The section's design width, in twips.
    pub width: Twips,
    /// Numeric id that report objects reference (SDK: SectionCode).
    pub section_code: i32,
    /// The section's formatting (suppress, underlay, background, …).
    pub format: SectionFormat,
    /// The report objects placed in this section.
    pub objects: Vec<ReportObject>,
    /// Conditional-format formulas attached to this section, as `(attribute name, formula text)`
    /// pairs in `<SectionAreaConditionFormulas>` emit order (e.g. `("EnableSuppress", "…")`).
    pub condition_formulas: Vec<(String, String)>,
}

/// Members shared by [`AreaFormat`] and [`SectionFormat`] (SDK: `ISectionAreaFormat` base).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SectionAreaFormatBase {
    /// Keep the whole section/area on one page rather than splitting it across a page break.
    pub keep_together: bool,
    /// Start a new page before this section/area.
    pub new_page_before: bool,
    /// Start a new page after this section/area.
    pub new_page_after: bool,
    /// Push this section to the bottom of the page (used for group footers).
    pub print_at_bottom_of_page: bool,
    /// Reset the page number to 1 after this section/area.
    pub reset_page_number_after: bool,
    /// Suppress (do not render) this section/area.
    pub suppress: bool,
}

/// SDK: `IAreaFormat` (XML `<AreaFormat>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AreaFormat {
    /// The formatting flags shared with sections.
    pub base: SectionAreaFormatBase,
    /// Hide this area unless the user drills into it (SDK `HideForDrillDown`).
    pub hide_for_drill_down: bool,
    /// Cap on visible records per page (0 = unlimited).
    pub visible_records_per_page: i32,
    /// Whether the page footer is clamped to the bottom of the page.
    pub clamp_page_footer: bool,
    /// Group-specific formatting, present only for group header/footer areas.
    pub group: Option<GroupAreaFormat>,
}

/// SDK: `IGroupAreaFormat` (XML `<GroupAreaFormat>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GroupAreaFormat {
    /// Keep the whole group together on one page where possible.
    pub keep_group_together: bool,
    /// Repeat the group header on each page the group spans.
    pub repeat_group_header: bool,
    /// Cap on visible groups per page (0 = unlimited).
    pub visible_groups_per_page: i32,
}

/// SDK: `ISectionFormat` (XML `<SectionFormat>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SectionFormat {
    /// The formatting flags shared with areas.
    pub base: SectionAreaFormatBase,
    /// Suppress the section when it produces no visible content (SDK `EnableSuppressIfBlank`).
    pub suppress_if_blank: bool,
    /// Render this section underlaid beneath the following ones (SDK `EnableUnderlaySection`).
    pub underlay_section: bool,
    /// The CSS class applied to the section in HTML output, when set.
    pub css_class: Option<String>,
    /// A per-section page-orientation override, when set.
    pub page_orientation: Option<PaperOrientation>,
    /// The section's background colour, when set.
    pub background_color: Option<Color>,
}
