//! Horizontal-tab resolution.
//!
//! A tab is a layout advance, not a glyph: a shaper maps `U+0009` to `.notdef` and paints a box. It
//! is therefore resolved here, before the Page IR, by splitting a run at its tabs and placing each
//! segment at the stop its tab advanced to. Backends then never see a control character, and the
//! stop rule lives in one crate instead of once per shaper.

use rpt_model::Twips;
use rpt_pages::{TextAlign, TextLayout, TextRun};

/// The tab-stop interval in twips: a quarter inch from the line's left edge. This is the engine's
/// own stop grid — text it draws after a tab lands on a multiple of this offset from the text
/// object's left edge, whatever the font.
const TAB_STOP: i32 = 360;

/// The pen position a tab advances to: the next tab stop beyond `pen` (twips from the line start).
fn next_stop(pen: f64) -> f64 {
    let stop = f64::from(TAB_STOP);
    ((pen / stop).floor() + 1.0) * stop
}

/// A local offset along the run's text direction, as a page-space `(dx, dy)`. Backends rotate a run
/// about its own top-left, so a segment placed `d` along the text must have its box origin moved by
/// the same rotation of `(d, 0)` — CCW degrees in a y-down space.
fn along_text(d: f64, rotation: f32) -> (f64, f64) {
    if rotation == 0.0 {
        return (d, 0.0);
    }
    let rad = f64::from(rotation).to_radians();
    (d * rad.cos(), -d * rad.sin())
}

