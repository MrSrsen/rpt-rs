//! The chart submodel: a chart's visual type/subtype, legend/gridline/axis styling, category
//! period, and the byte-legible title/axis-title strings ([`ChartDefinition`]).
//!
//! Self-contained: depends only on [`Color`]. Consumed by [`ChartObject`](super::ChartObject) and
//! re-exported from the parent module so `objects::ChartDefinition` and friends resolve unchanged.

use crate::Color;

/// The visual shape of a chart (bar / line / pie / …), decoded from the first enum field of
/// the `0x0121 ChartDefinition2` record (leaf `+0x4c`).
///
/// IMPORTANT — this axis has **no published numeric enum**. The RAS SDK's `CrChartTypeEnum`
/// (`crChartTypeDetail`, …) describes the chart *layout* (Detail / Group / Cross-Tab / OLAP), **not**
/// the visual shape — a Group chart varies this field, so it cannot be the layout axis. The
/// code→name mapping below is **inferred**, with no authoritative source: a pie-shaped
/// chart (no value axis, empty data-axis title) stores code `3`, a time-series over a date axis code
/// `1`, and an axis-bearing category chart code `0` (the engine default). Unobserved shapes
/// round-trip losslessly through [`ChartGraphType::Other`], their raw subtype/variant selector
/// preserved separately in [`ChartDefinition::graph_subtype`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartGraphType {
    /// Code `0` — a bar/column chart (the engine default; has both axes).
    #[default]
    Bar,
    /// Code `1` — a line chart, typically a time-series plotted over a date axis.
    Line,
    /// Code `2` — an area chart (a line chart whose region to the baseline is filled).
    /// Unobserved, but named so the renderer draws it as an area rather than silently falling back
    /// to bars.
    Area,
    /// Code `3` — a pie chart; carries no value axis.
    Pie,
    /// Code `4` — a doughnut chart (a pie with an inner-radius ring). The writer branch
    /// `type==3 || type==4` emits the pie-only detach/rotate enum pair for both codes, placing 4 in
    Doughnut,
    /// Code `5` — a 3-D riser ("3-D bar") chart; the `3d` variant stores `05`. One of the two
    /// inherently three-dimensional families ([`ChartDefinition::is_3d`]).
    Riser3D,
    /// Code `6` — a 3-D surface chart (a continuous meshed riser field). The writer branch
    /// `type==5 || type==6` emits an extra 3-D enum for both, grouping 6 with the 3-D riser; the
    /// other inherently 3-D family.
    Surface3D,
    /// Code `7` — an XY scatter chart (two numeric axes, markers, no connecting line). Unobserved;
    /// the code is its Chart Expert gallery index.
    Scatter,
    /// Code `8` — a radar / polar chart (angular category + radial value axis). Unobserved; Radar
    /// follows XY Scatter in the gallery.
    Radar,
    /// Code `9` — a bubble chart (an XY scatter whose marker size encodes a third value).
    Bubble,
    /// Code `10` — a stock (hi-lo / OHLC) chart.
    Stock,
    /// Code `11` — a numeric-axis chart (bar/line/area over a numeric or date X axis rather than an
    /// ordinal category).
    NumericAxis,
    /// Code `12` — a gauge (a dial/needle over an arc scale).
    Gauge,
    /// Code `13` — a Gantt chart (horizontal time bars per record).
    Gantt,
    /// Code `14` — a funnel chart (stacked proportional trapezoids; internal `XBI2_FUNNEL`).
    Funnel,
    /// Code `15` — a histogram (frequency bars over binned value ranges).
    Histogram,
    /// Any unobserved code with no named renderer dispatch, preserved verbatim.
    Other(i32),
}

