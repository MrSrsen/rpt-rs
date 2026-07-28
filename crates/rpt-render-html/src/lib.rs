//! HTML output backend for the [`rpt_pages`] Page IR, shaped to match the native Crystal engine's
//! RAS-direct HTML output structurally — not just positionally.
//!
//! Coordinate model: `px = round(twips / 15)` at 96 dpi — see [`rpt_render_util`] for the
//! cross-backend coordinate reference.
//!
//! The document mirrors that native HTML export's emission spec: an
//! XHTML 1.0 Transitional frame, one `<div class="crystalstyle" …overflow:hidden>` container per
//! page, and one positioned `<div>` per report object. Two per-report, deduplicated style tables
//! (`fc<uid>-N` typography classes and `ad<uid>-N` adornment/border classes) live in the `<style>`
//! block; objects reference them by class and carry only position/z-index inline.
//!
//! Object templates:
//! - **Template A** (`<p>`/stacked `<span display:block>`): every TextObject and any field whose
//!   value wrapped to ≥2 visual lines.
//! - **Template B** (nested `<table>`): a single-line FieldObject.
//! - Section background, Box, Line, Image: empty/near-empty positioned divs.
//!
//! Geometry converts twips → CSS px at 96 dpi with `px = round(twips / 15)` (round half away from
//! zero). Positions are the page-relative coordinates carried in the Page IR. The additive
//! `data-object`/`data-section`/`data-kind` attributes are kept on object divs for the parity
//! tooling; they are not part of the native output.

mod emit;
mod model;
mod tables;

