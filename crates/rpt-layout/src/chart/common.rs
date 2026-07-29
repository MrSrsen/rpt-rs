//! Shared chart primitives: the qualitative palette, the legend, and the Num+Ord axis frame
//! ([`chart_frame`]/[`Frame`]) plus the label/format helpers the per-type chart renderers
//! ([`super::bar`]/[`super::line`]/[`super::pie`]) build on.

use rpt_model::{ChartDefinition, Color, Rect, Twips};
use rpt_pages::{
    DrawOp, FontSpec, LineOp, ObjectKind, ObjectRef, Point, RectOp, Stroke, TextAlign, TextRun,
};

/// Slice-edge separator and the white a chart draws slice borders in.
pub(super) const WHITE: Color = Color {
    a: 255,
    r: 255,
    g: 255,
    b: 255,
};

/// An opaque RGB color (the palette's alpha is always fully opaque).
const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { a: 255, r, g, b }
}

/// Crystal's default chart palette, cycled per category/series. The scheme is not stored in the
/// `.rpt`, so it is hard-coded here to match the engine's default colors. The full sequence is
/// **20 colors**; the engine cycles it with period 20 (a chart's 21st mark reuses the first
/// color), so every family indexes it as `PALETTE[i % PALETTE.len()]`.
pub(super) const PALETTE: [Color; 20] = [
    rgb(0x3a, 0x65, 0x98),
    rgb(0xef, 0xa2, 0x52),
    rgb(0x00, 0x94, 0x70),
    rgb(0xdd, 0x58, 0x1f),
    rgb(0xa2, 0x2d, 0x62),
    rgb(0xfe, 0xce, 0x60),
    rgb(0x27, 0x75, 0x8b),
    rgb(0xda, 0x70, 0x62),
    rgb(0x44, 0x77, 0x11),
    rgb(0xc8, 0x27, 0x59),
    rgb(0x5d, 0x07, 0x9c),
    rgb(0xe3, 0xd6, 0x3c),
    rgb(0xda, 0xa4, 0xc8),
    rgb(0x33, 0x81, 0xcc),
    rgb(0xf1, 0xc2, 0x83),
    rgb(0xa4, 0x77, 0x34),
    rgb(0x92, 0xba, 0xe3),
    rgb(0xb6, 0x3d, 0x32),
    rgb(0x34, 0xce, 0x91),
    rgb(0xff, 0x7a, 0x59),
];

/// The fill color for pie/doughnut slice `i`: the same [`PALETTE`] cycled with the engine's
/// period-20 wrap (a 25-slice pie reuses the first five colors for its last five slices). Named for
/// the proportional families' call sites; identical to how the axis families index [`PALETTE`]
/// directly.
pub(super) fn slice_color(i: usize) -> Color {
    PALETTE[i % PALETTE.len()]
}

pub(super) const AXIS: Color = Color {
    a: 255,
    r: 0x55,
    g: 0x55,
    b: 0x55,
};
pub(super) const LABEL: Color = Color {
    a: 255,
    r: 0x22,
    g: 0x22,
    b: 0x22,
};
/// Light horizontal gridlines at the value-axis ticks (also the polar grid rings).
pub(super) const GRID: Color = Color {
    a: 255,
    r: 0xdd,
    g: 0xdd,
    b: 0xdd,
};

/// A chart text element, keying both the engine's per-element default font ([`chart_font`]) and the
/// stored per-element font override ([`ChartStyle::font`]). Every chart's text starts from the
/// shared default table; a variant that maps to a decoded [`ChartElementFont`](rpt_model::ChartElementFont)
/// prefers the stored face/size when the chart authored one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChartText {
    /// The chart heading — Arial 14 bold.
    Title,
    /// The subtitle under the title — Arial 10 normal.
    Subtitle,
    /// The footnote at the chart bottom — Arial 8 bold + italic.
    Footnote,
    /// The category-axis caption — Arial 8 bold.
    GroupAxisTitle,
    /// The value-axis caption — Arial 8 bold.
    DataAxisTitle,
    /// A category tick label — Arial 7 normal.
    GroupLabel,
    /// A data-value label annotating a riser/marker/slice — Arial 7 normal.
    DataLabel,
    /// A legend entry — Arial 7 normal.
    Legend,
}

impl ChartText {
    /// The index of this element in a chart's stored [`ChartDefinition::element_fonts`] run.
    ///
    /// The run is in the engine's text-element order, which interleaves the labels differently from
    /// this enum: `[Title, Subtitle, Footnote, GroupTitle, DataTitle, SeriesTitle, Legend, GroupLabel,
    /// DataLabel, SeriesLabel]`. The series elements have no counterpart here — the renderers draw no
    /// series title or series label — so nothing maps to index `5` or `9`.
    fn font_index(self) -> usize {
        match self {
            Self::Title => 0,
            Self::Subtitle => 1,
            Self::Footnote => 2,
            Self::GroupAxisTitle => 3,
            Self::DataAxisTitle => 4,
            Self::Legend => 6,
            Self::GroupLabel => 7,
            Self::DataLabel => 8,
        }
    }
}

/// The chart height every chart point size is quoted at: 4480 twips (224 pt). The engine sizes a
/// chart's text in proportion to the chart's **height** — its width has no effect — so a stored or
/// default point size renders at its face value only on a chart this tall, and a chart half as tall
/// draws every element at half the size.
pub(super) const FONT_REF_HEIGHT: i32 = 4480;

/// The stored font weight from which a chart text element draws bold, on the GDI scale a chart's
/// per-element weights use (`400` normal, `700` bold).
const BOLD_WEIGHT: u16 = 700;

/// The native engine's default font for a chart text `element`, at [`FONT_REF_HEIGHT`]. This is the
/// single source of truth for the per-element font defaults; the family renderers' size constants
/// below mirror it. Renderers resolve the size a chart actually draws at through
/// [`ChartStyle::font`], which scales this by the chart's height.
pub(crate) fn chart_font(element: ChartText) -> FontSpec {
    let (size_pt, bold, italic) = match element {
        ChartText::Title => (14.0, true, false),
        ChartText::Subtitle => (10.0, false, false),
        ChartText::Footnote => (8.0, true, true),
        ChartText::GroupAxisTitle | ChartText::DataAxisTitle => (8.0, true, false),
        ChartText::GroupLabel | ChartText::DataLabel | ChartText::Legend => (7.0, false, false),
    };
    FontSpec {
        family: "Arial".into(),
        size_pt,
        bold,
        italic,
        ..Default::default()
    }
}

/// A chart's stored definition together with the height it is drawn at — everything that decides
/// what font a chart text element renders in. Threaded through the renderers in place of the bare
/// definition, because a point size alone does not determine a size: the engine scales every chart
/// text element with the chart's height (see [`FONT_REF_HEIGHT`]).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ChartStyle<'a> {
    pub(crate) def: &'a ChartDefinition,
    /// The chart object's own height, before any caption or legend band is reserved from it.
    pub(crate) height: Twips,
}

impl ChartStyle<'_> {
    /// The point size a chart text quoted as `base_pt` at [`FONT_REF_HEIGHT`] renders at on this
    /// chart. The engine sizes chart text in whole device pixels at 96 dpi, so the scaled size is
    /// truncated to the pixel below (a quarter point) and never falls below one pixel.
    pub(super) fn scaled_pt(self, base_pt: f32) -> f32 {
        // Multiply before dividing: a size that lands exactly on a pixel must not truncate to the
        // one below because 96/72 is not representable.
        let px = f64::from(base_pt) * 4.0 * f64::from(self.height.0)
            / (3.0 * f64::from(FONT_REF_HEIGHT));
        (px.floor().max(1.0) * 0.75) as f32
    }

    /// The font a chart text `element` renders in: the engine's per-element default
    /// ([`chart_font`]) with the chart's stored per-element font layered on top, scaled to the
    /// chart's height.
    ///
    /// A stored [`ChartElementFont`](rpt_model::ChartElementFont) supplies the face `name` (when it is
    /// a real override — non-empty and not the `"Arial"` default sentinel) and the weight and slant.
    /// The point size is never among them, so it always comes from the default table. An element the
    /// record does not carry — an empty `element_fonts`, an out-of-range index, or the one element the
    /// style block omits (weight `0`) — keeps the default table's weight and slant too.
    pub(crate) fn font(self, element: ChartText) -> FontSpec {
        let mut font = chart_font(element);
        if let Some(stored) = self.def.element_fonts.get(element.font_index()) {
            if !stored.name.is_empty() && stored.name != "Arial" {
                font.family = stored.name.clone();
            }
            if stored.weight != 0 {
                font.bold = stored.weight >= BOLD_WEIGHT;
                font.italic = stored.italic;
            }
        }
        font.size_pt = self.scaled_pt(font.size_pt);
        font
    }

    /// The label font's rendered point size — the basis of the label metrics the axis fit and the
    /// legend measure with.
    pub(super) fn label_pt(self) -> f32 {
        self.scaled_pt(LABEL_PT)
    }

    /// Estimated advance of one category-label character, in twips: half an em of the label font
    /// (the mean advance of Arial's digits, punctuation and lower-case letters). The chart renderers
    /// get no `TextLayout`, so the axis sizes its labels from this estimate rather than real metrics.
    pub(super) fn label_char_w(self) -> i32 {
        (self.label_pt() * 20.0 / 2.0) as i32
    }

    /// One line of the label font, in twips (1.15 em — Arial's ascent + descent + line gap).
    pub(super) fn label_line_h(self) -> i32 {
        (self.label_pt() * 23.0) as i32
    }
}

