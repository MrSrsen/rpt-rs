//! 3-D surface mesh chart: categories run along X (the shared Num+Ord frame slots) and data series
//! recede along Z, with each series drawn as a continuous **top ribbon** — a strip of quads joining
//! consecutive category heights across the series' z-band. Unlike the riser there are no front/side
//! faces and no directional shading: the native surface fills each series a flat palette colour
//! rather than per-facet normal shading, and the ribbons are globally painter-sorted
//! back-to-front over the same scenery walls/floor.

use super::projection::{face, Projection, Vec3, ViewAngle};
use super::scene::{axes_3d, background_planes, compose, floor_grid, room_edges};
use crate::chart::common::{compute_frame, nice_scale, AxisTitles, PALETTE};
use rpt_model::Rect;
use rpt_pages::{DrawOp, ObjectKind, ObjectRef};

/// Build the draw-ops for a 3-D surface chart. `categories` are the X-axis slots; `series` is one row
/// per data binding (`name`, value-per-category), receding along Z. Each series is a top ribbon of
/// `categories.len() − 1` flat quads (`PALETTE[s]`, no shading), so a chart of `S` series over `C`
/// categories has `S × (C − 1)` data faces. Draws the three background planes first, then the ribbon
/// quads globally painter-sorted back-to-front, then the category labels. Returns an empty vec if
/// there is nothing to plot or a single category (no segment to span).
#[allow(clippy::too_many_arguments)]
pub(crate) fn surface_3d(
    rect: Rect,
    title: &str,
    categories: &[String],
    series: &[(String, Vec<f64>)],
    view_angle: rpt_model::ChartViewAngle,
    section_name: &str,
    obj_name: &str,
) -> Vec<DrawOp> {
    let (s_count, c_count) = (series.len(), categories.len());
    if s_count == 0 || c_count < 2 {
        return Vec::new();
    }
    let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(obj_name));

    // The value scale spans every series×category value so the tallest ridge never touches the ceiling;
    // the frame reserves the category slots off a synthetic per-category series carrying the column max.
    let global_max = series
        .iter()
        .flat_map(|(_, vals)| vals.iter().copied())
        .fold(0.0_f64, f64::max);
    let (max_val, _) = nice_scale(global_max);
    let frame_series: Vec<(String, f64)> = categories
        .iter()
        .enumerate()
        .map(|(c, label)| {
            let cat_max = series
                .iter()
                .map(|(_, vals)| vals.get(c).copied().unwrap_or(0.0))
                .fold(0.0_f64, f64::max);
            (label.clone(), cat_max)
        })
        .collect();
    let f = compute_frame(rect, title, AxisTitles::default(), &frame_series);

    let plot_left = f.plot_left;
    let plot_right = f.plot_right();
    let plot_top = f.plot_top();
    let plot_bottom = f.plot_bottom;

    let (pl, pr, pt, pb) = (
        plot_left as f64,
        plot_right as f64,
        plot_top as f64,
        plot_bottom as f64,
    );
    // The chart's decoded view-angle preset, perspective-fit into the plot box (as the riser).
    let proj = Projection::perspective(pl, pr, pt, pb, ViewAngle::for_preset(view_angle));
    let (grid, axis_labels) = axes_3d(&proj, &f, categories, &src);
    let mut background = background_planes(&proj, pl, pr, pt, pb, &src).to_vec();
    let x_div: Vec<f64> = (1..c_count)
        .map(|c| (plot_left + c as i32 * f.slot) as f64)
        .collect();
    background.extend(floor_grid(&proj, pb, pl, pr, &x_div, &[], &src));
    background.extend(grid);
    background.extend(room_edges(&proj, pl, pr, pt, pb, &src));

    // Category grid: a point per category, centred in its slot (as line/area), at the scaled height.
    let slot = f.slot;
    let x_at = |c: usize| (plot_left + c as i32 * slot + slot / 2) as f64;
    let y_at = |v: f64| (plot_bottom - ((v.max(0.0) / max_val) * f.plot_h as f64) as i32) as f64;

    // Each series is a top ribbon over its z-band; a small back-gap separates adjacent series' slabs.
    let band = 1.0 / s_count as f64;
    let gap = band * 0.15;
    let mut data_faces: Vec<(DrawOp, f64)> = Vec::new();
    for (s, (_, vals)) in series.iter().enumerate() {
        let zf = s as f64 * band;
        let zb = (s + 1) as f64 * band - gap;
        let color = PALETTE[s % PALETTE.len()];
        for c in 0..c_count - 1 {
            let (v0, v1) = (
                vals.get(c).copied().unwrap_or(0.0),
                vals.get(c + 1).copied().unwrap_or(0.0),
            );
            let (x0, x1) = (x_at(c), x_at(c + 1));
            let (y0, y1) = (y_at(v0), y_at(v1));
            // A flat quad joining the two category heights across the series' z-band (front → back).
            data_faces.push(face(
                &proj,
                &[
                    Vec3 {
                        x: x0,
                        y: y0,
                        z: zf,
                    },
                    Vec3 {
                        x: x1,
                        y: y1,
                        z: zf,
                    },
                    Vec3 {
                        x: x1,
                        y: y1,
                        z: zb,
                    },
                    Vec3 {
                        x: x0,
                        y: y0,
                        z: zb,
                    },
                ],
                color,
                None,
                &src,
            ));
        }
    }

    compose(background, data_faces, axis_labels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_pages::Fill;

    fn rect() -> Rect {
        Rect {
            left: rpt_model::Twips(100),
            top: rpt_model::Twips(100),
            width: rpt_model::Twips(6000),
            height: rpt_model::Twips(4000),
        }
    }

    fn fills(ops: &[DrawOp]) -> Vec<rpt_model::Color> {
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
    fn empty_or_single_category_yields_no_ops() {
        let cats = vec!["A".to_string(), "B".to_string()];
        let series = vec![("s".to_string(), vec![1.0, 2.0])];
        assert!(surface_3d(
            rect(),
            "T",
            &[],
            &series,
            rpt_model::ChartViewAngle::Standard,
            "S",
            "Graph1"
        )
        .is_empty());
        assert!(surface_3d(
            rect(),
            "T",
            &cats,
            &[],
            rpt_model::ChartViewAngle::Standard,
            "S",
            "Graph1"
        )
        .is_empty());
        // A single category has no segment to span.
        let one = vec!["A".to_string()];
        let s1 = vec![("s".to_string(), vec![1.0])];
        assert!(surface_3d(
            rect(),
            "T",
            &one,
            &s1,
            rpt_model::ChartViewAngle::Standard,
            "S",
            "Graph1"
        )
        .is_empty());
    }

    #[test]
    fn face_count_is_planes_plus_s_times_c_minus_one() {
        let cats: Vec<String> = (0..10).map(|c| format!("c{c}")).collect();
        let series = vec![
            ("s1".to_string(), (0..10).map(|c| c as f64 + 1.0).collect()),
            ("s2".to_string(), (0..10).map(|c| (10 - c) as f64).collect()),
        ];
        let ops = surface_3d(
            rect(),
            "Surf",
            &cats,
            &series,
            rpt_model::ChartViewAngle::Standard,
            "RH",
            "Graph1",
        );
        let polys = fills(&ops).len();
        // 3 scenery planes + S×(C−1) ribbon quads = 3 + 2×9 = 21.
        assert_eq!(
            polys,
            3 + 2 * 9,
            "3 planes + S×(C−1) ribbon quads, got {polys}"
        );
    }

    #[test]
    fn each_series_is_a_single_flat_palette_colour() {
        let cats: Vec<String> = (0..4).map(|c| format!("c{c}")).collect();
        let series = vec![
            ("s1".to_string(), vec![1.0, 5.0, 3.0, 7.0]),
            ("s2".to_string(), vec![2.0, 4.0, 6.0, 1.0]),
        ];
        let ops = surface_3d(
            rect(),
            "",
            &cats,
            &series,
            rpt_model::ChartViewAngle::Standard,
            "RH",
            "Graph1",
        );
        // Skip the 3 scenery planes; every ribbon quad is its series' flat palette colour (no shading,
        // unlike the riser's shade ladder). Painter-sorting interleaves the series, so count colours.
        let data = &fills(&ops)[3..];
        assert_eq!(data.len(), 2 * 3, "S×(C−1) ribbon quads");
        assert!(
            data.iter().all(|&c| c == PALETTE[0] || c == PALETTE[1]),
            "only the two series' flat palette colours appear (no shaded variants)"
        );
        let n0 = data.iter().filter(|&&c| c == PALETTE[0]).count();
        let n1 = data.iter().filter(|&&c| c == PALETTE[1]).count();
        assert_eq!((n0, n1), (3, 3), "each series contributes C−1 flat quads");
    }
}
