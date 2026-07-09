//! Helpers shared by both PDF writers ([`crate::writer_basic`] and the krilla backend): the
//! twip→point conversion, colour/geometry conversion, text-anchor math, and the Bézier constant.

use rpt_model::Color;
use rpt_pages::{Fill, TextAlign, TextRun};

/// 20 twips per PDF point.
pub(crate) use rpt_render_util::TWIPS_PER_POINT as TWIPS_PER_PT;

pub(crate) fn pt(twips: i32) -> f64 {
    twips as f64 / TWIPS_PER_PT
}

/// The x anchor for a text run given its box left/width and shaped width, in whatever unit the caller
/// works in (points here). Both writers place text at this anchor; only the emission (krilla op vs
/// raw operator) differs.
pub(crate) fn aligned_x(align: TextAlign, left: f64, box_w: f64, text_w: f64) -> f64 {
    match align {
        TextAlign::Left | TextAlign::Justified => left,
        TextAlign::Center => left + (box_w - text_w) / 2.0,
        TextAlign::Right => left + box_w - text_w,
    }
}

/// Rough width estimate (average 0.5 em per char) — the fallback anchor for a run with no resolved
/// metrics (centre/right placement only; the run is shaped by the writer/font when actually drawn).
pub(crate) fn approx_text_width(text: &str, size: f64) -> f64 {
    text.chars().count() as f64 * size * 0.5
}

/// The baseline offset below a run's top edge, in twips: the run's resolved ascent when it carries
/// metrics, else the ~0.8-em heuristic from the font point size. Returned in twips (f64, unrounded)
/// so the twips-space basic writer stays byte-exact; the point-space krilla backend scales it down.
pub(crate) fn baseline_offset_twips(run: &TextRun) -> f64 {
    match &run.metrics {
        Some(m) => m.ascent.0 as f64,
        None => run.font.size_pt as f64 * 0.8 * TWIPS_PER_PT,
    }
}

pub(crate) fn chan(v: u8) -> f64 {
    v as f64 / 255.0
}

/// The solid colour a fill paints as in the PDF backends. Gradient and hatch fills are not tiled
/// here; both writers fall back to the fill's [`Fill::representative_color`] (a gradient's midpoint
/// stop, a hatch's foreground). A [`Fill::Solid`] returns its own colour, so solid output is
/// unchanged by the fill widening.
pub(crate) fn solid_of(fill: &Fill) -> Color {
    fill.representative_color()
}

/// Kappa: the cubic-Bézier control-point offset (× radius) that approximates a quarter ellipse arc.
pub(crate) const KAPPA: f64 = 0.552_284_749_83;
