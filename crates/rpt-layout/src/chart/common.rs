//! Shared chart primitives: the qualitative palette, the legend, and the Num+Ord axis frame
//! ([`chart_frame`]/[`Frame`]) plus the label/format helpers the per-type chart renderers
//! ([`super::bar`]/[`super::line`]/[`super::pie`]) build on.

use rpt_model::{Color, Rect, Twips};
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

/// An opaque RGB colour (the palette's alpha is always fully opaque).
const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { a: 255, r, g, b }
}

/// Crystal's default chart palette, cycled per category/series. The scheme is not stored in the
/// `.rpt`, so it is hard-coded here to match the engine's default colours. The full sequence is
/// **20 colours**; the engine cycles it with period 20 (a chart's 21st mark reuses the first
/// colour), so every family indexes it as `PALETTE[i % PALETTE.len()]`.
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

/// The fill colour for pie/doughnut slice `i`: the same [`PALETTE`] cycled with the engine's
/// period-20 wrap (a 25-slice pie reuses the first five colours for its last five slices). Named for
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

/// The native engine's default chart-title point size (Arial 14, bold), applied to every chart
/// family's heading. The engine's per-element font defaults are: title 14 bold, axis titles 8 bold,
/// tick/data/legend labels 7; a subtitle is 10 and a footnote 8 bold-italic — neither is currently
/// drawn. Custom per-element fonts are not applied; every chart uses these defaults.
pub(super) const TITLE_PT: f32 = 14.0;
/// The native default axis-title point size (Arial 8, bold) — the value/category axis captions.
pub(super) const AXIS_TITLE_PT: f32 = 8.0;

/// A "nice" value-axis maximum ≥ `max` and a round tick step, via the 1/2/5×10ⁿ rule. Picks the
/// smallest step keeping the axis to ≲8 divisions, so the top tick lands on the data max where the
/// data max is a step multiple (140 → step 20, top tick 140), rather than rounding the maximum above
/// the data. Returns `(nice_max, step)`.
pub(super) fn nice_scale(max: f64) -> (f64, f64) {
    if max <= 0.0 || max.is_nan() {
        return (1.0, 1.0);
    }
    // Smallest 1/2/5×10ⁿ step ≥ max/8 → at most ~8 divisions. A larger divisor packs in more ticks;
    // a smaller one rounds the max a full step above the data.
    let raw_step = max / 8.0;
    let mag = 10f64.powf(raw_step.log10().floor());
    let norm = raw_step / mag;
    let nice = if norm <= 1.0 {
        1.0
    } else if norm <= 2.0 {
        2.0
    } else if norm <= 5.0 {
        5.0
    } else {
        10.0
    };
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

/// Reserve a band on `pos`'s side of `rect` for the legend, draw one (swatch + category label) entry
/// per series point (coloured to match the bars/slices), and return the legend draw-ops plus the
/// reduced rect the chart body should draw into. A single-series group chart legends its categories
/// (each a distinct colour), matching Crystal's group-chart legend. `per_slice` selects the swatch
/// colouring: `true` gives each entry a distinct colour ([`slice_color`], matching a pie/doughnut's
/// per-slice fills), `false` cycles the base [`PALETTE`] (matching the bar/line/area families).
/// Composed by the caller so each chart-type renderer stays legend-agnostic (it just draws into the
/// returned body rect).
pub(crate) fn legend(
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
    let pad = 90;
    let swatch = 150;
    let gap = 60;
    let font_pt: f32 = 7.0;

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
                text: truncate(label, 18),
                font: FontSpec {
                    family: "Arial".into(),
                    size_pt,
                    ..Default::default()
                },
                color: LABEL,
                align,
                rotation: 0.0,
                metrics: None,
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
            let fs = (font_pt * (entry_h as f32 / natural as f32)).clamp(4.0, font_pt);
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
}

impl Frame {
    /// Top of the plot rectangle (the full-scale line). Value labels clamp to this so a full-height
    /// riser's label stays inside the frame.
    pub(super) fn plot_top(&self) -> i32 {
        self.plot_bottom - self.plot_h
    }

    /// Right edge of the plot rectangle.
    pub(super) fn plot_right(&self) -> i32 {
        self.plot_right
    }
}

/// Compute the shared axis-chart frame (band reservation + `nice_scale`) without emitting any ops —
/// the pure geometry the series builder places into. [`emit_value_axis`] paints the title/axes/
/// gridlines from the same `Frame`; [`chart_frame`] runs both in sequence.
pub(super) fn compute_frame(
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
    let cat_h = (rh / 8).clamp(160, 320);
    let axis_w = (rw / 8).clamp(360, 900);
    let vtitle_w = axis_title_band(axis_titles.value);
    let htitle_h = axis_title_band(axis_titles.category);
    let pad = 60;

    let plot_left = rl + vtitle_w + axis_w;
    let plot_top = rt + title_h + pad;
    let plot_right = rl + rw - pad;
    let plot_bottom = rt + rh - cat_h - htitle_h;
    let plot_w = (plot_right - plot_left).max(1);
    let plot_h = (plot_bottom - plot_top).max(1);

    // Value scale: 0..max rounded to nice numbers so the axis reads 0 / step / 2·step / …;
    // bars/points scale to `max_val`, not the raw data max, so the tallest never touches the frame.
    // Guards against all-zero / negative-only series.
    let raw_max = series.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max);
    let (max_val, step) = nice_scale(raw_max);

    let n = series.len().max(1) as i32;
    Frame {
        plot_left,
        plot_bottom,
        plot_h,
        plot_right,
        slot: plot_w / n,
        cat_h,
        max_val,
        step,
    }
}

