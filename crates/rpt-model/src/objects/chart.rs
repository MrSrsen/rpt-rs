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
    /// `type==3 || type==4` emits the pie-only detach/rotate enum pair for both codes, grouping 4
    /// with the pie.
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

/// How wide a bar chart's risers are drawn, as a share of the space each category gets — the
/// `0x0121` styling block's first interior enum, the SDK `CrBarSizeEnum`.
///
/// A chart authored in the designer stores [`Large`](Self::Large); the other members come from the
/// SDK's own ordinals. Applies to the riser families; the pie families size their slices with
/// [`ChartPieSize`] instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartBarSize {
    /// Code `0` — the narrowest riser.
    Min,
    /// Code `1`.
    Small,
    /// Code `2`.
    Average,
    /// Code `3` — the designer's default.
    #[default]
    Large,
    /// Code `4` — the widest riser.
    Max,
    /// A code outside the SDK enum, kept verbatim.
    Other(u8),
}

impl ChartBarSize {
    /// Map the stored `CrBarSizeEnum` ordinal.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Min,
            1 => Self::Small,
            2 => Self::Average,
            3 => Self::Large,
            4 => Self::Max,
            other => Self::Other(other),
        }
    }
}

/// How large a pie/doughnut chart draws its circle — the SDK `CrPieSizeEnum`.
///
/// The ordinals are spaced by sixteen rather than by one, so a stored `16` is
/// [`Large`](Self::Large) and not the second member of a dense enum. Stored as a 16-bit word in the
/// sizing run that follows the chart's per-element font flags, alongside the widths the engine
/// derives from [`ChartBarSize`] and [`ChartMarkerSize`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartPieSize {
    /// Code `0` — fills the plot area.
    Max,
    /// Code `16` — the designer's default.
    #[default]
    Large,
    /// Code `32`.
    Average,
    /// Code `48`.
    Small,
    /// Code `64` — the smallest circle.
    Min,
    /// A code outside the SDK enum, kept verbatim.
    Other(u8),
}

impl ChartPieSize {
    /// Map the stored `CrPieSizeEnum` ordinal.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Max,
            16 => Self::Large,
            32 => Self::Average,
            48 => Self::Small,
            64 => Self::Min,
            other => Self::Other(other),
        }
    }
}

/// Which slice a pie/doughnut chart pulls away from the rest of the circle — the SDK
/// `CrSliceDetachmentEnum`.
///
/// Stored beside a superseded copy of [`ChartPieSize`] in the pair of enums only the pie families
/// write, so a chart of any other family carries neither and reports the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartSliceDetachment {
    /// Code `0` — every slice stays in the circle.
    #[default]
    NoDetachment,
    /// Code `1` — the smallest slice is pulled out.
    SmallestSlice,
    /// Code `2` — the largest slice is pulled out.
    LargestSlice,
    /// A code outside the SDK enum, kept verbatim.
    Other(u8),
}

impl ChartSliceDetachment {
    /// Map the stored `CrSliceDetachmentEnum` ordinal.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::NoDetachment,
            1 => Self::SmallestSlice,
            2 => Self::LargestSlice,
            other => Self::Other(other),
        }
    }
}

/// How large a line/scatter chart draws its data-point markers — the SDK `CrMarkerSizeEnum`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartMarkerSize {
    /// Code `0`.
    Small,
    /// Code `1`.
    MediumSmall,
    /// Code `2` — the designer's default.
    #[default]
    Medium,
    /// Code `3`.
    MediumLarge,
    /// Code `4`.
    Large,
    /// A code outside the SDK enum, kept verbatim. The engine stores such a code, and the vendor's
    /// own model reports no marker size at all for it.
    Other(u8),
}

impl ChartMarkerSize {
    /// Map the stored `CrMarkerSizeEnum` ordinal.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Small,
            1 => Self::MediumSmall,
            2 => Self::Medium,
            3 => Self::MediumLarge,
            4 => Self::Large,
            other => Self::Other(other),
        }
    }
}

