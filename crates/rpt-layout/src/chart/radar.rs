//! Radar / polar chart: categories are spaced evenly around a circle (angle = i/n·360°, starting at
//! 12 o'clock) and each category's value maps to a radius on a 0..nice_max radial scale. The series
//! is one closed polygon through the per-category radial points, over a polar grid of concentric
//! rings and radial spokes — no cartesian axes.

use super::common::{
    fmt_val, nice_scale, truncate, value_label, AXIS, GRID, LABEL, PALETTE, TITLE_PT,
};
use rpt_model::{Color, Rect, Twips};
use rpt_pages::{
    DrawOp, EllipseOp, FontSpec, LineOp, ObjectKind, ObjectRef, Point, PolygonOp, Stroke,
    TextAlign, TextRun,
};
use std::f64::consts::{FRAC_PI_2, TAU};

/// Build the draw-ops for a radar chart of `series` (category label → value): the categories are
/// spread evenly around a circle starting at the top, each value scaled to a radius on a shared
/// 0..nice_max radial scale. Draws concentric grid rings + radial spokes + a category label at each
/// rim, then the series as one closed polygon through the radial points (translucent fill + stroke)
/// with a small marker at each vertex. `show_labels` gates the per-vertex data-value labels. Returns
/// an empty vec if `series` is empty.
pub(crate) fn radar_chart(
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
        ops.push(title_op(rl, rt + pad / 2, rw, title_h, title, &src));
    }

    // Centre the polar plot in the area below the title, leaving room for the rim category labels.
    let box_top = rt + title_h + pad;
    let box_h = (rt + rh - pad - box_top).max(1);
    let box_w = (rw - 2 * pad).max(1);
    let cx = rl + rw / 2;
    let cy = box_top + box_h / 2;
    let radius = (box_w.min(box_h) / 2 * 4 / 5).max(1) as f64;

    let n = series.len() as i32;
    // Angle of category `i`: evenly spaced, starting at 12 o'clock and advancing clockwise.
    let angle = |i: i32| -FRAC_PI_2 + (i as f64 / n as f64) * TAU;

    let raw_max = series
        .iter()
        .map(|(_, v)| v.max(0.0))
        .fold(0.0_f64, f64::max);
    let (nice_max, step) = nice_scale(raw_max);
    let ticks = (nice_max / step).round().max(1.0) as i32;

    // Concentric grid rings at each radial tick.
    for t in 1..=ticks {
        let rr = (t as f64 * step / nice_max) * radius;
        ops.push(DrawOp::Ellipse(EllipseOp {
            bounds: Rect {
                left: Twips(cx - rr as i32),
                top: Twips(cy - rr as i32),
                width: Twips((2.0 * rr) as i32),
                height: Twips((2.0 * rr) as i32),
            },
            fill: None,
            stroke: Some(Stroke {
                color: GRID,
                width: Twips(10),
                style: rpt_pages::LineStyle::Single,
            }),
            source: src(),
        }));
    }

    // Radial spokes + a category label just past the rim of each spoke.
    for i in 0..n {
        let a = angle(i);
        let rim = Point {
            x: Twips(cx + (radius * a.cos()) as i32),
            y: Twips(cy + (radius * a.sin()) as i32),
        };
        ops.push(DrawOp::Line(LineOp {
            from: Point {
                x: Twips(cx),
                y: Twips(cy),
            },
            to: rim,
            stroke: Stroke {
                color: AXIS,
                width: Twips(10),
                style: rpt_pages::LineStyle::Single,
            },
            source: src(),
        }));
        let lx = cx + (radius * 1.12 * a.cos()) as i32;
        let ly = cy + (radius * 1.12 * a.sin()) as i32;
        ops.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(lx - 700),
                top: Twips(ly - 100),
                width: Twips(1400),
                height: Twips(200),
            },
            text: truncate(&series[i as usize].0, 14),
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

    // The series as one closed polygon through the per-category radial points.
    let points: Vec<Point> = series
        .iter()
        .enumerate()
        .map(|(i, (_, v))| {
            let a = angle(i as i32);
            let r = (v.max(0.0) / nice_max) * radius;
            Point {
                x: Twips(cx + (r * a.cos()) as i32),
                y: Twips(cy + (r * a.sin()) as i32),
            }
        })
        .collect();
    ops.push(DrawOp::Polygon(PolygonOp {
        points: points.clone(),
        closed: true,
        // A translucent fill of the series colour so overlapping grid stays visible through it.
        fill: Some(
            Color {
                a: 70,
                ..PALETTE[0]
            }
            .into(),
        ),
        stroke: Some(Stroke {
            color: PALETTE[0],
            width: Twips(30),
            style: rpt_pages::LineStyle::Single,
        }),
        source: src(),
    }));

    // A small filled marker at each vertex, plus the gated data-value label.
    const M: i32 = 70;
    for (i, p) in points.iter().enumerate() {
        ops.push(DrawOp::Ellipse(EllipseOp {
            bounds: Rect {
                left: Twips(p.x.0 - M),
                top: Twips(p.y.0 - M),
                width: Twips(M * 2),
                height: Twips(M * 2),
            },
            fill: Some(PALETTE[0].into()),
            stroke: None,
            source: src(),
        }));
        if show_labels {
            ops.push(value_label(
                p.x.0,
                p.y.0 - 220,
                &fmt_val(series[i].1),
                LABEL,
                &src,
            ));
        }
    }

    ops
}

/// A centered, bold chart title `TextRun` (shared shape with the pie/doughnut titles).
fn title_op(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    title: &str,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> DrawOp {
    DrawOp::Text(TextRun {
        bounds: Rect {
            left: Twips(x),
            top: Twips(y),
            width: Twips(w),
            height: Twips(h),
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series() -> Vec<(String, f64)> {
        vec![
            ("North".into(), 12.0),
            ("East".into(), 27.0),
            ("South".into(), 6.0),
            ("West".into(), 18.0),
        ]
    }

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
        assert!(radar_chart(rect(), "T", &[], true, "S", "G").is_empty());
    }

    /// n categories → one closed polygon with n vertices, at least one grid ring, and a rim label per
    /// category.
    #[test]
    fn draws_polygon_rings_and_category_labels() {
        let s = series();
        let ops = radar_chart(rect(), "Compass", &s, true, "RH", "G");
        let closed: Vec<&PolygonOp> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Polygon(p) if p.closed => Some(p),
                _ => None,
            })
            .collect();
        assert_eq!(closed.len(), 1, "one closed series polygon");
        assert_eq!(closed[0].points.len(), s.len(), "n vertices");
        // Grid rings are the stroked, unfilled ellipses (markers are filled).
        let rings = ops
            .iter()
            .filter(|o| matches!(o, DrawOp::Ellipse(e) if e.fill.is_none()))
            .count();
        assert!(rings >= 1, "at least one grid ring, got {rings}");
        let texts: Vec<String> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        for (label, _) in &s {
            assert!(texts.contains(label), "rim label {label} in {texts:?}");
        }
    }

    /// With "show value" off the polygon, rings, and rim labels still draw, but no per-vertex value
    /// label is emitted.
    #[test]
    fn show_labels_false_omits_value_labels() {
        // Off the nice-number ticks so a data value can't collide with an axis label.
        let s = vec![("A".into(), 12.0), ("B".into(), 27.0), ("C".into(), 6.0)];
        let ops = radar_chart(rect(), "T", &s, false, "RH", "G");
        let texts: Vec<String> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        for v in ["12", "27", "6"] {
            assert!(
                !texts.contains(&v.to_string()),
                "value label {v} must be omitted: {texts:?}"
            );
        }
    }
}