/// The native engine's default chart-title point size at [`FONT_REF_HEIGHT`] (Arial 14, bold) —
/// mirrors [`chart_font`]`(`[`ChartText::Title`]`)`, kept as a constant for the family renderers that
/// build their own [`FontSpec`] through [`ChartStyle::scaled_pt`].
pub(super) const TITLE_PT: f32 = 14.0;
/// The native default data/tick/legend-label point size at [`FONT_REF_HEIGHT`] (Arial 7).
pub(super) const LABEL_PT: f32 = 7.0;
/// The white separator stroke drawn between adjacent pie/doughnut slices.
pub(super) const SLICE_BORDER_W: Twips = Twips(20);
/// Legend layout (twips): outer padding, color-swatch size, and gap between stacked entries.
pub(super) const LEGEND_PAD: i32 = 90;
pub(super) const LEGEND_SWATCH: i32 = 150;
pub(super) const LEGEND_GAP: i32 = 60;

/// A centered chart text `TextRun` spanning `bounds` in the `element`'s resolved font (the chart's
/// stored per-element override, else the default table) — the shared shape for the title / subtitle /
/// footnote drawn around a chart.
pub(crate) fn chart_text_op(
    style: ChartStyle,
    bounds: Rect,
    text: &str,
    element: ChartText,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> DrawOp {
    DrawOp::Text(TextRun {
        bounds,
        text: text.to_string(),
        font: style.font(element),
        color: LABEL,
        align: TextAlign::Center,
        rotation: 0.0,
        metrics: None,
        character_spacing: Twips(0),
        source: src(),
    })
}

/// A centered, bold chart title `TextRun` spanning `(x, y, w, h)` — the shared shape the
/// proportional families (pie/doughnut/funnel/gauge/radar) draw at the top of their area.
pub(super) fn title_op(
    style: ChartStyle,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    title: &str,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> DrawOp {
    let bounds = Rect {
        left: Twips(x),
        top: Twips(y),
        width: Twips(w),
        height: Twips(h),
    };
    chart_text_op(style, bounds, title, ChartText::Title, src)
}

/// The centre `(cx, cy)` and radius (twips) of the disc a round family (pie/doughnut/gauge/radar)
/// draws in the area below the title, leaving a small margin (`pad` = 60) for outer labels. The
/// radius is returned as `i32`; f64-based renderers cast it.
pub(super) fn centered_disc(rect: Rect, title_h: i32) -> (i32, i32, i32) {
    let (rl, rt, rw, rh) = (rect.left.0, rect.top.0, rect.width.0, rect.height.0);
    let pad = 60;
    let box_top = rt + title_h + pad;
    let box_h = (rt + rh - pad - box_top).max(1);
    let box_w = (rw - 2 * pad).max(1);
    let cx = rl + rw / 2;
    let cy = box_top + box_h / 2;
    let radius = (box_w.min(box_h) / 2 * 4 / 5).max(1);
    (cx, cy, radius)
}

/// A category label centered on `(x, y)` — a 1400×200-twip label-font box the pie/doughnut families
/// draw at each slice's outer midpoint.
pub(super) fn disc_label(
    style: ChartStyle,
    x: i32,
    y: i32,
    text: &str,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> DrawOp {
    DrawOp::Text(TextRun {
        bounds: Rect {
            left: Twips(x - 700),
            top: Twips(y - 100),
            width: Twips(1400),
            height: Twips(200),
        },
        text: text.to_string(),
        font: style.font(ChartText::GroupLabel),
        color: LABEL,
        align: TextAlign::Center,
        rotation: 0.0,
        metrics: None,
        character_spacing: Twips(0),
        source: src(),
    })
}

/// The most divisions the engine puts on an auto-scaled value axis: at most 9 gaps, i.e. at most 10
/// tick labels counting the `0`. The step is the smallest [`AXIS_STEP_MANTISSAS`] value that keeps
/// the axis within it, which also pins the division count to 5..=9.
const MAX_AXIS_DIVISIONS: f64 = 9.0;

/// The mantissas the engine's auto-scaled tick step is drawn from, each ×10ⁿ. Note the `4`: a step of
/// 4/40/400 is common in the engine's output (a count axis maxing at 24 ticks 0/4/8/12/16/20/24) and
/// a plain 1/2/5 ladder cannot produce it.
const AXIS_STEP_MANTISSAS: [f64; 4] = [1.0, 2.0, 4.0, 5.0];

/// A "nice" value-axis maximum ≥ `max` and its round tick step, reproducing the engine's auto scale:
/// the step is the smallest 1/2/4/5×10ⁿ value ≥ `max / 9`, and the top tick is the first step
/// multiple at or above the data max (24 → step 4, top tick 24; 1_677_019.9 → step 200_000, top tick
/// 1_800_000). Returns `(nice_max, step)`.
pub(super) fn nice_scale(max: f64) -> (f64, f64) {
    // A non-finite (NaN/±Inf) or non-positive max falls back to a unit scale, so an axis never scales
    // to a NaN/Inf tick step (which would poison every riser height).
    if !max.is_finite() || max <= 0.0 {
        return (1.0, 1.0);
    }
    let raw_step = max / MAX_AXIS_DIVISIONS;
    let mag = 10f64.powf(raw_step.log10().floor());
    let norm = raw_step / mag;
    let nice = AXIS_STEP_MANTISSAS
        .iter()
        .copied()
        .find(|m| norm <= *m)
        .unwrap_or(10.0);
    let step = nice * mag;
    ((max / step).ceil() * step, step)
}

/// Where the legend sits relative to the plot. The caller maps the decoded
/// [`rpt_model::ChartLegendPosition`] onto this rendering-side enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegendPosition {
    Right,
    Left,
    Top,
    Bottom,
}

/// Series-legend chrome, in twips. The engine draws a white, black-bordered box holding one swatch
/// and the series name; the box's outer edge sits [`SERIES_MARGIN`] inside the chart rect and the
/// plot keeps [`SERIES_GAP`] clear of it. Sizes are absolute (like the Arial-7 label font they
/// surround), not a fraction of the chart.
const SERIES_MARGIN: i32 = 115;
const SERIES_GAP: i32 = 120;
const SERIES_PAD_LEAD: i32 = 120;
const SERIES_PAD_TRAIL: i32 = 200;
const SERIES_SWATCH_W: i32 = 135;
const SERIES_SWATCH_H: i32 = 120;
const SERIES_SWATCH_GAP: i32 = 82;
const SERIES_BOX_H: i32 = 360;

/// Width in twips of the series-legend box holding `name` — the chrome around a label of the
/// estimated width [`ChartStyle::label_char_w`] gives it.
fn series_box_w(style: ChartStyle, name: &str) -> i32 {
    SERIES_PAD_LEAD
        + SERIES_SWATCH_W
        + SERIES_SWATCH_GAP
        + name.chars().count() as i32 * style.label_char_w()
        + SERIES_PAD_TRAIL
}

/// Reserve a band on `pos`'s side of `rect` for a **single-series** legend naming the plotted series
/// (`Sum of id`, `Min of total`), and return its draw-ops plus the reduced rect the chart body should
/// draw into.
///
/// This is the legend the engine gives the families whose marks are all one color — area, line,
/// stock and radar — where a per-category swatch list would be meaningless. It is one boxed entry: a
/// [`PALETTE`]`[0]` swatch and the series name, right-aligned to a fixed margin inside the chart rect
/// with the plot kept clear of it. The per-category families keep [`legend`].
pub(crate) fn series_legend(
    style: ChartStyle,
    rect: Rect,
    pos: LegendPosition,
    name: &str,
    section_name: &str,
    obj_name: &str,
) -> (Vec<DrawOp>, Rect) {
    let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(obj_name));
    let (rl, rt, rw, rh) = (rect.left.0, rect.top.0, rect.width.0, rect.height.0);
    // Never let the legend crowd out the plot: cap the band at a third of the chart.
    let box_w = series_box_w(style, name).min((rw / 3).max(1));
    let box_h = SERIES_BOX_H.min(rh);
    let band = box_w + SERIES_MARGIN + SERIES_GAP;
    if band >= rw {
        return (Vec::new(), rect);
    }
    let (box_left, body) = match pos {
        LegendPosition::Left => (
            rl + SERIES_MARGIN,
            Rect {
                left: Twips(rl + band),
                top: Twips(rt),
                width: Twips(rw - band),
                height: Twips(rh),
            },
        ),
        // A top/bottom-positioned single-entry legend keeps the right-hand band, so the plot width
        // it leaves is the same whichever side the chart asked for.
        _ => (
            rl + rw - SERIES_MARGIN - box_w,
            Rect {
                left: Twips(rl),
                top: Twips(rt),
                width: Twips(rw - band),
                height: Twips(rh),
            },
        ),
    };
    let box_top = rt + (rh - box_h) / 2;
    let mut ops = vec![DrawOp::Rect(RectOp {
        bounds: Rect {
            left: Twips(box_left),
            top: Twips(box_top),
            width: Twips(box_w),
            height: Twips(box_h),
        },
        fill: Some(WHITE.into()),
        stroke: Some(Stroke {
            color: AXIS,
            width: Twips(15),
            style: rpt_pages::LineStyle::Single,
        }),
        corner_radius: Twips(0),
        source: src(),
    })];
    ops.push(DrawOp::Rect(RectOp {
        bounds: Rect {
            left: Twips(box_left + SERIES_PAD_LEAD),
            top: Twips(box_top + (box_h - SERIES_SWATCH_H) / 2),
            width: Twips(SERIES_SWATCH_W),
            height: Twips(SERIES_SWATCH_H),
        },
        fill: Some(PALETTE[0].into()),
        stroke: None,
        corner_radius: Twips(0),
        source: src(),
    }));
    let text_left = box_left + SERIES_PAD_LEAD + SERIES_SWATCH_W + SERIES_SWATCH_GAP;
    let line_h = style.label_line_h();
    ops.push(DrawOp::Text(TextRun {
        bounds: Rect {
            left: Twips(text_left),
            top: Twips(box_top + (box_h - line_h) / 2),
            width: Twips((box_left + box_w - SERIES_PAD_TRAIL - text_left).max(1)),
            height: Twips(line_h),
        },
        text: name.to_string(),
        font: style.font(ChartText::Legend),
        color: LABEL,
        align: TextAlign::Left,
        rotation: 0.0,
        metrics: None,
        character_spacing: Twips(0),
        source: src(),
    }));
    (ops, body)
}

/// Reserve a band on `pos`'s side of `rect` for the legend, draw one (swatch + category label) entry
/// per series point (colored to match the bars/slices), and return the legend draw-ops plus the
/// reduced rect the chart body should draw into. A single-series group chart legends its categories
/// (each a distinct color), matching Crystal's group-chart legend. `per_slice` selects the swatch
/// coloring: `true` gives each entry a distinct color ([`slice_color`], matching a pie/doughnut's
/// per-slice fills), `false` cycles the base [`PALETTE`] (matching the bar/line/area families).
/// Composed by the caller so each chart-type renderer stays legend-agnostic (it just draws into the
/// returned body rect).
pub(crate) fn legend(
    style: ChartStyle,
    rect: Rect,
    pos: LegendPosition,
    series: &[(String, f64)],
    per_slice: bool,
    section_name: &str,
    obj_name: &str,
) -> (Vec<DrawOp>, Rect) {
    let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(obj_name));
    let (rl, rt, rw, rh) = (rect.left.0, rect.top.0, rect.width.0, rect.height.0);
    let mut ops: Vec<DrawOp> = Vec::new();
    let pad = LEGEND_PAD;
    let swatch = LEGEND_SWATCH;
    let gap = LEGEND_GAP;
    let font_pt = style.label_pt();

    let swatch_op = |ops: &mut Vec<DrawOp>, x: i32, y: i32, size: i32, i: usize| {
        let fill = if per_slice {
            slice_color(i)
        } else {
            PALETTE[i % PALETTE.len()]
        };
        ops.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(x),
                top: Twips(y),
                width: Twips(size),
                height: Twips(size),
            },
            fill: Some(fill.into()),
            stroke: None,
            corner_radius: Twips(0),
            source: src(),
        }));
    };
    let label_op =
        |ops: &mut Vec<DrawOp>, x: i32, y: i32, w: i32, size_pt: f32, label: &str, align| {
            ops.push(DrawOp::Text(TextRun {
                bounds: Rect {
                    left: Twips(x),
                    top: Twips(y),
                    width: Twips(w),
                    height: Twips(((size_pt * 30.0) as i32).max(140)),
                },
                text: label.to_string(),
                font: FontSpec {
                    family: "Arial".into(),
                    size_pt,
                    ..Default::default()
                },
                color: LABEL,
                align,
                rotation: 0.0,
                metrics: None,
                character_spacing: Twips(0),
                source: src(),
            }));
        };

    match pos {
        LegendPosition::Right | LegendPosition::Left => {
            let band_w = (rw / 4).clamp(1400, 3200);
            let n = series.len().max(1) as i32;
            // Fit entries to the band height: when the natural pitch would overflow the box, compress
            // the pitch and scale the swatch + font down with it (matching the native legend
            // auto-fit) so a high-cardinality legend stays inside the chart rect.
            let avail = (rh - 120).max(1);
            let natural = swatch + gap;
            let entry_h = natural.min(avail / n);
            let sw = swatch.min((entry_h * 5 / 7).max(40));
            // The auto-fit floor is 4 pt, but never above the label font itself: on a chart short
            // enough that the label is already under 4 pt, the floor is the label size.
            let fs =
                (font_pt * (entry_h as f32 / natural as f32)).clamp(4.0_f32.min(font_pt), font_pt);
            let total_h = n * entry_h;
            let (band_left, body) = match pos {
                LegendPosition::Right => (
                    rl + rw - band_w,
                    Rect {
                        left: Twips(rl),
                        top: Twips(rt),
                        width: Twips((rw - band_w).max(1)),
                        height: Twips(rh),
                    },
                ),
                _ => (
                    rl,
                    Rect {
                        left: Twips(rl + band_w),
                        top: Twips(rt),
                        width: Twips((rw - band_w).max(1)),
                        height: Twips(rh),
                    },
                ),
            };
            let mut y = rt + (rh - total_h).max(0) / 2;
            for (i, (label, _)) in series.iter().enumerate() {
                let sy = y + (entry_h - sw) / 2;
                swatch_op(&mut ops, band_left + pad, sy, sw, i);
                label_op(
                    &mut ops,
                    band_left + pad + sw + gap,
                    sy,
                    band_w - pad * 2 - sw - gap,
                    fs,
                    label,
                    TextAlign::Left,
                );
                y += entry_h;
            }
            (ops, body)
        }
        LegendPosition::Top | LegendPosition::Bottom => {
            let band_h = (rh / 6).clamp(300, 700);
            let n = series.len().max(1) as i32;
            let slot = (rw - pad * 2) / n;
            let band_top = match pos {
                LegendPosition::Top => rt,
                _ => rt + rh - band_h,
            };
            let body = match pos {
                LegendPosition::Top => Rect {
                    left: Twips(rl),
                    top: Twips(rt + band_h),
                    width: Twips(rw),
                    height: Twips((rh - band_h).max(1)),
                },
                _ => Rect {
                    left: Twips(rl),
                    top: Twips(rt),
                    width: Twips(rw),
                    height: Twips((rh - band_h).max(1)),
                },
            };
            let y = band_top + (band_h - swatch) / 2;
            for (i, (label, _)) in series.iter().enumerate() {
                let x = rl + pad + i as i32 * slot;
                swatch_op(&mut ops, x, y, swatch, i);
                label_op(
                    &mut ops,
                    x + swatch + gap,
                    y,
                    slot - swatch - gap,
                    font_pt,
                    label,
                    TextAlign::Left,
                );
            }
            (ops, body)
        }
    }
}

