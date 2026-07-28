//! Serialization: the XHTML head (style tables) and the per-element body, matching the native
//! viewer's emission spec.

use crate::model::{Elem, Pos};
use crate::tables::Tables;
use rpt_model::Color;
use rpt_pages::{ImageFit, ObjectKind, TextAlign};
use std::fmt::Write as _;

// The following literals reproduce the native viewer's output exactly and are parity-locked: their
// values match the engine's HTML and must not change.

/// `z-index` of the `crystalstyle` container's default positioned `div`.
const Z_DEFAULT: u8 = 25;
/// `z-index` of a section band.
const Z_SECTION: u8 = 3;
/// `z-index` of an image box.
const Z_IMAGE: u8 = 10;
/// `z-index` of a line and of a chart (SVG) island.
const Z_LINE: u8 = 15;
/// The frame offset the viewer applies via `BODY` margins and the `crystalstyle` div's top/left, px.
const FRAME_OFFSET_PX: u8 = 31;
/// Symbol fallback family appended to every font stack so a viewer resolves glyphs the named family
/// lacks (⚠ etc.). Matches the face `rpt-text` bundles for the PDF/raster backends (DejaVu Sans);
/// the HTML/SVG backends name families and let the viewer resolve them, so naming it is enough.
const SYMBOL_FALLBACK_FAMILY: &str = "DejaVu Sans";

/// The four border sides of an adornment, in the order the engine emits them.
const SIDES: [&str; 4] = ["left", "right", "top", "bottom"];

// ---------------------------------------------------------------------------------------------
// Serialization
// ---------------------------------------------------------------------------------------------

pub(crate) fn emit_head(h: &mut String, tables: &Tables, uid: &str) {
    h.push_str(
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Transitional//EN\" \
         \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd\"><HTML><style>\n",
    );
    let _ = writeln!(
        h,
        "    div.crystalstyle div {{position:absolute; z-index:{Z_DEFAULT}}}"
    );
    h.push_str(
        "    div.crystalstyle a {text-decoration:none}\n\
         \x20\x20\x20\x20div.crystalstyle a img {border-style:none; border-width:0}\n\
         \x20\x20\x20\x20div.crystalstyle div.tbg {background: url(\"js/dhtmllib/images/transp.gif\") repeat-x repeat-y;}\n",
    );
    for (i, f) in tables.fonts.iter().enumerate() {
        let _ = write!(
            h,
            "\t.fc{uid}-{i} {{font-size:{size}pt;color:{color};font-family:\"{fam}\",\"{SYMBOL_FALLBACK_FAMILY}\";font-weight:{weight};",
            size = fmt_pt(f.size_milli),
            color = css_rgb(f.rgb),
            fam = f.family,
            weight = if f.bold { "bold" } else { "normal" },
        );
        if f.italic {
            h.push_str("font-style:italic;");
        }
        match (f.underline, f.strikethrough) {
            (true, true) => h.push_str("text-decoration:underline line-through !important;"),
            (true, false) => h.push_str("text-decoration:underline !important;"),
            (false, true) => h.push_str("text-decoration:line-through !important;"),
            (false, false) => {}
        }
        h.push_str("}\n");
    }
    for (i, a) in tables.adorns.iter().enumerate() {
        let _ = write!(h, "\t.ad{uid}-{i} {{");
        if let Some(bg) = a.bg {
            let _ = write!(
                h,
                "background-color:{c};layer-background-color:{c};",
                c = css_rgb(bg)
            );
        }
        let _ = write!(h, "border-color:{};", css_rgb(a.border_rgb));
        if a.has_border() {
            h.push_str("border-style:solid;border-width:0px;");
            for (side, (style, w)) in SIDES.iter().zip(a.sides.iter()) {
                let _ = write!(
                    h,
                    "border-{side}-style:{s};border-{side}-width:{w}px;",
                    s = style.css()
                );
            }
        } else {
            for side in SIDES {
                let _ = write!(h, "border-{side}-width:0px;");
            }
        }
        if a.radius_px > 0 {
            let _ = write!(h, "border-radius:{}px;", a.radius_px);
        }
        h.push_str("}\n");
    }
    // One class per distinct image: its bytes inlined once as a stretched background, referenced by
    // class at every placement so identical images (a per-page logo, duplicate thumbnails) embed once.
    for (i, im) in tables.images.iter().enumerate() {
        let _ = writeln!(
            h,
            "\t.im{uid}-{i} {{background-image:url(data:{media};base64,{data});\
             background-size:100% 100%;background-repeat:no-repeat;}}",
            media = escape_attr(&im.media_type),
            data = rpt_render_util::base64_encode(&im.bytes),
        );
    }
    let _ = write!(
        h,
        "</style>\n<TITLE>Crystal Report Viewer</TITLE>\n\
         <BODY BGCOLOR=\"FFFFFF\" LEFTMARGIN={FRAME_OFFSET_PX} TOPMARGIN={FRAME_OFFSET_PX}>\n\
         <Div class=\"crystalstyle\" style=\"position:absolute; top:{FRAME_OFFSET_PX}px; left:{FRAME_OFFSET_PX}px; \">\n",
    );
}