/// The shape a line/scatter chart draws its data-point markers with — the SDK `CrMarkerShapeEnum`,
/// whose ordinals are sparse (`Rectangle` is `1`, not `0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartMarkerShape {
    /// Code `1` — the designer's default.
    #[default]
    Rectangle,
    /// Code `4`.
    Circle,
    /// Code `5`.
    Diamond,
    /// Code `8`.
    Triangle,
    /// A code outside the SDK enum, kept verbatim.
    Other(u8),
}

impl ChartMarkerShape {
    /// Map the stored `CrMarkerShapeEnum` ordinal.
    pub fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Rectangle,
            4 => Self::Circle,
            5 => Self::Diamond,
            8 => Self::Triangle,
            other => Self::Other(other),
        }
    }
}

/// Whether the chart is drawn in colour or in black and white — the SDK `CrChartColorEnum`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartColorMode {
    /// Code `0` — colour.
    #[default]
    Color,
    /// Code `1` — black and white.
    BlackAndWhite,
    /// A code outside the SDK enum, kept verbatim.
    Other(u8),
}

impl ChartColorMode {
    /// Map the stored `CrChartColorEnum` ordinal.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Color,
            1 => Self::BlackAndWhite,
            other => Self::Other(other),
        }
    }
}

/// What the chart writes next to each plotted point — the SDK `CrChartDataPointEnum`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartDataPoint {
    /// Code `0` — nothing.
    #[default]
    None,
    /// Code `1` — the point's category label.
    ShowLabel,
    /// Code `2` — the point's value.
    ShowValue,
    /// A code outside the SDK enum, kept verbatim.
    Other(u8),
}

impl ChartDataPoint {
    /// Map the stored `CrChartDataPointEnum` ordinal.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::None,
            1 => Self::ShowLabel,
            2 => Self::ShowValue,
            other => Self::Other(other),
        }
    }
}

/// What a legend entry shows beside its key — the SDK `CrLegendLayoutEnum`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartLegendLayout {
    /// Code `0` — each series' share of the total.
    #[default]
    Percentage,
    /// Code `1` — each series' value.
    Amount,
    /// Code `2` — a layout the author placed by hand.
    Custom,
    /// A code outside the SDK enum, kept verbatim.
    Other(u8),
}

impl ChartLegendLayout {
    /// Map the stored `CrLegendLayoutEnum` ordinal.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Percentage,
            1 => Self::Amount,
            2 => Self::Custom,
            other => Self::Other(other),
        }
    }
}

/// How a chart's axis labels and data values are formatted numerically — the SDK
/// `CrNumberFormatEnum`, a fixed set of presets rather than a format string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartNumberFormat {
    /// Code `0`.
    #[default]
    NoDecimal,
    /// Code `1`.
    OneDecimal,
    /// Code `2`.
    TwoDecimal,
    /// Code `3`.
    CurrencyNoDecimal,
    /// Code `4`.
    CurrencyTwoDecimal,
    /// Code `5`.
    PercentNoDecimal,
    /// Code `6`.
    PercentOneDecimal,
    /// Code `7`.
    PercentTwoDecimal,
    /// A code outside the SDK enum, kept verbatim.
    Other(u8),
}

impl ChartNumberFormat {
    /// Map the stored `CrNumberFormatEnum` ordinal.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::NoDecimal,
            1 => Self::OneDecimal,
            2 => Self::TwoDecimal,
            3 => Self::CurrencyNoDecimal,
            4 => Self::CurrencyTwoDecimal,
            5 => Self::PercentNoDecimal,
            6 => Self::PercentOneDecimal,
            7 => Self::PercentTwoDecimal,
            other => Self::Other(other),
        }
    }
}

/// Whether an axis' division count is chosen by the engine or by the author — the SDK
/// `CrDivisionMethodEnum`. [`Manual`](Self::Manual) is what makes the axis' division count
/// (`…_divisions`) load-bearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartDivisionMethod {
    /// Code `0` — the engine picks the divisions.
    #[default]
    Automatic,
    /// Code `1` — the author's division count is used.
    Manual,
    /// A code outside the SDK enum, kept verbatim.
    Other(u8),
}