/// The value-axis (Y) and category-axis (X) titles an axis chart draws around its plot. The engine
/// draws the value-axis title (`data_axis_title`, e.g. "Sum of id") rotated 90° up the left of the
/// value axis and the category-axis title (`group_axis_title`, e.g. "created_at") horizontally below
/// the category labels. Empty strings reserve no band.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AxisTitles<'a> {
    /// Value-axis (Y) title, drawn rotated 90° CCW to the left of the tick labels.
    pub(crate) value: &'a str,
    /// Category-axis (X) title, drawn horizontally centered below the category labels.
    pub(crate) category: &'a str,
}

/// The shared context every chart renderer draws from: the chart's [`ChartStyle`] (its decoded
/// definition and the height its text scales with), the placed `rect`, the resolved `title`, the
/// axis titles, the source section/object names, and the "show value" flag. Bundles the arguments
/// every renderer signature needs instead of threading each one through separately; the per-chart
/// data (series/points/values/bars) stays a separate argument.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ChartCtx<'a> {
    pub(crate) style: ChartStyle<'a>,
    pub(crate) rect: Rect,
    pub(crate) title: &'a str,
    pub(crate) axis_titles: AxisTitles<'a>,
    pub(crate) section_name: &'a str,
    pub(crate) obj_name: &'a str,
    /// Whether the report's decoded "show value" flag draws the per-point data labels.
    pub(crate) show_labels: bool,
}