impl ChartGraphType {
    /// Map the raw `+0x4c` enum code to a [`ChartGraphType`]. The code equals the designer's Chart
    /// Expert gallery index (Bar 0 … Histogram 15). `9` (bubble) and `11` (numeric-axis) are
    /// provisional — they follow the gallery order. Every code past the gallery
    /// round-trips through [`ChartGraphType::Other`].
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::Bar,
            1 => Self::Line,
            2 => Self::Area,
            3 => Self::Pie,
            4 => Self::Doughnut,
            5 => Self::Riser3D,
            6 => Self::Surface3D,
            7 => Self::Scatter,
            8 => Self::Radar,
            9 => Self::Bubble,
            10 => Self::Stock,
            11 => Self::NumericAxis,
            12 => Self::Gauge,
            13 => Self::Gantt,
            14 => Self::Funnel,
            15 => Self::Histogram,
            other => Self::Other(other),
        }
    }
}

/// How a multi-series axis chart (bar / area / line) arranges its series within each category slot —
/// the "chart subtype" arrangement axis, distinct from the visual [`ChartGraphType`].
///
/// At render time the native engine uses this to drive both the axis limits ([stacked/percent
/// scaling](ChartArrangement::Percent)) and the riser placement. **On disk it is the low digit of
/// `graph_subtype`** (the
/// variant slot within the type's gallery band): for the Bar family `0`/`1`/`2` = clustered / stacked
/// / percent. The slot is reused for unrelated per-family variants
/// (e.g. the Area family's depth-effect bit), so [`ChartDefinition::arrangement`] decodes stacked/
/// percent only for the Bar family and reports [`Clustered`](Self::Clustered) elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartArrangement {
    /// Series drawn side-by-side within each category slot (the engine default).
    #[default]
    Clustered,
    /// Series accumulated on top of one another (stacked bars/areas).
    Stacked,
    /// Series stacked and normalized to 100% (the render-time percent mode).
    Percent,
}

/// Where a chart's legend is placed relative to the plot area, decoded from the high byte of the
/// legend `short` at the start of the `0x0121` styling struct (`+0x410`).
///
/// The codes are the SAP `CrLegendPositionEnum` — a placement combines an edge with a horizontal
/// alignment, so the value below the plot is `BottomCenter` (the engine reports
/// `crLegendPositionBottomCenter`), not a bare `Bottom`.
///
/// Four codes are decoded: `Right` (the engine default), `Left`, `BottomCenter`, and `Custom` (a
/// legend the designer manually dragged/resized, which the engine reports as
/// `crLegendPositionCustom`). Any unrecognized code defaults to `Right`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartLegendPosition {
    /// Code `0` — legend to the right of the plot (the engine default).
    #[default]
    Right,
    /// Code `1` — legend to the left of the plot.
    Left,
    /// Code `2` — legend below the plot, centered (`crLegendPositionBottomCenter`).
    BottomCenter,
    /// Code `3` — a manually positioned legend (`crLegendPositionCustom`); its exact placement is a
    /// stored geometry the model does not decode.
    Custom,
}

impl ChartLegendPosition {
    /// Map the raw `0x0121` legend-position enum byte (the high byte of the `+0x410` short) to a
    /// [`ChartLegendPosition`]. Anything outside `0`/`1`/`2`/`3` falls back to the default `Right`.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Right,
            1 => Self::Left,
            2 => Self::BottomCenter,
            3 => Self::Custom,
            _ => Self::Right,
        }
    }
}

/// The gridline mode of one of a chart's axes (the Axes-tab "Gridlines" choice), decoded from the
/// `0x0121` styling struct's per-axis gridline bytes (`CrGridTypeEnum`). A bitmask: bit0 = minor
/// gridlines, bit1 = major gridlines, so [`Both`](ChartGridType::Both) draws both. A default chart
/// stores [`None`](ChartGridType::None) on the group (category) axis and
/// [`Major`](ChartGridType::Major) on the value axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartGridType {
    /// Code `0` — no gridlines on this axis (the group-axis default).
    #[default]
    None,
    /// Code `1` — minor gridlines only (bit0).
    Minor,
    /// Code `2` — major gridlines only (bit1); the value-axis default.
    Major,
    /// Code `3` — both major and minor gridlines (bit0 | bit1).
    Both,
}

