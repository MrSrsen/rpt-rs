//! Projection of a 3-D chart scene to the 2-D page. Two variants share one interface: [`Projection::
//! Oblique`] is a fixed depth offset with no perspective divide (a cheap, view-angle-inexact
//! fallback), and [`Projection::Perspective`] is the real pinhole transform the native engine uses —
//! a view-angle rotation followed by a perspective divide, then an independent x/y scale that
//! stretches the projected room to fill the target box (measured: the engine does not letterbox a
//! uniform scale into the shorter axis). Faces are emitted as filled [`rpt_pages::DrawOp::Polygon`]s
//! and painter-sorted back-to-front by their floor-plane depth (value-independent, so risers sort by
//! grid cell, not height).

use rpt_model::{Color, Twips};
use rpt_pages::{DrawOp, LineStyle, ObjectRef, Point, PolygonOp, Stroke};

/// A point in the chart's 3-D space: `x`/`y` in plot-twip units (the 2-D plot plane, `y` growing
/// downward as in the page), `z` the normalized depth — `0.0` = the front plane nearest the viewer,
/// `1.0` = the back wall.
#[derive(Debug, Clone, Copy)]
pub(super) struct Vec3 {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) z: f64,
}

/// Shorthand for a scene-space point `(x, y, z)` — keeps the face-construction geometry readable.
pub(super) fn p3(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

/// A rectangle in page twips. A projection takes two: the **scene** box its `x`/`y` coordinates are
/// expressed in, and the **fit** box on the page its projected scene is scaled and centred into. They
/// are separate because the scene's coordinates come from the label frame while the room's size and
/// place come from the chart box.
#[derive(Debug, Clone, Copy)]
pub(super) struct PlotBox {
    pub(super) left: f64,
    pub(super) right: f64,
    pub(super) top: f64,
    pub(super) bottom: f64,
}

impl PlotBox {
    pub(super) fn new(left: f64, right: f64, top: f64, bottom: f64) -> Self {
        PlotBox {
            left,
            right,
            top,
            bottom,
        }
    }
}

/// A view angle: the elevation (about X) and rotation (about Y) that orient the scene, the pinhole
/// `eye` distance that drives the perspective divide, the room's proportions, the thickness of its
/// three slabs, and where the projected room lands in the chart box. The native engine picks one of
/// 16 presets per chart ([`rpt_model::ChartViewAngle`]); [`ViewAngle::for_preset`] maps each to its
/// concrete geometry. The stored preset is decoded (`0x0121` `+0x4cc`), so a chart requesting a
/// non-`Standard` angle resolves to that preset's room here.
///
/// Every field is per preset: a preset is not just an orientation but a whole room — its own lens,
/// its own floor/wall proportions, its own wall thickness, and its own size and place on the page. The
/// `Thick*` presets differ from their plain counterparts mostly in the slab fractions, and a few
/// presets carry walls so thin the engine culls their end-caps entirely.
#[derive(Debug, Clone, Copy)]
pub(super) struct ViewAngle {
    pub(super) elevation_deg: f64,
    pub(super) rotation_deg: f64,
    /// The pinhole eye distance. Smaller means a stronger perspective divide: the room's vertical
    /// edges splay outward from the projection centre as they rise, which is what separates the
    /// engine's rooms from a sheared box.
    pub(super) eye: f64,
    /// The scene's depth (z) world-extent as a fraction of the category extent. The z axis carries only
    /// a modest recede (a shallow "room"), so it is bounded here rather than tied to the full plot scale.
    pub(super) depth_frac: f64,
    /// The value (height) world-extent as a fraction of the category extent. The room is a flat box,
    /// not a cube: its walls are far shorter than the floor is wide.
    pub(super) height_frac: f64,
    /// The series wall's thickness, as a fraction of the category extent.
    pub(super) wall_thick_u: f64,
    /// The category wall's thickness, as a fraction of the series extent.
    pub(super) wall_thick_z: f64,
    /// The floor plate's thickness, as a fraction of the value extent.
    pub(super) floor_thick: f64,
    /// How large the projected room is drawn, as a multiple of the chart box. The room is NOT fit to a
    /// box: each preset draws at its own size, so a tall room really is drawn taller than a flat one
    /// rather than being shrunk back into the same rectangle.
    pub(super) viewport_scale: f64,
    /// Where the projection's origin lands in the chart box, as fractions of its width and height.
    pub(super) viewport_cx: f64,
    pub(super) viewport_cy: f64,
}

impl ViewAngle {
    /// The engine's default 3-D preset (`Standard`), and the fallback for any out-of-range or
    /// unrecognized preset code.
    #[rustfmt::skip]
    pub(super) const DEPTH_EFFECT: ViewAngle =
        va(35.18, 47.36, 5.209, (1.0085, 0.5941), (0.0747, 0.0637, 0.1512), (1.8369, 0.5044, 0.4550));

    /// The concrete view angle for a model [`ChartViewAngle`] preset. The chart decoder reads the stored
    /// preset from the `0x0121` `+0x4cc` enum (the 1-based `CrViewingAngleEnum`).
    ///
    /// Each preset is its own camera, room proportions, slab thicknesses, and chart-box placement.
    /// `BirdsEyeView`'s slabs are so thin the room carries almost no depth cue, and `ThickSeriesView`'s
    /// near edge-on rotation collapses two of the nine faces to nothing — both are known less precisely
    /// than the other fourteen presets.
    #[rustfmt::skip]
    pub(super) fn for_preset(preset: rpt_model::ChartViewAngle) -> ViewAngle {
        use rpt_model::ChartViewAngle as P;
        match preset {
            //                      elev    rot     eye   (depth, height)  (wall_u, wall_z, floor)   (scale, cx, cy)
            P::Standard          => ViewAngle::DEPTH_EFFECT,
            P::TallView          => va(45.89, 49.97,  9.413, (1.0660, 1.8606), (0.0574, 0.0628, 0.1216), (1.9060, 0.4216, 0.5167)),
            P::TopView           => va(80.14, 44.16,  7.116, (1.0123, 2.4176), (0.0434, 0.0426, 0.0289), (1.8281, 0.4794, 0.6102)),
            P::DistortedView     => va(36.38, 41.67,  3.296, (1.1325, 1.8008), (0.0126, 0.0214, 0.0106), (0.7062, 0.4939, 0.5266)),
            P::ShortView         => va(37.53, 50.11,  4.732, (1.0251, 0.2621), (0.0669, 0.0615, 0.2435), (1.8094, 0.5019, 0.4766)),
            P::GroupEyeView      => va(28.13, 26.81,  2.838, (0.9504, 0.3843), (0.0169, 0.0010, 0.0338), (1.1303, 0.4657, 0.4483)),
            P::GroupEmphasisView => va(15.80, 24.13,  4.235, (0.7705, 0.7141), (0.0327, 0.0844, 0.0637), (1.8544, 0.4761, 0.4731)),
            P::FewSeriesView     => va(18.66, 38.29,  2.877, (0.2141, 0.5146), (0.0264, 0.1790, 0.1447), (1.6437, 0.5408, 0.4473)),
            P::FewGroupsView     => va(30.26, 52.62, 14.897, (4.7132, 2.3301), (0.1134, 0.0454, 0.1419), (1.7661, 0.4575, 0.4592)),
            P::DistortedStdView  => va(27.75, 48.70,  1.994, (1.0047, 0.4913), (0.0082, 0.0061, 0.0230), (0.7190, 0.5131, 0.3756)),
            P::ThickGroupsView   => va(14.41, 63.74,  4.736, (1.1002, 0.6058), (0.0329, 0.0905, 0.0385), (1.6898, 0.5551, 0.4622)),
            P::ShorterView       => va(39.18, 50.46,  1.909, (1.0055, 0.1534), (0.0148, 0.0054, 0.2663), (0.7176, 0.5308, 0.3771)),
            P::ThickSeriesView   => va(20.86, 16.33,  4.227, (0.9043, 0.5808), (0.0778, 0.0010, 0.0892), (1.8834, 0.4711, 0.4479)),
            P::ThickStdView      => va(30.69, 42.99,  6.572, (1.0102, 0.8338), (0.1081, 0.1003, 0.1733), (1.9709, 0.4868, 0.4636)),
            P::BirdsEyeView      => va(47.35, 24.63,  4.272, (0.7740, 0.6159), (0.0010, 0.0248, 0.0010), (1.7272, 0.4817, 0.5018)),
            P::MaxView           => va(29.49, 42.74,  5.110, (1.0531, 0.7831), (0.0188, 0.0130, 0.0271), (1.8566, 0.4930, 0.4733)),
        }
    }
}

/// One preset's whole room, in the order the table above reads: the camera (elevation, rotation, eye),
/// the room's `(depth, height)` proportions, its `(series wall, category wall, floor)` slab
/// thicknesses, and the `(scale, cx, cy)` viewport that places the projection in the chart box.
const fn va(
    elevation_deg: f64,
    rotation_deg: f64,
    eye: f64,
    proportions: (f64, f64),
    slabs: (f64, f64, f64),
    viewport: (f64, f64, f64),
) -> ViewAngle {
    ViewAngle {
        elevation_deg,
        rotation_deg,
        eye,
        depth_frac: proportions.0,
        height_frac: proportions.1,
        wall_thick_u: slabs.0,
        wall_thick_z: slabs.1,
        floor_thick: slabs.2,
        viewport_scale: viewport.0,
        viewport_cx: viewport.1,
        viewport_cy: viewport.2,
    }
}

/// How the scene's 3-D points map to page points.
#[derive(Debug, Clone, Copy)]
pub(super) enum Projection {
    /// Depth `z` shifts a point by a fixed offset toward the upper-right, with no perspective divide,
    /// so the mapping is affine and integer-exact at the box corners. Retained as a view-angle-inexact
    /// fallback (the renderers use [`Projection::Perspective`]); exercised only by the regression test.
    #[allow(dead_code)]
    Oblique { dx_per_z: f64, dy_per_z: f64 },
    /// The native pinhole transform: rotate by the view angle, then divide by eye-distance-minus-depth.
    Perspective(Perspective),
}

/// The native perspective projection. The scene box `[pl,pr]×[pt,pb]` (twips) with the
/// normalized depth `z ∈ [0,1]` is mapped into a commensurate box whose category extent `u` spans 1
/// and whose value and series extents are its `height_frac`/`depth_frac` fractions of that (the room
/// is flat and wide, not a cube), rotated by `rot` (elevation ⊗ rotation about a right-handed
/// graphics frame with the series axis negated so the near corner faces the viewer), divided by the
/// eye-distance-minus-depth term (`k = 3/2`), then placed in the **chart box** by the view angle's own
/// viewport.
///
/// The projection is NOT fit to a rectangle. The engine draws each preset's room at that preset's own
/// scale and centre, so a tall room really is drawn taller rather than shrunk back into a shared
/// frame, and nothing the label frame reserves may move or resize it. The one thing the chart box
/// contributes is its proportions: the x and y scales differ by exactly the box's aspect ratio, so the
/// engine is really projecting into a square and letting the box stretch it. That is why a room in a
/// wide chart box is drawn wide.
#[derive(Debug, Clone, Copy)]
pub(super) struct Perspective {
    pl: f64,
    pr: f64,
    pt: f64,
    pb: f64,
    /// The series (z) world-extent as a fraction of the category extent (a shallow, bounded recede).
    depth_frac: f64,
    /// The value (y) world-extent as a fraction of the category extent (a flat box, not a cube).
    height_frac: f64,
    rot: [[f64; 3]; 3],
    eye: f64,
    k: f64,
    /// The chart box's width and height, the projection's scale along each — they differ by exactly
    /// the box's aspect ratio.
    scale_x: f64,
    scale_y: f64,
    /// Where the projection's origin lands on the page.
    origin_x: f64,
    origin_y: f64,
}

/// Multiply two 3×3 matrices (`a·b`).
fn matmul(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

impl Perspective {
    fn new(scene: PlotBox, chart: PlotBox, va: ViewAngle) -> Self {
        let (pl, pr, pt, pb) = (scene.left, scene.right, scene.top, scene.bottom);
        let (e, r) = (va.elevation_deg.to_radians(), va.rotation_deg.to_radians());
        // Elevation about X, rotation about Y; the composed matrix orients the scene.
        let rx = [
            [1.0, 0.0, 0.0],
            [0.0, e.cos(), -e.sin()],
            [0.0, e.sin(), e.cos()],
        ];
        let ry = [
            [r.cos(), 0.0, r.sin()],
            [0.0, 1.0, 0.0],
            [-r.sin(), 0.0, r.cos()],
        ];
        let (cw, ch) = (
            (chart.right - chart.left).max(1.0),
            (chart.bottom - chart.top).max(1.0),
        );
        Perspective {
            pl,
            pr: pr.max(pl + 1.0),
            pt,
            pb: pb.max(pt + 1.0),
            depth_frac: va.depth_frac,
            height_frac: va.height_frac,
            rot: matmul(rx, ry),
            eye: va.eye,
            k: 1.5,
            scale_x: cw * va.viewport_scale,
            scale_y: ch * va.viewport_scale,
            origin_x: chart.left + cw * va.viewport_cx,
            origin_y: chart.top + ch * va.viewport_cy,
        }
    }

    /// Map a point into the unit cube, rotate, and apply the perspective divide — returning the
    /// viewport-relative screen `(sx, sy)` and the rotated depth `rz` (positive toward the viewer).
    fn raw(&self, p: Vec3) -> (f64, f64, f64) {
        // Commensurate box: category `u` spans 1; the series and value axes are its bounded fractions.
        let u = (p.x - self.pl) / (self.pr - self.pl) - 0.5;
        let w = (self.pb - p.y) / (self.pb - self.pt);
        let v = (p.z - 0.5) * self.depth_frac;
        // Right-handed graphics frame (x right, y up, z toward viewer); negate the series axis so both
        // horizontal axes recede from a near corner that points at the viewer.
        let (gx, gy, gz) = (u, (w - 0.5) * self.height_frac, -v);
        let rx = self.rot[0][0] * gx + self.rot[0][1] * gy + self.rot[0][2] * gz;
        let ry = self.rot[1][0] * gx + self.rot[1][1] * gy + self.rot[1][2] * gz;
        let rz = self.rot[2][0] * gx + self.rot[2][1] * gy + self.rot[2][2] * gz;
        // The divide's numerator is any constant: the viewport scale absorbs it, so only the shape the
        // denominator gives the projection survives.
        let denom = (self.eye - rz * self.k).max(1e-6);
        let s = self.k / denom;
        // Page-y grows downward, so negate the up-axis screen component.
        (rx * s, -ry * s, rz)
    }

    fn project(&self, p: Vec3) -> Point {
        let (sx, sy, _) = self.raw(p);
        Point {
            x: Twips((self.origin_x + sx * self.scale_x).round() as i32),
            y: Twips((self.origin_y + sy * self.scale_y).round() as i32),
        }
    }

    /// The painter-sort key: larger = farther back. Computed from the point's **floor** position only
    /// (category `u` and series `v`), dropping the value height — so risers sort by their grid cell's
    /// distance, not their height (a tall far bar must still draw behind a short near one). The rotated
    /// depth grows toward the viewer, so negate it (farthest gets the largest key, drawn first).
    fn depth(&self, p: Vec3) -> f64 {
        let u = (p.x - self.pl) / (self.pr - self.pl) - 0.5;
        let v = (p.z - 0.5) * self.depth_frac;
        let (gx, gz) = (u, -v);
        -(self.rot[2][0] * gx + self.rot[2][2] * gz)
    }
}

impl Projection {
    /// A projection whose full depth (`z = 1.0`) shifts a point right by `depth_dx` and up by
    /// `depth_dy` twips. For a shallow riser both are ~`bar_w * 0.4`. The retained oblique fallback.
    #[allow(dead_code)]
    pub(super) fn oblique(depth_dx: i32, depth_dy: i32) -> Self {
        Projection::Oblique {
            dx_per_z: depth_dx as f64,
            dy_per_z: depth_dy as f64,
        }
    }

    /// The native perspective projection of the `scene` box (the twip rectangle the renderers' `x`/`y`
    /// coordinates live in) at the view angle `va`, placed in the `chart` box by that view angle's own
    /// viewport. The scene box only normalizes coordinates, so nothing the label frame reserves can
    /// move or resize the room.
    pub(super) fn perspective(scene: PlotBox, chart: PlotBox, va: ViewAngle) -> Self {
        Projection::Perspective(Perspective::new(scene, chart, va))
    }

    /// Project a 3-D point to a 2-D page point.
    pub(super) fn project(&self, p: Vec3) -> Point {
        match self {
            Projection::Oblique { dx_per_z, dy_per_z } => Point {
                x: Twips((p.x + p.z * dx_per_z).round() as i32),
                y: Twips((p.y - p.z * dy_per_z).round() as i32),
            },
            Projection::Perspective(persp) => persp.project(p),
        }
    }

    /// The view-space depth of a point — the painter-sort key (larger = farther back, drawn first).
    /// For [`Projection::Oblique`] this is the raw `z`, which equals the mean corner `z` of a face, so
    /// oblique output is byte-identical to the pre-perspective mean-z sort.
    pub(super) fn depth(&self, p: Vec3) -> f64 {
        match self {
            Projection::Oblique { .. } => p.z,
            Projection::Perspective(persp) => persp.depth(p),
        }
    }
}

/// Black, one device pixel wide: the pen the engine outlines every filled chart face with, its data
/// faces exactly as its scenery.
pub(super) const FACE_INK: Color = Color {
    a: 255,
    r: 0,
    g: 0,
    b: 0,
};
pub(super) const HAIRLINE: Twips = Twips(15);

/// The outline every filled face and every gridline carries.
pub(super) fn face_edge() -> Stroke {
    Stroke {
        color: FACE_INK,
        width: HAIRLINE,
        style: LineStyle::Single,
    }
}

/// Project `corners` and build a filled (optionally stroked) polygon face, returning it alongside the
/// face's view-space depth (the projection's depth of the corner centroid) so the caller can
/// painter-sort faces back-to-front.
pub(super) fn face(
    proj: &Projection,
    corners: &[Vec3],
    fill: Color,
    edge: Option<Stroke>,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> (DrawOp, f64) {
    let points: Vec<Point> = corners.iter().map(|c| proj.project(*c)).collect();
    let depth = if corners.is_empty() {
        0.0
    } else {
        let n = corners.len() as f64;
        let centroid = Vec3 {
            x: corners.iter().map(|c| c.x).sum::<f64>() / n,
            y: corners.iter().map(|c| c.y).sum::<f64>() / n,
            z: corners.iter().map(|c| c.z).sum::<f64>() / n,
        };
        proj.depth(centroid)
    };
    let op = DrawOp::Polygon(PolygonOp {
        points,
        closed: true,
        fill: Some(fill.into()),
        stroke: edge,
        source: src(),
    });
    (op, depth)
}

/// Directional-lighting ladder: the top face keeps the base color, the front face is 0.8×, and the
/// shadowed side face 0.6×.
pub(super) const FRONT_SHADE: f32 = 0.8;
pub(super) const SIDE_SHADE: f32 = 0.6;

/// Shade `c` by `factor`: `factor > 1` lerps each channel toward white, `factor < 1` toward black,
/// `factor == 1` is the color unchanged. Alpha is preserved. Used to fake directional lighting on a
/// riser's faces (lit top, shadowed side).
pub(super) fn shade(c: Color, factor: f32) -> Color {
    let lerp = |v: u8| -> u8 {
        let vf = v as f32;
        let out = if factor >= 1.0 {
            vf + (255.0 - vf) * (factor - 1.0)
        } else {
            vf * factor
        };
        out.round().clamp(0.0, 255.0) as u8
    };
    Color {
        a: c.a,
        r: lerp(c.r),
        g: lerp(c.g),
        b: lerp(c.b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A projection whose scene box is also its chart box — the identity case the geometry tests use.
    fn persp(pl: f64, pr: f64, pt: f64, pb: f64, va: ViewAngle) -> Projection {
        let b = PlotBox::new(pl, pr, pt, pb);
        Projection::perspective(b, b, va)
    }

    #[test]
    fn oblique_project_applies_fixed_depth_offset() {
        let proj = Projection::oblique(40, 30);
        // z = 0 (front plane) is the identity.
        assert_eq!(
            proj.project(Vec3 {
                x: 100.0,
                y: 200.0,
                z: 0.0
            }),
            Point::new(100, 200)
        );
        // z = 1 (back wall) shifts right by dx and up by dy.
        assert_eq!(
            proj.project(Vec3 {
                x: 100.0,
                y: 200.0,
                z: 1.0
            }),
            Point::new(140, 170)
        );
    }

    #[test]
    fn oblique_depth_is_raw_z() {
        // The painter key for oblique is the raw z, so a face's centroid depth equals its mean corner
        // z — keeping oblique output byte-identical to the pre-perspective mean-z sort.
        let proj = Projection::oblique(40, 30);
        for z in [0.0, 0.25, 1.0] {
            assert_eq!(
                proj.depth(Vec3 {
                    x: 500.0,
                    y: 500.0,
                    z
                }),
                z
            );
        }
    }

    #[test]
    fn perspective_keeps_the_room_inside_the_chart_box() {
        // Every corner of the room lands inside the chart box: each preset's viewport places a whole
        // room, so no preset's projection can spill out of the object it belongs to. This is also the
        // anti-shear invariant — no coordinate is tied to the scene width, so nothing leans.
        use rpt_model::ChartViewAngle as P;
        let (pl, pr, pt, pb) = (1000.0, 6000.0, 500.0, 4000.0);
        for preset in [
            P::Standard,
            P::TallView,
            P::TopView,
            P::DistortedView,
            P::ShortView,
            P::GroupEyeView,
            P::GroupEmphasisView,
            P::FewSeriesView,
            P::FewGroupsView,
            P::DistortedStdView,
            P::ThickGroupsView,
            P::ShorterView,
            P::ThickSeriesView,
            P::ThickStdView,
            P::BirdsEyeView,
            P::MaxView,
        ] {
            let proj = persp(pl, pr, pt, pb, ViewAngle::for_preset(preset));
            for &x in &[pl, pr] {
                for &y in &[pt, pb] {
                    for &z in &[0.0_f64, 1.0] {
                        let p = proj.project(Vec3 { x, y, z });
                        assert!(
                            (pl.round() as i32..=pr.round() as i32).contains(&p.x.0)
                                && (pt.round() as i32..=pb.round() as i32).contains(&p.y.0),
                            "{preset:?} corner ({x},{y},{z}) projects to {p:?} inside the chart box"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn perspective_centre_is_near_plot_centre() {
        // The cube centre (mid-category, mid-value, mid-depth) sits at the graphics origin, so it
        // projects near the plot centre (offset only by the mild perspective asymmetry, well under a
        // tenth of the plot span).
        let (pl, pr, pt, pb) = (1000.0, 6000.0, 500.0, 4000.0);
        let proj = persp(pl, pr, pt, pb, ViewAngle::DEPTH_EFFECT);
        let mid = proj.project(Vec3 {
            x: (pl + pr) / 2.0,
            y: (pt + pb) / 2.0,
            z: 0.5,
        });
        let (cx, cy) = (((pl + pr) / 2.0) as i32, ((pt + pb) / 2.0) as i32);
        assert!(
            (mid.x.0 - cx).abs() < ((pr - pl) / 10.0) as i32,
            "near cx: {mid:?}"
        );
        assert!(
            (mid.y.0 - cy).abs() < ((pb - pt) / 10.0) as i32,
            "near cy: {mid:?}"
        );
    }

    #[test]
    fn perspective_divide_matches_the_pinhole_formula() {
        // With a head-on view (no elevation/rotation) the pre-fit transform reduces to the bare
        // perspective divide: sx = u·k/(eye − z·k), k = 3/2 — where `u` is the normalized category
        // offset and `z` the rotated depth (here −v for a head-on view).
        let va = ViewAngle {
            elevation_deg: 0.0,
            rotation_deg: 0.0,
            eye: 4.0,
            depth_frac: 1.0,
            height_frac: 1.0,
            wall_thick_u: 0.06,
            wall_thick_z: 0.06,
            floor_thick: 0.13,
            viewport_scale: 1.0,
            viewport_cx: 0.5,
            viewport_cy: 0.5,
        };
        let b = PlotBox::new(0.0, 1000.0, 0.0, 1000.0);
        let persp = Perspective::new(b, b, va);
        // The far-right, mid-value, back-plane corner: u = +0.5, w−0.5 = 0, v = +0.5 → rotated z = −0.5.
        let (sx, _sy, rz) = persp.raw(Vec3 {
            x: 1000.0,
            y: 500.0,
            z: 1.0,
        });
        assert!((rz - (-0.5)).abs() < 1e-9, "head-on rotated depth is −v");
        let k = 1.5;
        let s = k / (4.0 - rz * k);
        assert!((sx - 0.5 * s).abs() < 1e-9, "sx = u·s: {sx} vs {}", 0.5 * s);
    }

    #[test]
    fn perspective_corner_orientation() {
        // The corner room: the near floor corner (category-min, series-front) sits lower on the page
        // than the back corner; category recedes up-right and series recedes up-left, so both floor
        // axes climb from a near corner that points at the viewer.
        let (pl, pr, pt, pb) = (0.0, 6000.0, 0.0, 4000.0);
        let proj = persp(pl, pr, pt, pb, ViewAngle::DEPTH_EFFECT);
        let corner = |x: f64, z: f64| proj.project(Vec3 { x, y: pb, z });
        let (near, far) = (corner(pl, 0.0), corner(pr, 1.0));
        assert!(
            near.y.0 > far.y.0,
            "near corner lower than the back corner: {near:?} {far:?}"
        );
        // Category axis (x, series-front): to the right and up the page.
        let (c0, c1) = (corner(pl, 0.0), corner(pr, 0.0));
        assert!(
            c1.x.0 > c0.x.0 && c1.y.0 < c0.y.0,
            "category recedes up-right"
        );
        // Series axis (z, category-front): to the left and up the page.
        let (s0, s1) = (corner(pl, 0.0), corner(pl, 1.0));
        assert!(s1.x.0 < s0.x.0 && s1.y.0 < s0.y.0, "series recedes up-left");
    }

    #[test]
    fn perspective_left_bar_face_is_viewer_facing() {
        // In the corner view a riser's viewer-facing vertical side is its LEFT face (smaller x): its
        // outward normal (−x) points at the viewer, so nudging it outward moves it nearer (smaller
        // depth), while the right face (+x) faces away (nudging outward moves it farther). This is why
        // the riser draws the x0 face and culls the x1 one.
        let proj = persp(0.0, 6000.0, 0.0, 4000.0, ViewAngle::DEPTH_EFFECT);
        let d = |x: f64| {
            proj.depth(Vec3 {
                x,
                y: 2000.0,
                z: 0.5,
            })
        };
        assert!(
            d(1990.0) < d(2000.0),
            "left face (−x) outward is nearer the viewer"
        );
        assert!(
            d(3010.0) > d(3000.0),
            "right face (+x) outward is farther from the viewer"
        );
    }

    #[test]
    fn perspective_depth_grows_toward_the_back() {
        // The painter key increases toward the back (larger = farther, drawn first), so a point at the
        // far-series plane (z = 1) sorts before the near one (z = 0) at the same category/value.
        let proj = persp(0.0, 1000.0, 0.0, 1000.0, ViewAngle::DEPTH_EFFECT);
        let near = proj.depth(Vec3 {
            x: 500.0,
            y: 500.0,
            z: 0.0,
        });
        let far = proj.depth(Vec3 {
            x: 500.0,
            y: 500.0,
            z: 1.0,
        });
        assert!(
            far > near,
            "back (z=1) sorts before front (z=0): {far} > {near}"
        );
    }

    #[test]
    fn standard_preset_centres_the_near_floor_corner() {
        // The Standard preset's rotation places the near floor corner (category-min, series-front)
        // close under the plot centre horizontally. Tolerance is a small fraction of the plot width.
        let (pl, pr, pt, pb) = (1000.0, 6000.0, 500.0, 4000.0);
        let proj = persp(pl, pr, pt, pb, ViewAngle::DEPTH_EFFECT);
        let cx = ((pl + pr) / 2.0) as i32;
        let near = proj.project(Vec3 {
            x: pl,
            y: pb,
            z: 0.0,
        });
        assert!(
            (near.x.0 - cx).abs() < ((pr - pl) / 20.0) as i32,
            "near corner {near:?} within 5% of plot centre x={cx}"
        );
    }

    #[test]
    fn for_preset_standard_is_depth_effect_and_presets_differ() {
        use rpt_model::ChartViewAngle as P;
        // Standard resolves to the shared default/fallback preset.
        let s = ViewAngle::for_preset(P::Standard);
        assert_eq!(s.elevation_deg, ViewAngle::DEPTH_EFFECT.elevation_deg);
        assert_eq!(s.rotation_deg, ViewAngle::DEPTH_EFFECT.rotation_deg);
        assert_eq!(s.depth_frac, ViewAngle::DEPTH_EFFECT.depth_frac);
        // TopView is decoded as a near-overhead angle (much steeper elevation than Standard).
        let top = ViewAngle::for_preset(P::TopView);
        assert!(
            top.elevation_deg > s.elevation_deg + 20.0,
            "TopView is much higher elevation: {} vs {}",
            top.elevation_deg,
            s.elevation_deg
        );
        // FewSeriesView compresses the depth (series) axis to a fifth of the category extent, the
        // opposite of what its name suggests; FewGroupsView stretches it to nearly five times.
        assert!(ViewAngle::for_preset(P::FewSeriesView).depth_frac < 0.5);
        assert!(ViewAngle::for_preset(P::FewGroupsView).depth_frac > 3.0);
        // A preset carries its own room, not just its own orientation: the thick presets' walls are
        // far heavier than the thin ones', and BirdsEyeView's are thin enough to vanish.
        assert!(
            ViewAngle::for_preset(P::ThickStdView).wall_thick_u
                > 10.0 * ViewAngle::for_preset(P::BirdsEyeView).wall_thick_u
        );
    }

    #[test]
    fn shade_lightens_above_one_and_darkens_below() {
        let base = Color {
            a: 200,
            r: 100,
            g: 100,
            b: 100,
        };
        assert_eq!(shade(base, 1.0), base, "factor 1.0 is identity");
        let lighter = shade(base, 1.25);
        let darker = shade(base, 0.8);
        assert!(lighter.r > base.r, "factor>1 lightens");
        assert!(darker.r < base.r, "factor<1 darkens");
        assert_eq!(lighter.a, 200, "alpha preserved when lightening");
        assert_eq!(darker.a, 200, "alpha preserved when darkening");
    }

    #[test]
    fn face_returns_centroid_depth() {
        let proj = Projection::oblique(40, 30);
        let corners = [
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Vec3 {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            Vec3 {
                x: 10.0,
                y: 0.0,
                z: 1.0,
            },
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        ];
        let (op, z) = face(&proj, &corners, Color::WHITE, None, &|| None);
        assert!(matches!(op, DrawOp::Polygon(_)), "emits a polygon face");
        assert!(
            (z - 0.5).abs() < 1e-9,
            "centroid depth is the mean corner z"
        );
    }
}
