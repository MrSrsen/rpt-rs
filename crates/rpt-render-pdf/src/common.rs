//! Helpers shared by both PDF writers ([`crate::writer_basic`] and the krilla backend): the
//! twip→point conversion, colour/geometry conversion, text-anchor math, and the Bézier constant.

use rpt_model::Color;
use rpt_pages::Fill;

/// 20 twips per PDF point.
pub(crate) use rpt_render_util::TWIPS_PER_POINT as TWIPS_PER_PT;

/// The text-anchor and metric-less baseline math, shared with the other backends.
pub(crate) use rpt_render_util::{aligned_x, baseline_offset_twips};

/// The minimum stroke width, in points: a hairline (0.25 pt at 72 dpi) so a stored width of 0 still
/// renders as a thin visible line rather than vanishing. Both writers clamp to this.
pub(crate) const MIN_STROKE_PT: f64 = 0.25;

pub(crate) fn pt(twips: i32) -> f64 {
    twips as f64 / TWIPS_PER_PT
}

/// Rough width estimate (average 0.5 em per char) — the fallback anchor for a run with no resolved
/// metrics (centre/right placement only; the run is shaped by the writer/font when actually drawn).
pub(crate) fn approx_text_width(text: &str, size: f64) -> f64 {
    text.chars().count() as f64 * size * 0.5
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
