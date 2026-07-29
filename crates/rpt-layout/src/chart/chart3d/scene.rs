//! Shared scene assembly for the 3-D chart families. Every 3-D renderer paints the same corner room —
//! a floor plate and two far-edge back walls meeting at a back vertical edge, wrapped by the value
//! gridlines ([`axes_3d`]) — behind its data, then globally painter-sorts its data faces back-to-front
//! and draws its labels last. This module owns that common skeleton so the riser, surface, and
//! area-ribbon renderers stay geometry-only.
//!
//! The room's construction follows the engine's: each of the three planes is a **slab**, so the two of
//! its side faces that turn toward the viewer are drawn alongside its main face — nine filled faces in
//! all, every one outlined in black.

use super::projection::{face, face_edge, p3, PlotBox, Projection, Vec3, ViewAngle};
use crate::chart::common::{
    compute_frame, fmt_val, nice_scale, AxisTitles, ChartStyle, Frame, LABEL,
};
use rpt_model::{Color, Rect, Twips};
use rpt_pages::{DrawOp, FontSpec, LineOp, ObjectRef, TextAlign, TextRun};

/// The engine's three scenery greys, picked by the face's orientation rather than by which plane it
/// belongs to: an up-facing plane (the floor's top, both wall tops) is white, a face normal to the
/// category axis is the light grey, and a face normal to the series axis is the dark grey.
const FACE_UP: Color = grey(0xff);
const FACE_CATEGORY: Color = grey(0xcc);
const FACE_SERIES: Color = grey(0x99);

/// An opaque grey.
const fn grey(v: u8) -> Color {
    Color {
        a: 255,
        r: v,
        g: v,
        b: v,
    }
}

/// How many filled faces the room contributes ahead of any data: three slabs of three faces each.
pub(super) const ROOM_FACES: usize = 9;

/// The chart box, as the projection's viewport rectangle. The engine's room is placed and sized off
/// the **chart box** alone: its nine faces land on the same coordinates whatever the family, the
/// value-tick count, the category labels or the data, so nothing the label frame reserves may move or
/// shrink it.
fn chart_box(rect: Rect) -> PlotBox {
    let (l, t) = (rect.left.0 as f64, rect.top.0 as f64);
    PlotBox::new(l, l + rect.width.0 as f64, t, t + rect.height.0 as f64)
}

