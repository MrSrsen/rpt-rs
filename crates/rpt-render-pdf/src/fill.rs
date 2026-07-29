//! The Page IR's non-solid fills as real PDF paint: [`rpt_pages::Fill::LinearGradient`] becomes an
//! axial (type 2) shading, [`rpt_pages::Fill::Hatch`] a tiling pattern.
//!
//! krilla expresses both in the surface's own user space and post-concatenates the current
//! transform itself, so the coordinates built here are the same twip→point coordinates the paths
//! use — nothing re-derives the page's y-flip.

use crate::common::pt;
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Transform};
use krilla::num::NormalizedF32;
use krilla::paint::{Fill, LinearGradient, Pattern, SpreadMethod, Stop, Stroke};
use krilla::surface::Surface;
use rpt_model::{Color, Rect};
use rpt_pages::HatchPattern;

/// A fill region's bounding box in points — the frame a gradient axis spans and a hatch tile is
/// phased against.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bounds {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
}

impl Bounds {
    /// The point-space box of a twip-space [`Rect`].
    pub(crate) fn of(r: &Rect) -> Bounds {
        Bounds {
            x: pt(r.left.0) as f32,
            y: pt(r.top.0) as f32,
            w: pt(r.width.0) as f32,
            h: pt(r.height.0) as f32,
        }
    }

    /// The box enclosing a set of points, for an op that has no stored rectangle (a polygon).
    pub(crate) fn of_points(points: impl IntoIterator<Item = (f32, f32)>) -> Bounds {
        let mut it = points.into_iter();
        let Some((x0, y0)) = it.next() else {
            return Bounds {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            };
        };
        let (mut l, mut t, mut r, mut b) = (x0, y0, x0, y0);
        for (x, y) in it {
            l = l.min(x);
            t = t.min(y);
            r = r.max(x);
            b = b.max(y);
        }
        Bounds {
            x: l,
            y: t,
            w: r - l,
            h: b - t,
        }
    }
}

/// The krilla fill for a Page-IR fill over `bounds`.
///
/// Gradients and hatches paint as a shading and a tiling pattern respectively. The solid
/// substitute is reached only by a fill that has no expressible geometry at all — see
/// [`gradient_paint`] for the two cases that produce one.
pub(crate) fn fill_of(surface: &mut Surface, fill: &rpt_pages::Fill, bounds: Bounds) -> Fill {
    match fill {
        rpt_pages::Fill::Solid(c) => solid(*c),
        rpt_pages::Fill::LinearGradient { stops, angle_deg } => {
            gradient_paint(stops, *angle_deg, bounds)
                .unwrap_or_else(|| solid(fill.representative_color()))
        }
        rpt_pages::Fill::Hatch { fg, bg, pattern } => hatch_paint(surface, *fg, *bg, *pattern),
    }
}

/// A flat color fill, with the color's alpha as the fill opacity.
pub(crate) fn solid(color: Color) -> Fill {
    let (rgb, opacity) = rgb_alpha(color);
    Fill {
        paint: rgb.into(),
        opacity,
        ..Fill::default()
    }
}

/// Split a `Color` into a krilla RGB paint and a normalized alpha.
pub(crate) fn rgb_alpha(color: Color) -> (rgb::Color, NormalizedF32) {
    let opacity = NormalizedF32::new(color.a as f32 / 255.0).unwrap_or(NormalizedF32::ONE);
    (rgb::Color::new(color.r, color.g, color.b), opacity)
}

