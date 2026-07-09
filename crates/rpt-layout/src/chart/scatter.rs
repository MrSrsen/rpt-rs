//! XY scatter chart (graph type code 7): a filled marker at each `(x, y)` data point over two
//! numeric axes, with no connecting line. Unlike the category families the X axis
//! is a continuous 0..nice_max scale over the first value binding, and the Y axis a 0..nice_max scale
//! over the second — one point per detail row rather than one per group.
//!
//! A **bubble** chart (graph type code 9) is the same XY plot with a third value binding sizing each
//! marker: a filled circle whose *area* is proportional to the third value (so its radius scales as
//! the square root), clamped to a legible pixel range. It shares this module's axis/gridline frame,
//! differing only in the marker shape (a sized circle rather than a fixed square).

use super::common::{
    compute_frame, emit_value_axis, fmt_val, nice_scale, AxisTitles, GRID, LABEL, PALETTE,
};
use rpt_model::{Rect, Twips};
use rpt_pages::{
    DrawOp, EllipseOp, FontSpec, LineOp, ObjectKind, ObjectRef, Point, PolygonOp, Stroke,
    TextAlign, TextRun,
};

/// Build the draw-ops for a scatter chart of `points` (`(x, y)` pairs, one per detail row): the two
/// numeric axes (X from the x values, Y from the y values, each `nice_scale`d), their gridlines and
/// tick labels, and a small filled marker at each point (no connecting line). `axis_titles.value` is
/// the Y-axis title and `axis_titles.category` the X-axis title. Returns an empty vec when there are
/// no points.
///
/// When `sizes` is `Some` (a bubble chart), it carries one third-value per point and each marker is a
/// filled **circle** whose area is proportional to that value (radius ∝ √value, clamped to a legible
/// range) rather than the fixed square drawn for a plain scatter (`None`). A `sizes` slice shorter
/// than `points` falls back to the minimum radius for the missing points.
pub(crate) fn scatter_chart(
    rect: Rect,
    title: &str,
    axis_titles: AxisTitles,
    points: &[(f64, f64)],
    sizes: Option<&[f64]>,
    section_name: &str,
    obj_name: &str,
) -> Vec<DrawOp> {
    if points.is_empty() {
        return Vec::new();
    }
    let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(obj_name));
    let mut ops: Vec<DrawOp> = Vec::new();

    // The Y scale, plot rectangle, axes and gridlines come from the shared value-axis frame, driven
    // by the y values (the slot/category count it also computes is unused — scatter has no category
    // slots). The X numeric scale is layered on below.
    let y_series: Vec<(String, f64)> = points.iter().map(|(_, y)| (String::new(), *y)).collect();
    let f = compute_frame(rect, title, axis_titles, &y_series);
    emit_value_axis(&mut ops, &f, rect, title, axis_titles, &y_series, &src);

    // X numeric scale: 0..nice_max over the x values, mapped across the plot rectangle.
    let x_data_max = points.iter().map(|(x, _)| *x).fold(0.0_f64, f64::max);
    let (x_max, x_step) = nice_scale(x_data_max);
    let plot_w = (f.plot_right() - f.plot_left).max(1) as f64;
    let x_at = |x: f64| f.plot_left + ((x.max(0.0) / x_max) * plot_w) as i32;
    let y_at = |y: f64| f.plot_bottom - ((y.max(0.0) / f.max_val) * f.plot_h as f64) as i32;

    // Vertical gridlines + X tick labels at each division (behind the markers). The 0-line is the
    // Y axis itself, so skip its gridline.
    let x_ticks = (x_max / x_step).round() as i32;
    for t in 0..=x_ticks {
        let xv = t as f64 * x_step;
        let x = x_at(xv);
        if t > 0 {
            ops.push(DrawOp::Line(LineOp {
                from: Point {
                    x: Twips(x),
                    y: Twips(f.plot_top()),
                },
                to: Point {
                    x: Twips(x),
                    y: Twips(f.plot_bottom),
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
                left: Twips(x - 400),
                top: Twips(f.plot_bottom + 30),
                width: Twips(800),
                height: Twips(220),
            },
            text: fmt_val(xv),
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

    // One filled marker per point, cycling the base palette (the engine colours each marker
    // distinctly). A plain scatter draws a fixed square (a closed polygon, so every backend renders
    // the same mark); a bubble chart draws a circle sized by its third value.
    match sizes {
        None => {
            const M: i32 = 65; // marker half-extent (~9px)
            for (idx, (x, y)) in points.iter().enumerate() {
                let cx = x_at(*x);
                let cy = y_at(*y);
                let fill = PALETTE[idx % PALETTE.len()];
                ops.push(DrawOp::Polygon(PolygonOp {
                    points: vec![
                        Point {
                            x: Twips(cx - M),
                            y: Twips(cy - M),
                        },
                        Point {
                            x: Twips(cx + M),
                            y: Twips(cy - M),
                        },
                        Point {
                            x: Twips(cx + M),
                            y: Twips(cy + M),
                        },
                        Point {
                            x: Twips(cx - M),
                            y: Twips(cy + M),
                        },
                    ],
                    closed: true,
                    fill: Some(fill.into()),
                    stroke: None,
                    source: src(),
                }));
            }
        }
        Some(sizes) => {
            // Bubble markers: area ∝ value, so radius ∝ √value, normalized to the largest bubble and
            // clamped to a legible pixel range. A non-positive or absent value collapses to the
            // minimum radius. Later (larger-value) points draw first would occlude small ones, but the
            // engine keeps source order; matching that, points draw in order.
            const R_MIN: f64 = 60.0; // ~4px radius
            const R_MAX: f64 = 320.0; // ~22px radius
            let v_max = sizes.iter().cloned().fold(0.0_f64, f64::max);
            for (idx, (x, y)) in points.iter().enumerate() {
                let cx = x_at(*x);
                let cy = y_at(*y);
                let v = sizes.get(idx).copied().unwrap_or(0.0).max(0.0);
                let r = if v_max > 0.0 {
                    R_MIN + (R_MAX - R_MIN) * (v / v_max).sqrt()
                } else {
                    R_MIN
                };
                let r = r.round() as i32;
                let fill = PALETTE[idx % PALETTE.len()];
                ops.push(DrawOp::Ellipse(EllipseOp {
                    bounds: Rect {
                        left: Twips(cx - r),
                        top: Twips(cy - r),
                        width: Twips(2 * r),
                        height: Twips(2 * r),
                    },
                    fill: Some(fill.into()),
                    stroke: None,
                    source: src(),
                }));
            }
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

    #[test]
    fn empty_points_yield_no_ops() {
        assert!(scatter_chart(rect(), "T", AxisTitles::default(), &[], None, "S", "G").is_empty());
    }

    /// One marker polygon per point, no connecting line, and both numeric axes' tick labels present.
    #[test]
    fn draws_one_marker_per_point_over_two_numeric_axes() {
        let pts = vec![(4.0, 40.0), (8.0, 120.0), (16.0, 200.0), (32.0, 240.0)];
        let titles = AxisTitles {
            value: "Sum of total",
            category: "Sum of id",
        };
        let ops = scatter_chart(rect(), "", titles, &pts, None, "RH", "G");
        // One closed polygon marker per point; no connecting line between markers.
        let markers = ops
            .iter()
            .filter(|o| matches!(o, DrawOp::Polygon(p) if p.closed))
            .count();
        assert_eq!(markers, pts.len(), "one marker per point");
        // Markers ascend in X as the x value grows (continuous X placement, not ordinal slots).
        let xs: Vec<i32> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Polygon(p) if p.closed => Some(p.points[0].x.0),
                _ => None,
            })
            .collect();
        assert!(
            xs.windows(2).all(|w| w[0] <= w[1]),
            "markers ascend by x: {xs:?}"
        );
        // Both axes are labelled: the Y scale (…240) and the X scale (…32) tick labels are emitted.
        let texts: Vec<String> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        assert!(
            texts.contains(&"200".to_string()),
            "Y tick label: {texts:?}"
        );
        assert!(texts.contains(&"30".to_string()), "X tick label: {texts:?}");
    }

    /// A bubble chart draws one filled circle (ellipse) per point over the same two numeric axes —
    /// no square polygon markers — and the circle's radius scales with the square root of its third
    /// value, so a 4× larger value yields a 2× larger radius.
    #[test]
    fn bubble_draws_sized_circles_with_sqrt_area_scaling() {
        let pts = vec![(4.0, 40.0), (8.0, 120.0), (16.0, 200.0)];
        // Sizes 100, 25, 0: the largest bubble (100) hits the max radius, the 25 bubble (√(25/100)=½)
        // sits half-way between min and max, and the 0 bubble collapses to the min radius.
        let sizes = vec![100.0, 25.0, 0.0];
        let ops = scatter_chart(
            rect(),
            "",
            AxisTitles::default(),
            &pts,
            Some(&sizes),
            "RH",
            "G",
        );
        // One ellipse per point; no square polygon markers.
        let radii: Vec<i32> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Ellipse(e) => Some(e.bounds.width.0 / 2),
                _ => None,
            })
            .collect();
        assert_eq!(radii.len(), pts.len(), "one circle per point");
        assert_eq!(
            ops.iter()
                .filter(|o| matches!(o, DrawOp::Polygon(p) if p.closed))
                .count(),
            0,
            "no square markers on a bubble chart"
        );
        // Radii are monotonically non-increasing for sizes 100 > 25 > 0, and the min-radius (v=0)
        // bubble is strictly smaller than the max-radius (v=100) one.
        assert!(
            radii[0] > radii[1] && radii[1] > radii[2],
            "radii: {radii:?}"
        );
        // √ area scaling: the 25 bubble's radius sits ~half-way between min and max (60 and 320 twips
        // → ~190), well below the max (320) and above the min (60).
        assert!(
            (180..=200).contains(&radii[1]),
            "sqrt-scaled mid radius ~190, got {}",
            radii[1]
        );
    }

    /// A `sizes` slice shorter than `points` sizes the missing points at the minimum radius rather
    /// than panicking.
    #[test]
    fn bubble_tolerates_short_sizes_slice() {
        let pts = vec![(1.0, 1.0), (2.0, 2.0)];
        let sizes = vec![10.0];
        let ops = scatter_chart(
            rect(),
            "",
            AxisTitles::default(),
            &pts,
            Some(&sizes),
            "RH",
            "G",
        );
        let circles = ops
            .iter()
            .filter(|o| matches!(o, DrawOp::Ellipse(_)))
            .count();
        assert_eq!(
            circles,
            pts.len(),
            "one circle per point despite short sizes"
        );
    }
}
