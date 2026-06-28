//! Report objects (SDK: `IReportObject` and its kinds).

use super::enums::{FieldValueType, LineStyle, PictureType, ReadingOrder};
use super::format::{Border, FieldFormat, FontColor, ObjectFormat};
use super::primitives::{Color, Formula, RecordRef, Rect, Twips};

/// SDK: `IReportObject` — the base every object shares; `kind` carries the per-type data.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ReportObject {
    pub name: String,
    pub bounds: Rect,
    pub border: Border,
    pub format: ObjectFormat,
    /// Parent section id this object lives in (SDK: SectionCode).
    pub section_code: i32,
    pub kind: ReportObjectKind,
    pub origin: RecordRef,
}

/// SDK: the concrete report-object subtype + its extra members.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ReportObjectKind {
    Field(FieldObject),
    Text(TextObject),
    FieldHeading(FieldHeadingObject),
    Box(BoxShape),
    Line(LineShape),
    Picture(PictureObject),
    Subreport(SubreportObject),
    /// SDK: `IBlobFieldObject` — an image/blob database field rendered as a picture.
    BlobField(BlobFieldObject),
    Chart(Box<ChartObject>),
    /// SDK: `ICrossTabObject` — a cross-tab grid (opener record `0xb8`, wrapped by `0xb9`). Carries
    /// its decoded row/column dimension field bindings ([`CrossTabObject`]) for `Field.UseCount`.
    CrossTab(CrossTabObject),
    /// SDK: `IOlapGridObject` — an OLAP grid. Typed marker; opener rtype not yet identified, so it
    /// is not yet produced by the decoder.
    OlapGrid,
    /// SDK: `IMapObject` — a geographic map. Typed marker; opener rtype not yet identified.
    Map,
    /// SDK: `IFlashObject` — an embedded Flash/Xcelsius object. Typed marker; opener rtype not yet
    /// identified.
    Flash,
    /// A deferred but not-yet-identified object kind — the raw opener code is preserved.
    Deferred(u16),
    /// An unmodelled object kind — the raw code is preserved.
    #[default]
    Unknown,
}

/// The kind of field a [`FieldObject`] displays, taken from the type byte in its `Contents`
/// opener record. Determines how the engine renders the object's `DataSource`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FieldRefKind {
    /// A database field — `{table.field}`.
    #[default]
    DatabaseField,
    /// A formula field — `{@name}`.
    Formula,
    /// A summary — `Sum ({field})`, `Count ({field})`, etc.
    Summary,
    /// A special field (print date, page-N-of-M, …) — a spaceless kind name.
    Special,
    /// An automatic group-name field — `GroupName ({group condition field})`.
    GroupName,
    /// A running-total field — `{#name}`.
    RunningTotal,
    /// A parameter field — `{?name}`.
    Parameter,
    /// Anything else; the raw reference is used verbatim.
    Unknown,
}

impl FieldRefKind {
    /// Map the field-object opener's type byte to a [`FieldRefKind`].
    pub fn from_code(code: u8) -> Self {
        match code {
            0x00 => Self::DatabaseField,
            0x01 => Self::Formula,
            0x02 => Self::Summary,
            0x03 => Self::Special,
            0x04 => Self::GroupName,
            0x06 => Self::Parameter,
            0x09 => Self::RunningTotal,
            _ => Self::Unknown,
        }
    }
}

/// SDK: `IFieldObject`.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct FieldObject {
    pub data_source: String,
    pub ref_kind: FieldRefKind,
    pub value_type: FieldValueType,
    pub font_color: FontColor,
    pub format: Option<FieldFormat>,
    /// For a summary field object (`ref_kind == Summary`), the index of its summary definition
    /// (`0x7e` record). Two placements of the same summary share a code; two summaries that render
    /// identically have distinct codes — so this is the identity used to deduplicate
    /// `<SummaryFields>`. `None` for non-summary objects.
    pub summary_code: Option<u16>,
}