/// The shared 3-D scene preamble every family draws before its data faces: builds the Num+Ord frame
/// off a synthetic per-category max series (so the tallest mark never touches the ceiling), the
/// perspective projection — scene coordinates from the frame, placed in the chart box by the view
/// angle's own viewport — and the assembled background scenery (the room's nine faces plus the value
/// gridlines). Returns `(frame, value-scale max, projection, background ops, axis labels)`; each
/// renderer then emits only its own geometry.
pub(super) fn setup_3d(
    style: ChartStyle,
    rect: Rect,
    title: &str,
    categories: &[String],
    series: &[(String, Vec<f64>)],
    view_angle: rpt_model::ChartViewAngle,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> (Frame, f64, Projection, Vec<DrawOp>, Vec<DrawOp>) {
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
    let f = compute_frame(style, rect, title, AxisTitles::default(), &frame_series);
    let (pl, pr, pt, pb) = (
        f.plot_left as f64,
        f.plot_right as f64,
        f.plot_top() as f64,
        f.plot_bottom as f64,
    );
    let va = ViewAngle::for_preset(view_angle);
    // The frame supplies the scene's coordinate space; the chart box supplies where the room lands.
    let proj = Projection::perspective(PlotBox::new(pl, pr, pt, pb), chart_box(rect), va);
    let (grid, axis_labels) = axes_3d(style, &proj, va, &f, categories, src);
    let mut background = room(&proj, va, pl, pr, pt, pb, src);
    background.extend(grid);
    (f, max_val, proj, background, axis_labels)
}

/// The room's slab thicknesses over the plot box `[pl,pr]×[pt,pb]`: the category wall's inset along the
/// normalized depth, the series wall's inset along `x` (twips), and the floor plate's depth along `y`.
/// Each is the view angle's own fraction of the extent it eats into — a preset carries its own wall
/// weight, and the thinnest presets' walls all but disappear.
fn thickness(va: ViewAngle, pl: f64, pr: f64, pt: f64, pb: f64) -> (f64, f64, f64) {
    (
        va.wall_thick_z,
        va.wall_thick_u * (pr - pl),
        va.floor_thick * (pb - pt),
    )
}

/// Build the room for a 3-D scene over the plot rectangle `[pl,pr]×[pt,pb]` (plot-twip units): the
/// floor plate, the category back wall (`z = 1`) and the series back wall (`x = pr`), in that draw
/// order (behind all data). The two walls meet at the back vertical edge (`x = pr, z = 1`) so the near
/// floor corner points at the viewer; the series wall's inner face and top stop at the category wall's
/// inner face, mitring the corner. Each plane is a slab, so it contributes its main face plus the two
/// side faces that turn toward the viewer. Shared by every 3-D family so the scenery is identical.
fn room(
    proj: &Projection,
    va: ViewAngle,
    pl: f64,
    pr: f64,
    pt: f64,
    pb: f64,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> Vec<DrawOp> {
    let (tz, tx, ty) = thickness(va, pl, pr, pt, pb);
    let (zi, xi, yl) = (1.0 - tz, pr - tx, pb + ty); // wall inner faces; floor plate underside
    let edge = Some(face_edge());
    let plane = |corners: [Vec3; 4], fill: Color| face(proj, &corners, fill, edge, src).0;
    let faces = vec![
        // Floor plate: the near-left lip, the top, and the near-right lip.
        plane(
            [
                p3(pl, yl, 1.0),
                p3(pl, yl, 0.0),
                p3(pl, pb, 0.0),
                p3(pl, pb, 1.0),
            ],
            FACE_CATEGORY,
        ),
        plane(
            [
                p3(pl, pb, 0.0),
                p3(pr, pb, 0.0),
                p3(pr, pb, 1.0),
                p3(pl, pb, 1.0),
            ],
            FACE_UP,
        ),
        plane(
            [
                p3(pl, pb, 0.0),
                p3(pr, pb, 0.0),
                p3(pr, yl, 0.0),
                p3(pl, yl, 0.0),
            ],
            FACE_SERIES,
        ),
        // Category wall: its near end-cap, its top, and its inner face.
        plane(
            [
                p3(pl, pb, 1.0),
                p3(pl, pb, zi),
                p3(pl, pt, zi),
                p3(pl, pt, 1.0),
            ],
            FACE_CATEGORY,
        ),
        plane(
            [
                p3(pl, pt, 1.0),
                p3(pl, pt, zi),
                p3(pr, pt, zi),
                p3(pr, pt, 1.0),
            ],
            FACE_UP,
        ),
        plane(
            [
                p3(pl, pb, zi),
                p3(pr, pb, zi),
                p3(pr, pt, zi),
                p3(pl, pt, zi),
            ],
            FACE_SERIES,
        ),
        // Series wall: its inner face, its top, and its near end-cap.
        plane(
            [
                p3(xi, pb, zi),
                p3(xi, pb, 0.0),
                p3(xi, pt, 0.0),
                p3(xi, pt, zi),
            ],
            FACE_CATEGORY,
        ),
        plane(
            [
                p3(xi, pt, zi),
                p3(xi, pt, 0.0),
                p3(pr, pt, 0.0),
                p3(pr, pt, zi),
            ],
            FACE_UP,
        ),
        plane(
            [
                p3(xi, pb, 0.0),
                p3(pr, pb, 0.0),
                p3(pr, pt, 0.0),
                p3(xi, pt, 0.0),
            ],
            FACE_SERIES,
        ),
    ];
    debug_assert_eq!(faces.len(), ROOM_FACES);
    faces
}

/// A small label-font axis label in the box `[left, left+width]` at `top`, aligned within it. Used
/// for the value-tick labels on the walls and the category labels on the floor.
fn axis_text(
    style: ChartStyle,
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
            size_pt: style.scaled_pt(7.0),
            ..Default::default()
        },
        color: LABEL,
        align,
        rotation: 0.0,
        metrics: None,
        character_spacing: Twips(0),
        source: src(),
    })
}

