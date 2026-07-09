//! Shared scene assembly for the 3-D chart families. Every 3-D renderer paints the same corner room —
//! a floor and two far-edge back walls meeting at a back vertical edge, wrapped by the value gridlines
//! ([`axes_3d`]) — behind its data, then globally painter-sorts its data faces back-to-front and draws
//! its labels last. This module owns that common skeleton so the riser, surface, and area-ribbon
//! renderers stay geometry-only.

use super::projection::{face, Projection, Vec3};
use crate::chart::common::{fmt_val, Frame, GRID, LABEL};
use rpt_model::{Color, Rect, Twips};
use rpt_pages::{
    DrawOp, FontSpec, LineOp, LineStyle, ObjectRef, Point, Stroke, TextAlign, TextRun,
};

/// The engine's scenery greys: the category back wall (`z = 1`), the
/// series back wall (`x = plot_right`), and the floor (`y = plot_bottom`). The native engine subdivides
/// each plane into three gridline bands; we draw one plane apiece plus the value gridlines (the 3→9
/// subdivision is an accepted approximation).
const BACK_WALL: Color = Color {
    a: 255,
    r: 0xcc,
    g: 0xcc,
    b: 0xcc,
};
const SIDE_WALL: Color = Color {
    a: 255,
    r: 0x99,
    g: 0x99,
    b: 0x99,
};
/// A light plane, clearly lighter than either wall but not white, so the floor reads against the page.
const FLOOR: Color = Color {
    a: 255,
    r: 0xe4,
    g: 0xe4,
    b: 0xe4,
};
/// The room's external silhouette edges: a medium grey outline around the box.
const EDGE: Color = Color {
    a: 255,
    r: 0x66,
    g: 0x66,
    b: 0x66,
};