impl ChartCtx<'_> {
    /// The draw-op `source` back-reference for this chart's object — the single construction site the
    /// renderer's draw-ops share.
    pub(crate) fn src(&self) -> Option<ObjectRef> {
        Some(ObjectRef::new(self.section_name, ObjectKind::Chart).named(self.obj_name))
    }
}

#[cfg(test)]
impl ChartStyle<'static> {
    /// A renderer-test style: a chart storing all defaults, drawn `height` tall.
    pub(crate) fn test(height: Twips) -> Self {
        static DEF: std::sync::OnceLock<ChartDefinition> = std::sync::OnceLock::new();
        ChartStyle {
            def: DEF.get_or_init(ChartDefinition::default),
            height,
        }
    }
}

#[cfg(test)]
impl ChartCtx<'static> {
    /// A renderer-test context: the fields that vary per test, with a default definition and fixed
    /// placeholder section/object names (no unit test asserts on the draw-op source).
    pub(crate) fn test(
        rect: Rect,
        title: &'static str,
        axis_titles: AxisTitles<'static>,
        show_labels: bool,
    ) -> Self {
        ChartCtx {
            style: ChartStyle::test(rect.height),
            rect,
            title,
            axis_titles,
            section_name: "S",
            obj_name: "G",
            show_labels,
        }
    }
}

/// The Num+Ord plot frame shared by the axis chart families (bar / line / area): the plot rectangle,
/// the category-slot width, and the 0..max value scale.
pub(super) struct Frame {
    pub(super) plot_left: i32,
    pub(super) plot_bottom: i32,
    pub(super) plot_h: i32,
    /// Right edge of the plot rectangle.
    pub(super) plot_right: i32,
    /// Horizontal width of one category's slot.
    pub(super) slot: i32,
    /// Reserved height of the category-label band under the axis.
    pub(super) cat_h: i32,
    /// The value-axis maximum (bars/points scale to this).
    pub(super) max_val: f64,
    /// The rounded value-axis tick step (`0`/`step`/`2·step`/… ticks).
    pub(super) step: f64,
    /// How the category labels are drawn (upright or rotated, and at what stride).
    pub(super) cats: CategoryAxis,
}

impl Frame {
    /// Top of the plot rectangle (the full-scale line). Value labels clamp to this so a full-height
    /// riser's label stays inside the frame.
    pub(super) fn plot_top(&self) -> i32 {
        self.plot_bottom - self.plot_h
    }
}

/// Compute the shared axis-chart frame (band reservation + `nice_scale`) without emitting any ops —
/// the pure geometry the series builder places into. [`emit_value_axis`] paints the title/axes/
/// gridlines from the same `Frame`; [`chart_frame`] runs both in sequence.
pub(super) fn compute_frame(
    style: ChartStyle,
    rect: Rect,
    title: &str,
    axis_titles: AxisTitles,
    series: &[(String, f64)],
) -> Frame {
    let (rl, rt, rw, rh) = (rect.left.0, rect.top.0, rect.width.0, rect.height.0);
    // Reserve bands: title on top, value-axis labels on the left, category labels on the bottom, plus
    // an extra band for each present axis title (rotated Y title far left, X title below the labels).
    let title_h = if title.is_empty() {
        0
    } else {
        (rh / 8).clamp(180, 360)
    };
    let axis_w = (rw / 8).clamp(360, 900);
    let vtitle_w = axis_title_band(axis_titles.value);
    let htitle_h = axis_title_band(axis_titles.category);
    let pad = 60;

    let plot_left = rl + vtitle_w + axis_w;
    let plot_top = rt + title_h + pad;
    let plot_right = rl + rw - pad;
    let plot_w = (plot_right - plot_left).max(1);

    // The label band is sized from the labels themselves: a rotated axis needs the extra depth its
    // 45° labels project, so the axis decision precedes the band reservation. It depends only on the
    // slot width, which the vertical bands do not affect.
    let n = series.len().max(1) as i32;
    let slot = plot_w / n;
    let widest = series
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0);
    let cats = category_axis(style, slot, widest);
    let cat_h = cats.band_h((rh / 8).clamp(160, 320));

    let plot_bottom = rt + rh - cat_h - htitle_h;
    let plot_h = (plot_bottom - plot_top).max(1);

    // Value scale: 0..max rounded to nice numbers so the axis reads 0 / step / 2·step / …;
    // bars/points scale to `max_val`, not the raw data max, so the tallest never touches the frame.
    // Guards against all-zero / negative-only series; non-finite (NaN/Inf) values are excluded so the
    // scale reflects the real finite data.
    let raw_max = series
        .iter()
        .map(|(_, v)| *v)
        .filter(|v| v.is_finite())
        .fold(0.0_f64, f64::max);
    let (max_val, step) = nice_scale(raw_max);

    Frame {
        plot_left,
        plot_bottom,
        plot_h,
        plot_right,
        slot,
        cat_h,
        max_val,
        step,
        cats,
    }
}

/// Emit the title, value scale, the two axes, and the tick labels/gridlines of `f` into `ops`. The
/// display bands (title/axis widths) are re-derived from `rect` — deterministically identical to the
/// reservation in [`compute_frame`].
pub(super) fn emit_value_axis(
    style: ChartStyle,
    ops: &mut Vec<DrawOp>,
    f: &Frame,
    rect: Rect,
    title: &str,
    axis_titles: AxisTitles,
    src: &dyn Fn() -> Option<ObjectRef>,
) {
    let (rl, rt, rw, rh) = (rect.left.0, rect.top.0, rect.width.0, rect.height.0);
    let title_h = if title.is_empty() {
        0
    } else {
        (rh / 8).clamp(180, 360)
    };
    let axis_w = (rw / 8).clamp(360, 900);
    let vtitle_w = axis_title_band(axis_titles.value);
    let pad = 60;
    let plot_top = rt + title_h + pad;

    // Title.
    if !title.is_empty() {
        ops.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(rl),
                top: Twips(rt + pad / 2),
                width: Twips(rw),
                height: Twips(title_h),
            },
            text: title.to_string(),
            font: style.font(ChartText::Title),
            color: LABEL,
            align: TextAlign::Center,
            rotation: 0.0,
            metrics: None,
            character_spacing: Twips(0),
            source: src(),
        }));
    }

    // Value-axis (Y) title, rotated 90° CCW up the left of the value axis. The box
    // origin is the bottom-left of the plot; rotating about it maps the box's horizontal extent
    // (`plot_h`) up the axis, so the center-aligned text sits at the vertical middle reading upward.
    if !axis_titles.value.is_empty() {
        ops.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(rl + pad),
                top: Twips(f.plot_bottom),
                width: Twips(f.plot_h),
                height: Twips(260),
            },
            text: axis_titles.value.to_string(),
            font: style.font(ChartText::DataAxisTitle),
            color: LABEL,
            align: TextAlign::Center,
            rotation: 90.0,
            metrics: None,
            character_spacing: Twips(0),
            source: src(),
        }));
    }

    // Category-axis (X) title, horizontal and centered below the category labels.
    if !axis_titles.category.is_empty() {
        ops.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(f.plot_left),
                top: Twips(f.plot_bottom + f.cat_h + 30),
                width: Twips((f.plot_right - f.plot_left).max(1)),
                height: Twips(260),
            },
            text: axis_titles.category.to_string(),
            font: style.font(ChartText::GroupAxisTitle),
            color: LABEL,
            align: TextAlign::Center,
            rotation: 0.0,
            metrics: None,
            character_spacing: Twips(0),
            source: src(),
        }));
    }

    let (max_val, step) = (f.max_val, f.step);
    let ticks = (max_val / step).round() as i32;
    let tick_y = |t: i32| f.plot_bottom - ((t as f64 * step / max_val) * f.plot_h as f64) as i32;

    // Horizontal gridlines + value-axis tick labels at each division (behind the series, which is
    // drawn by the caller after this frame). The 0-line is the x-axis itself, so skip its gridline.
    for t in 0..=ticks {
        let y = tick_y(t);
        if t > 0 {
            ops.push(DrawOp::Line(LineOp {
                from: Point {
                    x: Twips(f.plot_left),
                    y: Twips(y),
                },
                to: Point {
                    x: Twips(f.plot_right),
                    y: Twips(y),
                },
                stroke: Stroke {
                    color: GRID,
                    width: Twips(10),
                    style: rpt_pages::LineStyle::Single,
                },
                source: src(),
            }));
        }
        ops.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(rl + vtitle_w),
                top: Twips(y - 110),
                width: Twips(axis_w - pad),
                height: Twips(220),
            },
            text: fmt_val(t as f64 * step),
            font: FontSpec {
                family: "Arial".into(),
                size_pt: style.scaled_pt(8.0),
                ..Default::default()
            },
            color: LABEL,
            align: TextAlign::Right,
            rotation: 0.0,
            metrics: None,
            character_spacing: Twips(0),
            source: src(),
        }));
    }

    // Axes (drawn on top of the gridlines).
    let axis_stroke = Stroke {
        color: AXIS,
        width: Twips(15),
        style: rpt_pages::LineStyle::Single,
    };
    ops.push(DrawOp::Line(LineOp {
        from: Point {
            x: Twips(f.plot_left),
            y: Twips(plot_top),
        },
        to: Point {
            x: Twips(f.plot_left),
            y: Twips(f.plot_bottom),
        },
        stroke: axis_stroke,
        source: src(),
    }));
    ops.push(DrawOp::Line(LineOp {
        from: Point {
            x: Twips(f.plot_left),
            y: Twips(f.plot_bottom),
        },
        to: Point {
            x: Twips(f.plot_right),
            y: Twips(f.plot_bottom),
        },
        stroke: axis_stroke,
        source: src(),
    }));
}

