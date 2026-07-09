//! Projection of a 3-D chart scene to the 2-D page. Two variants share one interface: [`Projection::
//! Oblique`] is a fixed depth offset with no perspective divide (a cheap, view-angle-inexact
//! fallback), and [`Projection::Perspective`] is the real pinhole transform the native engine uses —
//! a view-angle rotation followed by a perspective divide, then a fit into the plot
//! box so no axis is tied to the plot width. Faces are emitted as filled [`rpt_pages::DrawOp::Polygon`]s
//! and painter-sorted back-to-front by their floor-plane depth (value-independent, so risers sort by
//! grid cell, not height).

use rpt_model::{Color, Twips};
use rpt_pages::{DrawOp, ObjectRef, Point, PolygonOp, Stroke};

/// A point in the chart's 3-D space: `x`/`y` in plot-twip units (the 2-D plot plane, `y` growing
/// downward as in the page), `z` the normalized depth — `0.0` = the front plane nearest the viewer,
/// `1.0` = the back wall.
#[derive(Debug, Clone, Copy)]
pub(super) struct Vec3 {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) z: f64,
}

/// A view angle: the elevation (about X) and rotation (about Y) that orient the scene, plus the
/// pinhole `eye`/`plane` distances that drive the perspective divide. The native engine picks one of
/// 16 presets per chart ([`rpt_model::ChartViewAngle`]); [`ViewAngle::for_preset`]
/// maps each to its concrete angle. Only `Standard` ([`ViewAngle::DEPTH_EFFECT`]) is currently
/// resolved from the stored bytes; other presets fall back to it.
#[derive(Debug, Clone, Copy)]
pub(super) struct ViewAngle {
    pub(super) elevation_deg: f64,
    pub(super) rotation_deg: f64,
    pub(super) eye: f64,
    pub(super) plane: f64,
    /// The scene's depth (z) world-extent as a fraction of the plot width. The z axis carries only a
    /// modest recede (a shallow "room"), so it is bounded here rather than tied to the full plot scale.
    pub(super) depth_frac: f64,
    /// The projected scene is scaled to fill this fraction of the plot box (a margin so the room's
    /// corners never touch the plot edge).
    pub(super) fit_frac: f64,
}

impl ViewAngle {
    /// The engine's default 3-D preset, and the fallback for any preset whose disk selector isn't
    /// decoded: elevation 36.1° about X, rotation 42.1° about Y, and a square floor
    /// (`depth_frac = 1.0`). Together these centre the near floor corner under the plot centre,
    /// matching the engine.
    pub(super) const DEPTH_EFFECT: ViewAngle = ViewAngle {
        elevation_deg: 36.1,
        rotation_deg: 42.1,
        eye: 4.5,
        plane: 1.5,
        depth_frac: 1.0,
        fit_frac: 0.9,
    };

