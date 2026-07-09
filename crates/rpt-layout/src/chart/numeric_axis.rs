//! Numeric-axis chart: a bar chart whose X axis is numeric/continuous rather than an ordinal
//! category slot. When the category labels parse as numbers, each bar is placed at its value's
//! scaled X position (a 0..nice_max numeric X scale); otherwise it falls back to the ordinal
//! [`super::bar::bar_chart`]. Only the bar variant is implemented; line/area/date sub-variants
//! are not.

use super::bar::bar_chart;
use super::common::{
    category_stride, compute_frame, emit_value_axis, fmt_val, nice_scale, value_label, AxisTitles,
    LABEL, PALETTE,
};
use rpt_model::{Rect, Twips};
use rpt_pages::{DrawOp, FontSpec, ObjectKind, ObjectRef, RectOp, TextAlign, TextRun};

/// Build the draw-ops for a numeric-axis chart of `series` (category label → value). If every
/// category label parses as a number the bars are placed along a continuous 0..nice_max X scale;
/// otherwise the ordinal [`bar_chart`] is used unchanged. `show_labels` gates the per-bar value
/// labels. Returns an empty vec if `series` is empty.
pub(crate) fn numeric_axis_chart(
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
    // Numeric only when every category key parses as a number; else ordinal fallback.
    let keys: Option<Vec<f64>> = series
        .iter()
        .map(|(k, _)| k.trim().parse::<f64>().ok())
        .collect();
    let keys = match keys {
        Some(k) => k,
        None => {
            return bar_chart(
                rect,
                title,
                axis_titles,
                series,
                show_labels,
                section_name,
                obj_name,
            )
        }
    };

    let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(obj_name));
    let mut ops: Vec<DrawOp> = Vec::new();
    // Reuse the shared value-axis frame for the Y scale, title, axes, and gridlines.
    let f = compute_frame(rect, title, axis_titles, series);
    emit_value_axis(&mut ops, &f, rect, title, axis_titles, series, &src);

    // Numeric X scale: 0..nice_max over the parsed keys, mapped into the plot rectangle.
    let key_max = keys.iter().copied().fold(0.0_f64, f64::max);
    let (x_max, _) = nice_scale(key_max);
    let plot_w = (f.plot_right() - f.plot_left).max(1) as f64;
    let x_at = |k: f64| f.plot_left + ((k.max(0.0) / x_max) * plot_w) as i32;

    let bar_w = (f.slot / 2).max(15);
    let stride = category_stride(&f, series.len());
    for (idx, ((label, val), key)) in series.iter().zip(&keys).enumerate() {
        let h = ((val.max(0.0) / f.max_val) * f.plot_h as f64) as i32;
        let cx = x_at(*key);
        let bx = (cx - bar_w / 2).max(f.plot_left);
        let by = f.plot_bottom - h;
        ops.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(bx),
                top: Twips(by),
                width: Twips(bar_w),
                height: Twips(h.max(1)),
            },
            fill: Some(PALETTE[0].into()),
            stroke: None,
            corner_radius: Twips(0),
            source: src(),
        }));
        if show_labels {
            ops.push(value_label(
                cx,
                (by - 230).max(f.plot_top()),
                &fmt_val(*val),
                LABEL,
                &src,
            ));
        }
        // Numeric axis label centred under the bar's X position (thinned when dense).
        if idx % stride != 0 {
            continue;
        }
        ops.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(cx - f.slot / 2),
                top: Twips(f.plot_bottom + 30),
                width: Twips(f.slot),
                height: Twips(f.cat_h),
            },
            text: label.clone(),
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

    fn bar_lefts(ops: &[DrawOp]) -> Vec<i32> {
        ops.iter()
            .filter_map(|o| match o {
                DrawOp::Rect(r) => Some(r.bounds.left.0),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn empty_series_yields_no_ops() {
        assert!(
            numeric_axis_chart(rect(), "T", AxisTitles::default(), &[], true, "S", "G").is_empty()
        );
    }

    /// Numeric keys → one bar per value, placed at increasing X positions with the gap proportional to
    /// the key spacing (the key-40 bar is roughly twice as far from the axis as the key-20 bar).
    #[test]
    fn numeric_keys_place_bars_at_scaled_x() {
        let s = vec![("10".into(), 12.0), ("20".into(), 27.0), ("40".into(), 6.0)];
        let ops = numeric_axis_chart(rect(), "N", AxisTitles::default(), &s, false, "RH", "G");
        let lefts = bar_lefts(&ops);
        assert_eq!(lefts.len(), 3, "one bar per value");
        assert!(
            lefts[0] < lefts[1] && lefts[1] < lefts[2],
            "bars ascend by key"
        );
        // Offsets from the key-10 bar: key 40 is further out than key 20 (continuous X spacing).
        let d20 = lefts[1] - lefts[0];
        let d40 = lefts[2] - lefts[0];
        assert!(d40 > d20, "key 40 is further out than key 20");
    }

    /// Non-numeric keys → ordinal fallback: bars are still produced (via the ordinal bar chart).
    #[test]
    fn non_numeric_keys_fall_back_to_ordinal() {
        let s = vec![("Alpha".into(), 12.0), ("Beta".into(), 27.0)];
        let ops = numeric_axis_chart(rect(), "N", AxisTitles::default(), &s, false, "RH", "G");
        assert_eq!(
            bar_lefts(&ops).len(),
            2,
            "ordinal fallback still draws bars"
        );
    }

    /// With "show value" off, bars still draw but no per-bar value label is emitted.
    #[test]
    fn show_labels_false_omits_value_labels() {
        let s = vec![("10".into(), 12.0), ("20".into(), 27.0), ("40".into(), 6.0)];
        let ops = numeric_axis_chart(rect(), "N", AxisTitles::default(), &s, false, "RH", "G");
        let texts: Vec<String> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(bar_lefts(&ops).len(), 3, "bars present");
        for v in ["12", "27", "6"] {
            assert!(
                !texts.contains(&v.to_string()),
                "value label {v} must be omitted: {texts:?}"
            );
        }
    }
}
