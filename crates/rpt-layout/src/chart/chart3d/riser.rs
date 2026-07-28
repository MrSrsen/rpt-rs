//! 3-D riser ("3-D bar") chart: categories run along X (the shared Num+Ord frame slots) and data
//! series recede along Z, so a clustered multi-series chart is a grid of extruded boxes. Each box is
//! three shaded faces (front, top, side) projected with the native perspective transform
//! ([`super::projection`]). Background walls/floor are drawn first; all data faces
//! are then globally painter-sorted back-to-front so nearer boxes overlap farther ones.

use super::projection::{face, p3, shade, ViewAngle, FRONT_SHADE, SIDE_SHADE};
use super::scene::{compose, setup_3d};
#[cfg(test)]
use crate::chart::common::AxisTitles;
use crate::chart::common::{fmt_val, value_frac, value_label, ChartCtx, LABEL, PALETTE};
#[cfg(test)]
use rpt_model::Rect;
use rpt_pages::DrawOp;

/// Upper bound on the fraction of a series' floor cell a riser's footprint may fill along the depth
/// axis, so clustered series never touch. The footprint is normally sized to be world-square (see
/// [`riser_3d`]); this only caps it when many series would otherwise crowd their cells.
const MAX_CELL_FILL: f64 = 0.85;