impl ChartGridType {
    /// Map the raw `CrGridTypeEnum` byte (bit0 minor, bit1 major) to a [`ChartGridType`]. Values above
    /// `3` keep only the low two bits; a value with neither bit set is [`None`](ChartGridType::None).
    pub fn from_code(code: u8) -> Self {
        match code & 0x03 {
            1 => Self::Minor,
            2 => Self::Major,
            3 => Self::Both,
            _ => Self::None,
        }
    }
}

/// The date-grouping period of a chart's "on change of `<date>`" category — the interval each
/// category bucket spans (weekly, monthly, …), the chart-side analogue of a report group's
/// [`Group::date_condition`](crate::Group::date_condition).
///
/// Decoded from the chart's category grid `0xe5` group record (the same record a report group uses),
/// at the SDK `CrGroupConditionEnum` ordinal byte `used + 3` (the fourth byte after the category
/// field reference), via [`from_sdk_ordinal`](Self::from_sdk_ordinal). This is the same encoding a
/// report group uses (weekly = 1, monthly = 4), reused here for the chart's category axis.
///
/// **Confidence.** `Weekly`, `Monthly`, and `Biweekly` are established. `Semimonthly`/`Quarterly`/
/// `SemiAnnually`/`Annually` are provisional.
/// `Daily` (ordinal 0) is deliberately **not** decoded from this byte: ordinal 0 also means
/// "no period" on a discrete category, and the legacy daily flag (`used + 4 == 0x02`) doubles as a
/// sort attribute on non-date fields, so mapping it here without a field-type gate would produce
/// false positives — a daily chart category currently reads `None`.
///
/// **A weekly-decoded chart can still render with biweekly-spaced axis labels; this is not a decode
/// gap.** A chart with no report group buckets its own detail rows and, once a period yields too many
/// divisions, the engine automatically thins a crowded axis down to biweekly-spaced labels while the
/// stored ordinal genuinely reads `Weekly` — the decode is correct even though the rendered spacing
/// looks biweekly. A genuinely biweekly chart stores `used + 3 == 2` directly and decodes as
/// `Biweekly`; the chart's riser-count field (`0x0126`) is unrelated to the axis interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartCategoryPeriod {
    /// SDK ordinal 0 — one bucket per day. Not decoded from `used + 3` (see the type docs); listed
    /// for completeness / render-side use.
    Daily,
    /// SDK ordinal 1 — one bucket per week.
    Weekly,
    /// SDK ordinal 2 — one bucket per two weeks (a stored ordinal of `2` renders a true 14-day
    /// bucket axis).
    Biweekly,
    /// SDK ordinal 3 — one bucket per half-month. Provisional (see the type docs).
    Semimonthly,
    /// SDK ordinal 4 — one bucket per month.
    Monthly,
    /// SDK ordinal 5 — one bucket per quarter. Provisional (see the type docs).
    Quarterly,
    /// SDK ordinal 6 — one bucket per half-year. Provisional (see the type docs).
    SemiAnnually,
    /// SDK ordinal 7 — one bucket per year. Provisional (see the type docs).
    Annually,
}

impl ChartCategoryPeriod {
    /// Map the SDK `CrGroupConditionEnum` ordinal (the chart category grid `0xe5` leaf byte
    /// `used + 3`) to a period. Ordinals `1..=7` map to the eight date periods; ordinal `0` returns
    /// `None` (it means daily *or* no-period and cannot be told apart here without a field-type gate —
    /// see the type docs), as does any out-of-range ordinal.
    pub fn from_sdk_ordinal(ordinal: u8) -> Option<Self> {
        match ordinal {
            1 => Some(Self::Weekly),
            2 => Some(Self::Biweekly),
            3 => Some(Self::Semimonthly),
            4 => Some(Self::Monthly),
            5 => Some(Self::Quarterly),
            6 => Some(Self::SemiAnnually),
            7 => Some(Self::Annually),
            _ => None,
        }
    }

