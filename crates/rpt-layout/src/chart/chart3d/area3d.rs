//! Depth-effect area chart: the flat 2-D area ribbon over the shared Num+Ord frame, given a shallow
//! extrusion toward the upper right. Routed here from the flat area path when the Area family's
//! depth-effect bit is set ([`rpt_model::ChartDefinition::has_depth_effect`]).
//!
//! This is **not** a 3-D scene: the engine draws no floor, no walls and no perspective for it, only
//! the ordinary 2-D frame plus a solid the ribbon casts up and to the right — a top face along each
//! crest segment and one end cap on the right, all a fixed shade darker than the ribbon. The two
//! genuinely three-dimensional families ([`super::riser`], [`super::surface`]) own the corner room.

use super::projection::{face_edge, shade};
use crate::chart::common::{
    category_label, chart_frame, compute_frame, fmt_val, value_frac, value_label, ChartCtx, LABEL,
    PALETTE,
};
use rpt_model::{Color, Rect, Twips};
use rpt_pages::{DrawOp, Point, PolygonOp};

/// How far the solid is cast, as a fraction of the plot box along each axis (right and up). The
/// engine's offset is 11 px across a 326 px plot and 6 px up a 178 px one — the same fraction of
/// both, so the cast follows the plot's aspect.
const DEPTH_FRAC: f64 = 0.0337;

/// The extruded faces' shade against the ribbon's own fill: the engine fills the ribbon with the
/// palette color and its faces with a fixed fraction of it.
const DEPTH_SHADE: f32 = 0.695;