/// SDK: `ITextObject`.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct TextObject {
    pub text: String,
    pub max_lines: i32,
    pub font_color: FontColor,
    pub reading_order: ReadingOrder,
    /// Embedded field/formula/parameter references (`0x00c4` records): `alias.name` for database
    /// fields, `@name` for formulas, `?name` for parameters — counted toward each field's UseCount.
    pub embedded_fields: Vec<String>,
    /// The object's rendered content for `<Text>`: literal runs (`0xc2`) and embedded references
    /// (`0xc4`, wrapped `{alias.field}`/`{@formula}`/`{?param}`) concatenated in document order.
    /// `text` keeps only the last literal run, so this is what the exporter emits.
    pub display: String,
}

/// SDK: `IFieldHeadingObject` (a text object bound to a `FieldObject`).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct FieldHeadingObject {
    pub field_object_name: String,
    pub text: String,
    pub max_lines: i32,
    pub font_color: FontColor,
}

/// SDK: `IDrawingObject` members shared by box and line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DrawingShape {
    pub right: Twips,
    pub bottom: Twips,
    pub line_style: LineStyle,
    pub line_thickness: Twips,
    pub line_color: Color,
    pub extend_to_bottom_of_section: bool,
}

/// SDK: `ILineObject`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LineShape {
    pub shape: DrawingShape,
    pub end_section_name: String,
}

/// SDK: `IBoxObject`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct BoxShape {
    pub shape: DrawingShape,
    pub end_section_name: String,
    pub corner_ellipse_width: Twips,
    pub corner_ellipse_height: Twips,
    pub fill_color: Option<Color>,
}

/// SDK: `IBlobFieldObject` — a database blob/image field shown as a picture. It has no exported
/// `DataSource`, but the bound field reference (`{table.field}`, from the `0x00b1` wrapper record
/// around the picture opener) is kept so `rpt-engine` can count it toward `Field.UseCount`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct BlobFieldObject {
    /// The bound database field reference, brace-wrapped (`{Command.agency_logo}`).
    pub data_source: String,
}

/// SDK: `IPictureObject`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct PictureObject {
    pub picture_type: PictureType,
    pub data: Vec<u8>,
    pub location_formula: Option<Formula>,
}

/// SDK: `ISubreportObject` (the placeholder; the report itself is in `Report::subreports`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct SubreportObject {
    pub subreport_name: String,
    pub on_demand: bool,
    /// Index `N` of the backing `Subdocument N` storage (from the `0xa3` opener's leaf bytes
    /// `[0..4]`, big-endian). Used to resolve [`Self::subreport_name`] from that subdocument's
    /// report-header record after the subreports are decoded.
    pub subdoc_index: u32,
}

/// SDK: `IChartObject` — chart definition + style (deferred detail).
///
/// The chart's persistent field bindings, decoded from its binding block. Each is the raw engine
/// reference form (`Table.field` for a database field, `@name` for a formula). These are not
/// exported to XML; they exist only for the derived `Field.UseCount`.
///
/// They are split by role because the engine references them a different number of times:
/// - `data_refs` — the chart's "show value" data bindings (the `0x007f`/`0x007e` field). A DB data
///   binding is referenced **once** (like a placed field object).
/// - `category_refs` — the chart's "on change of" category bindings (the `0xe5` grid-group). A DB
///   category is built as an internal group (condition + sort) per chart, so it is referenced
///   **twice per chart**.
///
/// `rpt-engine` aggregates these into `Field.UseCount`; a formula binding instead makes that formula
/// *live* (its own references then count).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ChartObject {
    pub data_refs: Vec<String>,
    pub category_refs: Vec<String>,
}

/// SDK: `ICrossTabObject` — the row/column dimension field bindings of a cross-tab grid.
///
/// `field_refs` are the cross-tab's persistent row/column ("on change of") dimension bindings,
/// decoded from the `0xe5` grid-group records carrying a `@Column #…`/`@Row #…` order marker. Each
/// is the raw engine reference form (`Table.field` or `@name`). The data-cell summaries (e.g.
/// `Sum of {Table.x}`) are counted via `<SummaryFields>` and are NOT included here. A DB dimension
/// is built as an internal group (condition + sort) plus an OLAP-grid registration, so the engine
/// references it **three times per dimension**; `rpt-engine` aggregates these into `Field.UseCount`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct CrossTabObject {
    pub field_refs: Vec<String>,
}