    /// The lowercase canonical token for this period — the same token stored on a report group's
    /// [`Group::date_condition`](crate::Group::date_condition) and matched by the render
    /// pipeline's date bucketer.
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Biweekly => "biweekly",
            Self::Semimonthly => "semimonthly",
            Self::Monthly => "monthly",
            Self::Quarterly => "quarterly",
            Self::SemiAnnually => "semiannually",
            Self::Annually => "annually",
        }
    }
}

/// The 3-D camera preset a chart is drawn with — the SDK `CrViewingAngleEnum`
/// (`ISCRChartStyleInternal.ViewingAngle`). Sixteen named presets; each selects a fixed
/// elevation/rotation and per-axis box aspect the native engine bakes into a 3-D rotation +
/// perspective (one fixed geometry block per preset). The
/// render side maps each variant to a concrete projection view angle.
///
/// Decoded from the `0x0121` styling struct's `+0x4cc` enum (present for every chart, but meaningful
/// only for the two 3-D families). The stored integer is the **1-based** `CrViewingAngleEnum` ordinal
/// (`crViewingAngleStandard = 1`, `TallView = 2`, … `MaxView = 16`) — see [`from_stored`](Self::from_stored).
/// An unrecognized/custom angle stores `0`, which decodes as [`Standard`](Self::Standard).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartViewAngle {
    /// Ordinal 0 — the default corner view (elevation ~36°, rotation ~42°, square floor).
    #[default]
    Standard,
    /// Ordinal 1 — a taller value axis.
    TallView,
    /// Ordinal 2 — looking down from nearly overhead (elevation ~80°).
    TopView,
    /// Ordinal 3 — a distorted (stretched value axis) standard view.
    DistortedView,
    /// Ordinal 4 — a shorter value axis.
    ShortView,
    /// Ordinal 5 — rotated to emphasise the group (category) axis from the side.
    GroupEyeView,
    /// Ordinal 6 — a low, group-emphasising view.
    GroupEmphasisView,
    /// Ordinal 7 — deepened series axis for a chart with few series.
    FewSeriesView,
    /// Ordinal 8 — shallow series axis for a chart with few groups.
    FewGroupsView,
    /// Ordinal 9 — a milder-elevation distorted standard view.
    DistortedStdView,
    /// Ordinal 10 — thickened group axis, low elevation.
    ThickGroupsView,
    /// Ordinal 11 — an even shorter value axis than [`ShortView`](Self::ShortView).
    ShorterView,
    /// Ordinal 12 — thickened series axis, strongly rotated.
    ThickSeriesView,
    /// Ordinal 13 — a thicker standard view.
    ThickStdView,
    /// Ordinal 14 — a steep overhead "bird's-eye" view.
    BirdsEyeView,
    /// Ordinal 15 — the widest/deepest overall box.
    MaxView,
}

impl ChartViewAngle {
    /// Map the stored `0x0121` `+0x4cc` byte — the **1-based** SDK `CrViewingAngleEnum` ordinal
    /// (`crViewingAngleStandard = 1` … `crViewingAngleMaxView = 16`) — to a preset. The value `0`
    /// (a custom, non-preset angle) and any out-of-range ordinal fall back to [`Standard`](Self::Standard).
    pub fn from_stored(code: u8) -> Self {
        match code {
            1 => Self::Standard,
            2 => Self::TallView,
            3 => Self::TopView,
            4 => Self::DistortedView,
            5 => Self::ShortView,
            6 => Self::GroupEyeView,
            7 => Self::GroupEmphasisView,
            8 => Self::FewSeriesView,
            9 => Self::FewGroupsView,
            10 => Self::DistortedStdView,
            11 => Self::ThickGroupsView,
            12 => Self::ShorterView,
            13 => Self::ThickSeriesView,
            14 => Self::ThickStdView,
            15 => Self::BirdsEyeView,
            16 => Self::MaxView,
            _ => Self::Standard,
        }
    }
}

