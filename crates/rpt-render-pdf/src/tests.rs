use crate::{render_page, render_pages, render_pages_basic};
use rpt_model::{Color, Twips};
use rpt_pages::{DrawOp, Fill, Page, TextAlign, TextRun};
use rpt_pages::{
    FontSpec, LineOp, LineStyle, ObjectKind, ObjectRef, PageSize, Point, RectOp, Stroke,
};

fn sample() -> Page {
    let mut p = Page::new(
        1,
        PageSize {
            width: Twips(12240),
            height: Twips(15840),
        },
    );
    p.push(DrawOp::Rect(RectOp {
        bounds: rpt_model::Rect {
            left: Twips(720),
            top: Twips(720),
            width: Twips(4000),
            height: Twips(400),
        },
        fill: Some(
            Color {
                a: 255,
                r: 230,
                g: 240,
                b: 255,
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
        source: Some(ObjectRef::new("Details", ObjectKind::Box)),
    }));
    p.push(DrawOp::Text(TextRun {
        bounds: rpt_model::Rect {
            left: Twips(760),
            top: Twips(760),
            width: Twips(3900),
            height: Twips(320),
        },
        text: "Hello (PDF)".into(),
        font: FontSpec {
            size_pt: 12.0,
            ..FontSpec::default()
        },
        color: Color {
            a: 255,
            r: 0,
            g: 0,
            b: 0,
        },
        align: TextAlign::Left,
        rotation: 0.0,
        metrics: None,
        source: None,
    }));
    p.push(DrawOp::Line(LineOp {
        from: Point::new(720, 1300),
        to: Point::new(4720, 1300),
        stroke: Stroke {
            color: Color {
                a: 255,
                r: 0,
                g: 0,
                b: 0,
            },
            width: Twips(20),
            style: LineStyle::Single,
        },
        source: None,
    }));
    p
}

#[test]
fn golden_basic_pdf_snapshot() {
    // The dependency-free writer emits fully deterministic PDF (no embedded fonts / timestamps),
    // so a byte-golden pins its exact structure — a content-op change fails even if the %PDF magic
    // and probe substrings still match.
    rpt_test_support::assert_golden_bytes(
        env!("CARGO_MANIFEST_DIR"),
        "basic.pdf",
        &render_pages_basic(std::slice::from_ref(&sample())),
    );
}

#[test]
fn basic_pdf_is_deterministic() {
    let a = render_pages_basic(std::slice::from_ref(&sample()));
    let b = render_pages_basic(std::slice::from_ref(&sample()));
    assert_eq!(a, b, "the basic PDF writer must be deterministic");
}

#[test]
fn render_pages_emits_pdf() {
    // The default entry point (krilla backend when enabled) must produce valid PDF bytes for a
    // page with text + rect + line without panicking.
    let bytes = render_page(&sample());
    assert!(bytes.starts_with(b"%PDF"), "must start with %PDF header");
    assert!(bytes.len() > 200, "must contain real content");
}

#[test]
fn render_pages_multipage() {
    let pages = vec![sample(), sample(), sample()];
    let bytes = render_pages(&pages);
    assert!(bytes.starts_with(b"%PDF"));
}

/// A text run that follows a stroked path op (line/rule) must be drawn fill-only. Text carries no
/// stroke, so a stroke left active on the surface by the preceding line would make krilla fill *and*
/// stroke the glyphs (text render mode `2 Tr`) — a doubled/haloed run. The page in `sample()` places
/// its line last, so add a text run after it and assert no glyph run is stroked.
#[cfg(feature = "krilla-backend")]
#[test]
fn text_after_line_is_fill_only() {
    let mut page = sample();
    page.push(DrawOp::Text(TextRun {
        bounds: rpt_model::Rect {
            left: Twips(720),
            top: Twips(1400),
            width: Twips(3900),
            height: Twips(320),
        },
        text: "After the line".into(),
        font: FontSpec {
            size_pt: 10.0,
            ..FontSpec::default()
        },
        color: Color {
            a: 255,
            r: 0,
            g: 0,
            b: 0,
        },
        align: TextAlign::Left,
        rotation: 0.0,
        metrics: None,
        source: None,
    }));
    let bytes = render_page(&page);
    let content = inflated_content(&bytes);
    assert!(
        content.contains(" TJ") || content.contains(" Tj"),
        "expected the krilla backend to emit glyph runs"
    );
    assert!(
        !content.contains("2 Tr") && !content.contains("1 Tr"),
        "text must be fill-only (0 Tr); a stroked run (1/2 Tr) means a leaked stroke doubled the glyphs"
    );
}

/// Concatenate every `/FlateDecode` content stream in `pdf`, inflated to text, for op-level asserts.
#[cfg(feature = "krilla-backend")]
fn inflated_content(pdf: &[u8]) -> String {
    let marker = b"stream";
    let mut out = String::new();
    let mut i = 0;
    while let Some(rel) = find(&pdf[i..], marker) {
        let mut s = i + rel + marker.len();
        // Skip the EOL after `stream` (CRLF or LF).
        if pdf.get(s) == Some(&b'\r') {
            s += 1;
        }
        if pdf.get(s) == Some(&b'\n') {
            s += 1;
        }
        let Some(erel) = find(&pdf[s..], b"endstream") else {
            break;
        };
        let end = s + erel;
        if let Ok(bytes) = miniz_oxide::inflate::decompress_to_vec_zlib(&pdf[s..end]) {
            out.push_str(&String::from_utf8_lossy(&bytes));
            out.push('\n');
        }
        i = end + b"endstream".len();
    }
    out
}

#[cfg(feature = "krilla-backend")]
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[test]
fn basic_writer_emits_valid_pdf_structure() {
    let bytes = render_pages_basic(std::slice::from_ref(&sample()));
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.starts_with("%PDF-1.4"));
    assert!(s.trim_end().ends_with("%%EOF"));
    assert!(s.contains("/Type /Catalog"));
    assert!(s.contains("/Type /Pages"));
    assert!(s.contains("/MediaBox [0 0 612.00 792.00]")); // letter in points
    assert!(s.contains("BT /F1 12.00 Tf"));
    assert!(s.contains("(Hello \\(PDF\\)) Tj")); // parens escaped
    assert!(s.contains("re\n")); // rect
    assert!(s.contains(" m ") && s.contains(" l S")); // line
    assert!(s.contains("xref"));
    assert!(s.contains("/Root 1 0 R"));
}

#[test]
fn basic_writer_multipage_count() {
    let pages = vec![sample(), sample(), sample()];
    let bytes = render_pages_basic(&pages);
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("/Count 3"));
}