/// Build the three background planes for a 3-D scene over the plot rectangle `[pl,pr]×[pt,pb]`
/// (plot-twip units): the category back wall (`z = 1`), the series back wall (`x = pr`), and the floor,
/// in that draw order (behind all data). The two walls meet at the back vertical edge (`x = pr, z = 1`)
/// so the near floor corner points at the viewer. Shared by every 3-D family so the scenery is
/// identical.
pub(super) fn background_planes(
    proj: &Projection,
    pl: f64,
    pr: f64,
    pt: f64,
    pb: f64,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> [DrawOp; 3] {
    let plane = |corners: [Vec3; 4], fill: Color| face(proj, &corners, fill, None, src).0;
    [
        plane(
            [
                Vec3 {
                    x: pl,
                    y: pt,
                    z: 1.0,
                },
                Vec3 {
                    x: pr,
                    y: pt,
                    z: 1.0,
                },
                Vec3 {
                    x: pr,
                    y: pb,
                    z: 1.0,
                },
                Vec3 {
                    x: pl,
                    y: pb,
                    z: 1.0,
                },
            ],
            BACK_WALL,
        ),
        plane(
            [
                Vec3 {
                    x: pr,
                    y: pt,
                    z: 0.0,
                },
                Vec3 {
                    x: pr,
                    y: pt,
                    z: 1.0,
                },
                Vec3 {
                    x: pr,
                    y: pb,
                    z: 1.0,
                },
                Vec3 {
                    x: pr,
                    y: pb,
                    z: 0.0,
                },
            ],
            SIDE_WALL,
        ),
        plane(
            [
                Vec3 {
                    x: pl,
                    y: pb,
                    z: 0.0,
                },
                Vec3 {
                    x: pr,
                    y: pb,
                    z: 0.0,
                },
                Vec3 {
                    x: pr,
                    y: pb,
                    z: 1.0,
                },
                Vec3 {
                    x: pl,
                    y: pb,
                    z: 1.0,
                },
            ],
            FLOOR,
        ),
    ]
}

/// A stroked line between two 3-D points.
fn edge_line(
    proj: &Projection,
    a: Vec3,
    b: Vec3,
    color: Color,
    width: i32,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> DrawOp {
    DrawOp::Line(LineOp {
        from: proj.project(a),
        to: proj.project(b),
        stroke: Stroke {
            color,
            width: Twips(width),
            style: LineStyle::Single,
        },
        source: src(),
    })
}

/// The room's external silhouette: the two floor front edges from the near corner, the floor/wall base
/// edges, the outer vertical wall edges, the back corner edge, and the wall top edges — a clean box
/// outline. No internal seams or hidden edges are stroked. Drawn behind the data so risers overlap the
/// wall edges they occlude.
pub(super) fn room_edges(
    proj: &Projection,
    pl: f64,
    pr: f64,
    pt: f64,
    pb: f64,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> Vec<DrawOp> {
    let p = |x: f64, y: f64, z: f64| Vec3 { x, y, z };
    // Floor corners (y = pb): near, category-far-front, back, series-far-front.
    let (a, b, c, d) = (
        p(pl, pb, 0.0),
        p(pr, pb, 0.0),
        p(pr, pb, 1.0),
        p(pl, pb, 1.0),
    );
    // Wall tops (y = pt): series-wall front-top, back-corner top, category-wall left-top.
    let (bt, ct, dt) = (p(pr, pt, 0.0), p(pr, pt, 1.0), p(pl, pt, 1.0));
    const W: i32 = 18;
    [
        (a, b),   // floor front-right edge (category axis front)
        (a, d),   // floor front-left edge (series axis front)
        (b, c),   // floor right edge (series wall base)
        (d, c),   // floor back edge (category wall base)
        (d, dt),  // category wall left vertical
        (dt, ct), // category wall top
        (c, ct),  // back corner vertical
        (b, bt),  // series wall front vertical
        (bt, ct), // series wall top
    ]
    .into_iter()
    .map(|(u, v)| edge_line(proj, u, v, EDGE, W, src))
    .collect()
}

/// The floor grid: the category slot divisions running front-to-back and, for a clustered chart, the
/// series-band divisions running left-to-right — so the floor reads as a grid the risers sit inset
/// within. `x_div` are the category slot boundaries (twips); `z_div` the series band boundaries in
/// `[0,1]`. Drawn light, behind the data.
pub(super) fn floor_grid(
    proj: &Projection,
    pb: f64,
    x_lo: f64,
    x_hi: f64,
    x_div: &[f64],
    z_div: &[f64],
    src: &dyn Fn() -> Option<ObjectRef>,
) -> Vec<DrawOp> {
    let mut ops = Vec::new();
    for &x in x_div {
        ops.push(edge_line(
            proj,
            Vec3 { x, y: pb, z: 0.0 },
            Vec3 { x, y: pb, z: 1.0 },
            GRID,
            8,
            src,
        ));
    }
    for &z in z_div {
        ops.push(edge_line(
            proj,
            Vec3 { x: x_lo, y: pb, z },
            Vec3 { x: x_hi, y: pb, z },
            GRID,
            8,
            src,
        ));
    }
    ops
}

/// A small 7-pt axis label in the box `[left, left+width]` at `top`, aligned within it. Used for the
/// value-tick labels on the walls and the category labels on the floor.
fn axis_text(
    left: i32,
    top: i32,
    width: i32,
    align: TextAlign,
    text: &str,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> DrawOp {
    DrawOp::Text(TextRun {
        bounds: Rect {
            left: Twips(left),
            top: Twips(top),
            width: Twips(width),
            height: Twips(200),
        },
        text: text.to_string(),
        font: FontSpec {
            family: "Arial".into(),
            size_pt: 7.0,
            ..Default::default()
        },
        color: LABEL,
        align,
        rotation: 0.0,
        metrics: None,
        source: src(),
    })
}

/// Draw the 3-D value axes and category labels for the corner room: horizontal value gridlines wrapping
/// both back walls with a tick label off each wall's outer edge (the engine's twin value columns), and
/// the category labels stepping along the front floor edge. Returns `(gridlines, labels)` — the
/// gridlines belong behind the data (drawn with the background), the labels on top. Value ticks come
/// from `f`'s scale; categories are centred in their floor slots.
pub(super) fn axes_3d(
    proj: &Projection,
    f: &Frame,
    categories: &[String],
    src: &dyn Fn() -> Option<ObjectRef>,
) -> (Vec<DrawOp>, Vec<DrawOp>) {
    const LBL_W: i32 = 780;
    let pl = f.plot_left as f64;
    let pr = f.plot_right() as f64;
    let pb = f.plot_bottom as f64;
    let plot_h = f.plot_h as f64;
    let (max_val, step) = (f.max_val, f.step);
    let ticks = if step > 0.0 {
        (max_val / step).round() as i32
    } else {
        0
    };

    let mut grid: Vec<DrawOp> = Vec::new();
    let mut labels: Vec<DrawOp> = Vec::new();
    let gline = |a: Point, b: Point| {
        DrawOp::Line(LineOp {
            from: a,
            to: b,
            stroke: Stroke {
                color: GRID,
                width: Twips(10),
                style: LineStyle::Single,
            },
            source: src(),
        })
    };

    for t in 0..=ticks {
        let y = pb - (t as f64 * step / max_val.max(1e-9)) * plot_h;
        let text = fmt_val(t as f64 * step);
        // Category back wall (z = 1): a gridline across it; the tick label hugs its left edge.
        let l1 = proj.project(Vec3 { x: pl, y, z: 1.0 });
        let r1 = proj.project(Vec3 { x: pr, y, z: 1.0 });
        grid.push(gline(l1, r1));
        labels.push(axis_text(
            l1.x.0 - LBL_W - 40,
            l1.y.0 - 100,
            LBL_W,
            TextAlign::Right,
            &text,
            src,
        ));
        // Series back wall (x = pr): a gridline receding front-to-back; the tick label hugs its front
        // edge on the right.
        let f0 = proj.project(Vec3 { x: pr, y, z: 0.0 });
        let b1 = proj.project(Vec3 { x: pr, y, z: 1.0 });
        grid.push(gline(f0, b1));
        labels.push(axis_text(
            f0.x.0 + 40,
            f0.y.0 - 100,
            LBL_W,
            TextAlign::Left,
            &text,
            src,
        ));
    }

    // Category labels along the front floor edge (z = 0), centred under each slot.
    let slot = f.slot;
    for (c, label) in categories.iter().enumerate() {
        let cx = (f.plot_left + c as i32 * slot + slot / 2) as f64;
        let a = proj.project(Vec3 {
            x: cx,
            y: pb,
            z: 0.0,
        });
        labels.push(axis_text(
            a.x.0 - LBL_W / 2,
            a.y.0 + 40,
            LBL_W,
            TextAlign::Center,
            label,
            src,
        ));
    }

    (grid, labels)
}

/// Compose a 3-D scene: emit `background` first (behind everything), then the `data_faces`
/// painter-sorted farthest-first (largest view-space depth) so nearer faces overlap farther ones,
/// then `labels` last so text is never overdrawn.
pub(super) fn compose(
    background: Vec<DrawOp>,
    mut data_faces: Vec<(DrawOp, f64)>,
    labels: Vec<DrawOp>,
) -> Vec<DrawOp> {
    let mut ops = background;
    data_faces.sort_by(|a, b| b.1.total_cmp(&a.1));
    ops.extend(data_faces.into_iter().map(|(op, _)| op));
    ops.extend(labels);
    ops
}

#[cfg(test)]
mod tests {
    use super::super::projection::ViewAngle;
    use super::*;

    fn proj() -> Projection {
        Projection::perspective(0.0, 6000.0, 0.0, 4000.0, ViewAngle::DEPTH_EFFECT)
    }

    #[test]
    fn room_edges_are_the_nine_external_edges() {
        // The outlined box is exactly nine external edges (two floor front edges, two floor/wall base
        // edges, two outer wall verticals, the back corner vertical, and two wall tops) — all strokes.
        let ops = room_edges(&proj(), 0.0, 6000.0, 0.0, 4000.0, &|| None);
        assert_eq!(ops.len(), 9, "nine external edges, got {}", ops.len());
        assert!(
            ops.iter().all(|o| matches!(o, DrawOp::Line(_))),
            "external edges are stroked lines"
        );
    }

    #[test]
    fn floor_grid_draws_one_line_per_division() {
        let x_div = [1500.0, 3000.0, 4500.0];
        let z_div = [1.0 / 3.0, 2.0 / 3.0];
        let ops = floor_grid(&proj(), 4000.0, 0.0, 6000.0, &x_div, &z_div, &|| None);
        assert_eq!(
            ops.len(),
            x_div.len() + z_div.len(),
            "one floor line per category/series division"
        );
    }
}