    /// The concrete view angle for a model [`ChartViewAngle`] preset. Elevation, rotation, and the
    /// floor `depth_frac` are the native engine's per-preset angles; `eye`/`plane`
    /// and `fit_frac` are shared across presets. Only `Standard`'s disk selector is currently decoded,
    /// so a chart requesting a non-default angle still resolves to `Standard`.
    pub(super) fn for_preset(preset: rpt_model::ChartViewAngle) -> ViewAngle {
        use rpt_model::ChartViewAngle as P;
        // Shared reconstruction constants across all presets.
        const fn va(elevation_deg: f64, rotation_deg: f64, depth_frac: f64) -> ViewAngle {
            ViewAngle {
                elevation_deg,
                rotation_deg,
                eye: 4.5,
                plane: 1.5,
                depth_frac,
                fit_frac: 0.9,
            }
        }
        match preset {
            P::Standard => ViewAngle::DEPTH_EFFECT,
            P::TallView => va(45.0, 38.0, 0.95),
            P::TopView => va(80.5, 46.0, 0.99),
            P::DistortedView => va(36.5, 47.0, 0.90),
            P::ShortView => va(38.0, 39.0, 1.00),
            P::GroupEyeView => va(28.5, 65.0, 1.00),
            P::GroupEmphasisView => va(16.0, 65.0, 1.29),
            P::FewSeriesView => va(20.5, 49.0, 4.96),
            P::FewGroupsView => va(31.0, 35.0, 0.22),
            P::DistortedStdView => va(28.0, 42.0, 1.00),
            P::ThickGroupsView => va(15.0, 20.0, 0.93),
            P::ShorterView => va(41.0, 39.0, 1.00),
            P::ThickSeriesView => va(21.5, 85.0, 1.04),
            P::ThickStdView => va(32.0, 47.0, 1.00),
            P::BirdsEyeView => va(48.0, 63.0, 1.35),
            P::MaxView => va(30.0, 47.0, 0.95),
        }
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

/// The native perspective projection. The plot box `[pl,pr]×[pt,pb]` (twips) with the
/// normalized depth `z ∈ [0,1]` is mapped into a commensurate unit cube (category `u`, value `w`,
/// series `v = (z−½)·depth_frac`, all in the same range), rotated by `rot` (elevation ⊗ rotation about
/// a right-handed graphics frame with the series axis negated so the near corner faces the viewer),
/// divided by the eye-distance-minus-depth term (`k = 3/2`), then fit into the plot box
/// by a single scale so no axis is tied to the plot width (which is what over-shears an affine mapping).
#[derive(Debug, Clone, Copy)]
pub(super) struct Perspective {
    pl: f64,
    pr: f64,
    pt: f64,
    pb: f64,
    /// The series (z) world-extent as a fraction of the category extent (a shallow, bounded recede).
    depth_frac: f64,
    rot: [[f64; 3]; 3],
    eye: f64,
    plane: f64,
    k: f64,
    /// The single fit scale and the pre-fit projected-bbox centre it is applied about.
    scale: f64,
    mx: f64,
    my: f64,
    plot_cx: f64,
    plot_cy: f64,
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
    fn new(pl: f64, pr: f64, pt: f64, pb: f64, va: ViewAngle) -> Self {
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
        let mut p = Perspective {
            pl,
            pr: pr.max(pl + 1.0),
            pt,
            pb: pb.max(pt + 1.0),
            depth_frac: va.depth_frac,
            rot: matmul(rx, ry),
            eye: va.eye,
            plane: va.plane,
            k: 1.5,
            scale: 1.0,
            mx: 0.0,
            my: 0.0,
            plot_cx: (pl + pr) / 2.0,
            plot_cy: (pt + pb) / 2.0,
        };
        // Fit the projected scene (its eight cube corners) into the plot box with one scale.
        let (mut sx0, mut sy0, mut sx1, mut sy1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &x in &[p.pl, p.pr] {
            for &y in &[p.pt, p.pb] {
                for &z in &[0.0_f64, 1.0] {
                    let (sx, sy, _) = p.raw(Vec3 { x, y, z });
                    sx0 = sx0.min(sx);
                    sy0 = sy0.min(sy);
                    sx1 = sx1.max(sx);
                    sy1 = sy1.max(sy);
                }
            }
        }
        p.mx = (sx0 + sx1) / 2.0;
        p.my = (sy0 + sy1) / 2.0;
        let (bw, bh) = ((sx1 - sx0).max(1e-6), (sy1 - sy0).max(1e-6));
        let (plot_w, plot_h) = (p.pr - p.pl, p.pb - p.pt);
        p.scale = va.fit_frac * (plot_w / bw).min(plot_h / bh);
        p
    }

    /// Map a point into the unit cube, rotate, and apply the perspective divide — returning the pre-fit
    /// screen `(sx, sy)` and the rotated depth `rz` (positive toward the viewer). The fit scale is not
    /// applied here so [`Perspective::new`] can bound the raw projection.
    fn raw(&self, p: Vec3) -> (f64, f64, f64) {
        // Commensurate cube: category `u` and value `w` span 1; series `v` is the bounded fraction.
        let u = (p.x - self.pl) / (self.pr - self.pl) - 0.5;
        let w = (self.pb - p.y) / (self.pb - self.pt);
        let v = (p.z - 0.5) * self.depth_frac;
        // Right-handed graphics frame (x right, y up, z toward viewer); negate the series axis so both
        // horizontal axes recede from a near corner that points at the viewer.
        let (gx, gy, gz) = (u, w - 0.5, -v);
        let rx = self.rot[0][0] * gx + self.rot[0][1] * gy + self.rot[0][2] * gz;
        let ry = self.rot[1][0] * gx + self.rot[1][1] * gy + self.rot[1][2] * gz;
        let rz = self.rot[2][0] * gx + self.rot[2][1] * gy + self.rot[2][2] * gz;
        let denom = (self.eye - rz * self.k).max(1e-6);
        let s = self.k * (self.eye - self.plane) / denom;
        // Page-y grows downward, so negate the up-axis screen component.
        (rx * s, -ry * s, rz)
    }

    fn project(&self, p: Vec3) -> Point {
        let (sx, sy, _) = self.raw(p);
        Point {
            x: Twips((self.plot_cx + (sx - self.mx) * self.scale).round() as i32),
            y: Twips((self.plot_cy + (sy - self.my) * self.scale).round() as i32),
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

    /// The native perspective projection over the plot box `[pl,pr]×[pt,pb]` (twips) at the view angle
    /// `va`. The projected scene is fit into the box, so no coordinate leaks the plot width into the
    /// depth axis.
    pub(super) fn perspective(pl: f64, pr: f64, pt: f64, pb: f64, va: ViewAngle) -> Self {
        Projection::Perspective(Perspective::new(pl, pr, pt, pb, va))
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

/// Shade `c` by `factor`: `factor > 1` lerps each channel toward white, `factor < 1` toward black,
/// `factor == 1` is the colour unchanged. Alpha is preserved. Used to fake directional lighting on a
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
    fn perspective_fits_scene_within_plot_box() {
        // The fit-to-bbox step guarantees the whole projected scene stays inside the plot box — this is
        // the anti-shear invariant: no coordinate is tied to the plot width, so nothing leans off-plot.
        let (pl, pr, pt, pb) = (1000.0, 6000.0, 500.0, 4000.0);
        let proj = Projection::perspective(pl, pr, pt, pb, ViewAngle::DEPTH_EFFECT);
        for &x in &[pl, pr] {
            for &y in &[pt, pb] {
                for &z in &[0.0_f64, 1.0] {
                    let p = proj.project(Vec3 { x, y, z });
                    assert!(
                        (pl.round() as i32..=pr.round() as i32).contains(&p.x.0)
                            && (pt.round() as i32..=pb.round() as i32).contains(&p.y.0),
                        "corner ({x},{y},{z}) projects to {p:?} inside the plot box"
                    );
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
        let proj = Projection::perspective(pl, pr, pt, pb, ViewAngle::DEPTH_EFFECT);
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
        // perspective divide: sx = u·k·(eye−plane)/(eye − z·k), k = 3/2 — where
        // `u` is the normalized category offset and `z` the rotated depth (here −v for a head-on view).
        let va = ViewAngle {
            elevation_deg: 0.0,
            rotation_deg: 0.0,
            eye: 4.0,
            plane: 1.0,
            depth_frac: 1.0,
            fit_frac: 1.0,
        };
        let persp = Perspective::new(0.0, 1000.0, 0.0, 1000.0, va);
        // The far-right, mid-value, back-plane corner: u = +0.5, w−0.5 = 0, v = +0.5 → rotated z = −0.5.
        let (sx, _sy, rz) = persp.raw(Vec3 {
            x: 1000.0,
            y: 500.0,
            z: 1.0,
        });
        assert!((rz - (-0.5)).abs() < 1e-9, "head-on rotated depth is −v");
        let k = 1.5;
        let s = k * (4.0 - 1.0) / (4.0 - rz * k);
        assert!((sx - 0.5 * s).abs() < 1e-9, "sx = u·s: {sx} vs {}", 0.5 * s);
    }

    #[test]
    fn perspective_corner_orientation() {
        // The corner room: the near floor corner (category-min, series-front) sits lower on the page
        // than the back corner; category recedes up-right and series recedes up-left, so both floor
        // axes climb from a near corner that points at the viewer.
        let (pl, pr, pt, pb) = (0.0, 6000.0, 0.0, 4000.0);
        let proj = Projection::perspective(pl, pr, pt, pb, ViewAngle::DEPTH_EFFECT);
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
        let proj = Projection::perspective(0.0, 6000.0, 0.0, 4000.0, ViewAngle::DEPTH_EFFECT);
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
        let proj = Projection::perspective(0.0, 1000.0, 0.0, 1000.0, ViewAngle::DEPTH_EFFECT);
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
        // The Standard preset (elevation 36.1°, rotation 42.1°, square floor) places the
        // near floor corner (category-min, series-front) close under the plot centre horizontally.
        // Tolerance is a small fraction of the plot width.
        let (pl, pr, pt, pb) = (1000.0, 6000.0, 500.0, 4000.0);
        let proj = Projection::perspective(pl, pr, pt, pb, ViewAngle::DEPTH_EFFECT);
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
        // The two distortion presets change the floor depth in opposite directions from square.
        assert!(ViewAngle::for_preset(P::FewSeriesView).depth_frac > 2.0);
        assert!(ViewAngle::for_preset(P::FewGroupsView).depth_frac < 0.5);
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