/// One chart text element's stored font — the face name plus, when explicitly stored, its point
/// size. Decoded from the `0x0121` per-element font run (see [`ChartDefinition::element_fonts`]).
///
/// Only the face **name** is a reliable per-element stored fact (each text element serializes its own
/// length-prefixed face string; `"Arial"` when the element uses the default face). A point **size** is
/// byte-located only for the Title element — every other element stores its size
/// as `0` (⇒ the engine's per-element default) so [`size_pt`](Self::size_pt) is `None` there. The
/// bold/italic/weight/colour a chart text element carries are engine per-element **defaults** (byte-
/// identical everywhere), not a stored per-report signal, so they are not modeled here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChartElementFont {
    /// The stored face name for this text element (`"Arial"` = the element's default face).
    pub name: String,
    /// The element's explicit point size, decoded from its stored size byte, or `None` when the byte
    /// is `0` (⇒ the engine's per-element default size). Decoded for the Title element of a
    /// **Bar-family** chart only: stored byte `0x0c` ⇒ 14 pt, `0x11` ⇒ 20 pt (the encoding is
    /// `pt = round(byte × 7 / 6)`). The size slot for the non-Title elements — and for the Title of the
    /// pie/area/3-D families, whose styling struct is laid out differently — sits at an offset this
    /// decoder does not resolve, so their `size_pt` is `None`.
    pub size_pt: Option<u16>,
}

