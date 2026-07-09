//! Gantt chart (graph type code 13): one horizontal time bar per detail record, spanning
//! `[start_date .. end_date]` on a shared date (time) X axis, with the records stacked top-to-bottom
//! one row each. Unlike the category families this is **per detail row**, not a
//! group summary: the chart binds two date fields (start, end) and draws a bar per record rather than
//! a riser per group.

use super::common::{truncate, AxisTitles, AXIS, GRID, LABEL, PALETTE, TITLE_PT};
use rpt_format_value::Date;
use rpt_model::{Rect, Twips};
use rpt_pages::{
    DrawOp, FontSpec, LineOp, ObjectKind, ObjectRef, Point, RectOp, Stroke, TextAlign, TextRun,
};

/// One Gantt row: a `label` and the `[start, end]` span in civil day-numbers ([`Date::to_days`], with
/// a fractional part for the time-of-day of a DateTime binding). `start`/`end` are normalized by the
/// caller so `start <= end`.
#[derive(Debug, Clone)]
pub(crate) struct GanttBar {
    pub(crate) label: String,
    pub(crate) start: f64,
    pub(crate) end: f64,
}

/// Build the draw-ops for a Gantt chart of `bars` (one per detail record): a date X axis spanning the
/// min start to the max end, one horizontal filled bar per record stacked top-to-bottom, its row
/// label on the left, and date-formatted tick labels along the bottom. `axis_titles.category` is the
/// date-axis title (drawn centered below the ticks). Returns an empty vec when there are no bars.
pub(crate) fn gantt_chart(
    rect: Rect,
    title: &str,
    axis_titles: AxisTitles,
    bars: &[GanttBar],
    section_name: &str,
    obj_name: &str,
) -> Vec<DrawOp> {
    if bars.is_empty() {
        return Vec::new();
    }
    let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(obj_name));
    let mut ops: Vec<DrawOp> = Vec::new();
    let (rl, rt, rw, rh) = (rect.left.0, rect.top.0, rect.width.0, rect.height.0);

    // Reserve bands: title on top, row labels on the left, the date axis (ticks + optional title) at
    // the bottom.
    let title_h = if title.is_empty() {
        0
    } else {
        (rh / 8).clamp(180, 360)
    };
    let label_w = (rw / 6).clamp(500, 1600);
    let axis_h = (rh / 8).clamp(160, 320);
    let xtitle_h = if axis_titles.category.is_empty() {
        0
    } else {
        300
    };
    let pad = 60;

    let plot_left = rl + label_w;
    let plot_top = rt + title_h + pad;
    let plot_right = rl + rw - pad;
    let plot_bottom = rt + rh - axis_h - xtitle_h;
    let plot_w = (plot_right - plot_left).max(1);
    let plot_h = (plot_bottom - plot_top).max(1);

    // Date X scale: min start .. max end, with a 2% margin each side so edge bars don't sit flush to
    // the frame. Guards a zero-width span (all records on one instant) to a single day.
    let mut lo = bars.iter().map(|b| b.start).fold(f64::INFINITY, f64::min);
    let mut hi = bars.iter().map(|b| b.end).fold(f64::NEG_INFINITY, f64::max);
    if hi <= lo {
        hi = lo + 1.0;
    }
    let margin = (hi - lo) * 0.02;
    lo -= margin;
    hi += margin;
    let span = hi - lo;
    let x_at = |day: f64| plot_left + (((day - lo) / span) * plot_w as f64) as i32;

    // Title, centered on top.
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

    // Vertical gridlines + date tick labels at evenly spaced divisions of the span (a simple linear
    // date scale). The count keeps labels ~a readable width apart.
    let n_ticks = (plot_w / 1400).clamp(2, 8);
    for t in 0..=n_ticks {
        let day = lo + span * (t as f64 / n_ticks as f64);
        let x = x_at(day);
        if t > 0 && t < n_ticks {
            ops.push(DrawOp::Line(LineOp {
                from: Point {
                    x: Twips(x),
                    y: Twips(plot_top),
                },
                to: Point {
                    x: Twips(x),
                    y: Twips(plot_bottom),
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
                left: Twips(x - 600),
                top: Twips(plot_bottom + 30),
                width: Twips(1200),
                height: Twips(axis_h),
            },
            text: fmt_date(day),
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
        }));
    }

    // Optional date-axis title, centered below the tick labels.
    if !axis_titles.category.is_empty() {
        ops.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(plot_left),
                top: Twips(plot_bottom + axis_h + 30),
                width: Twips(plot_w),
                height: Twips(260),
            },
            text: axis_titles.category.to_string(),
            font: FontSpec {
                family: "Arial".into(),
                size_pt: 8.0,
                ..Default::default()
            },
            color: LABEL,
            align: TextAlign::Center,
            rotation: 0.0,
            metrics: None,
            source: src(),
        }));
    }

    // One horizontal bar per record, stacked top-to-bottom. The row label is drawn in the left band,
    // thinned when the rows are too short for a readable label (as the category axis thins).
    let n = bars.len() as i32;
    let row_pitch = (plot_h / n).max(1);
    let bar_h = (row_pitch * 3 / 5).max(4);
    const MIN_ROW_LABEL_H: i32 = 200;
    let stride = if row_pitch >= MIN_ROW_LABEL_H {
        1
    } else {
        (MIN_ROW_LABEL_H as usize)
            .div_ceil(row_pitch.max(1) as usize)
            .max(1)
    };
    for (i, b) in bars.iter().enumerate() {
        let iy = i as i32;
        let slot_top = plot_top + iy * row_pitch;
        let by = slot_top + (row_pitch - bar_h) / 2;
        let bx = x_at(b.start);
        let bw = (x_at(b.end) - bx).max(15);
        ops.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(bx),
                top: Twips(by),
                width: Twips(bw),
                height: Twips(bar_h),
            },
            fill: Some(PALETTE[i % PALETTE.len()].into()),
            stroke: None,
            corner_radius: Twips(0),
            source: src(),
        }));
        if i % stride == 0 {
            ops.push(DrawOp::Text(TextRun {
                bounds: Rect {
                    left: Twips(rl + pad),
                    top: Twips(by + (bar_h - MIN_ROW_LABEL_H.min(bar_h)) / 2),
                    width: Twips((label_w - pad).max(1)),
                    height: Twips(MIN_ROW_LABEL_H.max(bar_h)),
                },
                text: truncate(&b.label, 18),
                font: FontSpec {
                    family: "Arial".into(),
                    size_pt: 7.0,
                    ..Default::default()
                },
                color: LABEL,
                align: TextAlign::Right,
                rotation: 0.0,
                metrics: None,
                source: src(),
            }));
        }
    }

    // Axes: the Y axis (left) and the date X axis (bottom), drawn on top of the gridlines/bars.
    let axis_stroke = Stroke {
        color: AXIS,
        width: Twips(15),
        style: rpt_pages::LineStyle::Single,
    };
    ops.push(DrawOp::Line(LineOp {
        from: Point {
            x: Twips(plot_left),
            y: Twips(plot_top),
        },
        to: Point {
            x: Twips(plot_left),
            y: Twips(plot_bottom),
        },
        stroke: axis_stroke,
        source: src(),
    }));
    ops.push(DrawOp::Line(LineOp {
        from: Point {
            x: Twips(plot_left),
            y: Twips(plot_bottom),
        },
        to: Point {
            x: Twips(plot_right),
            y: Twips(plot_bottom),
        },
        stroke: axis_stroke,
        source: src(),
    }));

    ops
}