/// Emit the title, value scale, the two axes, and the tick labels/gridlines of `f` into `ops`. The
/// display bands (title/axis widths) are re-derived from `rect` — deterministically identical to the
/// reservation in [`compute_frame`].
pub(super) fn emit_value_axis(
    ops: &mut Vec<DrawOp>,
    f: &Frame,
    rect: Rect,
    title: &str,
    axis_titles: AxisTitles,
    _series: &[(String, f64)],
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
            font: FontSpec {
                family: "Arial".into(),
                size_pt: TITLE_PT,
                bold: true,
                ..Default::default()
            },
            color: LABEL,
            align: TextAlign::Center,
            rotation: 0.0,
            metrics: None,
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
            font: FontSpec {
                family: "Arial".into(),
                size_pt: AXIS_TITLE_PT,
                bold: true,
                ..Default::default()
            },
            color: LABEL,
            align: TextAlign::Center,
            rotation: 90.0,
            metrics: None,
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
            font: FontSpec {
                family: "Arial".into(),
                size_pt: AXIS_TITLE_PT,
                bold: true,
                ..Default::default()
            },
            color: LABEL,
            align: TextAlign::Center,
            rotation: 0.0,
            metrics: None,
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
                size_pt: 8.0,
                ..Default::default()
            },
            color: LABEL,
            align: TextAlign::Right,
            rotation: 0.0,
            metrics: None,
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
    ops: &mut Vec<DrawOp>,
    rect: Rect,
    title: &str,
    axis_titles: AxisTitles,
    series: &[(String, f64)],
    src: &dyn Fn() -> Option<ObjectRef>,
) -> Frame {
    let f = compute_frame(rect, title, axis_titles, series);
    emit_value_axis(ops, &f, rect, title, axis_titles, series, src);
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
        font: FontSpec {
            family: "Arial".into(),
            size_pt: 7.0,
            ..Default::default()
        },
        color,
        align: TextAlign::Center,
        rotation: 0.0,
        metrics: None,
        source: src(),
    })
}

/// The stride at which category labels are drawn so a dense category axis doesn't overlap: when a
/// slot is wide enough for a readable label the stride is `1` (every label drawn); when slots are
/// narrower the stride grows so only every Nth label is emitted, spaced ~one readable label apart.
/// `n` is the category count. Shared by the axis families (bar/line/area/numeric) so they thin
/// identically.
pub(super) fn category_stride(f: &Frame, n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    // ~Twips a 7-pt category label needs to read without colliding with its neighbour.
    const MIN_LABEL_W: i32 = 620;
    let slot = f.slot.max(1) as usize;
    if slot >= MIN_LABEL_W as usize {
        return 1;
    }
    (MIN_LABEL_W as usize).div_ceil(slot).max(1)
}