/// An axial shading across `bounds` in the `angle_deg` direction.
///
/// Returns `None` — leaving the caller to paint a representative solid — only when there is no
/// axis to shade along: a gradient with no stops, or a box with no area. Every other gradient
/// paints as a real shading.
fn gradient_paint(stops: &[(f32, Color)], angle_deg: f32, b: Bounds) -> Option<Fill> {
    let stops = shading_stops(stops)?;
    if b.w <= 0.0 && b.h <= 0.0 {
        return None;
    }
    let (x1, y1, x2, y2) = gradient_axis(angle_deg, b);
    Some(Fill {
        paint: LinearGradient {
            x1,
            y1,
            x2,
            y2,
            // The coordinates above are already in the surface's user space, and `Pad` keeps krilla
            // on the plain axial-shading path (a repeating spread would encode a PostScript
            // function instead).
            transform: Transform::identity(),
            spread_method: SpreadMethod::Pad,
            stops,
            anti_alias: false,
        }
        .into(),
        ..Fill::default()
    })
}

/// The gradient axis `(x1, y1, x2, y2)` for `angle_deg` across `b`, in surface points.
///
/// Angle convention: 0° points along +x, so the gradient runs left→right, and the angle increases
/// **counter-clockwise as seen on the page**. The surface is y-down, so the direction vector is
/// `(cos θ, −sin θ)` and 90° therefore runs bottom→top. This is the sense the Page IR already gives
/// an angle in [`rpt_pages::TextRun::rotation`], and the one the IR's only other renderer used. The
/// axis is the same left→right span at 0° under either sign, so the convention is observable only on
/// an angle that is not a multiple of 180°.
///
/// The axis is centred on the box and extended to the box's furthest projection onto the
/// direction, which puts the first and last stop exactly on the bounding edges at any angle. For a
/// unit direction over an axis-aligned box that half-length is `w/2·|dx| + h/2·|dy|`.
fn gradient_axis(angle_deg: f32, b: Bounds) -> (f32, f32, f32, f32) {
    let rad = angle_deg.to_radians();
    let (dx, dy) = (rad.cos(), -rad.sin());
    let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    let half = b.w / 2.0 * dx.abs() + b.h / 2.0 * dy.abs();
    (
        cx - dx * half,
        cy - dy * half,
        cx + dx * half,
        cy + dy * half,
    )
}

/// The Page IR's stops as shading stops: clamped into `0..=1`, sorted, and extended to cover the
/// whole domain.
///
/// PDF stitching-function bounds must be non-decreasing and span `[0, 1]`, none of which the IR
/// guarantees, so an out-of-order or partial stop list is normalized rather than rejected. A stop
/// list that is empty after dropping non-finite offsets yields `None`.
fn shading_stops(stops: &[(f32, Color)]) -> Option<Vec<Stop>> {
    let mut ordered: Vec<(f32, Color)> = stops
        .iter()
        .filter(|(o, _)| o.is_finite())
        .map(|(o, c)| (o.clamp(0.0, 1.0), *c))
        .collect();
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (&(first_offset, first_color), &(last_offset, last_color)) =
        (ordered.first()?, ordered.last()?);
    if first_offset > 0.0 {
        ordered.insert(0, (0.0, first_color));
    }
    if last_offset < 1.0 {
        ordered.push((1.0, last_color));
    }
    Some(
        ordered
            .into_iter()
            .map(|(offset, color)| {
                let (rgb, opacity) = rgb_alpha(color);
                Stop {
                    offset: NormalizedF32::new(offset).unwrap_or(NormalizedF32::ZERO),
                    color: rgb.into(),
                    opacity,
                }
            })
            .collect(),
    )
}

/// The side of a hatch tile, in points.
///
/// The six [`HatchPattern`] variants are the six classic GDI `HS_*` brush styles, which
/// `CreateHatchBrush` defines as an 8×8-device-pixel cell with single-pixel lines. At the 96 dpi
/// logical inch that maps to 6 pt (120 twips) with 0.75 pt (15 twips) lines, which is what this
/// backend draws.
///
/// The mapping from GDI device pixels to a resolution-independent PDF pattern is this backend's
/// choice, not a measured constant — the shape of the pattern is right, but the scale (the
/// documented GDI convention) is conjectural.
const HATCH_TILE_PT: f32 = 6.0;

/// The hatch line width, in points — one device pixel at 96 dpi (see [`HATCH_TILE_PT`]).
const HATCH_LINE_PT: f32 = 0.75;

