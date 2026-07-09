//! Bar chart: one riser per category, from the baseline up to the group's summary value,
//! over the shared Num+Ord axis frame.

use super::common::{
    category_label, category_stride, chart_frame, fmt_val, value_label, AxisTitles, LABEL, PALETTE,
};
use rpt_model::{ChartArrangement, Rect, Twips};
use rpt_pages::{DrawOp, ObjectKind, ObjectRef, RectOp};

/// Build the draw-ops for a bar chart of `series` (category label → value) inside `rect` (twips).
/// `title` is drawn centered at the top when non-empty. `show_labels` gates the per-bar data-value
/// labels (the report's decoded "show value" flag). Returns an empty vec if `series` is empty
/// (the caller then keeps the placeholder + diagnostic).
pub(crate) fn bar_chart(
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

    // Bars: one riser per category, from the baseline up to the value.
    let bar_w = (f.slot * 3 / 5).max(15);
    let stride = category_stride(&f, series.len());
    for (i, (label, val)) in series.iter().enumerate() {
        let i = i as i32;
        let h = ((val.max(0.0) / f.max_val) * f.plot_h as f64) as i32;
        let bx = f.plot_left + i * f.slot + (f.slot - bar_w) / 2;
        let by = f.plot_bottom - h;
        ops.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(bx),
                top: Twips(by),
                width: Twips(bar_w),
                height: Twips(h.max(1)),
            },
            fill: Some(PALETTE[i as usize % PALETTE.len()].into()),
            stroke: None,
            corner_radius: Twips(0),
            source: src(),
        }));
        // Value label just above the bar top, gated on "show value".
        if show_labels {
            ops.push(value_label(
                bx + bar_w / 2,
                (by - 230).max(f.plot_top()),
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

/// Build the draw-ops for a multi-series bar chart: `categories` are the X-axis category labels,
/// `series_names` name each data series (for the caller's legend), and `values[cat][series]` is the
/// value of each series in each category. `arrangement` selects how the series share a category slot:
///
/// - [`Clustered`](ChartArrangement::Clustered): the series are drawn side-by-side within the slot.
/// - [`Stacked`](ChartArrangement::Stacked): the series are accumulated bottom-to-top; the value axis
///   spans `0..max(Σ series)`.
/// - [`Percent`](ChartArrangement::Percent): stacked then normalized so every category fills 0..100%.
///
/// Each series is coloured by its index in the shared palette (so the legend the caller composes from
/// `series_names` matches). `show_labels` gates the per-riser data-value labels. Returns an empty vec
/// if there are no categories or no series.
#[allow(clippy::too_many_arguments)]
pub(crate) fn bar_chart_multi(
    rect: Rect,
    title: &str,
    axis_titles: AxisTitles,
    categories: &[String],
    series_names: &[String],
    values: &[Vec<f64>],
    arrangement: ChartArrangement,
    show_labels: bool,
    section_name: &str,
    obj_name: &str,
) -> Vec<DrawOp> {
    let n_series = series_names.len();
    if categories.is_empty() || n_series == 0 {
        return Vec::new();
    }
    let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(obj_name));
    let mut ops: Vec<DrawOp> = Vec::new();

    // The value-axis scale depends on the arrangement: the tallest single riser (clustered), the
    // tallest stacked total (stacked), or a fixed 0..100 (percent). Build one representative value per
    // category so the shared frame reserves `categories.len()` slots and scales to the right maximum.
    let frame_series: Vec<(String, f64)> = categories
        .iter()
        .enumerate()
        .map(|(ci, c)| {
            let v = match arrangement {
                ChartArrangement::Clustered => {
                    values[ci].iter().copied().fold(0.0_f64, |m, v| m.max(v))
                }
                ChartArrangement::Stacked => values[ci].iter().map(|v| v.max(0.0)).sum(),
                ChartArrangement::Percent => 100.0,
            };
            (c.clone(), v)
        })
        .collect();
    let f = chart_frame(&mut ops, rect, title, axis_titles, &frame_series, &src);

    let n = n_series as i32;
    for (ci, cat) in categories.iter().enumerate() {
        let i = ci as i32;
        let slot_left = f.plot_left + i * f.slot;
        match arrangement {
            ChartArrangement::Clustered => {
                // Split the slot into `n_series` side-by-side sub-bars, centred in the slot.
                let group_w = (f.slot * 3 / 5).max(n * 8);
                let sub_w = (group_w / n).max(8);
                let group_left = slot_left + (f.slot - sub_w * n) / 2;
                for (s, val) in values[ci].iter().enumerate() {
                    let h = ((val.max(0.0) / f.max_val) * f.plot_h as f64) as i32;
                    let bx = group_left + s as i32 * sub_w;
                    let by = f.plot_bottom - h;
                    ops.push(riser(bx, by, sub_w, h, s, &src));
                    if show_labels {
                        ops.push(value_label(
                            bx + sub_w / 2,
                            (by - 230).max(f.plot_top()),
                            &fmt_val(*val),
                            LABEL,
                            &src,
                        ));
                    }
                }
            }
            ChartArrangement::Stacked | ChartArrangement::Percent => {
                // One riser accumulated bottom-to-top; percent normalizes each category to its own sum.
                let bar_w = (f.slot * 3 / 5).max(15);
                let bx = slot_left + (f.slot - bar_w) / 2;
                let total: f64 = values[ci].iter().map(|v| v.max(0.0)).sum();
                let mut acc_h = 0;
                for (s, val) in values[ci].iter().enumerate() {
                    let scaled = match arrangement {
                        ChartArrangement::Percent if total > 0.0 => val.max(0.0) / total * 100.0,
                        ChartArrangement::Percent => 0.0,
                        _ => val.max(0.0),
                    };
                    let h = ((scaled / f.max_val) * f.plot_h as f64) as i32;
                    let by = f.plot_bottom - acc_h - h;
                    ops.push(riser(bx, by, bar_w, h, s, &src));
                    if show_labels && h > 200 {
                        ops.push(value_label(
                            bx + bar_w / 2,
                            by + (h - 200) / 2,
                            &fmt_val(*val),
                            LABEL,
                            &src,
                        ));
                    }
                    acc_h += h;
                }
            }
        }
        ops.push(category_label(&f, i, cat, &src));
    }

    ops
}

/// A single riser rectangle at `(x, y)` of width `w` and height `h` (clamped ≥ 1), filled with the
/// palette colour for series index `s`.
fn riser(x: i32, y: i32, w: i32, h: i32, s: usize, src: &dyn Fn() -> Option<ObjectRef>) -> DrawOp {
    DrawOp::Rect(RectOp {
        bounds: Rect {
            left: Twips(x),
            top: Twips(y),
            width: Twips(w),
            height: Twips(h.max(1)),
        },
        fill: Some(PALETTE[s % PALETTE.len()].into()),
        stroke: None,
        corner_radius: Twips(0),
        source: src(),
    })
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
        assert!(bar_chart(r, "T", AxisTitles::default(), &[], true, "S", "Graph1").is_empty());
    }

    #[test]
    fn draws_bars_axes_and_labels() {
        let r = Rect {
            left: Twips(100),
            top: Twips(100),
            width: Twips(6000),
            height: Twips(4000),
        };
        // Values chosen off the nice-number tick boundaries (0/10/20/30) so the value-label
        // assertion below can't be satisfied by an axis tick label of the same text.
        let series = vec![
            ("Canada".into(), 12.0),
            ("USA".into(), 27.0),
            ("Mexico".into(), 6.0),
        ];
        let ops = bar_chart(
            r,
            "Cities",
            AxisTitles::default(),
            &series,
            true,
            "RH",
            "Graph1",
        );
        let bars = ops.iter().filter(|o| matches!(o, DrawOp::Rect(_))).count();
        let lines = ops.iter().filter(|o| matches!(o, DrawOp::Line(_))).count();
        let text_of = |o: &DrawOp| match o {
            DrawOp::Text(t) => Some(t.text.clone()),
            _ => None,
        };
        let texts: Vec<String> = ops.iter().filter_map(text_of).collect();
        assert_eq!(bars, 3, "one rect per bar");
        // 2 axes + one gridline per value-axis tick (nice-number scale).
        assert!(lines > 2, "axes + gridlines, got {lines}");
        // title + a tick label per division + 3 category labels + 3 data-value labels.
        assert!(
            texts.len() >= 3 + 2,
            "title + tick + categories, got {texts:?}"
        );
        // Each bar is annotated with its value.
        for v in ["12", "27", "6"] {
            assert!(
                texts.contains(&v.to_string()),
                "value label {v} in {texts:?}"
            );
        }
        // The tallest bar (USA=27) reaches nearest the top (smallest `top`).
        let tops: Vec<i32> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Rect(rr) => Some(rr.bounds.top.0),
                _ => None,
            })
            .collect();
        assert!(
            tops[1] < tops[0] && tops[1] < tops[2],
            "USA bar is tallest: {tops:?}"
        );
    }

    /// With "show value" off, the bars, axes, and category labels still draw, but no per-bar
    /// data-value label is emitted (the value texts are gone; the category texts remain).
    #[test]
    fn show_labels_false_omits_value_labels() {
        let r = Rect {
            left: Twips(100),
            top: Twips(100),
            width: Twips(6000),
            height: Twips(4000),
        };
        let series = vec![
            ("Canada".into(), 12.0),
            ("USA".into(), 27.0),
            ("Mexico".into(), 6.0),
        ];
        let ops = bar_chart(
            r,
            "Cities",
            AxisTitles::default(),
            &series,
            false,
            "RH",
            "Graph1",
        );
        let bars = ops.iter().filter(|o| matches!(o, DrawOp::Rect(_))).count();
        let lines = ops.iter().filter(|o| matches!(o, DrawOp::Line(_))).count();
        let texts: Vec<String> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(bars, 3, "bars still drawn without labels");
        assert!(lines > 2, "axes + gridlines still drawn, got {lines}");
        // Category labels remain; the off-tick data values (12/27/6) are gone.
        for c in ["Canada", "USA", "Mexico"] {
            assert!(texts.contains(&c.to_string()), "category {c} in {texts:?}");
        }
        for v in ["12", "27", "6"] {
            assert!(
                !texts.contains(&v.to_string()),
                "value label {v} must be omitted: {texts:?}"
            );
        }
    }

    fn multi_rect() -> Rect {
        Rect {
            left: Twips(100),
            top: Twips(100),
            width: Twips(8000),
            height: Twips(5000),
        }
    }

    /// The riser rectangles in emit order (the frame emits axes/gridlines as lines and labels as text,
    /// so every `Rect` op is a riser). Order is category-major, series-minor.
    fn risers(ops: &[DrawOp]) -> Vec<(i32, i32, i32, i32)> {
        ops.iter()
            .filter_map(|o| match o {
                DrawOp::Rect(r) => Some((
                    r.bounds.left.0,
                    r.bounds.top.0,
                    r.bounds.width.0,
                    r.bounds.height.0,
                )),
                _ => None,
            })
            .collect()
    }

    /// Clustered: `n_series · n_categories` risers, drawn side-by-side within each category slot
    /// (non-overlapping, left-to-right by series).
    #[test]
    fn clustered_places_series_side_by_side() {
        let categories = vec!["Q1".to_string(), "Q2".to_string()];
        let names = vec!["A".to_string(), "B".to_string()];
        let values = vec![vec![10.0, 20.0], vec![30.0, 5.0]];
        let ops = bar_chart_multi(
            multi_rect(),
            "T",
            AxisTitles::default(),
            &categories,
            &names,
            &values,
            ChartArrangement::Clustered,
            false,
            "RH",
            "G",
        );
        let rs = risers(&ops);
        assert_eq!(rs.len(), 4, "n_series·n_cat risers");
        // Category 0 = rs[0..2]: the two sub-bars sit side-by-side without overlapping.
        let (l0, _, w0, _) = rs[0];
        let (l1, ..) = rs[1];
        assert!(l0 < l1, "series B is right of series A within the slot");
        assert!(l0 + w0 <= l1, "clustered sub-bars do not overlap");
        // The taller value (B=20) is the taller riser within category 0.
        assert!(rs[1].3 > rs[0].3, "B (20) taller than A (10)");
    }

    /// Stacked: within a category the risers accumulate bottom-to-top — each series' bottom edge sits
    /// on the previous series' top edge.
    #[test]
    fn stacked_accumulates_bottom_to_top() {
        let categories = vec!["Q1".to_string(), "Q2".to_string()];
        let names = vec!["A".to_string(), "B".to_string()];
        let values = vec![vec![10.0, 20.0], vec![30.0, 5.0]];
        let ops = bar_chart_multi(
            multi_rect(),
            "T",
            AxisTitles::default(),
            &categories,
            &names,
            &values,
            ChartArrangement::Stacked,
            false,
            "RH",
            "G",
        );
        let rs = risers(&ops);
        assert_eq!(rs.len(), 4, "n_series·n_cat risers");
        // Per category, series 1 stacks on series 0: bottom(series1) == top(series0).
        for cat in 0..2 {
            let (_, t0, _, _) = rs[cat * 2];
            let (bx0, ..) = rs[cat * 2];
            let (bx1, t1, _, h1) = rs[cat * 2 + 1];
            assert_eq!(bx0, bx1, "stacked series share the same x");
            assert_eq!(t1 + h1, t0, "series 1 sits on top of series 0");
        }
    }

    /// Percent: every category is normalized to 100%, so two categories with different totals produce
    /// stacks of the same total height.
    #[test]
    fn percent_normalizes_each_category_to_full_height() {
        let categories = vec!["Q1".to_string(), "Q2".to_string()];
        let names = vec!["A".to_string(), "B".to_string()];
        // Totals differ (20 vs 120) but both must fill the plot to 100%.
        let values = vec![vec![10.0, 10.0], vec![30.0, 90.0]];
        let ops = bar_chart_multi(
            multi_rect(),
            "T",
            AxisTitles::default(),
            &categories,
            &names,
            &values,
            ChartArrangement::Percent,
            false,
            "RH",
            "G",
        );
        let rs = risers(&ops);
        assert_eq!(rs.len(), 4, "n_series·n_cat risers");
        let stack_h = |cat: usize| rs[cat * 2].3 + rs[cat * 2 + 1].3;
        // Both categories fill the same total height despite differing raw totals (±rounding).
        assert!(
            (stack_h(0) - stack_h(1)).abs() <= 2,
            "percent stacks fill equally: {} vs {}",
            stack_h(0),
            stack_h(1)
        );
        // Category 0 (10/10) splits evenly; category 1 (30/90) is 1:3.
        assert!(
            (rs[0].3 - rs[1].3).abs() <= 2,
            "even split for equal values"
        );
        assert!(rs[3].3 > rs[2].3 * 2, "90 riser dominates the 30 riser");
    }

    /// Multi-series with an empty category or series list yields no ops.
    #[test]
    fn multi_empty_yields_no_ops() {
        assert!(bar_chart_multi(
            multi_rect(),
            "T",
            AxisTitles::default(),
            &[],
            &["A".to_string()],
            &[],
            ChartArrangement::Clustered,
            false,
            "RH",
            "G",
        )
        .is_empty());
        assert!(bar_chart_multi(
            multi_rect(),
            "T",
            AxisTitles::default(),
            &["Q1".to_string()],
            &[],
            &[vec![]],
            ChartArrangement::Clustered,
            false,
            "RH",
            "G",
        )
        .is_empty());
    }
}
