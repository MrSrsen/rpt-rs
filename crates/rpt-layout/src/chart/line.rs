//! Line chart: a connecting polyline through the per-category points with a marker at each,
//! over the shared Num+Ord axis frame.

use super::common::{
    category_label, category_stride, chart_frame, fmt_val, value_label, AxisTitles, LABEL, PALETTE,
};
use rpt_model::{Rect, Twips};
use rpt_pages::{DrawOp, LineOp, ObjectKind, ObjectRef, Point, RectOp, Stroke};

/// Build the draw-ops for a line chart of `series` (category label → value): the shared axis frame
/// plus a connecting polyline through the per-category points with a marker at each. Drawn with
/// the existing `Line`/`Rect` ops — no new Page-IR primitive needed. `show_labels` gates the
/// per-point data-value labels (the report's decoded "show value" flag). Returns an empty vec if
/// `series` is empty.
pub(crate) fn line_chart(
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
    let line_stroke = Stroke {
        color: PALETTE[0],
        width: Twips(30),
        style: rpt_pages::LineStyle::Single,
    };
    // Connecting polyline: one segment between consecutive points.
    for i in 1..series.len() as i32 {
        ops.push(DrawOp::Line(LineOp {
            from: point(i - 1, series[(i - 1) as usize].1),
            to: point(i, series[i as usize].1),
            stroke: line_stroke,
            source: src(),
        }));
    }
    // Marker (small filled square) + (thinned) category label at each point.
    const M: i32 = 90; // marker half-extent (~0.06")
    let stride = category_stride(&f, series.len());
    for (i, (label, val)) in series.iter().enumerate() {
        let i = i as i32;
        let p = point(i, *val);
        ops.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(p.x.0 - M),
                top: Twips(p.y.0 - M),
                width: Twips(M * 2),
                height: Twips(M * 2),
            },
            fill: Some(PALETTE[0].into()),
            stroke: None,
            corner_radius: Twips(0),
            source: src(),
        }));
        // Value label above each marker, gated on "show value".
        if show_labels {
            ops.push(value_label(
                p.x.0,
                (p.y.0 - M - 230).max(f.plot_top()),
                &fmt_val(*val),
                LABEL,
                &src,
            ));
        }
        if (i as usize).is_multiple_of(stride) {
            ops.push(category_label(&f, i, label, &src));
        }
    }

    ops
}