/// SDK: `IChartObject.ChartDefinition` — the semantically meaningful slice of a chart's
/// configuration: its visual type, subtype, and the title/axis-title/data-label strings.
///
/// Internal (not exported). The title/axis-title strings plus the legend visible/position flags are
/// modeled; the remainder of the chart's styling (axis scale/min-max, marker/riser/colour state) is
/// opaque fixed-schema render styling, left deliberately undecoded because it has no reader value and
/// no way to validate.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChartDefinition {
    /// The chart's **data-layout** axis (Group / Detail / CrossTab / OLAP) — how the chart is bound
    /// to data, the RAS `ChartDefinition.ChartType` (see [`ChartLayoutType`](crate::ChartLayoutType)).
    /// Orthogonal to [`graph_type`](Self::graph_type) (the visual shape). Decoded from the chart's
    /// `0x011c` analytic-header leaf byte 2.
    pub layout_type: crate::ChartLayoutType,
    /// The chart's visual shape (see [`ChartGraphType`]); from the first `0x0121` enum (`+0x4c`).
    pub graph_type: ChartGraphType,
    /// The chart's subtype/variant selector (e.g. side-by-side vs. stacked vs. percent for a bar
    /// chart); the second `0x0121` enum (`+0x50`). Its taxonomy is unknown, so the raw engine code is
    /// preserved verbatim.
    pub graph_subtype: i32,
    /// The chart title (`0x0121` field `+0x54`).
    pub title: String,
    /// The chart subtitle (`+0x58`); normally empty.
    pub subtitle: String,
    /// The chart footnote (`+0x5c`); normally empty.
    pub footnote: String,
    /// The group ("on change of") axis title — the category / X axis (`+0x60`).
    pub group_axis_title: String,
    /// The data ("show value") axis title — the value / Y axis.
    pub data_axis_title: String,
    /// The chart's data-value description from the `0x011f ChartDataValue` analytic record (e.g.
    /// `"Count of Command.some_field"`); empty for the grouped `0x0126` analytic variant, which has
    /// no such labeled value.
    pub data_label: String,
    /// The date-grouping period of the chart's "on change of `<date>`" category axis, when the
    /// category is a date field grouped by a period (see [`ChartCategoryPeriod`]). Decoded from the
    /// chart's category grid `0xe5` group record's SDK-ordinal byte; `None` when the category is a
    /// discrete (non-periodic) field, a daily period (not decodable here), or the chart has no
    /// category. Lets the renderer bucket the chart's date axis by the authored period instead of a
    /// fixed default.
    pub category_period: Option<ChartCategoryPeriod>,
    /// Whether the chart's legend is shown — bit0 of the low byte of the `0x0121` styling struct's
    /// leading legend `short` (`+0x410`).
    pub legend_visible: bool,
    /// Where the legend is placed (see [`ChartLegendPosition`]) — the high byte of the same legend
    /// `short`. `Right`/`Left`/`Bottom` are established; `Top` is conjecture. When the legend is
    /// hidden the stored position is not meaningful (the engine resets it on save).
    pub legend_position: ChartLegendPosition,
    /// The group (category, X) axis gridline mode (see [`ChartGridType`]) — the `0x0121` styling
    /// struct's group-axis gridline byte. The axis families only; `None` for the
    /// Pie/Doughnut/Funnel/Gauge families, which carry no group axis.
    pub group_axis_gridlines: ChartGridType,
    /// The value (Y) axis gridline mode (see [`ChartGridType`]) — the `0x0121` styling struct's
    /// value-axis gridline byte. The axis families only; `None` for the
    /// Pie/Doughnut/Funnel/Gauge families, which carry no value axis.
    pub value_axis_gridlines: ChartGridType,
    /// Whether the chart shows per-point data-value labels — bit1 of the data-labels enum byte in the
    /// `0x0121` styling struct (`+0x4a8`), reached by a fixed-width walk from the legend
    /// `short` (`00`→`02` when enabled).
    pub data_labels_show_value: bool,
    /// Per-series RGB fill colours, in series order — empty when the chart uses automatic colouring
    /// (the common case) or when no explicit colours are byte-recoverable.
    ///
    /// **Not byte-recoverable from the `.rpt` in general.** Crystal has no
    /// fixed series palette: the engine default is a runtime `rand()`-seeded assignment, and the
    /// designer bakes chosen colours into the auto-recomputed colour/GDI-handle state at `0x0121`
    /// `+0x49c..+0x4a4` (flagged "auto-recomputed" — it changes on every dialog re-save and is not a
    /// stable stored RGB triple) and pushes them into runtime chart state, not as a decodable disk field.
    /// This field is the render-side hook: it is populated only if a stable, explicit per-series RGB
    /// is ever recovered; otherwise the render side must supply a built-in fallback palette.
    /// Unobserved: no explicit palette has been recovered from a stored chart.
    pub series_colors: Vec<Color>,
    /// The 3-D camera preset (see [`ChartViewAngle`]) — only meaningful for a 3-D chart
    /// ([`is_3d`](Self::is_3d)). Decoded from the `0x0121` styling struct's `+0x4cc` enum (the byte
    /// before the 3-D-only `+0x4d0` field), interpreted only for the two 3-D families (graph_type 5/6).
    /// The stored value is the 1-based `CrViewingAngleEnum` ordinal (`crViewingAngleStandard = 1`,
    /// `crViewingAngleDistortedView = 4`). A 2-D chart leaves it at the default.
    pub view_angle: ChartViewAngle,
    /// The chart's data ("show value") bindings as RAS `ChartDefinition.DataFields` FormulaForm
    /// strings — each a summary expression `Op ({data}, {category}[, "period"])` (e.g.
    /// `Sum ({Orders.Order Amount}, {Customer.Customer Name})`), or `Op ({data})` when the chart has
    /// no "on change of" category. `Op` is the summary operation applied to the data field, and the
    /// category is the innermost ("on change of") axis field. The optional `"period"` third operand
    /// is present only for a date category grouped by an explicit non-daily period (weekly/monthly/…);
    /// the implicit daily default of a date category is NOT recoverable here (it needs the datasource
    /// field type, which is not in the report definition), so a daily-grouped date category currently
    /// omits the operand.
    pub data_refs: Vec<String>,
    /// The chart's category ("on change of") bindings as RAS `ChartDefinition.ConditionFields`
    /// FormulaForm strings — each the brace-wrapped category field reference `{field}`, in axis order
    /// (outermost first). Empty when the chart has no category axis.
    pub category_refs: Vec<String>,
    /// Per-text-element fonts (the Chart Expert *Text* tab), decoded from the `0x0121` font run — the
    /// contiguous block of length-prefixed face-name strings that follows the axis-title strings.
    /// In **stored order**; index `0` is the Title element. The
    /// remaining indices follow the RAS `ChartTextOptions` element enumeration (**inferred**, not
    /// established): `[Title, Subtitle, Footnote, GroupTitle, DataTitle,
    /// SeriesTitle, Legend, DataLabel]`. Two further label-font strings (GroupLabel, SeriesLabel)
    /// live *after* the fixed styling struct and are not captured here (they are always the default
    /// `Arial`); only the eight contiguous element fonts are decoded.
    ///
    /// Each entry carries the stored face [`name`](ChartElementFont::name); only the Title element's
    /// [`size_pt`](ChartElementFont::size_pt) is byte-located (the others are the engine per-element
    /// default). Empty when the chart's `0x0121` leaf is truncated or the font run did not parse.
    /// This is the render-side hook for drawing each element in its authored face instead of a
    /// hardcoded `Arial`.
    pub element_fonts: Vec<ChartElementFont>,
}