pub(crate) fn emit_elem(h: &mut String, e: &Elem, uid: &str, container_w: i64) {
    match e {
        Elem::Section {
            id,
            top,
            height,
            bg,
        } => {
            let _ = write!(
                h,
                "    <div id=\"{id}\" style=\"z-index:{Z_SECTION};top:{top}px;left:0px;width:{w}px;height:{height}px;",
                id = escape_attr(id),
                w = container_w,
            );
            if let Some(bg) = bg {
                let _ = write!(
                    h,
                    "background-color:{c};layer-background-color:{c};",
                    c = css_rgb(*bg)
                );
            }
            h.push_str("\">\n\n    </div>\n");
        }
        Elem::Para {
            id,
            section,
            kind,
            pos,
            adorn,
            align,
            lines,
            line_height,
            rotation,
        } => {
            let mut style = pos_style(pos);
            if matches!(align, TextAlign::Right) {
                style.push_str("text-align:right;");
            }
            style.push_str(&rotate_style(*rotation));
            let _ = writeln!(
                h,
                "    <div{id} class=\"ad{uid}-{adorn}\"{data} style=\"{style}\">",
                id = id_attr(id),
                data = data_attrs(section, id.as_deref(), *kind),
            );
            let _ = write!(
                h,
                "        <p{align} style=\"position:relative;padding-left:1px;margin:0px;white-space:nowrap;\">",
                align = p_align(*align),
            );
            for (font, text, justify) in lines {
                // A justified line stretches to its usable width by spreading its inter-word spacing to
                // both edges (`text-align-last:justify` forces the single line; `white-space:normal`
                // lets it stretch). Non-justified lines keep the compact `nowrap` block, unchanged.
                let line_style = match justify {
                    Some(w) => format!(
                        "position:relative;display:block;line-height:{line_height}px;\
                         width:{w}px;text-align:justify;text-align-last:justify;white-space:normal;"
                    ),
                    None => format!("position:relative;display:block;line-height:{line_height}px;"),
                };
                let _ = write!(
                    h,
                    "<span style=\"{line_style}\"><span class=\"fc{uid}-{font}\">{t}</span></span>",
                    t = escape_html(text),
                );
            }
            h.push_str("</p>\n    </div>\n");
        }
        Elem::Cell {
            id,
            section,
            kind,
            pos,
            adorn,
            align,
            font,
            text,
            rotation,
        } => {
            let mut style = pos_style(pos);
            if matches!(align, TextAlign::Right) {
                style.push_str("text-align:right;");
            }
            style.push_str(&rotate_style(*rotation));
            let _ = write!(
                h,
                "    <div{id} class=\"ad{uid}-{adorn}\"{data} style=\"{style}\">\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20<table width=\"100%\" border=\"0\" cellpadding=\"0\" cellspacing=\"0\">\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<tr>\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<td><table width=\"100%\" border=\"0\" cellpadding=\"0\" cellspacing=\"0\">\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<tr>\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<td{td_align} nowrap=\"true\"><span class=\"fc{uid}-{font}\">{t}</span></td>\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20</tr>\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20</table></td>\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20</tr>\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20</table>\n    </div>\n",
                id = id_attr(id),
                data = data_attrs(section, id.as_deref(), *kind),
                td_align = td_align(*align),
                t = escape_html(text),
            );
        }
        Elem::BoxDiv { id, pos, adorn } => {
            let _ = writeln!(
                h,
                "    <div{id} class=\"ad{uid}-{adorn}\" style=\"{style}\"></div>",
                id = id_attr(id),
                style = pos_style(pos),
            );
        }
        Elem::Line {
            id,
            pos,
            horizontal,
            thick,
            rgb,
        } => {
            let side = if *horizontal { "top" } else { "left" };
            let _ = writeln!(
                h,
                "    <div{id} style=\"z-index:{Z_LINE};top:{t}px;left:{l}px;\
                 border-color:{c};border-style:solid;border-width:0px;\
                 border-{side}-width:{thick}px;width:{w}px;height:{ht}px;\"></div>",
                id = id_attr(id),
                t = pos.top,
                l = pos.left,
                c = css_rgb(*rgb),
                w = pos.width,
                ht = pos.height,
            );
        }
        Elem::Image {
            top,
            left,
            width,
            height,
            class,
            fit,
        } => {
            match class {
                // Bytes available: reference the shared image class (one embedded copy, N references).
                // `Contain` overrides the class's stretch (`background-size:100% 100%`) with an
                // aspect-preserving, centered fit — Crystal letterboxes pictures rather than distort.
                Some(i) => {
                    let fit_style = match fit {
                        ImageFit::Contain => {
                            "background-size:contain;background-position:center center;"
                        }
                        ImageFit::Fill => "",
                    };
                    let _ = write!(
                        h,
                        "    <div style=\"z-index:{Z_IMAGE};top:{top}px;left:{left}px;\">\n\
                         \x20\x20\x20\x20\x20\x20\x20\x20<div class=\"im{uid}-{i}\" style=\"width:{width}px;height:{height}px;{fit_style}\"></div>\n\
                         \x20\x20\x20\x20</div>\n",
                    );
                }
                // No bytes for this op (chart, or picture bytes not decoded): draw a visible
                // placeholder box rather than a broken reference to an unwritten file.
                None => {
                    let _ = write!(
                        h,
                        "    <div style=\"z-index:{Z_IMAGE};top:{top}px;left:{left}px;\">\n\
                         \x20\x20\x20\x20\x20\x20\x20\x20<div class=\"rpt-image-missing\" title=\"image not embedded\" style=\"width:{width}px;height:{height}px;border:1px dashed #b0b0b0;box-sizing:border-box;\"></div>\n\
                         \x20\x20\x20\x20</div>\n",
                    );
                }
            }
        }
        Elem::SvgIsland { pos, svg } => {
            // The chart as one positioned box; the inline <svg> (width/height 100%) fills it.
            let _ = writeln!(
                h,
                "    <div style=\"z-index:{Z_LINE};{}overflow:hidden;\">{svg}</div>",
                pos_style(pos),
            );
        }
    }
}

