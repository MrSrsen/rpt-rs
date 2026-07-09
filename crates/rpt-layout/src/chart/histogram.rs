//! Histogram chart (graph type code 15): the frequency distribution of one value field, binned into
//! equal-width ranges over the data's span, one bar per bin whose height is the count of values that
//! fall in it. The value axis is the frequency count; the category axis is the bin
//! boundaries.

use super::common::{
    category_stride, chart_frame, fmt_val, value_label, AxisTitles, LABEL, PALETTE,
};
use rpt_model::{Rect, Twips};
use rpt_pages::{DrawOp, FontSpec, ObjectKind, ObjectRef, RectOp, TextAlign, TextRun};

/// Build the draw-ops for a histogram of `values` binned into `bins` equal-width ranges: the shared
/// value-axis frame (the value axis being the bin frequency), one contiguous bar per bin, and the
/// bin-boundary labels along the category axis. `show_labels` gates the per-bar frequency labels.
/// Returns an empty vec when there are no values or fewer than one bin.
#[allow(clippy::too_many_arguments)]
pub(crate) fn histogram_chart(
    rect: Rect,
    title: &str,
    axis_titles: AxisTitles,
    values: &[f64],
    bins: usize,
    show_labels: bool,
    section_name: &str,
    obj_name: &str,
) -> Vec<DrawOp> {
    if values.is_empty() || bins == 0 {
        return Vec::new();
    }
    let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(obj_name));

    // Bin the values into `bins` equal-width ranges over [min, max]. A degenerate span (all equal)
    // collapses to a single populated bin.
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let width = if max > min {
        (max - min) / bins as f64
    } else {
        1.0
    };
    let mut counts = vec![0u32; bins];
    for &v in values {
        let idx = if max > min {
            (((v - min) / width) as usize).min(bins - 1)
        } else {
            0
        };
        counts[idx] += 1;
    }

    // The value axis is the frequency; label each slot by its bin's lower boundary.
    let series: Vec<(String, f64)> = counts
        .iter()
        .enumerate()
        .map(|(i, c)| (fmt_val(min + i as f64 * width), *c as f64))
        .collect();
    let mut ops: Vec<DrawOp> = Vec::new();
    let f = chart_frame(&mut ops, rect, title, axis_titles, &series, &src);

    // Contiguous bars (histogram bars touch), one per bin, cycling the base palette.
    let bar_w = (f.slot * 9 / 10).max(15);
    let stride = category_stride(&f, bins + 1);
    for (i, count) in counts.iter().enumerate() {
        let h = ((*count as f64 / f.max_val) * f.plot_h as f64) as i32;
        let bx = f.plot_left + i as i32 * f.slot + (f.slot - bar_w) / 2;
        let by = f.plot_bottom - h;
        ops.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(bx),
                top: Twips(by),
                width: Twips(bar_w),
                height: Twips(h.max(1)),
            },
            fill: Some(PALETTE[i % PALETTE.len()].into()),
            stroke: None,
            corner_radius: Twips(0),
            source: src(),
        }));
        if show_labels && *count > 0 {
            ops.push(value_label(
                bx + bar_w / 2,
                (by - 230).max(f.plot_top()),
                &fmt_val(*count as f64),
                LABEL,
                &src,
            ));
        }
    }

    // Bin-boundary labels at each slot's left edge, plus the final upper boundary (thinned when the
    // bin count is dense).
    let boundary = |i: usize| -> DrawOp {
        DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(f.plot_left + i as i32 * f.slot - f.slot / 2),
                top: Twips(f.plot_bottom + 30),
                width: Twips(f.slot),
                height: Twips(f.cat_h),
            },
            text: fmt_val(min + i as f64 * width),
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
        })
    };
    for i in 0..=bins {
        if i % stride == 0 {
            ops.push(boundary(i));
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
    fn empty_values_yield_no_ops() {
        assert!(
            histogram_chart(rect(), "T", AxisTitles::default(), &[], 7, false, "S", "G").is_empty()
        );
    }

    /// Seven bins over a 0..70 span yield seven bars, and a value lands in the bin whose range covers
    /// it (bin 0 gets the two low values, bin 6 the high one).
    #[test]
    fn bins_values_into_seven_bars() {
        let values = vec![1.0, 5.0, 35.0, 65.0, 69.0];
        let ops = histogram_chart(
            rect(),
            "",
            AxisTitles::default(),
            &values,
            7,
            false,
            "RH",
            "G",
        );
        let bars: Vec<&RectOp> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Rect(r) => Some(r),
                _ => None,
            })
            .collect();
        assert_eq!(bars.len(), 7, "one bar per bin");
        // Bars are contiguous left-to-right (a distribution, not spaced categories).
        let lefts: Vec<i32> = bars.iter().map(|b| b.bounds.left.0).collect();
        assert!(
            lefts.windows(2).all(|w| w[0] < w[1]),
            "bars ascend: {lefts:?}"
        );
        // The first bin (0..10) holds the two low values → tallest-or-equal; an empty bin is height 1.
        assert!(bars[0].bounds.height.0 > 1, "populated first bin");
    }

    /// With "show value" on, a per-bar frequency label is drawn for a populated bin.
    #[test]
    fn show_labels_true_draws_frequency() {
        let values = vec![1.0, 2.0, 3.0];
        let ops = histogram_chart(
            rect(),
            "",
            AxisTitles::default(),
            &values,
            3,
            true,
            "RH",
            "G",
        );
        let texts: Vec<String> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        // Three values spread across three bins → each bin count is 1; the frequency label "1" shows.
        assert!(
            texts.contains(&"1".to_string()),
            "frequency label present: {texts:?}"
        );
    }
}