/// Emit the shared axis-chart frame (title, value scale, the two axes, the max-value label) into
/// `ops` and return the plot geometry the series builder places into. A thin wrapper over
/// [`compute_frame`] + [`emit_value_axis`], preserved for the axis-chart renderers.
pub(super) fn chart_frame(
    style: ChartStyle,
    ops: &mut Vec<DrawOp>,
    rect: Rect,
    title: &str,
    axis_titles: AxisTitles,
    series: &[(String, f64)],
    src: &dyn Fn() -> Option<ObjectRef>,
) -> Frame {
    let f = compute_frame(style, rect, title, axis_titles, series);
    emit_value_axis(style, ops, &f, rect, title, axis_titles, src);
    f
}

/// The band width/height reserved for an axis title (0 when the title is empty). One 8-pt line plus a
/// little breathing room, both for the rotated value-axis title's column and the category-axis title's
/// row below the labels.
fn axis_title_band(title: &str) -> i32 {
    if title.is_empty() {
        0
    } else {
        300
    }
}

/// A data-value label: a small `color` text centered
/// horizontally on `x` with its top at `y`, used to annotate a bar top / line marker / pie slice with
/// its value. ~1000 twips wide so a formatted value stays centered without wrapping.
pub(super) fn value_label(
    style: ChartStyle,
    x: i32,
    y: i32,
    text: &str,
    color: Color,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> DrawOp {
    DrawOp::Text(TextRun {
        bounds: Rect {
            left: Twips(x - 500),
            top: Twips(y),
            width: Twips(1000),
            height: Twips(200),
        },
        text: text.to_string(),
        font: style.font(ChartText::DataLabel),
        color,
        align: TextAlign::Center,
        rotation: 0.0,
        metrics: None,
        character_spacing: Twips(0),
        source: src(),
    })
}

/// The fraction of the value axis a plotted value fills, clamped to `[0, 1]` so a non-finite
/// (NaN/Inf) or out-of-scale value — e.g. from a divide-by-zero formula, or a value exceeding the
/// scale because the max was excluded as non-finite — yields a bar/riser bounded by the plot height
/// rather than runaway (i32-saturating) geometry. Normal values (≤ the nice-scale ceiling) are
/// unaffected.
pub(super) fn value_frac(val: f64, max_val: f64) -> f64 {
    if max_val <= 0.0 || !max_val.is_finite() {
        return 0.0;
    }
    let f = val.max(0.0) / max_val;
    if f.is_finite() {
        f.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Gap between the category axis and the top of the label band.
const CAT_LABEL_GAP: i32 = 30;
/// `sin 45° = cos 45°`, the projection factor of a label rotated onto the category axis.
const DIAG: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// How the category axis draws its labels.
///
/// The engine keeps a category label upright only while it fits inside its own slot; a label wider
/// than its slot is drawn rotated 45° instead of being shrunk or elided, and the axis then draws
/// only every `stride`-th label so adjacent rotated baselines stay a full line apart. It never thins
/// an upright axis — an upright label fits by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CategoryAxis {
    /// Draw the label of every `stride`-th category (always ≥ 1, and always starting at category 0).
    pub(super) stride: usize,
    /// Whether the labels are rotated 45° rather than upright.
    pub(super) rotated: bool,
    /// Width of one label box, in twips: the slot when upright, the widest label's estimated advance
    /// when rotated (which is what the rotated box is right-aligned within).
    label_w: i32,
    /// One line of the chart's label font, in twips — the box height and the rotated-baseline
    /// clearance, both of which follow the font the chart's height gives it.
    line_h: i32,
}

impl CategoryAxis {
    /// The label band's height in twips: one line when upright, the rotated labels' vertical
    /// projection plus a line when rotated.
    fn band_h(self, base: i32) -> i32 {
        if !self.rotated {
            return base;
        }
        base.max(self.reach() + self.line_h + CAT_LABEL_GAP)
    }

    /// How far a rotated label box reaches back along each axis from the tick it ends on — its width
    /// projected onto the 45° diagonal.
    fn reach(self) -> i32 {
        (f64::from(self.label_w) * DIAG) as i32
    }

    /// The box for a rotated label whose text ends at the tick `(x, y)`. A run rotated θ CCW maps its
    /// local `(w, 0)` to `origin + (w·cos θ, −w·sin θ)`, so an origin down-left of the tick by that
    /// projection lands the right-aligned text's end on it. Shared by the 2-D axis families and the
    /// projected 3-D floor axis, which anchor at different points but draw the same label.
    pub(super) fn rotated_box(self, x: i32, y: i32) -> Rect {
        let reach = self.reach();
        Rect {
            left: Twips(x - reach),
            top: Twips(y + reach),
            width: Twips(self.label_w),
            height: Twips(self.line_h),
        }
    }
}

/// Decide how a category axis of `slot`-wide slots labels categories whose widest label is
/// `widest_chars` characters — see [`CategoryAxis`]. The label metrics come from `style`, so a
/// taller chart (which draws a larger label font) fits fewer characters in the same slot.
pub(super) fn category_axis(style: ChartStyle, slot: i32, widest_chars: usize) -> CategoryAxis {
    let slot = slot.max(1);
    let line_h = style.label_line_h();
    let w = widest_chars as i32 * style.label_char_w();
    if w <= slot {
        return CategoryAxis {
            stride: 1,
            rotated: false,
            label_w: slot,
            line_h,
        };
    }
    // Rotated 45°, adjacent labels sit `stride · slot · sin 45°` apart measured across their
    // baselines; that clearance must cover one line of text.
    let pitch = (f64::from(line_h) / DIAG).ceil() as i32;
    CategoryAxis {
        stride: (pitch.max(1) as usize).div_ceil(slot as usize).max(1),
        rotated: true,
        label_w: w,
        line_h,
    }
}

/// The category label under point/bar `i`. Upright labels are centered in their slot; rotated ones
/// are right-aligned so the end of the text meets the axis at the slot's centre, reading up and to
/// the right (the engine's 45° category label).
pub(super) fn category_label(
    style: ChartStyle,
    f: &Frame,
    i: i32,
    label: &str,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> DrawOp {
    let bounds = if f.cats.rotated {
        f.cats.rotated_box(
            f.plot_left + i * f.slot + f.slot / 2,
            f.plot_bottom + CAT_LABEL_GAP,
        )
    } else {
        Rect {
            left: Twips(f.plot_left + i * f.slot),
            top: Twips(f.plot_bottom + CAT_LABEL_GAP),
            width: Twips(f.slot),
            height: Twips(f.cat_h),
        }
    };
    DrawOp::Text(TextRun {
        bounds,
        text: label.to_string(),
        font: style.font(ChartText::GroupLabel),
        color: LABEL,
        align: if f.cats.rotated {
            TextAlign::Right
        } else {
            TextAlign::Center
        },
        rotation: if f.cats.rotated { 45.0 } else { 0.0 },
        metrics: None,
        character_spacing: Twips(0),
        source: src(),
    })
}

/// Format a value-axis tick / data label as the engine does: a plain decimal, at any magnitude — no
/// thousands separator, no magnitude abbreviation (`1800000`, never `1800.0k`), and no forced decimal
/// place.
///
/// A fractional value is shown to three significant digits with trailing zeros trimmed. That is
/// always enough to separate the ticks of a [`nice_scale`] ladder — the largest tick is nine steps,
/// so a tick never carries more than two more digits than its step — and it absorbs the accumulation
/// noise of a fractional step (`0.4 × 3` prints `1.2`, not `1.2000000000000002`).
pub(super) fn fmt_val(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    if !v.is_finite() {
        return format!("{v}");
    }
    if v.fract() == 0.0 {
        return format!("{v:.0}");
    }
    let digits = (v.abs().log10().floor() as i32).saturating_add(1);
    let decimals = (3 - digits).clamp(0, 6) as usize;
    let s = format!("{v:.decimals$}");
    match s.split_once('.') {
        Some((int, frac)) => {
            let frac = frac.trim_end_matches('0');
            if frac.is_empty() {
                int.to_string()
            } else {
                format!("{int}.{frac}")
            }
        }
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        category_axis, category_label, chart_font, chart_frame, compute_frame, emit_value_axis,
        fmt_val, legend, nice_scale, series_legend, slice_color, AxisTitles, ChartStyle, ChartText,
        LegendPosition, FONT_REF_HEIGHT, LEGEND_SWATCH, PALETTE, SERIES_MARGIN, SERIES_SWATCH_W,
    };
    use rpt_model::{ChartDefinition, ChartElementFont, Rect, Twips};
    use rpt_pages::{DrawOp, FontSpec};

    /// The single-series legend the engine gives area/line/stock: one boxed entry, its outer edge a
    /// fixed margin inside the chart rect, with the plot body kept clear of the whole band. Reserving
    /// the band is the point — a plot that keeps the legend's width draws too many category labels.
    #[test]
    fn series_legend_reserves_a_band_and_draws_one_boxed_entry() {
        let rect = Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(7200),
            height: Twips(4320),
        };
        let (ops, body) = series_legend(
            ChartStyle::test(rect.height),
            rect,
            LegendPosition::Right,
            "Min of total",
            "S",
            "G",
        );

        let texts: Vec<&str> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            texts,
            ["Min of total"],
            "exactly one entry, naming the series"
        );

        let rects: Vec<&rpt_pages::RectOp> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Rect(r) => Some(r),
                _ => None,
            })
            .collect();
        assert_eq!(rects.len(), 2, "the box and its swatch");
        let (r#box, swatch) = (rects[0], rects[1]);
        assert_eq!(
            r#box.bounds.left.0 + r#box.bounds.width.0,
            rect.width.0 - SERIES_MARGIN,
            "the box's outer edge sits a fixed margin inside the chart rect"
        );
        assert!(r#box.stroke.is_some(), "the engine borders the legend box");
        assert_eq!(swatch.bounds.width.0, SERIES_SWATCH_W);
        assert_eq!(
            swatch.fill,
            Some(PALETTE[0].into()),
            "a single series takes the first palette color"
        );
        assert!(
            r#box.bounds.left.0 > swatch.bounds.left.0 - SERIES_SWATCH_W,
            "the swatch sits inside the box"
        );

        assert!(
            body.width.0 < rect.width.0 - r#box.bounds.width.0,
            "the body clears the box AND the gap, not just the box"
        );
        assert_eq!(
            body.left, rect.left,
            "a right legend leaves the body in place"
        );

        // A left legend reserves the same width on the other side.
        let (_, left_body) = series_legend(
            ChartStyle::test(rect.height),
            rect,
            LegendPosition::Left,
            "Min of total",
            "S",
            "G",
        );
        assert_eq!(left_body.width, body.width);
        assert!(left_body.left.0 > rect.left.0);
    }

    /// A series name too long for the chart cannot squeeze the plot out of existence: the box is
    /// capped at a third of the width, and a rect too narrow for any band keeps its whole body.
    #[test]
    fn series_legend_never_crowds_out_the_plot() {
        let rect = Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(7200),
            height: Twips(4320),
        };
        let long = "Sum of an_extravagantly_long_column_name_from_a_wide_table";
        let (_, body) = series_legend(
            ChartStyle::test(rect.height),
            rect,
            LegendPosition::Right,
            long,
            "S",
            "G",
        );
        assert!(
            body.width.0 >= rect.width.0 / 2,
            "the band is capped, so the plot keeps most of the chart"
        );

        let tiny = Rect {
            width: Twips(300),
            ..rect
        };
        let (ops, body) = series_legend(
            ChartStyle::test(tiny.height),
            tiny,
            LegendPosition::Right,
            "Min of total",
            "S",
            "G",
        );
        assert!(ops.is_empty(), "no room for a legend draws none");
        assert_eq!(body, tiny, "and reserves nothing");
    }

    /// Chart labels are never elided: the engine clips text that overflows its box and draws no
    /// ellipsis anywhere in its own output, so a long category label reaches the draw-op intact and
    /// the axis sizes it from its real length (which is what rotates a crowded axis).
    #[test]
    fn labels_are_drawn_whole_rather_than_elided() {
        let long = "Hooked on Helmets and Other Very Long Customer Names";
        let f = compute_frame(
            ChartStyle::test(Twips(4320)),
            Rect {
                left: Twips(0),
                top: Twips(0),
                width: Twips(7200),
                height: Twips(4320),
            },
            "",
            AxisTitles {
                value: "",
                category: "",
            },
            &[(long.to_string(), 1.0), ("B".to_string(), 2.0)],
        );
        let DrawOp::Text(t) = category_label(ChartStyle::test(Twips(4320)), &f, 0, long, &|| None)
        else {
            panic!("category_label draws text");
        };
        assert_eq!(t.text, long, "no ellipsis, no cap");

        // The width estimate follows the real label, so a long label still rotates the axis.
        let slot = 400;
        assert!(category_axis(ChartStyle::test(Twips(4320)), slot, long.chars().count()).rotated);
        assert!(
            category_axis(ChartStyle::test(Twips(4320)), slot, long.chars().count()).stride
                >= category_axis(ChartStyle::test(Twips(4320)), slot, 8)
                    .stride
                    .max(1)
        );
    }

    /// The `compute_frame` + `emit_value_axis` split reproduces the monolithic `chart_frame` output
    /// byte-for-byte — the extraction is output-preserving, so the axis-chart renderers built on
    /// `chart_frame` are unaffected.
    #[test]
    fn split_matches_monolithic_chart_frame() {
        let rect = Rect {
            left: Twips(100),
            top: Twips(200),
            width: Twips(6000),
            height: Twips(4000),
        };
        let series = vec![
            ("A".to_string(), 12.0),
            ("B".to_string(), 27.0),
            ("C".to_string(), 6.0),
        ];
        let src = || None;

        let titles = AxisTitles {
            value: "Sum of id",
            category: "created_at",
        };
        let def = ChartDefinition::default();
        let mut whole: Vec<DrawOp> = Vec::new();
        let style = ChartStyle {
            def: &def,
            height: rect.height,
        };
        let f_whole = chart_frame(style, &mut whole, rect, "Title", titles, &series, &src);

        let mut split: Vec<DrawOp> = Vec::new();
        let f_split = compute_frame(style, rect, "Title", titles, &series);
        emit_value_axis(style, &mut split, &f_split, rect, "Title", titles, &src);

        assert_eq!(whole, split, "split emits identical ops");
        assert_eq!(f_whole.plot_left, f_split.plot_left);
        assert_eq!(f_whole.plot_right, f_split.plot_right);
        assert_eq!(f_whole.plot_bottom, f_split.plot_bottom);
        assert_eq!(f_whole.slot, f_split.slot);
    }

    /// The legend band is reserved on the correct side for every position, the returned body rect is
    /// reduced away from that side, and one swatch is emitted per series entry inside the band. This
    /// is what makes legend placement correct for *all* chart types — the type renderers only ever
    /// draw into the returned body rect, so they are placement-agnostic by construction.
    #[test]
    fn legend_reserves_the_correct_band_for_every_position() {
        let rect = Rect {
            left: Twips(1000),
            top: Twips(2000),
            width: Twips(8000),
            height: Twips(6000),
        };
        let series = vec![
            ("A".to_string(), 1.0),
            ("B".to_string(), 2.0),
            ("C".to_string(), 3.0),
        ];
        let swatch_xs = |ops: &[DrawOp]| -> Vec<i32> {
            ops.iter()
                .filter_map(|o| match o {
                    // Swatches are the 150-twip squares (labels are Text, not Rect).
                    DrawOp::Rect(r) if r.bounds.width.0 == LEGEND_SWATCH => Some(r.bounds.left.0),
                    _ => None,
                })
                .collect()
        };
        let swatch_ys = |ops: &[DrawOp]| -> Vec<i32> {
            ops.iter()
                .filter_map(|o| match o {
                    DrawOp::Rect(r) if r.bounds.width.0 == LEGEND_SWATCH => Some(r.bounds.top.0),
                    _ => None,
                })
                .collect()
        };
        let (rl, rt, rw, rh) = (1000, 2000, 8000, 6000);

        // Right: body hugs the left, swatches sit past the body's right edge.
        let (ops, body) = legend(
            ChartStyle::test(rect.height),
            rect,
            LegendPosition::Right,
            &series,
            false,
            "S",
            "G",
        );
        assert_eq!(body.left.0, rl, "right: body starts at rect left");
        assert!(body.width.0 < rw, "right: body narrower than rect");
        assert_eq!(swatch_xs(&ops).len(), 3, "one swatch per series entry");
        assert!(
            swatch_xs(&ops)
                .iter()
                .all(|&x| x >= body.left.0 + body.width.0),
            "right: swatches are right of the body"
        );

        // Left: body is pushed right, swatches sit left of the body.
        let (ops, body) = legend(
            ChartStyle::test(rect.height),
            rect,
            LegendPosition::Left,
            &series,
            false,
            "S",
            "G",
        );
        assert!(body.left.0 > rl, "left: body pushed right");
        assert!(body.width.0 < rw, "left: body narrower than rect");
        assert!(
            swatch_xs(&ops).iter().all(|&x| x < body.left.0),
            "left: swatches are left of the body"
        );

        // Top: body is pushed down, swatches sit above the body.
        let (ops, body) = legend(
            ChartStyle::test(rect.height),
            rect,
            LegendPosition::Top,
            &series,
            false,
            "S",
            "G",
        );
        assert!(body.top.0 > rt, "top: body pushed down");
        assert!(body.height.0 < rh, "top: body shorter than rect");
        assert!(
            swatch_ys(&ops).iter().all(|&y| y < body.top.0),
            "top: swatches are above the body"
        );

        // Bottom: body hugs the top, swatches sit below the body.
        let (ops, body) = legend(
            ChartStyle::test(rect.height),
            rect,
            LegendPosition::Bottom,
            &series,
            false,
            "S",
            "G",
        );
        assert_eq!(body.top.0, rt, "bottom: body starts at rect top");
        assert!(body.height.0 < rh, "bottom: body shorter than rect");
        assert!(
            swatch_ys(&ops)
                .iter()
                .all(|&y| y >= body.top.0 + body.height.0),
            "bottom: swatches are below the body"
        );
    }

    /// A label that fits its slot stays upright and every category is labelled; one that does not is
    /// rotated and the axis thins, and the thinning tightens as the slots narrow.
    #[test]
    fn category_axis_rotates_and_thins_only_when_a_label_overflows_its_slot() {
        // At the reference height the label font is its quoted 7 pt, so a character is 70 twips wide
        // and a line is 161 twips tall.
        let style = ChartStyle::test(Twips(FONT_REF_HEIGHT));
        // A 2-character label needs 140 twips.
        let upright = category_axis(style, 600, 2);
        assert_eq!(upright.stride, 1, "a fitting label is drawn at every slot");
        assert!(!upright.rotated);

        // Ten characters need 700 twips, so a 300-twip slot rotates. Rotated labels sit
        // `stride · slot · sin 45°` apart, which must clear one 161-twip line: 228 twips.
        let rotated = category_axis(style, 300, 10);
        assert!(rotated.rotated, "an overflowing label rotates");
        assert_eq!(rotated.stride, 1, "300 twips already clears a line");
        assert_eq!(category_axis(style, 150, 10).stride, 2);
        assert_eq!(category_axis(style, 40, 10).stride, 6);

        // A degenerate (zero-width) slot still yields a usable stride rather than dividing by zero.
        assert!(category_axis(style, 0, 10).stride >= 1);
    }

    /// A rotated category label is right-aligned so the end of its text lands on the tick at the
    /// centre of its slot, reading up and to the right; an upright one is centred in the slot.
    #[test]
    fn rotated_category_label_ends_on_its_tick() {
        let rect = Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(6000),
            height: Twips(4000),
        };
        let dense: Vec<(String, f64)> = (0..40)
            .map(|i| (format!("12/{i}/2024"), i as f64 + 1.0))
            .collect();
        let f = compute_frame(
            ChartStyle::test(rect.height),
            rect,
            "",
            AxisTitles::default(),
            &dense,
        );
        assert!(f.cats.rotated, "40 date labels in 6000 twips must rotate");
        let DrawOp::Text(t) =
            category_label(ChartStyle::test(rect.height), &f, 3, "12/3/2024", &|| None)
        else {
            panic!("category_label emits a text run");
        };
        assert_eq!(t.rotation, 45.0);
        assert_eq!(t.align, rpt_pages::TextAlign::Right);
        // The run's origin is down-left of the tick by the box's 45° projection, so rotating the
        // box's far edge about it lands the text's end on the tick.
        let tick = f.plot_left + 3 * f.slot + f.slot / 2;
        let reach = (f64::from(t.bounds.width.0) * super::DIAG) as i32;
        assert_eq!(t.bounds.left.0 + reach, tick);
        assert_eq!(t.bounds.top.0 - reach, f.plot_bottom + super::CAT_LABEL_GAP);
        // The band under the plot is deep enough to hold the rotated labels.
        assert!(
            f.cat_h >= reach,
            "band {} holds the {reach}-twip projection",
            f.cat_h
        );
    }

    /// The value→axis fraction is clamped to `[0, 1]` so no non-finite or out-of-scale value produces
    /// runaway (i32-saturating) bar/riser geometry; a normal in-scale value passes through.
    #[test]
    fn value_frac_clamps_non_finite_and_out_of_scale() {
        use super::value_frac;
        // In-scale values pass through unchanged.
        assert_eq!(value_frac(5.0, 10.0), 0.5);
        assert_eq!(value_frac(10.0, 10.0), 1.0);
        // Negatives fold to 0; over-scale clamps to 1.
        assert_eq!(value_frac(-3.0, 10.0), 0.0);
        assert_eq!(value_frac(50.0, 10.0), 1.0);
        // Non-finite value or scale → 0 (never NaN/Inf, never a saturating height).
        assert_eq!(value_frac(f64::INFINITY, 10.0), 0.0);
        assert_eq!(value_frac(f64::NAN, 10.0), 0.0);
        assert_eq!(value_frac(5.0, f64::INFINITY), 0.0);
        assert_eq!(value_frac(5.0, 0.0), 0.0);
    }

    /// A stored per-element face wins over the default table for the element it maps to
    /// (Title → `element_fonts[0]`), while the size stays sourced from the default table (a chart's
    /// per-element point size is not a `Contents` fact). A zero weight means the record carried no
    /// style entry, so the slant and weight stay default-sourced too.
    #[test]
    fn resolve_prefers_stored_override_for_title() {
        let def = ChartDefinition {
            element_fonts: vec![ChartElementFont {
                name: "Times New Roman".to_string(),
                is_default: false,
                weight: 0,
                italic: false,
            }],
            ..Default::default()
        };
        let font = ChartStyle {
            def: &def,
            height: Twips(FONT_REF_HEIGHT),
        }
        .font(ChartText::Title);
        assert_eq!(
            font.family, "Times New Roman",
            "stored face overrides Arial"
        );
        assert_eq!(
            font.size_pt,
            ChartStyle::test(Twips(FONT_REF_HEIGHT))
                .scaled_pt(chart_font(ChartText::Title).size_pt),
            "the size stays on the default table"
        );
        assert!(
            font.bold && !font.italic,
            "an unstored weight leaves the default table's bold/italic"
        );
    }

    /// A stored weight and slant win over the default table wherever the record carries them: an
    /// element the default table draws bold-upright renders normal-italic when that is what the
    /// chart stored, and one the table draws normal renders bold.
    #[test]
    fn resolve_prefers_stored_weight_and_slant() {
        let mut fonts = vec![
            ChartElementFont {
                name: "Arial".to_string(),
                is_default: true,
                weight: 400,
                italic: false,
            };
            10
        ];
        // GroupTitle (index 3) defaults to bold-upright; store normal-italic.
        fonts[3].weight = 400;
        fonts[3].italic = true;
        // GroupLabel (index 7) defaults to normal; store bold.
        fonts[7].weight = 700;
        let def = ChartDefinition {
            element_fonts: fonts,
            ..Default::default()
        };
        let font = |e| {
            ChartStyle {
                def: &def,
                height: Twips(FONT_REF_HEIGHT),
            }
            .font(e)
        };
        assert!(chart_font(ChartText::GroupAxisTitle).bold);
        let axis = font(ChartText::GroupAxisTitle);
        assert!(!axis.bold && axis.italic, "stored normal-italic wins");
        assert!(!chart_font(ChartText::GroupLabel).bold);
        assert!(font(ChartText::GroupLabel).bold, "stored bold wins");
    }

    /// A chart storing only defaults — every entry the `"Arial"` sentinel carrying the default
    /// table's own weight and slant, or no `element_fonts` at all — resolves every `ChartText` to
    /// exactly the default table when drawn at the reference height, since these defaults are what
    /// charts store.
    #[test]
    fn resolve_defaults_match_the_default_table() {
        let all_arial = ChartDefinition {
            element_fonts: (0..10)
                .map(|i| {
                    let (weight, italic) = match i {
                        0 | 2..=5 => (700, i == 2),
                        _ => (400, false),
                    };
                    ChartElementFont {
                        name: "Arial".to_string(),
                        is_default: true,
                        weight,
                        italic,
                    }
                })
                .collect(),
            ..Default::default()
        };
        let empty = ChartDefinition::default();
        for element in [
            ChartText::Title,
            ChartText::Subtitle,
            ChartText::Footnote,
            ChartText::GroupAxisTitle,
            ChartText::DataAxisTitle,
            ChartText::GroupLabel,
            ChartText::DataLabel,
            ChartText::Legend,
        ] {
            let default = chart_font(element);
            let expected = FontSpec {
                size_pt: ChartStyle::test(Twips(FONT_REF_HEIGHT)).scaled_pt(default.size_pt),
                ..default
            };
            let at_ref = |def| {
                ChartStyle {
                    def,
                    height: Twips(FONT_REF_HEIGHT),
                }
                .font(element)
            };
            assert_eq!(
                at_ref(&all_arial),
                expected,
                "all-Arial resolves {element:?} to the default table"
            );
            assert_eq!(
                at_ref(&empty),
                expected,
                "empty element_fonts resolves {element:?} to the default table"
            );
        }
    }

    /// Chart text scales with the chart's **height** — its width has no effect (device pixels at
    /// 96 dpi, so a point is 4/3 of a pixel).
    ///
    /// The chart title and axis titles scale by truncating to the pixel below at every height. A
    /// tick/category/legend label can differ by one pixel (a quarter point): the engine rounds
    /// that element to the nearest pixel rather than truncating it, a distinction this shared
    /// formula does not reproduce.
    #[test]
    fn chart_text_scales_with_the_chart_height() {
        // (chart height in px, engine label px, engine axis-title px, engine title px)
        let engine = [
            (72, 2, 2, 4),
            (144, 5, 5, 9),
            (216, 7, 7, 13),
            (288, 9, 10, 18),
            (432, 14, 15, 27),
            (576, 18, 20, 36),
        ];
        let px = |pt: f32| (pt * 4.0 / 3.0).round() as i32;
        for (h_px, label, axis, title) in engine {
            let style = ChartStyle::test(Twips(h_px * 15));
            assert_eq!(
                px(style.font(ChartText::Title).size_pt),
                title,
                "title at {h_px} px tall"
            );
            assert_eq!(
                px(style.font(ChartText::DataAxisTitle).size_pt),
                axis,
                "axis title at {h_px} px tall"
            );
            let ours = px(style.font(ChartText::GroupLabel).size_pt);
            assert!(
                (ours - label).abs() <= 1,
                "label at {h_px} px tall: {ours} px vs the engine's {label} px"
            );
        }
    }

    /// The engine's auto-scale rule for a range of data maxima: `(data max, axis max, step)`.
    #[test]
    fn nice_scale_matches_engine_ticks() {
        for (max, axis_max, step) in [
            // A currency sum: 0/200000/…/1800000.
            (1_677_019.90, 1_800_000.0, 200_000.0),
            // A count: 0/4/8/12/16/20/24 — a step no 1/2/5 ladder can produce.
            (24.0, 24.0, 4.0),
            (65_180.80, 70_000.0, 10_000.0),
            (748_755.94, 800_000.0, 100_000.0),
            (103_535.50, 120_000.0, 20_000.0),
            (217.8, 240.0, 40.0),
            // A small count, again on the mantissa 4: 0/4/8/…/32.
            (30.0, 32.0, 4.0),
            // The max lands on the top tick when it is a step multiple.
            (140.0, 140.0, 20.0),
        ] {
            assert_eq!(nice_scale(max), (axis_max, step), "data max {max}");
        }
    }

    /// The step is the smallest 1/2/4/5×10ⁿ value that keeps the axis to at most 9 divisions, so the
    /// division count always lands in 5..=9 and a max that is a step multiple is the top tick.
    #[test]
    fn nice_scale_caps_at_nine_divisions() {
        for max in [1.0, 3.0, 9.0, 10.0, 24.0, 45.0, 90.0, 100.0, 217.8, 1e6] {
            let (axis_max, step) = nice_scale(max);
            let divisions = (axis_max / step).round();
            assert!(
                (5.0..=9.0).contains(&divisions),
                "{max} → {divisions} divisions (max {axis_max}, step {step})"
            );
            assert!(
                axis_max >= max,
                "{max} → axis max {axis_max} below the data"
            );
        }
        // A count of exactly 10 takes step 2 (step 1 would need 10 divisions), and 9 takes step 1.
        assert_eq!(nice_scale(10.0), (10.0, 2.0));
        assert_eq!(nice_scale(9.0), (9.0, 1.0));
        // Top tick lands on the data max where the max is a step multiple.
        assert_eq!(nice_scale(140.0), (140.0, 20.0));
        assert_eq!(nice_scale(1000.0), (1000.0, 200.0));
        // Degenerate / non-finite inputs fall back to a unit scale (never a NaN/Inf tick step).
        assert_eq!(nice_scale(0.0), (1.0, 1.0));
        assert_eq!(nice_scale(-5.0), (1.0, 1.0));
        assert_eq!(nice_scale(f64::NAN), (1.0, 1.0));
        assert_eq!(nice_scale(f64::INFINITY), (1.0, 1.0));
        assert_eq!(nice_scale(f64::NEG_INFINITY), (1.0, 1.0));
        // A finite extreme magnitude still yields a finite scale + step.
        let (m, s) = nice_scale(1e18);
        assert!(
            m.is_finite() && s.is_finite() && s > 0.0,
            "1e18 → finite scale"
        );
    }

    /// A numeric label is a plain decimal at every magnitude — the engine writes `1800000`, never a
    /// `k`-abbreviated `1800.0k`, and never a forced decimal place on a whole number.
    #[test]
    fn fmt_val_never_abbreviates() {
        for (v, want) in [
            (0.0, "0"),
            (-0.0, "0"),
            (4.0, "4"),
            (24.0, "24"),
            (999.0, "999"),
            (1000.0, "1000"),
            (200_000.0, "200000"),
            (1_800_000.0, "1800000"),
            (1_677_019.9, "1677020"),
            (0.4, "0.4"),
            // Accumulated fractional-step noise still prints the intended tick.
            (0.4 * 3.0, "1.2"),
            (-1500.0, "-1500"),
            // Three significant digits on a fractional value, at any magnitude.
            (5.142_857_142_857_143, "5.14"),
            (0.05, "0.05"),
            (0.45, "0.45"),
            (0.004, "0.004"),
        ] {
            assert_eq!(fmt_val(v), want, "fmt_val({v})");
        }
        // Every tick of a fractional-step ladder stays distinct.
        let (axis_max, step) = nice_scale(0.4);
        let ticks: Vec<String> = (0..=(axis_max / step).round() as i32)
            .map(|t| fmt_val(t as f64 * step))
            .collect();
        assert_eq!(
            ticks,
            ["0", "0.05", "0.1", "0.15", "0.2", "0.25", "0.3", "0.35", "0.4"]
        );
        // Every tick of a large-magnitude axis is a plain integer.
        let (axis_max, step) = nice_scale(1_677_019.90);
        let ticks: Vec<String> = (0..=(axis_max / step).round() as i32)
            .map(|t| fmt_val(t as f64 * step))
            .collect();
        assert_eq!(
            ticks,
            [
                "0", "200000", "400000", "600000", "800000", "1000000", "1200000", "1400000",
                "1600000", "1800000"
            ]
        );
    }

    /// The 20-color palette is Crystal's default sequence, and `slice_color` cycles it with
    /// period 20 — the first 20 slices are all distinct, and slice 20 wraps back to slice 0.
    #[test]
    fn slice_color_matches_default_palette_and_cycles_at_20() {
        // The full palette sequence.
        let captured: [(u8, u8, u8); 20] = [
            (0x3a, 0x65, 0x98),
            (0xef, 0xa2, 0x52),
            (0x00, 0x94, 0x70),
            (0xdd, 0x58, 0x1f),
            (0xa2, 0x2d, 0x62),
            (0xfe, 0xce, 0x60),
            (0x27, 0x75, 0x8b),
            (0xda, 0x70, 0x62),
            (0x44, 0x77, 0x11),
            (0xc8, 0x27, 0x59),
            (0x5d, 0x07, 0x9c),
            (0xe3, 0xd6, 0x3c),
            (0xda, 0xa4, 0xc8),
            (0x33, 0x81, 0xcc),
            (0xf1, 0xc2, 0x83),
            (0xa4, 0x77, 0x34),
            (0x92, 0xba, 0xe3),
            (0xb6, 0x3d, 0x32),
            (0x34, 0xce, 0x91),
            (0xff, 0x7a, 0x59),
        ];
        for (i, &(r, g, b)) in captured.iter().enumerate() {
            let c = slice_color(i);
            assert_eq!((c.r, c.g, c.b), (r, g, b), "palette index {i}");
            assert_eq!(PALETTE[i], c, "slice_color mirrors PALETTE at {i}");
        }
        // The first 20 are all distinct; index 20 wraps back to index 0 (period-20 cycle).
        let distinct: std::collections::BTreeSet<(u8, u8, u8)> = (0..20)
            .map(|i| {
                let c = slice_color(i);
                (c.r, c.g, c.b)
            })
            .collect();
        assert_eq!(distinct.len(), 20, "20 distinct captured colors");
        assert_eq!(slice_color(20), slice_color(0), "cycles at 20");
        assert_eq!(slice_color(24), slice_color(4), "slice 24 reuses color 4");
    }
}
