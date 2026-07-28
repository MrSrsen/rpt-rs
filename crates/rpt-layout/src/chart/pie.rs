//! Pie chart: no axes/scale — each slice's sweep angle is
//! `value / total × 360°`, drawn as a filled polygon wedge with the category label at its
//! outer midpoint. The `Faked3DRegular` subtype adds a tilted-ellipse face with an extruded crust
//! (see [`depth_pie`]); the flat variant is [`flat_pie`].

#[cfg(test)]
use super::common::AxisTitles;
use super::common::{
    centered_disc, disc_label, slice_color, title_op, truncate, value_label, ChartCtx,
    SLICE_BORDER_W, WHITE,
};
#[cfg(test)]
use rpt_model::Rect;
use rpt_model::{Color, Twips};
use rpt_pages::{DrawOp, LineStyle, Point, PolygonOp, Stroke};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// Vertical squash of the pie face for the 3-D variant: the disc is drawn as an ellipse
/// `ry = PIE_3D_TILT · rx`, faking a viewer looking down on the pie at an angle.
const PIE_3D_TILT: f64 = 0.6;
/// Crust (rim) height for the 3-D variant, as a fraction of the pie radius.
const PIE_3D_CRUST_FRAC: f64 = 0.16;
/// Shade factor applied to a slice's colour on its extruded crust wall (a darker, shadowed rim).
const PIE_3D_CRUST_SHADE: f32 = 0.65;

/// Build the draw-ops for a pie chart of `series` (category label → value): no axes/scale — each
/// slice's sweep angle is `value / total × 360°`, drawn as a filled polygon wedge (the arc
/// tessellated to segments), with the category label at the slice's outer midpoint. Values ≤ 0 are
/// ignored. `show_labels` gates the per-slice percentage data labels (the report's decoded
/// "show value" flag). `depth` selects the `Faked3DRegular` variant (a tilted-ellipse face with an
/// extruded crust) over the flat 2-D pie. Returns an empty vec if there is nothing positive to plot.
pub(crate) fn pie_chart(cx: &ChartCtx, series: &[(String, f64)], depth: bool) -> Vec<DrawOp> {
    if depth {
        depth_pie(cx, series)
    } else {
        flat_pie(cx, series)
    }
}

/// The flat 2-D pie: centre-anchored wedges on a circular disc.
fn flat_pie(cx: &ChartCtx, series: &[(String, f64)]) -> Vec<DrawOp> {
    let total: f64 = series.iter().map(|(_, v)| v.max(0.0)).sum();
    if series.is_empty() || total <= 0.0 {
        return Vec::new();
    }
    let &ChartCtx {
        def,
        rect,
        title,
        show_labels,
        ..
    } = cx;
    let src = || cx.src();
    let mut ops: Vec<DrawOp> = Vec::new();
    let (rl, rt, rw, rh) = (rect.left.0, rect.top.0, rect.width.0, rect.height.0);
    let pad = 60;

    let title_h = if title.is_empty() {
        0
    } else {
        (rh / 8).clamp(180, 360)
    };
    if !title.is_empty() {
        ops.push(title_op(def, rl, rt + pad / 2, rw, title_h, title, &src));
    }

    // Center the disc in the area below the title; leave a small margin for outer labels.
    let (cx, cy, radius) = centered_disc(rect, title_h);

    let mut angle = -FRAC_PI_2; // first slice starts at 12 o'clock
    for (i, (label, val)) in series.iter().enumerate() {
        let frac = val.max(0.0) / total;
        if frac <= 0.0 {
            continue;
        }
        let sweep = frac * TAU;
        // Tessellate the arc: adaptive segments at ~30-twip flatness.
        let steps = ((sweep * radius as f64 / 30.0).ceil() as i32).clamp(2, 512);
        let mut points = Vec::with_capacity(steps as usize + 2);
        points.push(Point {
            x: Twips(cx),
            y: Twips(cy),
        });
        for s in 0..=steps {
            let a = angle + sweep * (s as f64 / steps as f64);
            points.push(Point {
                x: Twips(cx + (radius as f64 * a.cos()) as i32),
                y: Twips(cy + (radius as f64 * a.sin()) as i32),
            });
        }
        ops.push(DrawOp::Polygon(PolygonOp {
            points,
            closed: true,
            fill: Some(slice_color(i).into()),
            // A thin white border separates adjacent slices.
            stroke: Some(Stroke {
                color: WHITE,
                width: SLICE_BORDER_W,
                style: LineStyle::Single,
            }),
            source: src(),
        }));
        let mid = angle + sweep / 2.0;
        // Percentage data label inside the slice, at mid-radius, in white for contrast on the fill,
        // gated on "show value". Skipped for thin slices where it would not fit.
        if show_labels && frac >= 0.05 {
            let ir = radius as f64 * 0.6;
            ops.push(value_label(
                cx + (ir * mid.cos()) as i32,
                cy + (ir * mid.sin()) as i32 - 100,
                &format!("{:.0}%", frac * 100.0),
                WHITE,
                &src,
            ));
        }
        // Category label at the slice's outer midpoint.
        let lr = radius as f64 * 1.02;
        let lx = cx + (lr * mid.cos()) as i32;
        let ly = cy + (lr * mid.sin()) as i32;
        ops.push(disc_label(lx, ly, &truncate(label, 16), &src));
        angle += sweep;
    }

    ops
}