/// A category label rotated 45° into `bounds`, right-aligned so its text ends on the tick the box was
/// placed against — the 3-D floor axis's form of the flat families' rotated category label.
fn rotated_axis_text(
    style: ChartStyle,
    bounds: Rect,
    text: &str,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> DrawOp {
    DrawOp::Text(TextRun {
        bounds,
        text: text.to_string(),
        font: FontSpec {
            family: "Arial".into(),
            size_pt: style.scaled_pt(7.0),
            ..Default::default()
        },
        color: LABEL,
        align: TextAlign::Right,
        rotation: 45.0,
        metrics: None,
        character_spacing: Twips(0),
        source: src(),
    })
}

/// Draw the 3-D value axes and category labels for the corner room: horizontal value gridlines wrapping
/// both back walls — across each wall's inner face and on around its near end-cap — with a tick label
/// off each wall's outer edge (the engine's twin value columns), and the category labels stepping along
/// the front floor edge. Returns `(gridlines, labels)` — the gridlines belong behind the data (drawn
/// with the background), the labels on top. Value ticks come from `f`'s scale; categories are centred
/// in their floor slots.
pub(super) fn axes_3d(
    style: ChartStyle,
    proj: &Projection,
    va: ViewAngle,
    f: &Frame,
    categories: &[String],
    src: &dyn Fn() -> Option<ObjectRef>,
) -> (Vec<DrawOp>, Vec<DrawOp>) {
    const LBL_W: i32 = 780;
    let pl = f.plot_left as f64;
    let pr = f.plot_right as f64;
    let pt = f.plot_top() as f64;
    let pb = f.plot_bottom as f64;
    let plot_h = f.plot_h as f64;
    let (tz, tx, _) = thickness(va, pl, pr, pt, pb);
    let (zi, xi) = (1.0 - tz, pr - tx);
    let (max_val, step) = (f.max_val, f.step);
    let ticks = if step > 0.0 {
        (max_val / step).round() as i32
    } else {
        0
    };

    let mut grid: Vec<DrawOp> = Vec::new();
    let mut labels: Vec<DrawOp> = Vec::new();
    let gline = |a: Vec3, b: Vec3| {
        DrawOp::Line(LineOp {
            from: proj.project(a),
            to: proj.project(b),
            stroke: face_edge(),
            source: src(),
        })
    };

    for t in 0..=ticks {
        let y = pb - (t as f64 * step / max_val.max(1e-9)) * plot_h;
        let text = fmt_val(t as f64 * step);
        // Category back wall: a gridline across its inner face, continuing over its near end-cap; the
        // tick label hugs the cap's outer edge.
        grid.push(gline(p3(pl, y, zi), p3(pr, y, zi)));
        grid.push(gline(p3(pl, y, 1.0), p3(pl, y, zi)));
        let l1 = proj.project(p3(pl, y, 1.0));
        labels.push(axis_text(
            style,
            l1.x.0 - LBL_W - 40,
            l1.y.0 - 100,
            LBL_W,
            TextAlign::Right,
            &text,
            src,
        ));
        // Series back wall: a gridline receding front-to-back across its inner face and over its near
        // end-cap; the tick label hugs the cap's outer edge on the right.
        grid.push(gline(p3(xi, y, zi), p3(xi, y, 0.0)));
        grid.push(gline(p3(xi, y, 0.0), p3(pr, y, 0.0)));
        let f0 = proj.project(p3(pr, y, 0.0));
        labels.push(axis_text(
            style,
            f0.x.0 + 40,
            f0.y.0 - 100,
            LBL_W,
            TextAlign::Left,
            &text,
            src,
        ));
    }

    // Category labels along the front floor edge (z = 0), thinned and rotated exactly as the flat
    // families' axis: centred under each slot while they fit it, else every `stride`-th label rotated
    // 45° so its text ends on the floor edge.
    let slot = f.slot;
    for (c, label) in categories.iter().enumerate() {
        if !c.is_multiple_of(f.cats.stride) {
            continue;
        }
        let cx = (f.plot_left + c as i32 * slot + slot / 2) as f64;
        let a = proj.project(p3(cx, pb, 0.0));
        if f.cats.rotated {
            let b = f.cats.rotated_box(a.x.0, a.y.0 + 40);
            labels.push(rotated_axis_text(style, b, label, src));
        } else {
            labels.push(axis_text(
                style,
                a.x.0 - LBL_W / 2,
                a.y.0 + 40,
                LBL_W,
                TextAlign::Center,
                label,
                src,
            ));
        }
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
    use super::super::projection::{ViewAngle, FACE_INK};
    use super::*;

    fn proj() -> Projection {
        let b = PlotBox::new(0.0, 6000.0, 0.0, 4000.0);
        Projection::perspective(b, b, ViewAngle::DEPTH_EFFECT)
    }

    #[test]
    fn room_is_nine_outlined_faces_in_the_engine_greys() {
        // Three slabs of three faces each: main face plus the two side faces turned toward the viewer.
        let ops = room(
            &proj(),
            ViewAngle::DEPTH_EFFECT,
            0.0,
            6000.0,
            0.0,
            4000.0,
            &|| None,
        );
        assert_eq!(ops.len(), 9, "nine scenery faces, got {}", ops.len());
        let fills: Vec<Color> = ops
            .iter()
            .map(|o| match o {
                DrawOp::Polygon(p) => match p.fill.as_ref().expect("scenery faces are filled") {
                    rpt_pages::Fill::Solid(c) => *c,
                    other => panic!("scenery faces are solid, got {other:?}"),
                },
                other => panic!("scenery faces are polygons, got {other:?}"),
            })
            .collect();
        // Each slab contributes one category-normal, one up-facing and one series-normal face.
        assert_eq!(
            fills,
            [
                FACE_CATEGORY,
                FACE_UP,
                FACE_SERIES,
                FACE_CATEGORY,
                FACE_UP,
                FACE_SERIES,
                FACE_CATEGORY,
                FACE_UP,
                FACE_SERIES,
            ]
        );
        assert!(
            ops.iter().all(|o| matches!(
                o,
                DrawOp::Polygon(p) if p.stroke.as_ref().is_some_and(|s| s.color == FACE_INK)
            )),
            "every scenery face is outlined in the room ink"
        );
    }

    #[test]
    fn the_room_ignores_everything_the_label_frame_reserves() {
        // The engine's room lands on the same coordinates whatever the tick count, the category
        // labels or the title, so the room must be fit off the chart box and never off the plot box
        // the axis frame leaves behind.
        let rect = Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(7200),
            height: Twips(4320),
        };
        let def = rpt_model::ChartDefinition::default();
        let faces = |title: &str, cats: &[&str], vals: Vec<f64>| {
            let categories: Vec<String> = cats.iter().map(|c| c.to_string()).collect();
            let series = vec![("s".to_string(), vals)];
            let (_, _, _, background, _) = setup_3d(
                ChartStyle {
                    def: &def,
                    height: rect.height,
                },
                rect,
                title,
                &categories,
                &series,
                rpt_model::ChartViewAngle::Standard,
                &|| None,
            );
            background[..ROOM_FACES].to_vec()
        };
        let a = faces("A short title", &["1", "2", "3"], vec![9.0, 4.0, 7.0]);
        let b = faces(
            "",
            &["a much longer category label", "another one", "and a third"],
            vec![1_400.0, 900.0, 1_100.0],
        );
        assert_eq!(
            format!("{a:?}"),
            format!("{b:?}"),
            "the room is identical across tick counts, label widths and titles"
        );
    }

    #[test]
    fn the_walls_stand_on_the_floor_plate() {
        // The floor's top face and the walls' outer surfaces share the plot box, so the room reads as
        // one solid corner rather than three floating planes.
        let (tz, tx, ty) = thickness(ViewAngle::DEPTH_EFFECT, 0.0, 6000.0, 0.0, 4000.0);
        assert!(tz > 0.0 && tx > 0.0 && ty > 0.0, "slabs have thickness");
        assert!(
            tx < 6000.0 && ty < 4000.0,
            "slabs are thin next to the room"
        );
    }
}
