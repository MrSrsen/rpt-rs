//! Area chart: a line chart whose region between the polyline and the baseline is filled,
//! over the shared Num+Ord axis frame. Uses the [`DrawOp::Polygon`] primitive for the fill,
//! then draws the connecting line and value/category labels on top. Unlike the line chart it draws
//! no per-point markers — the engine's area is a clean ribbon — and its category axis
//! is thinned like the other axis families so a dense series' labels don't overlap.

use super::common::{
    category_label, category_stride, chart_frame, fmt_val, value_label, AxisTitles, LABEL, PALETTE,
};
use rpt_model::{Color, Rect, Twips};
use rpt_pages::{DrawOp, LineOp, ObjectKind, ObjectRef, Point, PolygonOp, Stroke};

/// Build the draw-ops for an area chart of `series` (category label → value): the shared axis frame,
/// a translucent fill polygon from the baseline up to the per-category points, then the connecting
/// polyline and value/category labels. `show_labels` gates the per-point data-value labels (the
/// report's decoded "show value" flag). Returns an empty vec if `series` is empty.
pub(crate) fn area_chart(
    rect: Rect,
    title: &str,
    axis_titles: AxisTitles,
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
    let f = chart_frame(&mut ops, rect, title, axis_titles, series, &src);

    // Point per category, centered in its slot; y scales from the baseline like a bar's top.
    let point = |i: i32, val: f64| Point {
        x: Twips(f.plot_left + i * f.slot + f.slot / 2),
        y: Twips(f.plot_bottom - ((val.max(0.0) / f.max_val) * f.plot_h as f64) as i32),
    };

    // Fill polygon: baseline under the first point → each point → baseline under the last point.
    let mut fill = Vec::with_capacity(series.len() + 2);
    fill.push(Point {
        x: point(0, series[0].1).x,
        y: Twips(f.plot_bottom),
    });
    for (i, (_, val)) in series.iter().enumerate() {
        fill.push(point(i as i32, *val));
    }
    fill.push(Point {
        x: point(series.len() as i32 - 1, series[series.len() - 1].1).x,
        y: Twips(f.plot_bottom),
    });
    // A light tint of the series colour for the fill. Pre-blended toward white (opaque) rather than
    // alpha-based: the Page-IR backends carry no fill-opacity, so a real translucent colour would
    // render opaque and inconsistently across SVG/PDF/raster — this looks translucent everywhere.
    let base = PALETTE[0];
    let blend = |c: u8| (c as f64 * 0.25 + 255.0 * 0.75) as u8;
    let tint = Color {
        a: 255,
        r: blend(base.r),
        g: blend(base.g),
        b: blend(base.b),
    };
    ops.push(DrawOp::Polygon(PolygonOp {
        points: fill,
        closed: true,
        fill: Some(tint.into()),
        stroke: None,
        source: src(),
    }));

    // Connecting polyline (opaque) on top of the fill.
    let line_stroke = Stroke {
        color: base,
        width: Twips(30),
        style: rpt_pages::LineStyle::Single,
    };
    for i in 1..series.len() as i32 {
        ops.push(DrawOp::Line(LineOp {
            from: point(i - 1, series[(i - 1) as usize].1),
            to: point(i, series[i as usize].1),
            stroke: line_stroke,
            source: src(),
        }));
    }

    // Value label + (thinned) category label at each point — no per-point markers, unlike the line
    // chart: the engine's area is a clean ribbon.
    let stride = category_stride(&f, series.len());
    for (i, (label, val)) in series.iter().enumerate() {
        let i = i as i32;
        let p = point(i, *val);
        // Value label above each point, gated on "show value".
        if show_labels {
            ops.push(value_label(
                p.x.0,
                (p.y.0 - 90 - 230).max(f.plot_top()),
                &fmt_val(*val),
                LABEL,
                &src,
            ));
        }
        if i as usize % stride == 0 {
            ops.push(category_label(&f, i, label, &src));
        }
    }

    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_series_yields_no_ops() {
        let r = Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(3000),
            height: Twips(2000),
        };
        assert!(area_chart(r, "T", AxisTitles::default(), &[], true, "S", "Graph1").is_empty());
    }

    #[test]
    fn fills_under_the_line_and_labels_points() {
        let r = Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(6000),
            height: Twips(4000),
        };
        let series = vec![
            ("Jan".into(), 12.0),
            ("Feb".into(), 27.0),
            ("Mar".into(), 6.0),
        ];
        let ops = area_chart(
            r,
            "Trend",
            AxisTitles::default(),
            &series,
            true,
            "RH",
            "Graph1",
        );
        // Exactly one fill polygon, closed, spanning point count + 2 baseline anchors.
        let polys: Vec<&PolygonOp> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Polygon(p) => Some(p),
                _ => None,
            })
            .collect();
        assert_eq!(polys.len(), 1, "one area fill polygon");
        assert!(polys[0].closed);
        assert_eq!(polys[0].points.len(), series.len() + 2);
        // Each point is annotated with its value.
        let texts: Vec<String> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        for v in ["12", "27", "6"] {
            assert!(
                texts.contains(&v.to_string()),
                "value label {v} in {texts:?}"
            );
        }
    }

    /// The area ribbon draws no per-point marker rects (the engine's area is a clean fill), and a
    /// dense category axis is thinned so only a subset of the labels is emitted.
    #[test]
    fn no_markers_and_thins_dense_category_labels() {
        let r = Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(6000),
            height: Twips(4000),
        };
        // 40 categories in a 6000-twip-wide rect → slots far narrower than a readable label.
        let series: Vec<(String, f64)> =
            (0..40).map(|i| (format!("p{i}"), i as f64 + 1.0)).collect();
        let ops = area_chart(
            r,
            "Trend",
            AxisTitles::default(),
            &series,
            false,
            "RH",
            "Graph1",
        );
        // No marker rects at all (the area is fill + line only).
        let rects = ops.iter().filter(|o| matches!(o, DrawOp::Rect(_))).count();
        assert_eq!(rects, 0, "area draws no per-point marker rects");
        // Category labels are thinned: far fewer than the 40 categories.
        let cat_labels = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) if t.text.starts_with('p') => Some(()),
                _ => None,
            })
            .count();
        assert!(
            cat_labels > 0 && cat_labels < series.len() / 2,
            "dense category axis thinned: {cat_labels} of {} labels",
            series.len()
        );
    }
}