impl ChartDivisionMethod {
    /// Map the stored `CrDivisionMethodEnum` ordinal.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Automatic,
            1 => Self::Manual,
            other => Self::Other(other),
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
/// false positives — a daily chart category reads `None`.
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

/// One chart text element's stored font — its face name, weight and slant, and whether the element
/// was authored at all. Decoded from the `0x0121` per-element font run
/// (see [`ChartDefinition::element_fonts`]).
///
/// The face **name**, the **weight** and the **italic** flag are per-element stored facts. A point
/// **size** is not: the chart's per-element point sizes live in the chart's `CHART …` sidecar
/// stream, not in `Contents` — two reports whose only difference is a 20 pt vs a 24 pt chart title
/// are byte-identical across every `Contents` record but for the title's face string.
/// [`is_default`](Self::is_default) is what `Contents` carries in the size's place: the flag that
/// decides whether the engine's per-element default font applies.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChartElementFont {
    /// The stored face name for this text element (`"Arial"` = the element's default face).
    pub name: String,
    /// Whether this element's font is left entirely at the engine's per-element default (size,
    /// weight and slant) rather than authored on the chart's Text tab.
    ///
    /// Decoded from the element's entry in the `0x0121` per-element flag array: the stored word
    /// `0x0110` means "not authored" and any other value means the element carries an authored font.
    /// A consumer that resolves a chart's fonts applies its default table exactly where this is
    /// `true`; where it is `false`, the size the engine uses is not in `Contents` (see the type
    /// docs).
    pub is_default: bool,
    /// The element's stored font weight on the GDI scale — `400` normal, `700` bold.
    ///
    /// `0` where the record carries no per-element style block for the element: either the leaf ends
    /// before the block, or the element has no slot in it (the block holds nine entries, one short of
    /// the ten text elements). A consumer takes the weight from its default table in that case.
    pub weight: u16,
    /// Whether the element's text is italic. Meaningful only alongside a non-zero
    /// [`weight`](Self::weight) — the two are read from the same per-element style entry, so a `false`
    /// with `weight == 0` means "not stored", not "upright".
    pub italic: bool,
}

