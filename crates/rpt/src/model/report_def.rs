//! Report definition — areas/sections/objects (SDK: `IReportDefinition`).

use super::enums::{AreaSectionKind, PaperOrientation};
use super::objects::ReportObject;
use super::primitives::{Color, Twips};

/// SDK: `IReportDefinition` — the layout half of the report.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ReportDefinition {
    pub areas: Vec<Area>,
}

/// SDK: `IArea` — a group of like sections.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Area {
    pub kind: AreaSectionKind,
    pub name: String,
    pub format: AreaFormat,
    pub sections: Vec<Section>,
}

/// SDK: `ISection`.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Section {
    pub kind: AreaSectionKind,
    pub name: String,
    pub height: Twips,
    pub width: Twips,
    /// Numeric id that report objects reference (SDK: SectionCode).
    pub section_code: i32,
    pub format: SectionFormat,
    pub objects: Vec<ReportObject>,
    /// Conditional-format formulas attached to this section, as `(attribute name, formula text)`
    /// pairs in `<SectionAreaConditionFormulas>` emit order (e.g. `("EnableSuppress", "…")`).
    pub condition_formulas: Vec<(String, String)>,
}

/// Members shared by [`AreaFormat`] and [`SectionFormat`] (SDK: `ISectionAreaFormat` base).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SectionAreaFormatBase {
    pub keep_together: bool,
    pub new_page_before: bool,
    pub new_page_after: bool,
    pub print_at_bottom_of_page: bool,
    pub reset_page_number_after: bool,
    pub suppress: bool,
}

/// SDK: `IAreaFormat` (XML `<AreaFormat>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct AreaFormat {
    pub base: SectionAreaFormatBase,
    pub hide_for_drill_down: bool,
    pub visible_records_per_page: i32,
    pub clamp_page_footer: bool,
    pub group: Option<GroupAreaFormat>,
}

/// SDK: `IGroupAreaFormat` (XML `<GroupAreaFormat>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GroupAreaFormat {
    pub keep_group_together: bool,
    pub repeat_group_header: bool,
    pub visible_groups_per_page: i32,
}

/// SDK: `ISectionFormat` (XML `<SectionFormat>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct SectionFormat {
    pub base: SectionAreaFormatBase,
    pub suppress_if_blank: bool,
    pub underlay_section: bool,
    pub css_class: Option<String>,
    pub page_orientation: Option<PaperOrientation>,
    pub background_color: Option<Color>,
}
