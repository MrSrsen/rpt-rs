//! 3-D area ribbon chart: each data series is drawn as an extruded area silhouette — the flat 2-D area
//! polygon (category tops down to the baseline) swept from a front z-plane back to a rear one, closed
//! by a top ribbon along the crest and left/right end caps so the solid reads closed. Series recede
//! along Z like the riser; faces are flat-shaded per series (lit crest, mid-shade front, darker
//! back/caps) and globally painter-sorted back-to-front over the shared scenery walls/floor. Routed
//! from the flat area path when the Area family's depth-effect bit is set
//! ([`rpt_model::ChartDefinition::has_depth_effect`]).

use super::projection::{face, p3, shade, Vec3, BACK_SHADE, FRONT_SHADE};
use super::scene::{compose, setup_3d};
#[cfg(test)]
use crate::chart::common::AxisTitles;
use crate::chart::common::{fmt_val, value_label, ChartCtx, LABEL, PALETTE};
#[cfg(test)]
use rpt_model::Rect;
use rpt_pages::DrawOp;

/// Build the draw-ops for a 3-D area chart. `categories` are the X-axis slots; `series` is one row per
/// data binding (`name`, value-per-category), receding along Z (a single series is `S == 1`). Each
/// series is an extruded area solid: a front silhouette polygon, a back one, a `C − 1`-quad top ribbon
/// along the crest, and two end caps — `C + 3` faces per series. Draws the three background planes
/// first, then every face globally painter-sorted back-to-front, then category (and optional value)
/// labels. Returns an empty vec if there is nothing to plot.
pub(crate) fn area_3d(
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

    // Frame, projection, and background scenery come from the shared 3-D scene setup; the area family
    // has a single depth band, so it reserves no series-depth floor grid.
    let (f, max_val, proj, background, axis_labels) =
        setup_3d(rect, title, categories, series, view_angle, &[], &src);

    let plot_left = f.plot_left;
    let plot_top = f.plot_top();
    let plot_bottom = f.plot_bottom;
    let pb = plot_bottom as f64;

    let slot = f.slot;
    let x_at = |c: usize| (plot_left + c as i32 * slot + slot / 2) as f64;
    let y_at = |v: f64| (plot_bottom - ((v.max(0.0) / max_val) * f.plot_h as f64) as i32) as f64;
    let base_y = pb;

    let band = 1.0 / s_count as f64;
    let gap = band * 0.15;
    let mut data_faces: Vec<(DrawOp, f64)> = Vec::new();
    let mut labels: Vec<DrawOp> = Vec::new();
    for (s, (_, vals)) in series.iter().enumerate() {
        let zf = s as f64 * band;
        let zb = (s + 1) as f64 * band - gap;
        let color = PALETTE[s % PALETTE.len()];

        // Front and back silhouette polygons: baseline under the first point → each category top →
        // baseline under the last point, one at the front z-plane and one at the back.
        let silhouette = |z: f64| -> Vec<Vec3> {
            let mut poly = Vec::with_capacity(c_count + 2);
            poly.push(Vec3 {
                x: x_at(0),
                y: base_y,
                z,
            });
            for (c, v) in vals.iter().enumerate() {
                poly.push(Vec3 {
                    x: x_at(c),
                    y: y_at(*v),
                    z,
                });
            }
            poly.push(Vec3 {
                x: x_at(c_count - 1),
                y: base_y,
                z,
            });
            poly
        };
        data_faces.push(face(
            &proj,
            &silhouette(zf),
            shade(color, FRONT_SHADE),
            None,
            &src,
        ));
        data_faces.push(face(
            &proj,
            &silhouette(zb),
            shade(color, BACK_SHADE),
            None,
            &src,
        ));

        // Top ribbon: a lit quad joining each pair of consecutive crest points front-to-back.
        for c in 0..c_count.saturating_sub(1) {
            let (y0, y1) = (y_at(vals[c]), y_at(vals[c + 1]));
            let (x0, x1) = (x_at(c), x_at(c + 1));
            data_faces.push(face(
                &proj,
                &[
                    p3(x0, y0, zf),
                    p3(x1, y1, zf),
                    p3(x1, y1, zb),
                    p3(x0, y0, zb),
                ],
                color,
                None,
                &src,
            ));
        }

        // Left and right end caps (baseline → crest, swept front-to-back) so the solid reads closed.
        let cap = |c: usize| -> [Vec3; 4] {
            let (x, y) = (x_at(c), y_at(vals[c]));
            [
                Vec3 {
                    x,
                    y: base_y,
                    z: zf,
                },
                Vec3 { x, y, z: zf },
                Vec3 { x, y, z: zb },
                Vec3 {
                    x,
                    y: base_y,
                    z: zb,
                },
            ]
        };
        data_faces.push(face(&proj, &cap(0), shade(color, BACK_SHADE), None, &src));
        data_faces.push(face(
            &proj,
            &cap(c_count - 1),
            shade(color, BACK_SHADE),
            None,
            &src,
        ));

        if show_labels {
            for (c, v) in vals.iter().enumerate() {
                let top = proj.project(p3(x_at(c), y_at(*v), zf));
                labels.push(value_label(
                    top.x.0,
                    (top.y.0 - 230).max(plot_top),
                    &fmt_val(*v),
                    LABEL,
                    &src,
                ));
            }
        }
    }

    let mut all_labels = axis_labels;
    all_labels.extend(labels);

    compose(background, data_faces, all_labels)
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
    fn empty_yields_no_ops() {
        let cats = vec!["A".to_string(), "B".to_string()];
        let series = vec![("s".to_string(), vec![1.0, 2.0])];
        let cx = ChartCtx::test(rect(), "T", AxisTitles::default(), false);
        assert!(area_3d(&cx, &[], &series, rpt_model::ChartViewAngle::Standard).is_empty());
        assert!(area_3d(&cx, &cats, &[], rpt_model::ChartViewAngle::Standard).is_empty());
    }

    #[test]
    fn single_series_face_count_is_planes_plus_c_plus_three() {
        let cats: Vec<String> = (0..5).map(|c| format!("c{c}")).collect();
        let series = vec![("s".to_string(), vec![3.0, 8.0, 5.0, 9.0, 4.0])];
        let ops = area_3d(
            &ChartCtx::test(rect(), "Area3D", AxisTitles::default(), false),
            &cats,
            &series,
            rpt_model::ChartViewAngle::Standard,
        );
        let polys = fills(&ops).len();
        // 3 scenery planes + (front + back + (C−1) crest quads + 2 caps) = 3 + (2 + 4 + 2) = 11.
        assert_eq!(polys, 3 + (5 + 3), "3 planes + C+3 faces, got {polys}");
    }

    #[test]
    fn multi_series_face_count_scales_with_series() {
        let cats: Vec<String> = (0..4).map(|c| format!("c{c}")).collect();
        let series = vec![
            ("s1".to_string(), vec![1.0, 5.0, 3.0, 7.0]),
            ("s2".to_string(), vec![2.0, 4.0, 6.0, 1.0]),
        ];
        let ops = area_3d(
            &ChartCtx::test(rect(), "", AxisTitles::default(), false),
            &cats,
            &series,
            rpt_model::ChartViewAngle::Standard,
        );
        let polys = fills(&ops).len();
        // 3 planes + S×(C+3) = 3 + 2×7 = 17.
        assert_eq!(polys, 3 + 2 * (4 + 3), "3 planes + S×(C+3), got {polys}");
        // Each series' faces use only its base palette colour and the two shaded variants.
        let data = &fills(&ops)[3..];
        for base in [PALETTE[0], PALETTE[1]] {
            let ok = [base, shade(base, FRONT_SHADE), shade(base, BACK_SHADE)];
            assert!(
                data.iter().filter(|c| ok.contains(c)).count() >= 7,
                "series colour + its shades appear on its faces"
            );
        }
    }
}