/// The `Faked3DRegular` pie: the disc is a tilted ellipse (`ry = PIE_3D_TILT · rx`) whose front rim
/// (the lower half, `sin a > 0`) is extruded downward by a shaded crust. The crust walls are drawn
/// first so the ellipse face overlaps their upper edge; front slices' labels are pushed down by the
/// crust height so they clear the rim.
fn depth_pie(cx: &ChartCtx, series: &[(String, f64)]) -> Vec<DrawOp> {
    let total: f64 = series.iter().map(|(_, v)| v.max(0.0)).sum();
    if series.is_empty() || total <= 0.0 {
        return Vec::new();
    }
    let &ChartCtx {
        def,
        rect,
        title,
        show_labels,
        ..
    } = cx;
    let src = || cx.src();
    let mut ops: Vec<DrawOp> = Vec::new();
    let (rl, rt, rw, rh) = (rect.left.0, rect.top.0, rect.width.0, rect.height.0);
    let pad = 60;

    let title_h = if title.is_empty() {
        0
    } else {
        (rh / 8).clamp(180, 360)
    };
    if !title.is_empty() {
        ops.push(title_op(def, rl, rt + pad / 2, rw, title_h, title, &src));
    }

    let (cx, cy, radius) = centered_disc(rect, title_h);
    let rx = radius as f64;
    let ry = rx * PIE_3D_TILT;
    let crust = (rx * PIE_3D_CRUST_FRAC).max(1.0);
    // Centre the whole solid (ellipse face + crust) vertically in the disc box.
    let cxf = cx as f64;
    let cyf = cy as f64 - crust / 2.0;

    // Slice boundaries (start/end angle) for positive slices, starting at 12 o'clock.
    let mut slices: Vec<(usize, &str, f64, f64, f64)> = Vec::new();
    let mut angle = -FRAC_PI_2;
    for (i, (label, val)) in series.iter().enumerate() {
        let frac = val.max(0.0) / total;
        if frac <= 0.0 {
            continue;
        }
        let sweep = frac * TAU;
        slices.push((i, label.as_str(), angle, angle + sweep, frac));
        angle += sweep;
    }

    // Crust walls along the front rim only (`sin a > 0`, i.e. `a ∈ (0, π)`): each slice's outer edge
    // extruded downward by `crust`, shaded darker. Drawn before the face so the ellipse overlaps the
    // crust's upper edge.
    for &(i, _, a0, a1, _) in &slices {
        let lo = a0.max(0.0);
        let hi = a1.min(PI);
        if hi <= lo {
            continue;
        }
        let steps = (((hi - lo) * rx / 30.0).ceil() as i32).clamp(1, 512);
        let wall = darken(slice_color(i), PIE_3D_CRUST_SHADE);
        for s in 0..steps {
            let aa = lo + (hi - lo) * (s as f64 / steps as f64);
            let ab = lo + (hi - lo) * ((s + 1) as f64 / steps as f64);
            let (tax, tay) = (cxf + rx * aa.cos(), cyf + ry * aa.sin());
            let (tbx, tby) = (cxf + rx * ab.cos(), cyf + ry * ab.sin());
            ops.push(DrawOp::Polygon(PolygonOp {
                points: vec![
                    Point {
                        x: Twips(tax as i32),
                        y: Twips(tay as i32),
                    },
                    Point {
                        x: Twips(tbx as i32),
                        y: Twips(tby as i32),
                    },
                    Point {
                        x: Twips(tbx as i32),
                        y: Twips((tby + crust) as i32),
                    },
                    Point {
                        x: Twips(tax as i32),
                        y: Twips((tay + crust) as i32),
                    },
                ],
                closed: true,
                fill: Some(wall.into()),
                stroke: None,
                source: src(),
            }));
        }
    }

    // The tilted ellipse face: a wedge per slice, plus its labels.
    for &(i, label, a0, a1, frac) in &slices {
        let sweep = a1 - a0;
        let steps = ((sweep * rx / 30.0).ceil() as i32).clamp(2, 512);
        let mut points = Vec::with_capacity(steps as usize + 2);
        points.push(Point {
            x: Twips(cxf as i32),
            y: Twips(cyf as i32),
        });
        for s in 0..=steps {
            let a = a0 + sweep * (s as f64 / steps as f64);
            points.push(Point {
                x: Twips((cxf + rx * a.cos()) as i32),
                y: Twips((cyf + ry * a.sin()) as i32),
            });
        }
        ops.push(DrawOp::Polygon(PolygonOp {
            points,
            closed: true,
            fill: Some(slice_color(i).into()),
            stroke: Some(Stroke {
                color: WHITE,
                width: SLICE_BORDER_W,
                style: LineStyle::Single,
            }),
            source: src(),
        }));
        let mid = a0 + sweep / 2.0;
        // Front slices sit over the crust, so their labels drop by the crust height to clear it.
        let drop = if mid.sin() > 0.0 { crust } else { 0.0 };
        if show_labels && frac >= 0.05 {
            let ir = 0.6;
            ops.push(value_label(
                (cxf + rx * ir * mid.cos()) as i32,
                (cyf + ry * ir * mid.sin() + drop) as i32 - 100,
                &format!("{:.0}%", frac * 100.0),
                WHITE,
                &src,
            ));
        }
        let lr = 1.04;
        ops.push(disc_label(
            (cxf + rx * lr * mid.cos()) as i32,
            (cyf + ry * lr * mid.sin() + drop) as i32,
            &truncate(label, 16),
            &src,
        ));
    }

    ops
}

