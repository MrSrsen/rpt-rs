//! Stock chart (graph type code 10): a vertical hi-lo bar per category over a numeric value axis.
//! A High-Low subtype draws just the low→high bar; a High-Low-Open-Close subtype
//! adds a left open tick and a right close tick. One bar per category (the report's group / the
//! chart's own bucketing), the low and high being that category's minimum and maximum of the bound
//! value fields.

use super::common::{category_label, category_stride, chart_frame, AxisTitles, PALETTE};
use rpt_model::{Rect, Twips};
use rpt_pages::{DrawOp, LineOp, ObjectKind, ObjectRef, Point, RectOp, Stroke};

/// One category's stock values: the low→high range, plus optional open/close ticks (present only for
/// the OHLC subtype).
#[derive(Debug, Clone)]
pub(crate) struct StockPoint {
    pub(crate) label: String,
    pub(crate) high: f64,
    pub(crate) low: f64,
    /// Open value — a left tick when present (OHLC subtype).
    pub(crate) open: Option<f64>,
    /// Close value — a right tick when present (OHLC subtype).
    pub(crate) close: Option<f64>,
}

/// Build the draw-ops for a stock chart of `points` (one per category): the shared value-axis frame,
/// a thin vertical low→high bar per category, and — for OHLC points — a left open tick and a right
/// close tick. Category labels are thinned like the other axis families. Returns an empty vec when
/// there are no points.
pub(crate) fn stock_chart(
    rect: Rect,
    title: &str,
    axis_titles: AxisTitles,
    points: &[StockPoint],
    section_name: &str,
    obj_name: &str,
) -> Vec<DrawOp> {
    if points.is_empty() {
        return Vec::new();
    }
    let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(obj_name));
    let mut ops: Vec<DrawOp> = Vec::new();

    // The value scale reserves `points.len()` category slots and scales to the tallest high.
    let series: Vec<(String, f64)> = points.iter().map(|p| (p.label.clone(), p.high)).collect();
    let f = chart_frame(&mut ops, rect, title, axis_titles, &series, &src);
    let y_at = |v: f64| f.plot_bottom - ((v.max(0.0) / f.max_val) * f.plot_h as f64) as i32;

    let bar_w = (f.slot / 6).clamp(30, 120);
    let tick = bar_w.max(45);
    let bar_fill = PALETTE[0];
    let stroke = Stroke {
        color: bar_fill,
        width: Twips(15),
        style: rpt_pages::LineStyle::Single,
    };
    let stride = category_stride(&f, points.len());
    for (i, p) in points.iter().enumerate() {
        let cx = f.plot_left + i as i32 * f.slot + f.slot / 2;
        let hy = y_at(p.high);
        let ly = y_at(p.low);
        // The low→high bar, drawn as a thin vertical rect (min 1 twip tall so a flat range shows).
        ops.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(cx - bar_w / 2),
                top: Twips(hy),
                width: Twips(bar_w),
                height: Twips((ly - hy).max(1)),
            },
            fill: Some(bar_fill.into()),
            stroke: None,
            corner_radius: Twips(0),
            source: src(),
        }));
        // OHLC ticks: open on the left, close on the right.
        if let Some(open) = p.open {
            let oy = y_at(open);
            ops.push(DrawOp::Line(LineOp {
                from: Point {
                    x: Twips(cx - tick),
                    y: Twips(oy),
                },
                to: Point {
                    x: Twips(cx),
                    y: Twips(oy),
                },
                stroke,
                source: src(),
            }));
        }
        if let Some(close) = p.close {
            let cy = y_at(close);
            ops.push(DrawOp::Line(LineOp {
                from: Point {
                    x: Twips(cx),
                    y: Twips(cy),
                },
                to: Point {
                    x: Twips(cx + tick),
                    y: Twips(cy),
                },
                stroke,
                source: src(),
            }));
        }
        if i % stride == 0 {
            ops.push(category_label(&f, i as i32, &p.label, &src));
        }
    }

    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect {
            left: Twips(100),
            top: Twips(100),
            width: Twips(6000),
            height: Twips(4000),
        }
    }

    fn hilo(label: &str, high: f64, low: f64) -> StockPoint {
        StockPoint {
            label: label.into(),
            high,
            low,
            open: None,
            close: None,
        }
    }

    #[test]
    fn empty_points_yield_no_ops() {
        assert!(stock_chart(rect(), "T", AxisTitles::default(), &[], "S", "G").is_empty());
    }

    /// A hi-lo point draws one thin vertical bar per category (taller than it is wide) and no ticks.
    #[test]
    fn hilo_draws_one_vertical_bar_per_category() {
        let pts = vec![hilo("Jan", 200.0, 40.0), hilo("Feb", 120.0, 80.0)];
        let ops = stock_chart(rect(), "", AxisTitles::default(), &pts, "RH", "G");
        let bars: Vec<&RectOp> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Rect(r) => Some(r),
                _ => None,
            })
            .collect();
        assert_eq!(bars.len(), pts.len(), "one bar per category");
        assert!(
            bars.iter().all(|b| b.bounds.height.0 > b.bounds.width.0),
            "hi-lo bars are vertical"
        );
    }

    /// The taller hi-lo range yields a taller bar than a narrow range.
    #[test]
    fn taller_range_makes_a_taller_bar() {
        let pts = vec![hilo("wide", 240.0, 0.0), hilo("narrow", 120.0, 100.0)];
        let ops = stock_chart(rect(), "", AxisTitles::default(), &pts, "RH", "G");
        let heights: Vec<i32> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Rect(r) => Some(r.bounds.height.0),
                _ => None,
            })
            .collect();
        assert!(
            heights[0] > heights[1],
            "wider range → taller bar: {heights:?}"
        );
    }

    /// An OHLC point adds an open tick (left) and a close tick (right) — two extra line segments per
    /// point beyond the two axis lines the frame draws.
    #[test]
    fn ohlc_adds_open_and_close_ticks() {
        let pts = vec![StockPoint {
            label: "Jan".into(),
            high: 200.0,
            low: 40.0,
            open: Some(60.0),
            close: Some(180.0),
        }];
        let ops = stock_chart(rect(), "", AxisTitles::default(), &pts, "RH", "G");
        // The two frame axes plus the open + close ticks = 4 line ops (no gridlines at this scale
        // would still leave the two axes; the ticks are the delta over the hi-lo case).
        let lines = ops.iter().filter(|o| matches!(o, DrawOp::Line(_))).count();
        let hilo_lines = {
            let h = vec![hilo("Jan", 200.0, 40.0)];
            stock_chart(rect(), "", AxisTitles::default(), &h, "RH", "G")
                .iter()
                .filter(|o| matches!(o, DrawOp::Line(_)))
                .count()
        };
        assert_eq!(lines, hilo_lines + 2, "OHLC adds an open and a close tick");
    }
}
