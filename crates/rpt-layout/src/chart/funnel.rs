//! Funnel chart: proportional horizontal bands stacked top-to-bottom, each band's width scaled by
//! its value relative to the series maximum. Each band is a trapezoid whose top edge is this value's
//! width and whose bottom edge is the next value's width, so a descending series tapers to a tip.
//! No axes.

use super::common::{fmt_val, truncate, LABEL, PALETTE, TITLE_PT, WHITE};
use rpt_model::{Rect, Twips};
use rpt_pages::{
    DrawOp, FontSpec, ObjectKind, ObjectRef, Point, PolygonOp, Stroke, TextAlign, TextRun,
};

/// Build the draw-ops for a funnel chart of `series` (category label → value): stacked proportional
/// bands, widest at the top and tapering down, each band a centered trapezoid from this value's width
/// (top edge) to the next value's width (bottom edge). Bands are palette-cycled with a thin white
/// separator. The category label always draws; `show_labels` gates the per-band value label. Returns
/// an empty vec if `series` is empty or every value is ≤ 0.
pub(crate) fn funnel_chart(
    rect: Rect,
    title: &str,
    series: &[(String, f64)],
    show_labels: bool,
    section_name: &str,
    obj_name: &str,
) -> Vec<DrawOp> {
    let max = series
        .iter()
        .map(|(_, v)| v.max(0.0))
        .fold(0.0_f64, f64::max);
    if series.is_empty() || max <= 0.0 {
        return Vec::new();
    }
    let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(obj_name));
    let mut ops: Vec<DrawOp> = Vec::new();
    let (rl, rt, rw, rh) = (rect.left.0, rect.top.0, rect.width.0, rect.height.0);
    let pad = 60;

    let title_h = if title.is_empty() {
        0
    } else {
        (rh / 8).clamp(180, 360)
    };
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

    let cx = rl + rw / 2;
    let plot_top = rt + title_h + pad;
    let plot_h = (rt + rh - pad - plot_top).max(1);
    let plot_w = (rw - 2 * pad).max(1) as f64;

    let n = series.len();
    let band_h = plot_h / n as i32;
    // Each value's band width, scaled so the largest value fills the plot width.
    let width_of = |v: f64| (v.max(0.0) / max) * plot_w;

    for (i, (label, val)) in series.iter().enumerate() {
        let top_w = width_of(*val);
        // The bottom edge is the next value's width; the final band tapers to a tip.
        let bot_w = if i + 1 < n {
            width_of(series[i + 1].1)
        } else {
            top_w * 0.6
        };
        let ty = plot_top + i as i32 * band_h;
        let by = ty + band_h;
        let hw_top = (top_w / 2.0) as i32;
        let hw_bot = (bot_w / 2.0) as i32;
        ops.push(DrawOp::Polygon(PolygonOp {
            points: vec![
                Point {
                    x: Twips(cx - hw_top),
                    y: Twips(ty),
                },
                Point {
                    x: Twips(cx + hw_top),
                    y: Twips(ty),
                },
                Point {
                    x: Twips(cx + hw_bot),
                    y: Twips(by),
                },
                Point {
                    x: Twips(cx - hw_bot),
                    y: Twips(by),
                },
            ],
            closed: true,
            fill: Some(PALETTE[i % PALETTE.len()].into()),
            stroke: Some(Stroke {
                color: WHITE,
                width: Twips(20),
                style: rpt_pages::LineStyle::Single,
            }),
            source: src(),
        }));

        // Category (always) + value (gated) labels, centred in the band.
        let mid_y = ty + band_h / 2;
        let text = if show_labels {
            format!("{}  {}", truncate(label, 16), fmt_val(*val))
        } else {
            truncate(label, 16)
        };
        ops.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(cx - 1400),
                top: Twips(mid_y - 100),
                width: Twips(2800),
                height: Twips(200),
            },
            text,
            font: FontSpec {
                family: "Arial".into(),
                size_pt: 7.0,
                ..Default::default()
            },
            color: WHITE,
            align: TextAlign::Center,
            rotation: 0.0,
            metrics: None,
            source: src(),
        }));
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

    /// The trapezoids in emit order (each is a 4-point closed polygon). Returns the top-edge width.
    fn band_top_widths(ops: &[DrawOp]) -> Vec<i32> {
        ops.iter()
            .filter_map(|o| match o {
                DrawOp::Polygon(p) if p.closed && p.points.len() == 4 => {
                    Some(p.points[1].x.0 - p.points[0].x.0)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn empty_series_yields_no_ops() {
        assert!(funnel_chart(rect(), "T", &[], true, "S", "G").is_empty());
    }

    /// n values → n trapezoid bands whose top widths decrease monotonically as the values decrease.
    #[test]
    fn bands_taper_with_decreasing_values() {
        let s = vec![
            ("Leads".into(), 100.0),
            ("Qualified".into(), 60.0),
            ("Proposals".into(), 25.0),
            ("Won".into(), 10.0),
        ];
        let ops = funnel_chart(rect(), "Sales", &s, true, "RH", "G");
        let widths = band_top_widths(&ops);
        assert_eq!(widths.len(), s.len(), "one trapezoid per value");
        for w in widths.windows(2) {
            assert!(w[0] > w[1], "widths taper: {widths:?}");
        }
    }

    /// With "show value" off, the bands and category labels draw, but the value text is dropped.
    #[test]
    fn show_labels_false_omits_value_labels() {
        let s = vec![("Leads".into(), 100.0), ("Won".into(), 12.0)];
        let with = funnel_chart(rect(), "T", &s, true, "RH", "G");
        let without = funnel_chart(rect(), "T", &s, false, "RH", "G");
        let has_value = |ops: &[DrawOp], v: &str| {
            ops.iter().any(|o| match o {
                DrawOp::Text(t) => t.text.contains(v),
                _ => false,
            })
        };
        assert!(has_value(&with, "12"), "value shown when enabled");
        assert!(!has_value(&without, "12"), "value omitted when disabled");
        // Category labels remain in both.
        assert!(has_value(&without, "Leads"), "category label remains");
    }
}