/// A tiling pattern painting `fg` hatch lines over a `bg` field.
///
/// Every [`HatchPattern`] variant is expressible: each is one or two straight lines in a square
/// cell, so none falls back to a solid. Identical patterns collapse to a single PDF pattern object
/// — krilla caches a pattern by the hash of its stream, transform and size, and two hatches with
/// the same colors and variant build byte-identical streams.
fn hatch_paint(surface: &mut Surface, fg: Color, bg: Color, pattern: HatchPattern) -> Fill {
    let t = HATCH_TILE_PT;
    let stream = {
        let mut builder = surface.stream_builder();
        let mut tile = builder.surface();

        // The field first, covering the whole cell, then the lines over it.
        let mut bg_path = PathBuilder::new();
        bg_path.move_to(0.0, 0.0);
        bg_path.line_to(t, 0.0);
        bg_path.line_to(t, t);
        bg_path.line_to(0.0, t);
        bg_path.close();
        if let Some(path) = bg_path.finish() {
            tile.set_fill(Some(solid(bg)));
            tile.set_stroke(None);
            tile.draw_path(&path);
        }

        let (rgb, opacity) = rgb_alpha(fg);
        let stroke = Stroke {
            paint: rgb.into(),
            width: HATCH_LINE_PT,
            opacity,
            ..Stroke::default()
        };
        let mut line_path = PathBuilder::new();
        for (x1, y1, x2, y2) in hatch_lines(pattern, t) {
            line_path.move_to(x1, y1);
            line_path.line_to(x2, y2);
        }
        if let Some(path) = line_path.finish() {
            tile.set_fill(None);
            tile.set_stroke(Some(stroke));
            tile.draw_path(&path);
        }
        tile.finish();
        builder.finish()
    };
    Fill {
        paint: Pattern {
            stream,
            // The cell is built in the surface's own units, so it needs no placement transform of
            // its own; krilla concatenates the current transform when it writes the pattern matrix.
            transform: Transform::identity(),
            width: t,
            height: t,
        }
        .into(),
        ..Fill::default()
    }
}