/// The category label under point/bar `i`, centered in its slot.
pub(super) fn category_label(
    f: &Frame,
    i: i32,
    label: &str,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> DrawOp {
    DrawOp::Text(TextRun {
        bounds: Rect {
            left: Twips(f.plot_left + i * f.slot),
            top: Twips(f.plot_bottom + 30),
            width: Twips(f.slot),
            height: Twips(f.cat_h),
        },
        text: truncate(label, 16),
        font: FontSpec {
            family: "Arial".into(),
            size_pt: 7.0,
            ..Default::default()
        },
        color: LABEL,
        align: TextAlign::Center,
        rotation: 0.0,
        metrics: None,
        source: src(),
    })
}

/// Format a value axis / data label compactly (drop trailing zeros; thousands as `k`).
pub(super) fn fmt_val(v: f64) -> String {
    if v.abs() >= 1000.0 {
        format!("{:.1}k", v / 1000.0)
    } else if v.fract().abs() < 1e-9 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// Truncate a label to `max` chars with an ellipsis (char-safe).
pub(super) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::{
        category_stride, chart_frame, compute_frame, emit_value_axis, legend, nice_scale,
        slice_color, AxisTitles, LegendPosition, PALETTE,
    };
    use rpt_model::{Rect, Twips};
    use rpt_pages::DrawOp;

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
        let mut whole: Vec<DrawOp> = Vec::new();
        let f_whole = chart_frame(&mut whole, rect, "Title", titles, &series, &src);

        let mut split: Vec<DrawOp> = Vec::new();
        let f_split = compute_frame(rect, "Title", titles, &series);
        emit_value_axis(&mut split, &f_split, rect, "Title", titles, &series, &src);

        assert_eq!(whole, split, "split emits identical ops");
        assert_eq!(f_whole.plot_left, f_split.plot_left);
        assert_eq!(f_whole.plot_right(), f_split.plot_right());
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
                    DrawOp::Rect(r) if r.bounds.width.0 == 150 => Some(r.bounds.left.0),
                    _ => None,
                })
                .collect()
        };
        let swatch_ys = |ops: &[DrawOp]| -> Vec<i32> {
            ops.iter()
                .filter_map(|o| match o {
                    DrawOp::Rect(r) if r.bounds.width.0 == 150 => Some(r.bounds.top.0),
                    _ => None,
                })
                .collect()
        };
        let (rl, rt, rw, rh) = (1000, 2000, 8000, 6000);

        // Right: body hugs the left, swatches sit past the body's right edge.
        let (ops, body) = legend(rect, LegendPosition::Right, &series, false, "S", "G");
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
        let (ops, body) = legend(rect, LegendPosition::Left, &series, false, "S", "G");
        assert!(body.left.0 > rl, "left: body pushed right");
        assert!(body.width.0 < rw, "left: body narrower than rect");
        assert!(
            swatch_xs(&ops).iter().all(|&x| x < body.left.0),
            "left: swatches are left of the body"
        );

        // Top: body is pushed down, swatches sit above the body.
        let (ops, body) = legend(rect, LegendPosition::Top, &series, false, "S", "G");
        assert!(body.top.0 > rt, "top: body pushed down");
        assert!(body.height.0 < rh, "top: body shorter than rect");
        assert!(
            swatch_ys(&ops).iter().all(|&y| y < body.top.0),
            "top: swatches are above the body"
        );

        // Bottom: body hugs the top, swatches sit below the body.
        let (ops, body) = legend(rect, LegendPosition::Bottom, &series, false, "S", "G");
        assert_eq!(body.top.0, rt, "bottom: body starts at rect top");
        assert!(body.height.0 < rh, "bottom: body shorter than rect");
        assert!(
            swatch_ys(&ops)
                .iter()
                .all(|&y| y >= body.top.0 + body.height.0),
            "bottom: swatches are below the body"
        );
    }

    /// A few wide categories are all labelled (stride 1); a dense axis thins to roughly one label
    /// per readable slot so the drawn labels stay ~a label-width apart.
    #[test]
    fn category_stride_thins_only_when_dense() {
        let rect = Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(6000),
            height: Twips(4000),
        };
        let series6: Vec<(String, f64)> =
            (0..6).map(|i| (format!("c{i}"), i as f64 + 1.0)).collect();
        let f6 = compute_frame(rect, "", AxisTitles::default(), &series6);
        assert_eq!(
            category_stride(&f6, 6),
            1,
            "6 wide slots: every label drawn"
        );

        let series50: Vec<(String, f64)> =
            (0..50).map(|i| (format!("c{i}"), i as f64 + 1.0)).collect();
        let f50 = compute_frame(rect, "", AxisTitles::default(), &series50);
        let stride = category_stride(&f50, 50);
        assert!(stride > 1, "50 narrow slots thinned (stride {stride})");
        // The drawn labels are ~one readable slot-width apart, so far fewer than all 50.
        let drawn = (0..50).filter(|i| i % stride == 0).count();
        assert!(drawn < 25, "dense axis thinned to {drawn} of 50 labels");
    }

    #[test]
    fn nice_scale_rounds_to_1_2_5_decades() {
        // Top tick lands on the data max: 140 → step 20 → 0/20/40/…/140 (~7 divisions).
        assert_eq!(nice_scale(140.0), (140.0, 20.0));
        assert_eq!(nice_scale(25.0), (25.0, 5.0));
        assert_eq!(nice_scale(10.0), (10.0, 2.0));
        assert_eq!(nice_scale(1000.0), (1000.0, 200.0));
        assert_eq!(nice_scale(3.0), (3.0, 0.5));
        // Degenerate inputs fall back to a unit scale.
        assert_eq!(nice_scale(0.0), (1.0, 1.0));
        assert_eq!(nice_scale(-5.0), (1.0, 1.0));
        assert_eq!(nice_scale(f64::NAN), (1.0, 1.0));
    }

    /// The 20-colour palette is Crystal's default sequence, and `slice_color` cycles it with
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
        assert_eq!(distinct.len(), 20, "20 distinct captured colours");
        assert_eq!(slice_color(20), slice_color(0), "cycles at 20");
        assert_eq!(slice_color(24), slice_color(4), "slice 24 reuses colour 4");
    }
}