/// Darken `c` toward black by `factor` (`0..1`), preserving alpha — the shadowed shade for a 3-D
/// pie's crust wall.
fn darken(c: Color, factor: f32) -> Color {
    let f = |v: u8| (v as f32 * factor).round().clamp(0.0, 255.0) as u8;
    Color {
        a: c.a,
        r: f(c.r),
        g: f(c.g),
        b: f(c.b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series() -> Vec<(String, f64)> {
        vec![
            ("Canada".into(), 40.0),
            ("USA".into(), 35.0),
            ("Mexico".into(), 25.0),
        ]
    }

    fn rect() -> Rect {
        Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(6000),
            height: Twips(6000),
        }
    }

    /// With "show value" on, each slice draws a wedge, a category label, and a percentage data label.
    #[test]
    fn show_labels_true_draws_percentages() {
        let ops = pie_chart(
            &ChartCtx::test(rect(), "Split", AxisTitles::default(), true),
            &series(),
            false,
        );
        let texts: Vec<String> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        for p in ["40%", "35%", "25%"] {
            assert!(
                texts.contains(&p.to_string()),
                "percentage {p} in {texts:?}"
            );
        }
    }

    /// With "show value" off, the wedges and category labels still draw, but no percentage data label
    /// is emitted.
    #[test]
    fn show_labels_false_omits_percentages() {
        let ops = pie_chart(
            &ChartCtx::test(rect(), "Split", AxisTitles::default(), false),
            &series(),
            false,
        );
        let wedges = ops
            .iter()
            .filter(|o| matches!(o, DrawOp::Polygon(_)))
            .count();
        let texts: Vec<String> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(wedges, 3, "one wedge per slice without labels");
        for c in ["Canada", "USA", "Mexico"] {
            assert!(texts.contains(&c.to_string()), "category {c} in {texts:?}");
        }
        assert!(
            !texts.iter().any(|t| t.ends_with('%')),
            "no percentage labels: {texts:?}"
        );
    }

    /// The 3-D variant draws the three slice faces plus a shaded crust: it emits more polygons than
    /// the flat pie, and at least one crust polygon carries a darkened (non-palette) fill.
    #[test]
    fn depth_pie_draws_shaded_crust() {
        let flat = pie_chart(
            &ChartCtx::test(rect(), "Split", AxisTitles::default(), false),
            &series(),
            false,
        );
        let deep = pie_chart(
            &ChartCtx::test(rect(), "Split", AxisTitles::default(), false),
            &series(),
            true,
        );

        let polys = |ops: &[DrawOp]| {
            ops.iter()
                .filter(|o| matches!(o, DrawOp::Polygon(_)))
                .count()
        };
        assert!(
            polys(&deep) > polys(&flat),
            "3-D pie adds crust polygons: {} vs {}",
            polys(&deep),
            polys(&flat)
        );

        // The three slice faces keep their palette colour; the crust walls are darkened copies, so a
        // shaded fill that is not any base slice colour must appear.
        use rpt_pages::Fill;
        let base: Vec<Color> = (0..3).map(slice_color).collect();
        let fills: Vec<Color> = deep
            .iter()
            .filter_map(|o| match o {
                DrawOp::Polygon(p) => match &p.fill {
                    Some(Fill::Solid(c)) => Some(*c),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        let has_face = base.iter().all(|b| fills.contains(b));
        let has_crust = fills.iter().any(|c| !base.contains(c));
        assert!(has_face, "every slice face keeps its palette colour");
        assert!(has_crust, "a darkened crust wall is emitted");
    }

    /// Degenerate and extreme inputs must not panic and must stay bounded: an empty or all-zero
    /// series draws nothing; a single slice draws; a thousand slices stay within the tessellation cap;
    /// a NaN/Inf value is tolerated (folded away by the positive-total guard) rather than panicking.
    #[test]
    fn degenerate_and_high_cardinality_inputs_do_not_panic() {
        let r = rect();
        // Empty and all-zero series → nothing to plot.
        let plain = ChartCtx::test(r, "", AxisTitles::default(), false);
        let labeled = ChartCtx::test(r, "", AxisTitles::default(), true);
        assert!(pie_chart(&plain, &[], false).is_empty());
        let zeros = vec![("a".to_string(), 0.0), ("b".to_string(), 0.0)];
        assert!(pie_chart(&labeled, &zeros, true).is_empty());

        // Single slice (flat and 3-D).
        let one = vec![("only".to_string(), 5.0)];
        assert!(!pie_chart(&labeled, &one, false).is_empty());
        assert!(!pie_chart(&labeled, &one, true).is_empty());

        // A thousand slices: bounded op count, no panic, in both variants.
        let many: Vec<(String, f64)> = (0..1000).map(|i| (format!("c{i}"), 1.0)).collect();
        let titled = ChartCtx::test(r, "T", AxisTitles::default(), true);
        for depth in [false, true] {
            let ops = pie_chart(&titled, &many, depth);
            assert!(!ops.is_empty());
            // Every polygon stays within the per-slice tessellation cap (512 + centre/close).
            for op in &ops {
                if let DrawOp::Polygon(p) = op {
                    assert!(p.points.len() <= 2 * 514, "bounded tessellation");
                }
            }
        }

        // NaN / Inf values are tolerated without panicking.
        let bad = vec![
            ("nan".to_string(), f64::NAN),
            ("inf".to_string(), f64::INFINITY),
            ("neg".to_string(), -3.0),
            ("ok".to_string(), 2.0),
        ];
        let _ = pie_chart(&titled, &bad, false);
        let _ = pie_chart(&titled, &bad, true);
    }

    /// The crust is the front-facing lower rim only: every darkened crust polygon lies below the
    /// ellipse centre (`sin a > 0` maps to `y > cy`), never on the hidden back rim.
    #[test]
    fn crust_walls_are_on_the_front_rim() {
        use rpt_pages::Fill;
        let deep = pie_chart(
            &ChartCtx::test(rect(), "", AxisTitles::default(), false),
            &series(),
            true,
        );
        // Reconstruct the ellipse centre the renderer uses (empty title → disc centred on the rect).
        let (_cx, cy, radius) = centered_disc(rect(), 0);
        let crust = (radius as f64 * PIE_3D_CRUST_FRAC).max(1.0);
        let cyf = cy as f64 - crust / 2.0;

        let base: Vec<Color> = (0..3).map(slice_color).collect();
        let crust_polys: Vec<&PolygonOp> = deep
            .iter()
            .filter_map(|o| match o {
                DrawOp::Polygon(p) => match &p.fill {
                    Some(Fill::Solid(c)) if !base.contains(c) => Some(p),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert!(!crust_polys.is_empty(), "some crust is drawn");
        for p in &crust_polys {
            // Endpoint segments touch the centre line where `sin a = 0`; allow a twip of rounding.
            assert!(
                p.points.iter().all(|pt| pt.y.0 as f64 >= cyf - 1.0),
                "crust wall stays on the front (lower) rim, below the ellipse centre"
            );
        }
    }
}
