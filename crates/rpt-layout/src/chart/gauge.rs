//! Gauge chart: a dial — a 270° arc scale from 0 to nice_max (opening at the bottom), tick marks and
//! labels around it, and a needle from the centre pointing at the series aggregate (its total, or its
//! single value when there is one category). A small hub sits at the centre and the aggregate value
//! prints under the dial.

use super::common::{fmt_val, nice_scale, AXIS, LABEL, PALETTE, TITLE_PT};
use rpt_model::{Rect, Twips};
use rpt_pages::{
    DrawOp, EllipseOp, FontSpec, LineOp, ObjectKind, ObjectRef, Point, PolygonOp, Stroke,
    TextAlign, TextRun,
};
use std::f64::consts::PI;

/// The dial sweeps 270° opening at the bottom: from 135° (bottom-left) clockwise through the top to
/// 45° (bottom-right).
const START: f64 = 3.0 * PI / 4.0;
const SPAN: f64 = 3.0 * PI / 2.0;

/// Build the draw-ops for a gauge chart of `series` (category label → value): a 270° arc scale from 0
/// to nice_max with tick marks + labels, and a needle from the centre pointing at the aggregate
/// value (the series total). `show_labels` gates the big aggregate-value readout under the dial; the
/// tick labels and title always draw. Returns an empty vec if `series` is empty.
pub(crate) fn gauge_chart(
    rect: Rect,
    title: &str,
    series: &[(String, f64)],
    show_labels: bool,
    section_name: &str,
    obj_name: &str,
) -> Vec<DrawOp> {
    if series.is_empty() {
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

    let value: f64 = series.iter().map(|(_, v)| *v).sum();
    let (nice_max, step) = nice_scale(value);
    let ticks = (nice_max / step).round().max(1.0) as i32;

    // Centre the dial in the area below the title, leaving room for the value readout below it.
    let box_top = rt + title_h + pad;
    let box_h = (rt + rh - pad - box_top).max(1);
    let box_w = (rw - 2 * pad).max(1);
    let cx = rl + rw / 2;
    let cy = box_top + box_h / 2;
    let radius = (box_w.min(box_h) / 2 * 4 / 5).max(1) as f64;

    let angle_at = |frac: f64| START + frac.clamp(0.0, 1.0) * SPAN;
    let at = |a: f64, r: f64| Point {
        x: Twips(cx + (r * a.cos()) as i32),
        y: Twips(cy + (r * a.sin()) as i32),
    };

    // The arc scale itself, tessellated to an open polyline.
    let steps = 96;
    let arc: Vec<Point> = (0..=steps)
        .map(|s| at(START + (s as f64 / steps as f64) * SPAN, radius))
        .collect();
    ops.push(DrawOp::Polygon(PolygonOp {
        points: arc,
        closed: false,
        fill: None,
        stroke: Some(Stroke {
            color: AXIS,
            width: Twips(20),
            style: rpt_pages::LineStyle::Single,
        }),
        source: src(),
    }));

    // Tick marks + labels around the arc.
    for t in 0..=ticks {
        let frac = t as f64 / ticks as f64;
        let a = angle_at(frac);
        ops.push(DrawOp::Line(LineOp {
            from: at(a, radius),
            to: at(a, radius * 0.9),
            stroke: Stroke {
                color: AXIS,
                width: Twips(15),
                style: rpt_pages::LineStyle::Single,
            },
            source: src(),
        }));
        let lp = at(a, radius * 1.12);
        ops.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(lp.x.0 - 500),
                top: Twips(lp.y.0 - 100),
                width: Twips(1000),
                height: Twips(200),
            },
            text: fmt_val(t as f64 * step),
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

    // The needle, from the centre to the value's fraction of the scale.
    let frac = value / nice_max;
    ops.push(DrawOp::Line(LineOp {
        from: Point {
            x: Twips(cx),
            y: Twips(cy),
        },
        to: at(angle_at(frac), radius * 0.8),
        stroke: Stroke {
            color: PALETTE[3],
            width: Twips(40),
            style: rpt_pages::LineStyle::Single,
        },
        source: src(),
    }));

    // Centre hub.
    let hub = 90;
    ops.push(DrawOp::Ellipse(EllipseOp {
        bounds: Rect {
            left: Twips(cx - hub),
            top: Twips(cy - hub),
            width: Twips(hub * 2),
            height: Twips(hub * 2),
        },
        fill: Some(AXIS.into()),
        stroke: None,
        source: src(),
    }));

    // The aggregate value printed under the dial (gated on "show value").
    if show_labels {
        ops.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(rl),
                top: Twips(cy + radius as i32 / 2),
                width: Twips(rw),
                height: Twips(280),
            },
            text: fmt_val(value),
            font: FontSpec {
                family: "Arial".into(),
                size_pt: 12.0,
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
            height: Twips(6000),
        }
    }

    #[test]
    fn empty_series_yields_no_ops() {
        assert!(gauge_chart(rect(), "T", &[], true, "S", "G").is_empty());
    }

    /// One needle (a Line from the hub centre) + an arc (an open polyline) + the aggregate value
    /// readout; the needle angle lies within the 270° arc span.
    #[test]
    fn draws_needle_arc_and_value() {
        // total = 22 → nice_max 25 (nice_scale), so the needle sits below full-scale.
        let s = vec![("A".into(), 15.0), ("B".into(), 7.0)];
        let ops = gauge_chart(rect(), "Load", &s, true, "RH", "G");

        // The hub is the only filled ellipse; its centre is the dial centre.
        let hub = ops
            .iter()
            .find_map(|o| match o {
                DrawOp::Ellipse(e) if e.fill.is_some() => Some(e.bounds),
                _ => None,
            })
            .expect("hub ellipse");
        let cx = hub.left.0 + hub.width.0 / 2;
        let cy = hub.top.0 + hub.height.0 / 2;

        // Exactly one open polyline arc.
        let arcs = ops
            .iter()
            .filter(|o| matches!(o, DrawOp::Polygon(p) if !p.closed))
            .count();
        assert_eq!(arcs, 1, "one arc polyline");

        // The needle is the Line originating at the centre.
        let needle = ops
            .iter()
            .find_map(|o| match o {
                DrawOp::Line(l) if l.from.x.0 == cx && l.from.y.0 == cy => Some(l),
                _ => None,
            })
            .expect("needle line from centre");
        let dx = (needle.to.x.0 - cx) as f64;
        let dy = (needle.to.y.0 - cy) as f64;
        let mut a = dy.atan2(dx);
        if a < 0.0 {
            a += 2.0 * PI;
        }
        // The 270° arc opening at the bottom covers [135°,360°) ∪ [0°,45°].
        let deg = a.to_degrees();
        assert!(
            deg >= 135.0 || deg <= 45.0,
            "needle angle {deg}° within the arc span"
        );

        // The aggregate value (22) prints under the dial when labels are on.
        let texts: Vec<String> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        assert!(
            texts.contains(&"22".to_string()),
            "value readout in {texts:?}"
        );
    }

    /// With "show value" off, the arc/needle/ticks draw but the big aggregate readout is omitted.
    #[test]
    fn show_labels_false_omits_value_readout() {
        // total = 22; ticks are 0/5/10/15/20/25, none of which equals 22.
        let s = vec![("A".into(), 15.0), ("B".into(), 7.0)];
        let ops = gauge_chart(rect(), "T", &s, false, "RH", "G");
        let has_value = ops.iter().any(|o| match o {
            DrawOp::Text(t) => t.text == "22",
            _ => false,
        });
        assert!(!has_value, "aggregate readout omitted");
        // The needle still draws.
        assert!(
            ops.iter().any(|o| matches!(o, DrawOp::Line(_))),
            "needle/ticks still drawn"
        );
    }
}
