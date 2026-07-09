//! Doughnut chart: a pie with a hollow centre — each slice is an annular ring segment (bounded by
//! the outer disc arc and an inner-radius arc) rather than a centre-anchored wedge. The slice sweep
//! math is identical to [`super::pie`] (`value / total × 360°`, starting at 12 o'clock).

use super::common::{slice_color, truncate, value_label, LABEL, TITLE_PT, WHITE};
use rpt_model::{Rect, Twips};
use rpt_pages::{
    DrawOp, FontSpec, ObjectKind, ObjectRef, Point, PolygonOp, Stroke, TextAlign, TextRun,
};
use std::f64::consts::{FRAC_PI_2, TAU};

/// Fraction of the outer radius at which the doughnut hole begins (the inner ring edge).
const INNER_RATIO: f64 = 0.55;

/// Build the draw-ops for a doughnut chart of `series` (category label → value): like a pie, but each
/// slice is a filled ring segment between the outer radius `R` and the inner radius `INNER_RATIO·R`,
/// leaving a hollow centre. Sweep angles are `value / total × 360°` starting at 12 o'clock; values
/// ≤ 0 are ignored. `show_labels` gates the per-slice percentage data labels (the report's decoded
/// "show value" flag); the category label sits outside the ring like a pie. Returns an empty vec if
/// there is nothing positive to plot.
pub(crate) fn doughnut_chart(
    rect: Rect,
    title: &str,
    series: &[(String, f64)],
    show_labels: bool,
    section_name: &str,
    obj_name: &str,
) -> Vec<DrawOp> {
    let total: f64 = series.iter().map(|(_, v)| v.max(0.0)).sum();
    if series.is_empty() || total <= 0.0 {
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

    // Centre the disc in the area below the title; leave a small margin for outer labels.
    let box_top = rt + title_h + pad;
    let box_h = (rt + rh - pad - box_top).max(1);
    let box_w = (rw - 2 * pad).max(1);
    let cx = rl + rw / 2;
    let cy = box_top + box_h / 2;
    let radius = (box_w.min(box_h) / 2 * 4 / 5).max(1) as f64;
    let inner = radius * INNER_RATIO;

    let mut angle = -FRAC_PI_2; // first slice starts at 12 o'clock
    for (i, (label, val)) in series.iter().enumerate() {
        let frac = val.max(0.0) / total;
        if frac <= 0.0 {
            continue;
        }
        let sweep = frac * TAU;
        // Tessellate the arc: adaptive segments at ~30-twip flatness.
        let steps = ((sweep * radius / 30.0).ceil() as i32).clamp(2, 512);
        let at = |r: f64, a: f64| Point {
            x: Twips(cx + (r * a.cos()) as i32),
            y: Twips(cy + (r * a.sin()) as i32),
        };
        // Ring segment: walk the outer arc forward, then the inner arc back, and close — so the slice
        // is an annulus wedge with a hole at the centre rather than a full pie wedge.
        let mut points = Vec::with_capacity(2 * (steps as usize + 1));
        for s in 0..=steps {
            points.push(at(radius, angle + sweep * (s as f64 / steps as f64)));
        }
        for s in (0..=steps).rev() {
            points.push(at(inner, angle + sweep * (s as f64 / steps as f64)));
        }
        ops.push(DrawOp::Polygon(PolygonOp {
            points,
            closed: true,
            fill: Some(slice_color(i).into()),
            // A thin white border separates adjacent slices.
            stroke: Some(Stroke {
                color: WHITE,
                width: Twips(20),
                style: rpt_pages::LineStyle::Single,
            }),
            source: src(),
        }));
        let mid = angle + sweep / 2.0;
        // Percentage data label at the mid-ring radius, in white for contrast on the fill, gated on
        // "show value". Skipped for thin slices where it would not fit.
        if show_labels && frac >= 0.05 {
            let mr = radius * (1.0 + INNER_RATIO) / 2.0;
            ops.push(value_label(
                cx + (mr * mid.cos()) as i32,
                cy + (mr * mid.sin()) as i32 - 100,
                &format!("{:.0}%", frac * 100.0),
                WHITE,
                &src,
            ));
        }
        // Category label at the slice's outer midpoint.
        let lr = radius * 1.02;
        let lx = cx + (lr * mid.cos()) as i32;
        let ly = cy + (lr * mid.sin()) as i32;
        ops.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(lx - 700),
                top: Twips(ly - 100),
                width: Twips(1400),
                height: Twips(200),
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
        }));
        angle += sweep;
    }

    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series() -> Vec<(String, f64)> {
        vec![
            ("Canada".into(), 40.0),
            ("USA".into(), 35.0),
            ("Mexico".into(), 25.0),
        ]
    }

    /// Ray-casting point-in-polygon over the twip vertices.
    fn contains(poly: &PolygonOp, px: i32, py: i32) -> bool {
        let pts = &poly.points;
        let mut inside = false;
        let mut j = pts.len() - 1;
        for i in 0..pts.len() {
            let (xi, yi) = (pts[i].x.0 as f64, pts[i].y.0 as f64);
            let (xj, yj) = (pts[j].x.0 as f64, pts[j].y.0 as f64);
            if (yi > py as f64) != (yj > py as f64) {
                let x_cross = (xj - xi) * (py as f64 - yi) / (yj - yi) + xi;
                if (px as f64) < x_cross {
                    inside = !inside;
                }
            }
            j = i;
        }
        inside
    }

    /// Each positive value becomes one closed ring-segment polygon, and none of them contains the
    /// disc centre — proving the hollow doughnut hole (an empty title centres the disc on the rect).
    #[test]
    fn draws_closed_ring_segments_with_a_hole() {
        let r = Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(6000),
            height: Twips(6000),
        };
        let ops = doughnut_chart(r, "", &series(), false, "RH", "Graph1");
        let polys: Vec<&PolygonOp> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Polygon(p) => Some(p),
                _ => None,
            })
            .collect();
        assert_eq!(polys.len(), 3, "one ring segment per slice");
        // With an empty title the disc is centred on the rect (see the box math).
        let (cx, cy) = (3000, 3000);
        for p in &polys {
            assert!(p.closed, "ring segment is a closed polygon");
            assert!(
                !contains(p, cx, cy),
                "the centre lies in the hole, not inside any ring segment"
            );
        }
    }

    /// With "show value" on, each slice draws a ring segment, a category label, and a percentage data
    /// label at the mid-ring.
    #[test]
    fn show_labels_true_draws_percentages() {
        let r = Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(6000),
            height: Twips(6000),
        };
        let ops = doughnut_chart(r, "Split", &series(), true, "RH", "Graph1");
        let texts: Vec<String> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        for p in ["40%", "35%", "25%"] {
            assert!(
                texts.contains(&p.to_string()),
                "percentage {p} in {texts:?}"
            );
        }
    }

    /// With "show value" off, the ring segments and category labels still draw, but no percentage data
    /// label is emitted.
    #[test]
    fn show_labels_false_omits_percentages() {
        let r = Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(6000),
            height: Twips(6000),
        };
        let ops = doughnut_chart(r, "Split", &series(), false, "RH", "Graph1");
        let rings = ops
            .iter()
            .filter(|o| matches!(o, DrawOp::Polygon(_)))
            .count();
        let texts: Vec<String> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(rings, 3, "one ring segment per slice without labels");
        for c in ["Canada", "USA", "Mexico"] {
            assert!(texts.contains(&c.to_string()), "category {c} in {texts:?}");
        }
        assert!(
            !texts.iter().any(|t| t.ends_with('%')),
            "no percentage labels: {texts:?}"
        );
    }
}