/// Build the draw-ops for a 3-D riser chart. `categories` are the X-axis slots; `series` is one row
/// per data binding (`name`, value-per-category), receding along Z. A single-series chart is just
/// `series.len() == 1`. Colours are per-category for a single series and per-series for several. Draws
/// the three background planes first, then every box's three shaded faces globally painter-sorted
/// back-to-front, then the labels. Returns an empty vec if there is nothing to plot. `show_labels`
/// gates the per-box data-value label.
pub(crate) fn riser_3d(
    cx: &ChartCtx,
    categories: &[String],
    series: &[(String, Vec<f64>)],
    view_angle: rpt_model::ChartViewAngle,
) -> Vec<DrawOp> {
    let (s_count, c_count) = (series.len(), categories.len());
    if s_count == 0 || c_count == 0 {
        return Vec::new();
    }
    let &ChartCtx {
        rect,
        title,
        show_labels,
        ..
    } = cx;
    let src = || cx.src();

    // Series-depth floor-grid divisions: one band per series (clustered), empty for a single series.
    let z_div: Vec<f64> = if s_count > 1 {
        (1..s_count).map(|s| s as f64 / s_count as f64).collect()
    } else {
        Vec::new()
    };
    // Frame, projection, and background scenery from the shared 3-D scene setup.
    let (f, max_val, proj, background, axis_labels) =
        setup_3d(rect, title, categories, series, view_angle, &z_div, &src);

    let bar_w = (f.slot * 3 / 5).max(15);
    let plot_left = f.plot_left;
    let plot_right = f.plot_right;
    let plot_top = f.plot_top();
    let plot_bottom = f.plot_bottom;
    let (pl, pr, _pt, pb) = (
        plot_left as f64,
        plot_right as f64,
        plot_top as f64,
        plot_bottom as f64,
    );
    let va = ViewAngle::for_preset(view_angle);

    // Every box's three faces, collected with their view-space depth for a single global painter sort.
    // The floor depth is divided into one cell per series. Each riser has a world-square footprint — its
    // z-depth matches its x-width in world units (`bar_w / plot_w`, dividing out the projection's
    // `depth_frac` so `z=1` spans the category axis) — so the top face reads square, matching the engine.
    // A single series therefore leaves the room floor mostly empty rather than stretching one deep slab
    // across it; the footprint is only capped when many clustered series would otherwise crowd their
    // cells. Each riser is centred in its series cell.
    let cell_depth = 1.0 / s_count as f64;
    let bar_depth = (bar_w as f64 / ((pr - pl) * va.depth_frac)).min(cell_depth * MAX_CELL_FILL);
    let mut labels: Vec<DrawOp> = Vec::new();
    let mut data_faces: Vec<(DrawOp, f64)> = Vec::new();
    for (s, (_, vals)) in series.iter().enumerate() {
        let cell_center = (s as f64 + 0.5) * cell_depth;
        let zf = cell_center - bar_depth / 2.0;
        let zb = cell_center + bar_depth / 2.0;
        for (c, val) in vals.iter().enumerate() {
            let color = if s_count == 1 {
                PALETTE[c % PALETTE.len()]
            } else {
                PALETTE[s % PALETTE.len()]
            };
            let h = (value_frac(*val, max_val) * f.plot_h as f64) as i32;
            let x0 = (plot_left + c as i32 * f.slot + (f.slot - bar_w) / 2) as f64;
            let x1 = x0 + bar_w as f64;
            let ytop = (plot_bottom - h.max(1)) as f64;
            let ybottom = pb;
            // One painter key per box (its floor-cell centre), shared by all three faces so a box's
            // faces stay grouped and boxes sort by grid distance — a far box never interleaves a near
            // one, regardless of height.
            let box_key = proj.depth(p3((x0 + x1) / 2.0, ybottom, (zf + zb) / 2.0));

            // Front (z = zf), the mid-shade face nearest the viewer.
            data_faces.push((
                face(
                    &proj,
                    &[
                        p3(x0, ytop, zf),
                        p3(x1, ytop, zf),
                        p3(x1, ybottom, zf),
                        p3(x0, ybottom, zf),
                    ],
                    shade(color, FRONT_SHADE),
                    None,
                    &src,
                )
                .0,
                box_key,
            ));
            // Top (y = ytop), the lit face: the base colour unshaded.
            data_faces.push((
                face(
                    &proj,
                    &[
                        p3(x0, ytop, zf),
                        p3(x1, ytop, zf),
                        p3(x1, ytop, zb),
                        p3(x0, ytop, zb),
                    ],
                    color,
                    None,
                    &src,
                )
                .0,
                box_key,
            ));
            // Side (x = x0), the shadowed viewer-facing left face (the right face x = x1 faces away and
            // is culled).
            data_faces.push((
                face(
                    &proj,
                    &[
                        p3(x0, ytop, zf),
                        p3(x0, ytop, zb),
                        p3(x0, ybottom, zb),
                        p3(x0, ybottom, zf),
                    ],
                    shade(color, SIDE_SHADE),
                    None,
                    &src,
                )
                .0,
                box_key,
            ));

            if show_labels {
                let top = proj.project(p3((x0 + x1) / 2.0, ytop, zf));
                labels.push(value_label(
                    top.x.0,
                    top.y.0.saturating_sub(230).max(plot_top),
                    &fmt_val(*val),
                    LABEL,
                    &src,
                ));
            }
        }
    }

    // Labels last (the projected value/category axes, then any per-box value labels) so text is never
    // overdrawn.
    let mut all_labels = axis_labels;
    all_labels.extend(labels);

    compose(background, data_faces, all_labels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_model::Color;
    use rpt_pages::Fill;

    fn rect() -> Rect {
        Rect {
            left: rpt_model::Twips(100),
            top: rpt_model::Twips(100),
            width: rpt_model::Twips(6000),
            height: rpt_model::Twips(4000),
        }
    }

    fn single(vals: &[(&str, f64)]) -> (Vec<String>, Vec<(String, Vec<f64>)>) {
        let cats: Vec<String> = vals.iter().map(|(n, _)| n.to_string()).collect();
        let series = vec![("v".to_string(), vals.iter().map(|(_, v)| *v).collect())];
        (cats, series)
    }

    fn fills(ops: &[DrawOp]) -> Vec<Color> {
        ops.iter()
            .filter_map(|o| match o {
                DrawOp::Polygon(p) => match &p.fill {
                    Some(Fill::Solid(c)) => Some(*c),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn empty_yields_no_ops() {
        let cx = ChartCtx::test(rect(), "T", AxisTitles::default(), true);
        assert!(riser_3d(&cx, &[], &[], rpt_model::ChartViewAngle::Standard).is_empty());
        let (cats, series) = single(&[("A", 1.0)]);
        assert!(riser_3d(&cx, &cats, &[], rpt_model::ChartViewAngle::Standard).is_empty());
        assert!(riser_3d(&cx, &[], &series, rpt_model::ChartViewAngle::Standard).is_empty());
    }

    #[test]
    fn emits_value_gridlines_and_floor_labels() {
        // The corner room draws value gridlines (Line ops) wrapping the walls and a category label per
        // slot on the floor — the 3-D axes, not the flat 2-D frame.
        let (cats, series) = single(&[("A", 12.0), ("B", 27.0), ("C", 6.0)]);
        let ops = riser_3d(
            &ChartCtx::test(rect(), "Cities", AxisTitles::default(), false),
            &cats,
            &series,
            rpt_model::ChartViewAngle::Standard,
        );
        let lines = ops.iter().filter(|o| matches!(o, DrawOp::Line(_))).count();
        let texts = ops.iter().filter(|o| matches!(o, DrawOp::Text(_))).count();
        assert!(lines > 0, "value gridlines wrap the walls, got {lines}");
        assert!(
            texts >= cats.len(),
            "at least one floor label per category, got {texts}"
        );
    }

    #[test]
    fn draws_three_planes_and_three_faces_per_box() {
        let (cats, series) = single(&[("A", 12.0), ("B", 27.0), ("C", 6.0)]);
        let ops = riser_3d(
            &ChartCtx::test(rect(), "Cities", AxisTitles::default(), true),
            &cats,
            &series,
            rpt_model::ChartViewAngle::Standard,
        );
        let polys = ops
            .iter()
            .filter(|o| matches!(o, DrawOp::Polygon(_)))
            .count();
        // 3 scenery planes + 3 faces per (1 series × 3 categories) box.
        assert_eq!(polys, 3 + 3 * 3, "3 planes + 3 faces/box, got {polys}");
    }

    #[test]
    fn multi_series_face_count_is_planes_plus_s_c_three() {
        let cats = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let series = vec![
            ("s1".to_string(), vec![10.0, 20.0, 30.0]),
            ("s2".to_string(), vec![15.0, 25.0, 5.0]),
            ("s3".to_string(), vec![8.0, 12.0, 18.0]),
        ];
        let ops = riser_3d(
            &ChartCtx::test(rect(), "", AxisTitles::default(), false),
            &cats,
            &series,
            rpt_model::ChartViewAngle::Standard,
        );
        let polys = fills(&ops).len();
        assert_eq!(
            polys,
            3 + 3 * 3 * 3,
            "3 planes + S×C×3 data faces, got {polys}"
        );
    }

    #[test]
    fn shade_ladder_top_is_base_front_and_side_darker() {
        // A single box: after the 3 scenery planes come its three faces, painter-sorted. The base
        // colour is PALETTE[0]; the top keeps it, the front is 0.8× and the side 0.6×.
        let (cats, series) = single(&[("A", 20.0)]);
        let ops = riser_3d(
            &ChartCtx::test(rect(), "", AxisTitles::default(), false),
            &cats,
            &series,
            rpt_model::ChartViewAngle::Standard,
        );
        let f = fills(&ops);
        let base = PALETTE[0];
        let lum = |c: Color| c.r as u32 + c.g as u32 + c.b as u32;
        // The base colour appears exactly once among the box faces (the top face).
        assert!(f.contains(&base), "top face keeps the base colour");
        let box_faces = &f[3..];
        let top = lum(base);
        let others: Vec<u32> = box_faces
            .iter()
            .filter(|&&c| c != base)
            .map(|&c| lum(c))
            .collect();
        assert_eq!(others.len(), 2, "front + side beside the base-coloured top");
        assert!(
            others.iter().all(|&l| l < top),
            "front and side are darker than the base top: {others:?} vs {top}"
        );
        let (front, side) = (others[0].max(others[1]), others[0].min(others[1]));
        assert!(front > side, "front (0.8×) brighter than side (0.6×)");
    }

    /// A large category×series grid and non-finite / degenerate values must not panic and must stay
    /// bounded (the painter sort is NaN-safe via `total_cmp`; heights fold NaN/Inf through the value
    /// scale). Also covers the misconfigured single-cell 3-D chart.
    #[test]
    fn large_grid_and_degenerate_3d_do_not_panic() {
        let cats: Vec<String> = (0..40).map(|i| format!("c{i}")).collect();
        let series: Vec<(String, Vec<f64>)> = (0..12)
            .map(|s| {
                let vals = (0..40)
                    .map(|c| match (s + c) % 5 {
                        0 => f64::NAN,
                        1 => f64::INFINITY,
                        2 => -3.0,
                        3 => 1e17,
                        _ => (c as f64) + 1.0,
                    })
                    .collect();
                (format!("s{s}"), vals)
            })
            .collect();
        let ops = riser_3d(
            &ChartCtx::test(rect(), "Stress", AxisTitles::default(), true),
            &cats,
            &series,
            rpt_model::ChartViewAngle::Standard,
        );
        assert!(!ops.is_empty(), "a large grid still emits faces");

        // A single cell (one category, one series) is a valid, if degenerate, 3-D chart.
        let (c1, s1) = single(&[("only", 0.0)]);
        let _ = riser_3d(
            &ChartCtx::test(rect(), "", AxisTitles::default(), true),
            &c1,
            &s1,
            rpt_model::ChartViewAngle::Standard,
        );
    }

    #[test]
    fn farther_box_faces_precede_nearer_ones() {
        // Two series receding in z: every face of the farther (back) series must be emitted before
        // every face of the nearer (front) series, so the nearer boxes overlap the farther ones.
        let cats = vec!["A".to_string()];
        // Series 0 sits at the FRONT (z-band starting at 0 = nearest the viewer, drawn last); series 1
        // recedes to the BACK (larger z = farther, drawn first). Colours: series 0 → PALETTE[0],
        // series 1 → PALETTE[1].
        let series = vec![
            ("front".to_string(), vec![20.0]),
            ("back".to_string(), vec![20.0]),
        ];
        let ops = riser_3d(
            &ChartCtx::test(rect(), "", AxisTitles::default(), false),
            &cats,
            &series,
            rpt_model::ChartViewAngle::Standard,
        );
        // Skip the 3 scenery planes; the next 6 fills are the two boxes' faces in painter order.
        let data = &fills(&ops)[3..];
        assert_eq!(data.len(), 6, "3 faces × 2 boxes");
        // Every BACK-series (series 1, farther) fill must precede every FRONT-series (series 0) fill.
        let shades = |base: Color| [base, shade(base, FRONT_SHADE), shade(base, SIDE_SHADE)];
        let (front, back) = (shades(PALETTE[0]), shades(PALETTE[1]));
        let back_last = data.iter().rposition(|c| back.contains(c)).unwrap();
        let front_first = data.iter().position(|c| front.contains(c)).unwrap();
        assert!(
            back_last < front_first,
            "all back-series faces precede all front-series faces: {back_last} < {front_first}"
        );
    }
}