/// The foreground line segments of a hatch cell of side `t`, as `(x1, y1, x2, y2)`.
///
/// The straight patterns put their line down the middle of the cell. The diagonals run corner to
/// corner and repeat the opposite two corners as half-length segments, so a stroked diagonal joins
/// up across the cell boundary instead of leaving a notch where the cell clips it.
fn hatch_lines(pattern: HatchPattern, t: f32) -> Vec<(f32, f32, f32, f32)> {
    let h = t / 2.0;
    // Bottom-left→top-right, plus the two corner stubs that continue it into the next cell.
    let forward = [(0.0, t, t, 0.0), (0.0, h, h, 0.0), (h, t, t, h)];
    // Top-left→bottom-right, likewise.
    let backward = [(0.0, 0.0, t, t), (h, 0.0, t, h), (0.0, h, h, t)];
    match pattern {
        HatchPattern::Horizontal => vec![(0.0, h, t, h)],
        HatchPattern::Vertical => vec![(h, 0.0, h, t)],
        HatchPattern::ForwardDiagonal => forward.to_vec(),
        HatchPattern::BackwardDiagonal => backward.to_vec(),
        HatchPattern::Cross => vec![(0.0, h, t, h), (h, 0.0, h, t)],
        HatchPattern::DiagonalCross => forward.iter().chain(&backward).copied().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(angle: f32) -> (f32, f32, f32, f32) {
        gradient_axis(
            angle,
            Bounds {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 40.0,
            },
        )
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn zero_degrees_runs_left_to_right_across_the_box() {
        let (x1, y1, x2, y2) = axis(0.0);
        assert!(close(x1, 0.0) && close(x2, 100.0), "{x1} {x2}");
        assert!(close(y1, 20.0) && close(y2, 20.0), "{y1} {y2}");
    }

    #[test]
    fn ninety_degrees_runs_bottom_to_top() {
        // Counter-clockwise on a y-down surface: the axis starts at the bottom edge.
        let (x1, y1, x2, y2) = axis(90.0);
        assert!(close(x1, 50.0) && close(x2, 50.0), "{x1} {x2}");
        assert!(close(y1, 40.0) && close(y2, 0.0), "{y1} {y2}");
    }

    #[test]
    fn one_eighty_reverses_the_zero_degree_axis() {
        let (x1, y1, x2, y2) = axis(180.0);
        assert!(close(x1, 100.0) && close(x2, 0.0), "{x1} {x2}");
        assert!(close(y1, 20.0) && close(y2, 20.0), "{y1} {y2}");
    }

    #[test]
    fn diagonal_axis_reaches_the_box_corners() {
        // At 45° the half-length is (w+h)/2·cos45, so the axis ends sit outside the box's own
        // corners only along the diagonal direction — the projection of every corner is covered.
        let (x1, y1, x2, y2) = axis(45.0);
        let half = (100.0 / 2.0 + 40.0 / 2.0) * std::f32::consts::FRAC_1_SQRT_2;
        assert!(
            close(x1, 50.0 - half * std::f32::consts::FRAC_1_SQRT_2),
            "{x1}"
        );
        assert!(
            close(y1, 20.0 + half * std::f32::consts::FRAC_1_SQRT_2),
            "{y1}"
        );
        assert!(
            close(x2, 50.0 + half * std::f32::consts::FRAC_1_SQRT_2),
            "{x2}"
        );
        assert!(
            close(y2, 20.0 - half * std::f32::consts::FRAC_1_SQRT_2),
            "{y2}"
        );
    }

    #[test]
    fn stops_are_sorted_clamped_and_span_the_domain() {
        let c = Color {
            a: 255,
            r: 1,
            g: 2,
            b: 3,
        };
        let stops = shading_stops(&[(0.8, c), (0.2, c)]).expect("two stops");
        let offsets: Vec<f32> = stops.iter().map(|s| s.offset.get()).collect();
        // Sorted, and padded out to both ends of the domain.
        assert_eq!(offsets, vec![0.0, 0.2, 0.8, 1.0]);
    }

    #[test]
    fn out_of_range_offsets_are_clamped_not_dropped() {
        let c = Color {
            a: 255,
            r: 0,
            g: 0,
            b: 0,
        };
        let stops = shading_stops(&[(-2.0, c), (0.5, c), (7.0, c)]).expect("three stops");
        let offsets: Vec<f32> = stops.iter().map(|s| s.offset.get()).collect();
        assert_eq!(offsets, vec![0.0, 0.5, 1.0]);
    }

    #[test]
    fn empty_stops_have_no_shading() {
        assert!(shading_stops(&[]).is_none());
    }

    #[test]
    fn every_hatch_variant_draws_at_least_one_line() {
        for pattern in [
            HatchPattern::Horizontal,
            HatchPattern::Vertical,
            HatchPattern::ForwardDiagonal,
            HatchPattern::BackwardDiagonal,
            HatchPattern::Cross,
            HatchPattern::DiagonalCross,
        ] {
            let lines = hatch_lines(pattern, HATCH_TILE_PT);
            assert!(!lines.is_empty(), "{pattern:?} drew no lines");
            for (x1, y1, x2, y2) in lines {
                for v in [x1, y1, x2, y2] {
                    assert!(
                        (0.0..=HATCH_TILE_PT).contains(&v),
                        "{pattern:?} left the cell: {v}"
                    );
                }
            }
        }
    }

    #[test]
    fn polygon_bounds_enclose_every_point() {
        let b = Bounds::of_points([(10.0, 5.0), (-2.0, 40.0), (7.0, 12.0)]);
        assert!(close(b.x, -2.0) && close(b.y, 5.0), "{b:?}");
        assert!(close(b.w, 12.0) && close(b.h, 35.0), "{b:?}");
    }
}