fn one_op_page(op: DrawOp) -> Page {
    let mut p = Page::new(
        1,
        PageSize {
            width: Twips(2000),
            height: Twips(2000),
        },
    );
    p.push(op);
    p
}

#[test]
fn basic_writer_emits_ellipse_beziers() {
    use rpt_pages::EllipseOp;
    let page = one_op_page(DrawOp::Ellipse(EllipseOp {
        bounds: rpt_model::Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(400),
            height: Twips(400),
        },
        fill: Some(
            Color {
                a: 255,
                r: 10,
                g: 20,
                b: 30,
            }
            .into(),
        ),
        stroke: None,
        source: None,
    }));
    let s = String::from_utf8_lossy(&render_pages_basic(std::slice::from_ref(&page))).into_owned();
    // Four cubic-Bézier arcs and a fill.
    assert_eq!(s.matches(" c\n").count(), 4, "{s}");
    assert!(s.contains("f\n"));
}

#[test]
fn basic_writer_rounded_rect_emits_beziers_not_re() {
    let rounded = DrawOp::Rect(RectOp {
        bounds: rpt_model::Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(3000),
            height: Twips(1500),
        },
        fill: Some(
            Color {
                a: 255,
                r: 10,
                g: 20,
                b: 30,
            }
            .into(),
        ),
        stroke: None,
        corner_radius: Twips(300),
        source: None,
    });
    let s = String::from_utf8_lossy(&render_pages_basic(&[one_op_page(rounded)])).into_owned();
    // Four corner arcs, no `re` rectangle operator, closed and filled.
    assert_eq!(s.matches(" c\n").count(), 4, "{s}");
    assert!(!s.contains(" re\n"), "{s}");
    assert!(s.contains("h\n") && s.contains("f\n"), "{s}");
}

#[test]
fn basic_writer_rotated_text_wraps_in_cm() {
    let rotated = DrawOp::Text(TextRun {
        bounds: rpt_model::Rect {
            left: Twips(200),
            top: Twips(200),
            width: Twips(600),
            height: Twips(300),
        },
        text: "R".into(),
        font: FontSpec::default(),
        color: Color {
            a: 255,
            r: 0,
            g: 0,
            b: 0,
        },
        align: TextAlign::Left,
        rotation: 90.0,
        metrics: None,
        source: None,
    });
    let s = String::from_utf8_lossy(&render_pages_basic(&[one_op_page(rotated)])).into_owned();
    // A rotation `cm` wraps the text object.
    assert!(s.contains(" cm\n"), "{s}");
    assert!(s.contains("q ") && s.contains("Q\n"));
}

#[test]
fn basic_writer_justified_text_sets_word_spacing() {
    let justified = |align, width| {
        DrawOp::Text(TextRun {
            bounds: rpt_model::Rect {
                left: Twips(0),
                top: Twips(0),
                width: Twips(width),
                height: Twips(240),
            },
            text: "two words here".into(),
            font: FontSpec::default(),
            color: Color {
                a: 255,
                r: 0,
                g: 0,
                b: 0,
            },
            align,
            rotation: 0.0,
            metrics: None,
            source: None,
        })
    };
    // A justified run under its box width sets a positive word spacing (`Tw`) and resets it (`0 Tw`).
    let s = String::from_utf8_lossy(&render_pages_basic(&[one_op_page(justified(
        TextAlign::Justified,
        9000,
    ))]))
    .into_owned();
    assert!(s.contains(" Tw "), "expected word spacing, got: {s}");
    assert!(s.contains("0 Tw ET"), "word spacing reset, got: {s}");
    // A left run emits no word spacing.
    let left = String::from_utf8_lossy(&render_pages_basic(&[one_op_page(justified(
        TextAlign::Left,
        9000,
    ))]))
    .into_owned();
    assert!(!left.contains(" Tw "), "{left}");
}

#[test]
fn basic_writer_solid_fill_unchanged_and_gradient_falls_back() {
    // A gradient fill paints as its representative (midpoint) solid colour — same `re`/`f` shape as
    // a solid fill, so the basic writer stays deterministic and valid.
    let grad = DrawOp::Rect(RectOp {
        bounds: rpt_model::Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(400),
            height: Twips(400),
        },
        fill: Some(Fill::LinearGradient {
            stops: vec![
                (
                    0.0,
                    Color {
                        a: 255,
                        r: 0,
                        g: 0,
                        b: 0,
                    },
                ),
                (
                    1.0,
                    Color {
                        a: 255,
                        r: 255,
                        g: 255,
                        b: 255,
                    },
                ),
            ],
            angle_deg: 0.0,
        }),
        stroke: None,
        corner_radius: Twips(0),
        source: None,
    });
    let s = String::from_utf8_lossy(&render_pages_basic(&[one_op_page(grad)])).into_owned();
    assert!(s.contains("re\n") && s.contains("f\n"));
}