/// Split `run` at its horizontal tabs into positioned runs, each anchored at the stop its tab
/// advanced to. A run with no tab is returned unchanged, so this is inert for all but tabbed text.
///
/// The segments carry the resolved position outright ([`TextAlign::Left`], box left at the stop), so
/// the alignment anchor is computed here from the run's full tabbed advance — the same width the
/// unsplit run would have handed the backend. A justified tabbed line loses its inter-word
/// stretching: the tab stops already fix where its parts sit.
pub(crate) fn expand_tabs(run: TextRun, layout: &dyn TextLayout) -> Vec<TextRun> {
    if !run.text.contains('\t') {
        return vec![run];
    }
    // Walk the pen across the segments, jumping to the next stop at each tab. `pen` ends as the
    // run's full advance, tabs included.
    let mut pen = 0.0;
    let mut placed: Vec<(f64, String)> = Vec::new();
    for (i, seg) in run.text.split('\t').enumerate() {
        if i > 0 {
            pen = next_stop(pen);
        }
        placed.push((pen, seg.to_string()));
        pen += crate::text::spaced_width_twips(layout, seg, &run.font, run.character_spacing);
    }
    let box_w = f64::from(run.bounds.width.0);
    let anchor = match run.align {
        TextAlign::Center => (box_w - pen) / 2.0,
        TextAlign::Right => box_w - pen,
        TextAlign::Left | TextAlign::Justified => 0.0,
    };
    let mut out = Vec::with_capacity(placed.len());
    for (offset, text) in placed {
        // A tab that ends the run, or two in a row, leaves an empty segment: it advanced the pen
        // and has nothing to draw.
        if text.is_empty() {
            continue;
        }
        let d = anchor + offset;
        let (dx, dy) = along_text(d, run.rotation);
        let mut seg = run.clone();
        if let Some(m) = seg.metrics.as_mut() {
            m.advance = Twips(crate::text::spaced_width_twips(
                layout,
                &text,
                &run.font,
                run.character_spacing,
            ) as i32);
        }
        seg.text = text;
        seg.bounds.left = Twips(run.bounds.left.0 + dx.round() as i32);
        seg.bounds.top = Twips(run.bounds.top.0 + dy.round() as i32);
        // The segment box runs from its stop to the original box's right edge (upright text only —
        // a rotated run's box width is across the text, not along it).
        if run.rotation == 0.0 {
            seg.bounds.width = Twips((run.bounds.width.0 - dx.round() as i32).max(0));
        }
        seg.align = TextAlign::Left;
        out.push(seg);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_model::{Color, Rect};
    use rpt_pages::{ApproxLayout, FontSpec, TextMetrics};

    fn run(text: &str) -> TextRun {
        TextRun {
            bounds: Rect {
                left: Twips(1000),
                top: Twips(500),
                width: Twips(8000),
                height: Twips(240),
            },
            text: text.to_string(),
            font: FontSpec::default(),
            color: Color::default(),
            align: TextAlign::Left,
            rotation: 0.0,
            metrics: Some(TextMetrics {
                advance: Twips(0),
                ascent: Twips(160),
                line_height: Twips(240),
            }),
            character_spacing: Twips(0),
            source: None,
        }
    }

    #[test]
    fn next_stop_advances_to_the_next_quarter_inch() {
        assert_eq!(next_stop(0.0), 360.0);
        assert_eq!(next_stop(1.0), 360.0);
        assert_eq!(next_stop(359.9), 360.0);
        // Landing exactly on a stop still advances — a tab is never a zero-width no-op.
        assert_eq!(next_stop(360.0), 720.0);
        assert_eq!(next_stop(361.0), 720.0);
    }

    #[test]
    fn a_run_without_a_tab_is_untouched() {
        let r = run("Printed Date:");
        let out = expand_tabs(r.clone(), &ApproxLayout);
        assert_eq!(out, vec![r]);
    }

    #[test]
    fn a_tab_advances_the_pen_instead_of_drawing_a_glyph() {
        // The tab must not survive into the Page IR (a shaper would paint it as `.notdef`), and the
        // text after it starts at a tab stop measured from the run's left edge.
        let layout = ApproxLayout;
        let out = expand_tabs(run("ab\tcd"), &layout);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|r| !r.text.contains('\t')));
        assert_eq!(out[0].text, "ab");
        assert_eq!(out[1].text, "cd");
        assert_eq!(out[0].bounds.left, Twips(1000));
        let advanced = out[1].bounds.left.0 - out[0].bounds.left.0;
        assert_eq!(advanced % TAB_STOP, 0, "segment starts on a tab stop");
        assert!(advanced as f64 > layout.width_twips("ab", &out[0].font));
        // Each segment measures only its own text, and the box runs to the original right edge.
        assert_eq!(
            out[1].metrics.unwrap().advance,
            Twips(layout.width_twips("cd", &out[1].font) as i32)
        );
        assert_eq!(out[1].bounds.width, Twips(8000 - advanced));
    }

    #[test]
    fn consecutive_tabs_each_advance_one_stop() {
        let out = expand_tabs(run("ab\t\tcd"), &ApproxLayout);
        // The empty segment between the two tabs draws nothing.
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].bounds.left.0 - out[0].bounds.left.0, 2 * TAB_STOP);
    }

    #[test]
    fn a_trailing_tab_leaves_one_run_and_no_control_character() {
        let out = expand_tabs(run("Total for X:\t"), &ApproxLayout);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "Total for X:");
        assert_eq!(out[0].bounds.left, Twips(1000));
    }

    #[test]
    fn alignment_anchors_on_the_full_tabbed_advance() {
        // Centre/right anchor the whole tabbed line, then the segments keep their stop spacing; each
        // segment is positioned outright, so backends never re-align them.
        let layout = ApproxLayout;
        let mut r = run("ab\tcd");
        r.align = TextAlign::Right;
        let out = expand_tabs(r, &layout);
        let total = f64::from(TAB_STOP) + layout.width_twips("cd", &out[0].font);
        assert_eq!(out[0].bounds.left, Twips(1000 + (8000.0 - total) as i32));
        assert_eq!(out[1].bounds.left.0 - out[0].bounds.left.0, TAB_STOP);
        assert!(out.iter().all(|r| r.align == TextAlign::Left));
    }

    #[test]
    fn a_quarter_turn_run_advances_along_its_own_text_direction() {
        // 90° CCW flows the text up the page, so the stop moves the box origin in -y, not +x.
        let mut r = run("ab\tcd");
        r.rotation = 90.0;
        let out = expand_tabs(r, &ApproxLayout);
        assert_eq!(out[1].bounds.left, out[0].bounds.left);
        assert_eq!(out[0].bounds.top.0 - out[1].bounds.top.0, TAB_STOP);
    }
}