/// Format a civil day-number as a compact `M/D/YY` tick label.
fn fmt_date(day: f64) -> String {
    let d = Date::from_days(day.round() as i64);
    format!("{}/{}/{:02}", d.month, d.day, d.year.rem_euclid(100))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect {
            left: Twips(100),
            top: Twips(100),
            width: Twips(8000),
            height: Twips(4000),
        }
    }

    fn bar(label: &str, start: (i32, u8, u8), end: (i32, u8, u8)) -> GanttBar {
        GanttBar {
            label: label.into(),
            start: Date::new(start.0, start.1, start.2).to_days() as f64,
            end: Date::new(end.0, end.1, end.2).to_days() as f64,
        }
    }

    #[test]
    fn empty_bars_yield_no_ops() {
        assert!(gantt_chart(rect(), "T", AxisTitles::default(), &[], "S", "G").is_empty());
    }

    /// One horizontal bar per record, each spanning its start→end on the shared date axis: an earlier
    /// bar starts left of a later one, and a longer span is a wider rect.
    #[test]
    fn draws_one_horizontal_bar_per_record_spanning_the_date_range() {
        let bars = vec![
            bar("A", (2024, 1, 1), (2024, 1, 10)),
            bar("B", (2024, 1, 5), (2024, 1, 8)),
            bar("C", (2024, 1, 15), (2024, 1, 31)),
        ];
        let ops = gantt_chart(rect(), "Schedule", AxisTitles::default(), &bars, "RH", "G");
        let rects: Vec<(i32, i32, i32, i32)> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Rect(r) => Some((
                    r.bounds.left.0,
                    r.bounds.top.0,
                    r.bounds.width.0,
                    r.bounds.height.0,
                )),
                _ => None,
            })
            .collect();
        assert_eq!(rects.len(), 3, "one bar per record");
        // Rows stack top-to-bottom (increasing `top`).
        assert!(
            rects[0].1 < rects[1].1 && rects[1].1 < rects[2].1,
            "stacked"
        );
        // Bar C starts on 1/15, right of bar A which starts on 1/1.
        assert!(rects[2].0 > rects[0].0, "later start is further right");
        // Bar A spans 9 days, bar B only 3 → A is wider.
        assert!(rects[0].2 > rects[1].2, "longer span is a wider bar");
        // Two axes are drawn.
        let lines = ops.iter().filter(|o| matches!(o, DrawOp::Line(_))).count();
        assert!(lines >= 2, "at least the two axes, got {lines}");
        // Date tick labels are formatted M/D/YY (the January range).
        let texts: Vec<String> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        assert!(
            texts.iter().any(|t| t.starts_with("1/")),
            "January tick labels: {texts:?}"
        );
        // Each row is labelled.
        for l in ["A", "B", "C"] {
            assert!(texts.contains(&l.to_string()), "row label {l}: {texts:?}");
        }
    }

    /// A zero-width span (start == end on every record) still draws a visible minimum-width bar rather
    /// than collapsing to nothing or dividing by a zero range.
    #[test]
    fn zero_width_span_still_draws_minimum_bars() {
        let bars = vec![
            bar("A", (2024, 3, 1), (2024, 3, 1)),
            bar("B", (2024, 3, 1), (2024, 3, 1)),
        ];
        let ops = gantt_chart(rect(), "", AxisTitles::default(), &bars, "RH", "G");
        let widths: Vec<i32> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Rect(r) => Some(r.bounds.width.0),
                _ => None,
            })
            .collect();
        assert_eq!(widths.len(), 2, "one bar per record");
        assert!(
            widths.iter().all(|&w| w >= 15),
            "minimum bar width: {widths:?}"
        );
    }
}