impl ChartDefinition {
    /// Whether the chart is drawn with a genuine 3-D scene (perspective-projected risers/walls or a
    /// meshed surface) rather than flat 2-D geometry — the signal a renderer needs to pick the 3-D
    /// geometry path (the engine routes real 3-D math for the 3-D families, not a
    /// 2-D offset).
    ///
    /// Decoded from the [`graph_type`](Self::graph_type) disk enum: `true` only for the inherently
    /// three-dimensional families [`Riser3D`](ChartGraphType::Riser3D) (code 5) and
    /// [`Surface3D`](ChartGraphType::Surface3D) (code 6, inferred).
    ///
    /// **This is distinct from the 2-D "depth effect" checkbox** (a shallow-z look on an otherwise
    /// flat chart), which does *not* make a chart 3-D and is reported separately by
    /// [`has_depth_effect`](Self::has_depth_effect). `is_3d` stays `false` for a 2-D type with depth
    /// enabled.
    pub fn is_3d(&self) -> bool {
        matches!(
            self.graph_type,
            ChartGraphType::Riser3D | ChartGraphType::Surface3D
        )
    }

    /// Whether the 2-D "depth effect" checkbox is on — a shallow-z (extruded) look drawn on an
    /// otherwise flat chart, distinct from a genuine 3-D chart type ([`is_3d`](Self::is_3d) stays
    /// `false`). A renderer can use this to give a 2-D chart a shallow-3-D appearance.
    ///
    /// The stored "depth effect" is the family's `Faked3D` [`graph_subtype`](Self::graph_subtype)
    /// variant, so the test is **family-scoped** (the depth offset differs per family, and the same
    /// raw bit means an unrelated arrangement in another family):
    /// - **Area**: bit `0x02` of the subtype (`0x14`→`0x16` when depth is enabled). The stored byte
    ///   encodes only `{percent (bit0), depth/Faked3D (bit1)}` offset by 20 — it does **not** carry the
    ///   full 6-value SDK `CrAreaChartStyleSubtypeEnum` (20..=25). The six enum values store as
    ///   `absolute`→20, `stacked`→20, `percent`→21, `faked3dabsolute`→22, `faked3dstacked`→22,
    ///   `faked3dpercent`→23: Absolute and Stacked collapse to the same stored byte for a single-series
    ///   area chart (they are visually identical), so `graph_subtype` faithfully mirrors the stored byte
    ///   and the RAS getter's Absolute→Stacked / Faked3DAbsolute→Faked3DStacked normalization (e.g. it
    ///   reports SDK 21 for a chart storing 20) is a query-time single-series display artifact, not a
    ///   decode gap. Bit `0x02` is correct across all four stored bytes; Absolute (20) and Stacked (21)
    ///   remain indistinguishable under 2+ series.
    /// - **Pie**: subtype `31` (`Faked3DRegular`) vs `30` (`Regular`) — the only depth variant in the
    ///   pie band (Multiple/Proportional have no `Faked3D` sibling).
    ///
    /// The Bar/Line families reuse the low subtype slot for the [`Percent`](ChartArrangement::Percent)/
    /// stacked arrangement, so their depth-effect bit collides with the arrangement variant and is not
    /// separable from this field.
    pub fn has_depth_effect(&self) -> bool {
        match self.graph_type {
            ChartGraphType::Area => self.graph_subtype & 0x02 != 0,
            ChartGraphType::Pie => self.graph_subtype == 31,
            _ => false,
        }
    }