use crate::emit::{emit_elem, emit_head};
use crate::model::{build_page, PageModel};
use crate::tables::Tables;
use rpt_pages::{ImageAsset, Page};
use rpt_render_util::TWIPS_PER_PX;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Page margin the RAS host applies to each page container, in px (= 360 twips = 0.25").
pub(crate) const PAGE_MARGIN_PX: i64 = 24;

/// `round(twips / 15)`, rounding half away from zero (96 dpi), matching the native engine.
pub(crate) fn px(twips: i32) -> i64 {
    let v = twips as f64 / TWIPS_PER_PX;
    if v >= 0.0 {
        (v + 0.5).floor() as i64
    } else {
        -((-v + 0.5).floor() as i64)
    }
}

/// Render a slice of pages to one self-contained, native-shaped HTML document. Image ops are
/// drawn as placeholders (no bytes are available); use [`render_pages_with_assets`] to embed images.
pub fn render_pages(pages: &[Page]) -> String {
    render_pages_with_assets(pages, &BTreeMap::new())
}

/// Like [`render_pages`], but embeds each image op whose `image_id` has an entry in `assets` as an
/// inline `data:` URI, so the output stays a single self-contained file (safe to write to a pipe).
/// An image op with no matching asset is drawn as a visible placeholder box.
pub fn render_pages_with_assets(pages: &[Page], assets: &BTreeMap<String, ImageAsset>) -> String {
    let mut tables = Tables::default();
    let models: Vec<PageModel> = pages
        .iter()
        .map(|p| build_page(p, &mut tables, assets))
        .collect();
    let uid = tables.uid();

    let mut h = String::new();
    emit_head(&mut h, &tables, &uid);

    // Page containers, concatenated. Container `top` accumulates page-absolutely; page 1 carries a
    // top margin, the last page a bottom margin (matching the native RAS host).
    let mut cum_top: i64 = 0;
    let last = models.len().saturating_sub(1);
    for (i, m) in models.iter().enumerate() {
        let margin = if i == 0 {
            format!("margin-top:{PAGE_MARGIN_PX}px;")
        } else if i == last {
            format!("margin-bottom:{PAGE_MARGIN_PX}px;")
        } else {
            String::new()
        };
        let _ = writeln!(
            h,
            "<div class=\"crystalstyle\" style=\"{margin}margin-left:{m}px;margin-right:{m}px;\
             top:{top}px;left:0px;width:{w}px;height:{ht}px;overflow:hidden;\">",
            m = PAGE_MARGIN_PX,
            top = cum_top,
            w = m.width,
            ht = m.height,
        );
        for e in &m.elems {
            emit_elem(&mut h, e, &uid, m.width);
        }
        h.push_str("</div>\n");
        cum_top += m.height + if i == 0 { PAGE_MARGIN_PX } else { 0 };
    }

    h.push_str("</Div>\n</BODY>\n</HTML>\n");
    h
}

/// Render a single page to a self-contained HTML document.
pub fn render_page(page: &Page) -> String {
    render_pages(std::slice::from_ref(page))
}

/// The HTML backend as a [`PageBackend`](rpt_pages::PageBackend): one self-contained document embedding the document's
/// [`assets`](rpt_pages::PagedDocument::assets), so a caller never threads images separately.
#[derive(Debug, Default, Clone, Copy)]
pub struct HtmlBackend;

/// Knobs for [`HtmlBackend`]. None today (images come from the document's assets); the struct exists
/// so future HTML options are an additive field, not a signature change.
#[derive(Debug, Default, Clone, Copy)]
pub struct HtmlOptions;

impl rpt_pages::PageBackend for HtmlBackend {
    type Output = String;
    type Options = HtmlOptions;

    fn render(&self, doc: &rpt_pages::PagedDocument, _opts: &HtmlOptions) -> String {
        render_pages_with_assets(&doc.pages, &doc.assets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_model::{Color, Rect, Twips};
    use rpt_pages::{
        DrawOp, FontSpec, ImageAsset, ImageFit, LineOp, LineStyle, ObjectKind, ObjectRef, Page,
        PageSize, Point, RectOp, Stroke, TextAlign, TextRun,
    };

    fn text_run(
        left: i32,
        top: i32,
        w: i32,
        h: i32,
        text: &str,
        kind: ObjectKind,
        name: &str,
    ) -> DrawOp {
        DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(left),
                top: Twips(top),
                width: Twips(w),
                height: Twips(h),
            },
            text: text.into(),
            font: FontSpec::default(),
            color: Color {
                a: 255,
                r: 0,
                g: 0,
                b: 0,
            },
            align: TextAlign::Left,
            rotation: 0.0,
            metrics: None,
            source: Some(ObjectRef::new("DetailSection1", kind).named(name)),
        })
    }

    /// A text run stamped with a placement instance id (as the layout engine emits).
    fn text_run_inst(top: i32, text: &str, name: &str, instance: u32) -> DrawOp {
        let DrawOp::Text(mut t) = text_run(420, top, 660, 240, text, ObjectKind::Field, name)
        else {
            unreachable!()
        };
        t.source = t.source.map(|s| s.with_instance(instance));
        DrawOp::Text(t)
    }

    fn page_with(ops: Vec<DrawOp>) -> Page {
        let mut p = Page::new(
            1,
            PageSize {
                width: Twips(12240),
                height: Twips(15840),
            },
        );
        for op in ops {
            p.push(op);
        }
        p
    }

    /// A rotated text object emits a CSS rotate transform about its top-left (our CCW angle negated to
    /// CSS's clockwise-positive `rotate`); upright text emits no transform.
    #[test]
    fn rotated_text_object_emits_css_transform() {
        let rotated = |deg: f32| {
            let DrawOp::Text(mut t) = text_run(200, 300, 1400, 3276, "Rot", ObjectKind::Text, "R")
            else {
                unreachable!()
            };
            t.rotation = deg;
            render_page(&page_with(vec![DrawOp::Text(t)]))
        };
        assert!(rotated(90.0).contains("transform:rotate(-90.0000deg);transform-origin:top left;"));
        assert!(rotated(270.0).contains("transform:rotate(-270.0000deg)"));
        assert!(!rotated(0.0).contains("transform:rotate"));
    }

    /// A justified wrapped line stretches to both edges via `text-align-last:justify`; a left line does
    /// not, and a justified line with no inter-word gap is left alone.
    #[test]
    fn justified_line_emits_justify_css() {
        let line = |align, text: &str| {
            let DrawOp::Text(mut t) = text_run(100, 100, 2000, 240, text, ObjectKind::Text, "J")
            else {
                unreachable!()
            };
            t.align = align;
            render_page(&page_with(vec![DrawOp::Text(t)]))
        };
        assert!(line(TextAlign::Justified, "two words here").contains("text-align-last:justify"));
        assert!(!line(TextAlign::Left, "two words here").contains("text-align-last:justify"));
        assert!(!line(TextAlign::Justified, "singleword").contains("text-align-last:justify"));
    }

    /// A deterministic page exercising the op kinds and attributes the `contains` probes don't pin:
    /// a right-aligned field, a multi-word text object (escaping + spacing), a filled+stroked box,
    /// and a line.
    fn snapshot_page() -> Page {
        let field = DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(420),
                top: Twips(300),
                width: Twips(1200),
                height: Twips(240),
            },
            text: "42".into(),
            font: FontSpec::default(),
            color: Color {
                a: 255,
                r: 0,
                g: 0,
                b: 0,
            },
            align: TextAlign::Right,
            rotation: 0.0,
            metrics: None,
            source: Some(ObjectRef::new("Details", ObjectKind::Field).named("qty")),
        });
        let label = DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(150),
                top: Twips(300),
                width: Twips(3000),
                height: Twips(240),
            },
            text: "A & B < C".into(),
            font: FontSpec::default(),
            color: Color {
                a: 255,
                r: 20,
                g: 40,
                b: 60,
            },
            align: TextAlign::Left,
            rotation: 0.0,
            metrics: None,
            source: Some(ObjectRef::new("Details", ObjectKind::Text).named("label")),
        });
        let box_op = DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(120),
                top: Twips(240),
                width: Twips(4000),
                height: Twips(360),
            },
            fill: Some(
                Color {
                    a: 255,
                    r: 240,
                    g: 240,
                    b: 240,
                }
                .into(),
            ),
            stroke: Some(Stroke {
                color: Color {
                    a: 255,
                    r: 0,
                    g: 0,
                    b: 0,
                },
                width: Twips(15),
                style: LineStyle::Single,
            }),
            corner_radius: Twips(0),
            source: Some(ObjectRef::new("Details", ObjectKind::Box).named("frame")),
        });
        let line = DrawOp::Line(LineOp {
            from: Point::new(120, 620),
            to: Point::new(4120, 620),
            stroke: Stroke {
                color: Color {
                    a: 255,
                    r: 0,
                    g: 0,
                    b: 0,
                },
                width: Twips(10),
                style: LineStyle::Single,
            },
            source: Some(ObjectRef::new("Details", ObjectKind::Line).named("rule")),
        });
        page_with(vec![box_op, line, label, field])
    }

    #[test]
    fn golden_html_page_snapshot() {
        rpt_test_support::assert_golden(
            env!("CARGO_MANIFEST_DIR"),
            "page.html",
            &render_page(&snapshot_page()),
        );
    }

    #[test]
    fn frame_is_reportrenderer_shaped() {
        let html = render_page(&page_with(vec![text_run(
            150,
            150,
            3000,
            300,
            "A & B",
            ObjectKind::Field,
            "name1",
        )]));
        assert!(html.contains("XHTML 1.0 Transitional"));
        assert!(html.contains("<TITLE>Crystal Report Viewer</TITLE>"));
        assert!(html.contains("BGCOLOR=\"FFFFFF\" LEFTMARGIN=31 TOPMARGIN=31"));
        assert!(html.contains("div.crystalstyle div {position:absolute; z-index:25}"));
        // One page container, content width = 816 - 48 = 768px.
        assert!(html.contains("width:768px;height:"));
        assert!(html.contains("overflow:hidden;"));
        // Ampersand escaped, spaces → &nbsp;.
        assert!(html.contains("A&nbsp;&amp;&nbsp;B"));
    }

    #[test]
    fn single_line_field_uses_table_template() {
        let html = render_page(&page_with(vec![text_run(
            420,
            1939,
            660,
            240,
            "1",
            ObjectKind::Field,
            "id1",
        )]));
        assert!(html.contains("id=\"id1\""));
        assert!(html
            .contains("<table width=\"100%\" border=\"0\" cellpadding=\"0\" cellspacing=\"0\">"));
        assert!(html.contains("nowrap=\"true\""));
        assert!(html.contains("data-object=\"id1\""));
    }

    #[test]
    fn text_object_uses_paragraph_template() {
        let html = render_page(&page_with(vec![text_run(
            420,
            1519,
            660,
            240,
            "ID",
            ObjectKind::Text,
            "Text9",
        )]));
        assert!(html.contains("id=\"Text9\""));
        assert!(html.contains(
            "<p style=\"position:relative;padding-left:1px;margin:0px;white-space:nowrap;\">"
        ));
        assert!(html.contains("display:block;line-height:"));
        assert!(!html.contains("<table"));
    }

    #[test]
    fn stacked_lines_merge_into_one_paragraph() {
        // Two runs of the same object, one line-height apart → one Template-A div, two spans.
        let html = render_page(&page_with(vec![
            text_run(9711, 1332, 1095, 240, "Numeric", ObjectKind::Text, "Text13"),
            text_run(9711, 1572, 1095, 240, "Code", ObjectKind::Text, "Text13"),
        ]));
        // Exactly one object div for Text13.
        assert_eq!(html.matches("id=\"Text13\"").count(), 1);
        // Both lines present as stacked spans, top = min run top (1332/15 = 89).
        assert!(html.contains(">Numeric</span>"));
        assert!(html.contains(">Code</span>"));
        assert!(html.contains("top:89px;"));
    }

    #[test]
    fn instance_id_groups_exactly_regardless_of_gap() {
        // Two runs sharing an instance id are one placed object: they merge into one div even when the
        // vertical gap far exceeds the line-height heuristic (which would have split them).
        let html = render_page(&page_with(vec![
            text_run_inst(1000, "line one", "wrapped", 7),
            text_run_inst(9000, "line two", "wrapped", 7),
        ]));
        assert_eq!(
            html.matches("id=\"wrapped\"").count(),
            1,
            "same instance id → one object div"
        );
        assert!(html.contains(">line&nbsp;one</span>"));
        assert!(html.contains(">line&nbsp;two</span>"));

        // Same name but distinct instance ids are two placements → two divs, even one line apart.
        let html = render_page(&page_with(vec![
            text_run_inst(1000, "a", "dup", 1),
            text_run_inst(1240, "b", "dup", 2),
        ]));
        assert_eq!(
            html.matches("id=\"dup\"").count(),
            2,
            "distinct instance ids → separate divs"
        );
    }

    #[test]
    fn distinct_rows_do_not_merge() {
        // Same field name on two detail rows a cell-height apart → two separate divs.
        let html = render_page(&page_with(vec![
            text_run(420, 1939, 660, 840, "1", ObjectKind::Field, "id1"),
            text_run(420, 2779, 660, 840, "2", ObjectKind::Field, "id1"),
        ]));
        assert_eq!(html.matches("id=\"id1\"").count(), 2);
    }

    #[test]
    fn font_and_adornment_classes_are_deduped() {
        let html = render_page(&page_with(vec![
            text_run(0, 0, 660, 240, "a", ObjectKind::Field, "f1"),
            text_run(0, 500, 660, 240, "b", ObjectKind::Field, "f2"),
        ]));
        // Two identical default fonts dedupe to a single fc class definition.
        let uid_defs: Vec<_> = html.match_indices(".fc").collect();
        assert_eq!(
            uid_defs.len(),
            1,
            "duplicate fonts should dedupe to one class"
        );
    }

    #[test]
    fn geometry_rounds_half_away_from_zero() {
        // 1474 twips / 15 = 98.27 → 98; 221/15 = 14.73 → 15.
        assert_eq!(px(1474), 98);
        assert_eq!(px(221), 15);
        assert_eq!(px(11474), 765);
        assert_eq!(px(0), 0);
    }

    #[test]
    fn section_background_is_inline_and_empty() {
        let mut p = page_with(vec![]);
        p.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(360),
                top: Twips(1939),
                width: Twips(12240),
                height: Twips(840),
            },
            fill: Some(
                Color {
                    a: 255,
                    r: 255,
                    g: 255,
                    b: 0,
                }
                .into(),
            ),
            stroke: None,
            corner_radius: Twips(0),
            source: Some(ObjectRef::new("DetailSection1", ObjectKind::Section)),
        }));
        let html = render_page(&p);
        assert!(html.contains("id=\"DetailSection1\""));
        assert!(html.contains("z-index:3;"));
        assert!(html.contains("background-color:#ffff00;layer-background-color:#ffff00;"));
    }

    #[test]
    fn bordered_text_object_merges_box_rect_as_adornment() {
        let mut p = page_with(vec![text_run(
            440,
            440,
            11340,
            600,
            "Title",
            ObjectKind::Text,
            "Text7",
        )]);
        p.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(440),
                top: Twips(440),
                width: Twips(11340),
                height: Twips(600),
            },
            fill: Some(
                Color {
                    a: 255,
                    r: 255,
                    g: 255,
                    b: 255,
                }
                .into(),
            ),
            stroke: Some(Stroke {
                color: Color {
                    a: 255,
                    r: 0,
                    g: 255,
                    b: 255,
                },
                width: Twips(60),
                style: LineStyle::Double,
            }),
            corner_radius: Twips(0),
            source: Some(ObjectRef::new("DetailSection1", ObjectKind::Box).named("Text7")),
        }));
        let html = render_page(&p);
        // The box rect is not a standalone box; it becomes Text7's adornment class.
        assert_eq!(html.matches("id=\"Text7\"").count(), 1);
        assert!(html.contains("border-color:#00ffff;"));
        assert!(html.contains("border-top-style:double;"));
    }

    #[test]
    fn rounded_box_emits_border_radius() {
        let mut p = page_with(vec![]);
        p.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(440),
                top: Twips(440),
                width: Twips(3000),
                height: Twips(1500),
            },
            fill: None,
            stroke: Some(Stroke {
                color: Color {
                    a: 255,
                    r: 0,
                    g: 0,
                    b: 0,
                },
                width: Twips(15),
                style: LineStyle::Single,
            }),
            // 225 twips / 15 = 15px.
            corner_radius: Twips(225),
            source: Some(ObjectRef::new("DetailSection1", ObjectKind::Box).named("Box1")),
        }));
        let html = render_page(&p);
        assert!(html.contains("border-radius:15px;"), "{html}");
        // A square box (radius 0) must not emit the property.
        let mut sq = page_with(vec![]);
        sq.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(440),
                top: Twips(440),
                width: Twips(3000),
                height: Twips(1500),
            },
            fill: None,
            stroke: Some(Stroke {
                color: Color {
                    a: 255,
                    r: 0,
                    g: 0,
                    b: 0,
                },
                width: Twips(15),
                style: LineStyle::Single,
            }),
            corner_radius: Twips(0),
            source: Some(ObjectRef::new("DetailSection1", ObjectKind::Box).named("Box1")),
        }));
        assert!(!render_page(&sq).contains("border-radius:"));
    }

    fn image_page(id: &str) -> Page {
        image_page_fit(id, ImageFit::Fill)
    }

    fn image_page_fit(id: &str, fit: ImageFit) -> Page {
        let mut p = Page::new(
            1,
            PageSize {
                width: Twips(4000),
                height: Twips(4000),
            },
        );
        p.push(DrawOp::Image(rpt_pages::ImageOp {
            bounds: Rect {
                left: Twips(100),
                top: Twips(100),
                width: Twips(720),
                height: Twips(720),
            },
            image_id: id.to_string(),
            fit,
            source: Some(ObjectRef::new("DetailSection1", ObjectKind::Image).named(id)),
        }));
        p
    }

    #[test]
    fn image_without_asset_renders_placeholder_not_dangling_ref() {
        let html = render_page(&image_page("Picture1"));
        // No broken reference to an unwritten sidecar file, and a visible placeholder instead.
        assert!(!html.contains("images/Picture1.png"));
        assert!(!html.contains("<img"));
        assert!(html.contains("rpt-image-missing"));
    }

    #[test]
    fn image_with_asset_inlines_data_uri() {
        // A 1x1 PNG (real magic so sniff_media_type accepts it).
        let png: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d,
        ];
        let mut assets = BTreeMap::new();
        assets.insert(
            "Picture1".to_string(),
            ImageAsset {
                media_type: "image/png".to_string(),
                bytes: png.to_vec(),
            },
        );
        let html = render_pages_with_assets(&[image_page("Picture1")], &assets);
        // The bytes are embedded once as a background-image CSS class, referenced by class.
        assert!(html.contains("background-image:url(data:image/png;base64,"));
        assert!(!html.contains("images/Picture1.png"));
        // The encoded PNG header round-trips through our base64.
        assert!(html.contains(&rpt_render_util::base64_encode(png)));
        // The placement references the class rather than inlining an <img> per occurrence.
        assert!(!html.contains("<img"));
        assert!(html.contains("class=\"im"));
    }

    #[test]
    fn contain_image_overrides_stretch_with_aspect_fit() {
        let png: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d,
        ];
        let mut assets = BTreeMap::new();
        assets.insert(
            "Picture1".to_string(),
            ImageAsset {
                media_type: "image/png".to_string(),
                bytes: png.to_vec(),
            },
        );
        // Fill leaves the class's 100%-stretch; Contain adds an inline override to letterbox.
        let fill = render_pages_with_assets(&[image_page("Picture1")], &assets);
        assert!(!fill.contains("background-size:contain"));
        let contain =
            render_pages_with_assets(&[image_page_fit("Picture1", ImageFit::Contain)], &assets);
        assert!(contain.contains("background-size:contain;background-position:center center;"));
    }

    #[test]
    fn identical_image_bytes_embed_once_referenced_many() {
        // The same PNG placed on two pages must be inlined once (one data: URI) and referenced twice.
        let png: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d,
        ];
        let mut assets = BTreeMap::new();
        for id in ["Logo#0", "Logo#1"] {
            assets.insert(
                id.to_string(),
                ImageAsset {
                    media_type: "image/png".to_string(),
                    bytes: png.to_vec(),
                },
            );
        }
        let html = render_pages_with_assets(&[image_page("Logo#0"), image_page("Logo#1")], &assets);
        // One embedded copy of the bytes.
        assert_eq!(
            html.matches("background-image:url(data:image/png;base64,")
                .count(),
            1
        );
        // Two references to the shared class.
        assert_eq!(html.matches("class=\"im").count(), 2);
    }
}