/// Build the draw-ops for a depth-effect area chart: the flat axis frame, then each series as a
/// ribbon with its cast solid — the top face of every crest segment plus the right end cap behind
/// it, so the ribbon reads as an extruded slab. `categories` are the X-axis slots; `series` is one
/// row per data binding (`name`, value-per-category). Returns an empty vec if there is nothing to
/// plot.
pub(crate) fn area_3d(
    cx: &ChartCtx,
    categories: &[String],
    series: &[(String, Vec<f64>)],
) -> Vec<DrawOp> {
    let (s_count, c_count) = (series.len(), categories.len());
    if s_count == 0 || c_count == 0 {
        return Vec::new();
    }
    let &ChartCtx {
        style,
        rect,
        title,
        axis_titles,
        show_labels,
        ..
    } = cx;
    let src = || cx.src();

    // The frame is scaled to the tallest value in any series, so every ribbon fits under the ceiling.
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

    // The solid is cast to the right, so the frame gives that width back: size the cast off the
    // full-width frame, then lay the real frame out in what is left. Nothing is reserved above —
    // the engine lets the cast overshoot the plot's top edge.
    let probe = compute_frame(style, rect, title, axis_titles, &frame_series);
    let (dx, dy) = (
        ((probe.plot_right - probe.plot_left) as f64 * DEPTH_FRAC).round() as i32,
        (probe.plot_h as f64 * DEPTH_FRAC).round() as i32,
    );
    let body = Rect {
        width: Twips((rect.width.0 - dx).max(1)),
        ..rect
    };

    let mut ops: Vec<DrawOp> = Vec::new();
    let f = chart_frame(
        style,
        &mut ops,
        body,
        title,
        axis_titles,
        &frame_series,
        &src,
    );

    let point = |c: usize, val: f64| Point {
        x: Twips(f.plot_left + c as i32 * f.slot + f.slot / 2),
        y: Twips(f.plot_bottom - (value_frac(val, f.max_val) * f.plot_h as f64) as i32),
    };
    let cast = |p: Point| Point {
        x: Twips(p.x.0 + dx),
        y: Twips(p.y.0 - dy),
    };
    let quad = |corners: [Point; 4], fill: Color| {
        DrawOp::Polygon(PolygonOp {
            points: corners.to_vec(),
            closed: true,
            fill: Some(fill.into()),
            stroke: Some(face_edge()),
            source: src(),
        })
    };

    let mut labels: Vec<DrawOp> = Vec::new();
    for (s, (_, vals)) in series.iter().enumerate() {
        let color = PALETTE[s % PALETTE.len()];
        let face = shade(color, DEPTH_SHADE);
        let at = |c: usize| point(c, vals.get(c).copied().unwrap_or(0.0));

        // The cast solid, behind the ribbon: one top face per crest segment, then the right end cap
        // (the left one faces away from the cast and is never drawn). A segment that climbs more
        // steeply than the cast turns its top face away from the viewer, so it is culled — otherwise
        // it wedges out above the crest, which the engine never shows.
        for c in 0..c_count.saturating_sub(1) {
            let (p0, p1) = (at(c), at(c + 1));
            let (sx, sy) = (p1.x.0 - p0.x.0, p1.y.0 - p0.y.0);
            if sy * dx + sx * dy > 0 {
                ops.push(quad([p0, p1, cast(p1), cast(p0)], face));
            }
        }
        let last = at(c_count - 1);
        let base = Point {
            x: last.x,
            y: Twips(f.plot_bottom),
        };
        ops.push(quad([last, base, cast(base), cast(last)], face));

        // The ribbon itself: baseline under the first point → each crest point → baseline under the
        // last, filled with the series color.
        let mut ribbon = Vec::with_capacity(c_count + 2);
        ribbon.push(Point {
            x: at(0).x,
            y: Twips(f.plot_bottom),
        });
        ribbon.extend((0..c_count).map(at));
        ribbon.push(base);
        ops.push(DrawOp::Polygon(PolygonOp {
            points: ribbon,
            closed: true,
            fill: Some(color.into()),
            stroke: Some(face_edge()),
            source: src(),
        }));

        if show_labels {
            for c in 0..c_count {
                let p = at(c);
                labels.push(value_label(
                    style,
                    p.x.0,
                    (p.y.0 - 230).max(f.plot_top()),
                    &fmt_val(vals.get(c).copied().unwrap_or(0.0)),
                    LABEL,
                    &src,
                ));
            }
        }
    }

    // Labels last so no ribbon overdraws them.
    for (c, label) in categories.iter().enumerate() {
        if c.is_multiple_of(f.cats.stride) {
            ops.push(category_label(style, &f, c as i32, label, &src));
        }
    }
    ops.extend(labels);
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::common::AxisTitles;
    use rpt_pages::Fill;

    fn rect() -> Rect {
        Rect {
            left: Twips(100),
            top: Twips(100),
            width: Twips(6000),
            height: Twips(4000),
        }
    }

    fn polygons(ops: &[DrawOp]) -> Vec<(Color, Vec<Point>)> {
        ops.iter()
            .filter_map(|o| match o {
                DrawOp::Polygon(p) => match &p.fill {
                    Some(Fill::Solid(c)) => Some((*c, p.points.clone())),
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
        assert!(area_3d(&cx, &[], &series).is_empty());
        assert!(area_3d(&cx, &cats, &[]).is_empty());
    }

    #[test]
    fn draws_no_room_only_the_ribbon_and_its_cast() {
        // The depth effect is a 2-D chart with a cast solid: a top face per (viewer-facing) crest
        // segment, one end cap and the ribbon — and none of the corner room's nine grey scenery
        // faces. A falling series turns every crest segment toward the viewer, so none is culled.
        let cats: Vec<String> = (0..5).map(|c| format!("c{c}")).collect();
        let series = vec![("s".to_string(), vec![9.0, 8.0, 6.0, 5.0, 3.0])];
        let ops = area_3d(
            &ChartCtx::test(rect(), "Area", AxisTitles::default(), false),
            &cats,
            &series,
        );
        let polys = polygons(&ops);
        let base = PALETTE[0];
        let face = shade(base, DEPTH_SHADE);
        assert_eq!(
            polys.iter().filter(|(c, _)| *c == face).count(),
            5,
            "C−1 crest faces plus the end cap"
        );
        assert_eq!(
            polys.iter().filter(|(c, _)| *c == base).count(),
            1,
            "one ribbon"
        );
        let grey = |c: &Color| c.r == c.g && c.g == c.b;
        assert!(
            !polys.iter().any(|(c, _)| grey(c)),
            "no grey scenery: the engine draws no room for a depth-effect area"
        );
        // The engine strokes the ribbon and every cast face with a black hairline.
        use super::super::projection::FACE_INK;
        assert!(
            ops.iter().all(|o| !matches!(o, DrawOp::Polygon(p)
                if p.stroke.as_ref().is_none_or(|s| s.color != FACE_INK))),
            "every filled face carries the black outline"
        );
    }

    #[test]
    fn the_cast_goes_up_and_to_the_right() {
        let cats: Vec<String> = (0..3).map(|c| format!("c{c}")).collect();
        let series = vec![("s".to_string(), vec![4.0, 9.0, 6.0])];
        let ops = area_3d(
            &ChartCtx::test(rect(), "", AxisTitles::default(), false),
            &cats,
            &series,
        );
        let polys = polygons(&ops);
        let face = shade(PALETTE[0], DEPTH_SHADE);
        let (_, first) = polys
            .iter()
            .find(|(c, _)| *c == face)
            .expect("a crest face");
        // A crest face is [p0, p1, cast(p1), cast(p0)]: the cast corners sit right of and above their
        // originals by the same offset.
        let (d0x, d0y) = (first[3].x.0 - first[0].x.0, first[3].y.0 - first[0].y.0);
        let (d1x, d1y) = (first[2].x.0 - first[1].x.0, first[2].y.0 - first[1].y.0);
        assert!(
            d0x > 0 && d0y < 0,
            "cast is up and to the right: {d0x},{d0y}"
        );
        assert_eq!((d0x, d0y), (d1x, d1y), "both corners cast by one offset");
    }

    #[test]
    fn a_segment_climbing_steeper_than_the_cast_is_culled() {
        // Its top face turns away from the viewer, so the engine never draws it — and drawing it
        // would wedge a dark triangle out above the crest.
        let cats: Vec<String> = (0..2).map(|c| format!("c{c}")).collect();
        let cast_faces = |vals: Vec<f64>| {
            let series = vec![("s".to_string(), vals)];
            let ops = area_3d(
                &ChartCtx::test(rect(), "", AxisTitles::default(), false),
                &cats,
                &series,
            );
            polygons(&ops)
                .iter()
                .filter(|(c, _)| *c == shade(PALETTE[0], DEPTH_SHADE))
                .count()
        };
        // Both charts have one segment and one end cap; only the shallow one shows its top face.
        assert_eq!(cast_faces(vec![9.0, 2.0]), 2, "a falling segment is drawn");
        assert_eq!(cast_faces(vec![1.0, 9.0]), 1, "a steep climb is culled");
    }

    #[test]
    fn multi_series_stacks_ribbon_and_cast_per_series() {
        let cats: Vec<String> = (0..4).map(|c| format!("c{c}")).collect();
        let series = vec![
            ("s1".to_string(), vec![7.0, 5.0, 3.0, 1.0]),
            ("s2".to_string(), vec![6.0, 4.0, 2.0, 1.0]),
        ];
        let ops = area_3d(
            &ChartCtx::test(rect(), "", AxisTitles::default(), false),
            &cats,
            &series,
        );
        let polys = polygons(&ops);
        for &base in &PALETTE[..2] {
            assert_eq!(
                polys.iter().filter(|(c, _)| *c == base).count(),
                1,
                "one ribbon per series"
            );
            assert_eq!(
                polys
                    .iter()
                    .filter(|(c, _)| *c == shade(base, DEPTH_SHADE))
                    .count(),
                4,
                "C−1 crest faces plus the end cap per series"
            );
        }
    }
}