/// SDK: `IChartObject.ChartDefinition` — a chart's configuration: its visual type and subtype, its
/// title/axis-title/data-label strings, its legend, and its styling block (shape and size enums,
/// the four gridline modes, and the per-axis range / scaling / division settings).
///
/// What is opaque is the render state the engine keeps beside these: the colour and GDI-handle
/// words it recomputes on every save, and the graphics-engine template blob that lives outside this
/// record altogether.
#[derive(Debug, Clone, PartialEq, Default)]
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
    /// Whether a bar chart's risers stand up from the category axis rather than running across it.
    pub is_vertical_bar: bool,
    /// How wide the chart draws its bars/risers (see [`ChartBarSize`]).
    pub bar_size: ChartBarSize,
    /// How large a pie/doughnut chart draws its circle (see [`ChartPieSize`]).
    pub pie_size: ChartPieSize,
    /// Which slice a pie/doughnut chart pulls out of the circle (see [`ChartSliceDetachment`]).
    pub slice_detachment: ChartSliceDetachment,
    /// How large the chart draws its data-point markers (see [`ChartMarkerSize`]).
    pub marker_size: ChartMarkerSize,
    /// The shape of the chart's data-point markers (see [`ChartMarkerShape`]).
    pub marker_shape: ChartMarkerShape,
    /// Whether the chart is drawn in colour or black and white (see [`ChartColorMode`]).
    pub chart_color: ChartColorMode,
    /// What the chart writes beside each plotted point (see [`ChartDataPoint`]).
    ///
    /// [`data_labels_show_value`](Self::data_labels_show_value) is the same stored byte read as a
    /// single "shows its value" flag.
    pub data_point: ChartDataPoint,
    /// How the chart formats the value it writes beside a data point (see [`ChartNumberFormat`]).
    pub data_value_number_format: ChartNumberFormat,
    /// What a legend entry shows beside its key (see [`ChartLegendLayout`]).
    pub legend_layout: ChartLegendLayout,
    /// The group (category, X) axis gridline mode (see [`ChartGridType`]) — the first of the four
    /// consecutive gridline bytes in the `0x0121` styling block, which run in the axis order
    /// group / series / value / value-2.
    ///
    /// Every family stores it, the axis-less ones (Pie, Doughnut, Gauge, Gantt, Funnel, Histogram)
    /// included; a family that draws no group axis simply has nothing to apply it to, which is the
    /// renderer's concern and not the decoder's.
    pub group_axis_gridlines: ChartGridType,
    /// The series (depth) axis gridline mode — the second of the four gridline bytes.
    pub series_axis_gridlines: ChartGridType,
    /// The value (Y) axis gridline mode (see [`ChartGridType`]) — the third gridline byte. Stored
    /// by every family, on the same terms as
    /// [`group_axis_gridlines`](Self::group_axis_gridlines).
    pub value_axis_gridlines: ChartGridType,
    /// The secondary value (Y2) axis gridline mode — the fourth gridline byte, for a chart with a
    /// second value axis.
    pub value_axis2_gridlines: ChartGridType,
    /// The value (Y) axis lower bound, used when the axis is scaled to the author's range rather
    /// than to the data (see [`value_axis_auto_range`](Self::value_axis_auto_range)).
    ///
    /// The record stores three `(min, max)` double pairs in the axis order value / value-2 /
    /// series, and each of the per-axis runs below repeats that order.
    pub value_axis_min: f64,
    /// The value (Y) axis upper bound.
    pub value_axis_max: f64,
    /// The secondary value (Y2) axis lower bound.
    pub value_axis2_min: f64,
    /// The secondary value (Y2) axis upper bound.
    pub value_axis2_max: f64,
    /// The series (depth) axis lower bound.
    pub series_axis_min: f64,
    /// The series (depth) axis upper bound.
    pub series_axis_max: f64,
    /// How the value (Y) axis formats its scale labels (see [`ChartNumberFormat`]).
    pub value_axis_number_format: ChartNumberFormat,
    /// How the secondary value (Y2) axis formats its scale labels.
    pub value_axis2_number_format: ChartNumberFormat,
    /// How the series (depth) axis formats its scale labels.
    pub series_axis_number_format: ChartNumberFormat,
    /// Whether the value (Y) axis takes its range from the data rather than from
    /// [`value_axis_min`](Self::value_axis_min)/[`value_axis_max`](Self::value_axis_max).
    pub value_axis_auto_range: bool,
    /// Whether the secondary value (Y2) axis takes its range from the data.
    pub value_axis2_auto_range: bool,
    /// Whether the series (depth) axis takes its range from the data.
    pub series_axis_auto_range: bool,
    /// Whether the value (Y) axis scales its labels to a magnitude suffix rather than writing them
    /// in full.
    pub value_axis_auto_scale: bool,
    /// Whether the secondary value (Y2) axis scales its labels.
    pub value_axis2_auto_scale: bool,
    /// Whether the series (depth) axis scales its labels.
    pub series_axis_auto_scale: bool,
    /// Whether the value (Y) axis' division count is the engine's or the author's (see
    /// [`ChartDivisionMethod`]).
    pub value_axis_division_method: ChartDivisionMethod,
    /// Whether the secondary value (Y2) axis' division count is the engine's or the author's.
    pub value_axis2_division_method: ChartDivisionMethod,
    /// Whether the series (depth) axis' division count is the engine's or the author's.
    pub series_axis_division_method: ChartDivisionMethod,
    /// How many divisions the value (Y) axis is cut into when
    /// [`value_axis_division_method`](Self::value_axis_division_method) is
    /// [`Manual`](ChartDivisionMethod::Manual). `1` is the stored default.
    pub value_axis_divisions: u32,
    /// How many divisions the secondary value (Y2) axis is cut into.
    pub value_axis2_divisions: u32,
    /// How many divisions the series (depth) axis is cut into.
    pub series_axis_divisions: u32,
    /// Whether the chart writes each point's value beside it — the same stored byte as
    /// [`data_point`](Self::data_point), read as a single flag
    /// ([`ShowValue`](ChartDataPoint::ShowValue)).
    pub data_labels_show_value: bool,
    /// Per-series RGB fill colors, in series order — empty when the chart uses automatic coloring
    /// (the common case) or when no explicit colors are byte-recoverable.
    ///
    /// **Not byte-recoverable from the `.rpt` in general.** Crystal has no
    /// fixed series palette: the engine default is a runtime `rand()`-seeded assignment, and the
    /// designer bakes chosen colors into the auto-recomputed color/GDI-handle state at `0x0121`
    /// `+0x49c..+0x4a4` (flagged "auto-recomputed" — it changes on every dialog re-save and is not a
    /// stable stored RGB triple) and pushes them into runtime chart state, not as a decodable disk field.
    /// This field is the render-side hook: it is populated only if a stable, explicit per-series RGB
    /// is ever recovered; otherwise the render side must supply a built-in fallback palette.
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
    /// field type, which is not in the report definition), so a daily-grouped date category omits the
    /// operand.
    pub data_refs: Vec<String>,
    /// The chart's category ("on change of") bindings as RAS `ChartDefinition.ConditionFields`
    /// FormulaForm strings — each the brace-wrapped category field reference `{field}`, in axis order
    /// (outermost first). Empty when the chart has no category axis.
    pub category_refs: Vec<String>,
    /// Per-text-element fonts (the Chart Expert *Text* tab), decoded from the `0x0121` font run.
    /// Ten entries in the engine's `ChartTextOptions` element order: `[Title, Subtitle, Footnote,
    /// GroupTitle, DataTitle, SeriesTitle, Legend, GroupLabel, DataLabel, SeriesLabel]`.
    ///
    /// The record writes the faces in a different order from everything else it indexes by element —
    /// eight of them in the string block that follows the axis-title strings, and the two series
    /// elements' past the fixed styling struct — so a face is placed at its element's index here
    /// rather than in stored order. The per-element
    /// [`is_default`](ChartElementFont::is_default) flags and the
    /// [`weight`](ChartElementFont::weight)/[`italic`](ChartElementFont::italic) entries that follow
    /// them are already in element order.
    ///
    /// Empty when the chart's `0x0121` leaf is truncated or the font run did not parse. This is the
    /// render-side hook for drawing each element in its authored face, weight and slant instead of a
    /// hardcoded `Arial`, and for knowing which elements the engine's default font table applies to.
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
    /// - **Bar**: subtypes `3`/`4`/`5` (`Faked3DSideBySide`/`Faked3DStacked`/`Faked3DPercent`) are the
    ///   depth siblings of `0`/`1`/`2`. Unlike Area, the bar band stores the SDK
    ///   `CrBarChartStyleSubtypeEnum` ordinal verbatim — all six values round-trip to six distinct
    ///   stored bytes — so depth and the [`arrangement`](Self::arrangement) are separable after all;
    ///   they are not two readings of the same slot.
    /// - **Pie**: subtype `31` (`Faked3DRegular`) vs `30` (`Regular`) — the only depth variant in the
    ///   pie band (Multiple `32` / MultipleProportional `33` have no `Faked3D` sibling, so a
    ///   "band offset is odd" shortcut would misreport `33`).
    ///
    /// Every other family reports `false` as a **fact, not a gap**: their SDK subtype enums declare no
    /// `Faked3D` member at all, so the gallery has no depth entry to store. `CrLineChartStyleSubtypeEnum`
    /// (`10..=15`) spends its extra three slots on markers, not depth; `CrDoughnutChartStyleSubtypeEnum`
    /// (`40..=42`), `CrXYScatterChartStyleSubtypeEnum` (`70..=73`), `CrRadarChartStyleSubtypeEnum`
    /// (`80..=82`), `CrBubbleChartStyleSubtypeEnum` (`90..=91`) and `CrStockedChartStyleSubtypeEnum`
    /// (`100..=101`) have no depth variant either. The inherently three-dimensional
    /// [`Riser3D`](ChartGraphType::Riser3D)/[`Surface3D`](ChartGraphType::Surface3D) families are
    /// reported by [`is_3d`](Self::is_3d) instead — a real 3-D scene is not the 2-D depth checkbox.
    pub fn has_depth_effect(&self) -> bool {
        match self.graph_type {
            ChartGraphType::Bar => (3..=5).contains(&self.graph_subtype),
            ChartGraphType::Area => self.graph_subtype & 0x02 != 0,
            ChartGraphType::Pie => self.graph_subtype == 31,
            _ => false,
        }
    }

    /// How the chart arranges multiple series within each category slot (clustered / stacked /
    /// percent) — the render dispatch for the axis families (bar/area/line).
    ///
    /// Decoded from the variant slot within the type's 10-wide gallery band
    /// ([`graph_subtype`](Self::graph_subtype)). For the **Bar family** the band stores the SDK
    /// `CrBarChartStyleSubtypeEnum` ordinal verbatim, and its six values are three arrangements
    /// crossed with the depth-effect flag: `0`/`3` = clustered (side-by-side), `1`/`4` = stacked,
    /// `2`/`5` = percent, the depth half of each pair being reported by
    /// [`has_depth_effect`](Self::has_depth_effect). Other families reuse the slot for unrelated
    /// variants (the Area band's depth bit, the Line band's markers), so the stacked/percent read is
    /// scoped to Bar; every other family reports [`Clustered`](ChartArrangement::Clustered).
    pub fn arrangement(&self) -> ChartArrangement {
        if self.graph_type == ChartGraphType::Bar {
            match self.graph_subtype {
                1 | 4 => ChartArrangement::Stacked,
                2 | 5 => ChartArrangement::Percent,
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
        // Pie Multiple/Proportional (32/33) have no depth sibling — and 33 is the case a
        // "band offset is odd" shortcut would get wrong.
        assert!(!def(ChartGraphType::Pie, 33).has_depth_effect());
        // Area: bit 0x02 toggles depth (base 20 → +2).
        assert!(def(ChartGraphType::Area, 22).has_depth_effect());
        assert!(!def(ChartGraphType::Area, 20).has_depth_effect());
        // Bar: the stored byte is the SDK ordinal, so 3/4/5 are the depth siblings of 0/1/2.
        for flat in 0..=2 {
            assert!(!def(ChartGraphType::Bar, flat).has_depth_effect());
            assert!(def(ChartGraphType::Bar, flat + 3).has_depth_effect());
        }
        // An area subtype is meaningless on Bar — depth must not leak across families.
        assert!(!def(ChartGraphType::Bar, 22).has_depth_effect());
        // Line and Doughnut declare no Faked3D member at all: false across their whole band.
        for sub in 10..=15 {
            assert!(!def(ChartGraphType::Line, sub).has_depth_effect());
        }
        for sub in 40..=42 {
            assert!(!def(ChartGraphType::Doughnut, sub).has_depth_effect());
        }
        // A genuinely 3-D family is `is_3d`, not the 2-D depth checkbox.
        assert!(!def(ChartGraphType::Riser3D, 50).has_depth_effect());
        assert!(def(ChartGraphType::Riser3D, 50).is_3d());
    }

    /// The Bar band pairs each arrangement with its depth sibling, so the arrangement survives the
    /// depth flag: `0`/`3` clustered, `1`/`4` stacked, `2`/`5` percent.
    #[test]
    fn bar_arrangement_survives_the_depth_sibling() {
        let bar = |graph_subtype| ChartDefinition {
            graph_type: ChartGraphType::Bar,
            graph_subtype,
            ..Default::default()
        };
        use ChartArrangement::*;
        for (sub, want) in [
            (0, Clustered),
            (1, Stacked),
            (2, Percent),
            (3, Clustered),
            (4, Stacked),
            (5, Percent),
        ] {
            assert_eq!(bar(sub).arrangement(), want, "bar subtype {sub}");
        }
        // Only Bar reads the slot as an arrangement.
        let line = ChartDefinition {
            graph_type: ChartGraphType::Line,
            graph_subtype: 11,
            ..Default::default()
        };
        assert_eq!(line.arrangement(), Clustered);
    }
}