    /// How the chart arranges multiple series within each category slot (clustered / stacked /
    /// percent) — the render dispatch for the axis families (bar/area/line).
    ///
    /// Decoded from the low digit of [`graph_subtype`](Self::graph_subtype) — the variant slot within
    /// the type's 10-wide gallery band. For the **Bar family** the slot selects the
    /// series arrangement: `0` = clustered (side-by-side), `1` = stacked, `2` = percent. Other
    /// families reuse the slot for unrelated variants (e.g. the Area family's depth-effect bit, see
    /// [`has_depth_effect`](Self::has_depth_effect)), so the stacked/percent read is scoped to Bar;
    /// every other family reports [`Clustered`](ChartArrangement::Clustered).
    pub fn arrangement(&self) -> ChartArrangement {
        if self.graph_type == ChartGraphType::Bar {
            match self.graph_subtype % 10 {
                1 => ChartArrangement::Stacked,
                2 => ChartArrangement::Percent,
                _ => ChartArrangement::Clustered,
            }
        } else {
            ChartArrangement::Clustered
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ChartViewAngle::from_stored` maps each **1-based** SDK `CrViewingAngleEnum` ordinal (1..=16)
    /// to its preset in order; `0` (custom/unset) and any out-of-range ordinal fall back to the
    /// default `Standard`.
    #[test]
    fn chart_view_angle_from_stored_maps_ordinals() {
        use ChartViewAngle::*;
        let order = [
            Standard,
            TallView,
            TopView,
            DistortedView,
            ShortView,
            GroupEyeView,
            GroupEmphasisView,
            FewSeriesView,
            FewGroupsView,
            DistortedStdView,
            ThickGroupsView,
            ShorterView,
            ThickSeriesView,
            ThickStdView,
            BirdsEyeView,
            MaxView,
        ];
        for (i, want) in order.iter().enumerate() {
            let code = (i + 1) as u8;
            assert_eq!(ChartViewAngle::from_stored(code), *want, "ordinal {code}");
        }
        // Stored 4 = crViewingAngleDistortedView.
        assert_eq!(ChartViewAngle::from_stored(4), DistortedView);
        assert_eq!(
            ChartViewAngle::from_stored(0),
            Standard,
            "0 (custom/unset) → default"
        );
        assert_eq!(
            ChartViewAngle::from_stored(17),
            Standard,
            "out-of-range → default"
        );
        assert_eq!(
            ChartViewAngle::from_stored(255),
            Standard,
            "out-of-range → default"
        );
        assert_eq!(ChartViewAngle::default(), Standard);
    }

    /// `has_depth_effect` is family-scoped: the Pie `Faked3DRegular` subtype (31) vs `Regular` (30),
    /// and the Area depth bit (`0x02`), each read as depth only for their own family.
    #[test]
    fn has_depth_effect_is_family_scoped() {
        let def = |graph_type, graph_subtype| ChartDefinition {
            graph_type,
            graph_subtype,
            ..Default::default()
        };
        // Pie: 31 = Faked3DRegular (depth on), 30 = Regular (depth off).
        assert!(def(ChartGraphType::Pie, 31).has_depth_effect());
        assert!(!def(ChartGraphType::Pie, 30).has_depth_effect());
        // Pie Multiple/Proportional (32/33) have no depth sibling.
        assert!(!def(ChartGraphType::Pie, 32).has_depth_effect());
        // Area: bit 0x02 toggles depth (base 20 → +2).
        assert!(def(ChartGraphType::Area, 22).has_depth_effect());
        assert!(!def(ChartGraphType::Area, 20).has_depth_effect());
        // Pie subtype 22 is meaningless — depth must not leak across families.
        assert!(!def(ChartGraphType::Bar, 22).has_depth_effect());
    }
}