fn pos_style(p: &Pos) -> String {
    format!(
        "top:{}px;left:{}px;width:{}px;height:{}px;",
        p.top, p.left, p.width, p.height
    )
}

fn id_attr(id: &Option<String>) -> String {
    match id {
        Some(n) if !n.is_empty() => format!(" id=\"{}\"", escape_attr(n)),
        _ => String::new(),
    }
}

fn data_attrs(section: &str, object: Option<&str>, kind: ObjectKind) -> String {
    format!(
        " data-section=\"{}\" data-object=\"{}\" data-kind=\"{:?}\"",
        escape_attr(section),
        escape_attr(object.unwrap_or("")),
        kind
    )
}

fn p_align(a: TextAlign) -> &'static str {
    match a {
        TextAlign::Center => " align=\"center\"",
        TextAlign::Right => " align=\"right\"",
        _ => "",
    }
}

/// The CSS transform that rotates a text element about its top-left corner. Our rotation is degrees
/// counter-clockwise; CSS `rotate()` is clockwise-positive, so the angle is negated. `0.0` (upright)
/// emits nothing, keeping non-rotated output byte-identical.
fn rotate_style(rotation: f32) -> String {
    if rotation == 0.0 {
        String::new()
    } else {
        format!(
            "transform:rotate({:.4}deg);transform-origin:top left;",
            -rotation
        )
    }
}

fn td_align(a: TextAlign) -> &'static str {
    match a {
        TextAlign::Center => " align=\"center\"",
        TextAlign::Right => " align=\"right\"",
        _ => "",
    }
}

fn fmt_pt(size_milli: i32) -> String {
    if size_milli % 1000 == 0 {
        (size_milli / 1000).to_string()
    } else {
        let s = format!("{:.3}", size_milli as f64 / 1000.0);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn css_rgb(c: (u8, u8, u8)) -> String {
    Color {
        a: 255,
        r: c.0,
        g: c.1,
        b: c.2,
    }
    .to_hex()
}

/// Escape text content and turn spaces into `&nbsp;` (the engine pre-measures geometry, so runs
/// never reflow — `white-space:nowrap` plus non-breaking spaces). The `&`/`<`/`>` escaping is the
/// shared XML text escape; the `&nbsp;` step is HTML-backend-specific.
fn escape_html(s: &str) -> String {
    rpt_render_util::escape_xml_text(s).replace(' ', "&nbsp;")
}

use rpt_render_util::escape_xml_attr as escape_attr;
