use crate::{
    render_page, render_pages, try_render_pages, try_render_pages_with_options, Conformance,
    FontSource, PdfError, PdfOptions, Producer, Timestamp, RPT_RS_PRODUCER,
};
use rpt_model::{Color, Twips};
use rpt_pages::{DrawOp, Fill, Page, TextAlign, TextRun};
use rpt_pages::{
    FontSpec, LineOp, LineStyle, ObjectKind, ObjectRef, PageSize, Point, RectOp, Stroke,
};
use rpt_test_support::pdf::operator_listing;
use std::collections::BTreeMap;

/// Opaque black — `Color::default()` is fully transparent, which would paint nothing.
const BLACK: Color = Color {
    a: 255,
    r: 0,
    g: 0,
    b: 0,
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
            color: BLACK,
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
        color: BLACK,
        align: TextAlign::Left,
        rotation: 0.0,
        metrics: None,
        character_spacing: Twips(0),
        source: None,
    }));
    p.push(DrawOp::Line(LineOp {
        from: Point::new(720, 1300),
        to: Point::new(4720, 1300),
        stroke: Stroke {
            color: BLACK,
            width: Twips(20),
            style: LineStyle::Single,
        },
        source: None,
    }));
    p
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

/// The normalized operator listing of a one-op page — the assertion surface for the op-level tests.
///
/// Normalized rather than the raw inflated stream: PDF makes the whitespace around operators
/// optional, so how the writer breaks its lines is not something an op-level assertion should be
/// pinned to. The listing re-emits one operator per line with its operands.
fn content_of(op: DrawOp) -> String {
    operator_listing(&render_pages(&[one_op_page(op)]))
}

#[test]
fn render_pages_emits_pdf() {
    // The entry point must produce valid PDF bytes for a page with text + rect + line.
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

#[test]
fn pdf_is_deterministic() {
    // Nothing in the writer reads the clock or an RNG, and krilla's own serialization is ordered, so
    // two renders of the same pages must be byte-identical — the property every committed PDF
    // comparison depends on.
    let a = render_pages(&[sample(), sample()]);
    let b = render_pages(&[sample(), sample()]);
    assert_eq!(a, b, "the PDF writer must be deterministic");
}

#[test]
fn an_empty_page_slice_still_emits_one_page() {
    let bytes = render_pages(&[]);
    assert!(bytes.starts_with(b"%PDF"));
    assert!(bytes.len() > 200);
}

#[test]
fn a_degenerate_page_size_clamps_instead_of_losing_the_document() {
    // A zero/negative page size is clamped to a minimal page, so one bad page cannot abort
    // serialization of the whole document.
    let bad = Page::new(
        1,
        PageSize {
            width: Twips(0),
            height: Twips(-500),
        },
    );
    let bytes =
        try_render_pages(&[bad, sample()]).expect("a clamped page size must still serialize");
    assert!(bytes.starts_with(b"%PDF"));
    assert!(
        operator_listing(&bytes).contains(" TJ") || operator_listing(&bytes).contains(" Tj"),
        "the good page's text must survive the bad page"
    );
}

#[test]
fn the_failure_page_names_the_cause() {
    // The infallible entry points return this instead of degrading silently. It must be a valid PDF
    // that draws glyphs, so the reader sees the message rather than a blank page.
    let bytes = crate::writer_krilla::render_failure_page(&PdfError::Font("no glyf table".into()));
    assert!(bytes.starts_with(b"%PDF"));
    let content = operator_listing(&bytes);
    assert!(
        content.contains(" TJ") || content.contains(" Tj"),
        "the failure page must draw its message: {content}"
    );
}

/// A text run that follows a stroked path op (line/rule) must be drawn fill-only. Text carries no
/// stroke, so a stroke left active on the surface by the preceding line would make krilla fill *and*
/// stroke the glyphs (text render mode `2 Tr`) — a doubled/haloed run. The page in `sample()` places
/// its line last, so add a text run after it and assert no glyph run is stroked.
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
        color: BLACK,
        align: TextAlign::Left,
        rotation: 0.0,
        metrics: None,
        character_spacing: Twips(0),
        source: None,
    }));
    let bytes = render_page(&page);
    let content = operator_listing(&bytes);
    assert!(
        content.contains(" TJ") || content.contains(" Tj"),
        "expected the writer to emit glyph runs"
    );
    assert!(
        !content.contains("2 Tr") && !content.contains("1 Tr"),
        "text must be fill-only (0 Tr); a stroked run (1/2 Tr) means a leaked stroke doubled the glyphs"
    );
}

#[test]
fn ellipse_is_four_beziers() {
    use rpt_pages::EllipseOp;
    let s = content_of(DrawOp::Ellipse(EllipseOp {
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
    // krilla has no ellipse primitive: four cubic-Bézier quarter arcs, closed and filled.
    assert_eq!(s.matches(" c\n").count(), 4, "{s}");
    assert!(s.contains("f\n"), "{s}");
}

#[test]
fn rounded_rect_emits_beziers_not_re() {
    let s = content_of(DrawOp::Rect(RectOp {
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
    }));
    // Four corner arcs, no `re` rectangle operator, closed and filled.
    assert_eq!(s.matches(" c\n").count(), 4, "{s}");
    assert!(!s.contains(" re\n"), "{s}");
    assert!(s.contains("h\n") && s.contains("f\n"), "{s}");
}

#[test]
fn rotated_text_wraps_in_a_transform() {
    let s = content_of(DrawOp::Text(TextRun {
        bounds: rpt_model::Rect {
            left: Twips(200),
            top: Twips(200),
            width: Twips(600),
            height: Twips(300),
        },
        text: "R".into(),
        font: FontSpec::default(),
        color: BLACK,
        align: TextAlign::Left,
        rotation: 90.0,
        metrics: None,
        character_spacing: Twips(0),
        source: None,
    }));
    // The rotation rides in the `cm` transform matrix, not in the text matrix: krilla folds the
    // pushed rotation into the page's own y-flip, so the matrix stops being axis-aligned while the
    // `Tm` stays the plain flip an upright run gets.
    let cm_line = |s: &str| {
        s.lines()
            .find(|l| l.ends_with(" cm"))
            .unwrap_or_default()
            .to_string()
    };
    assert!(
        !cm_line(&s).starts_with("1 0 0 -1"),
        "the rotation must reach the transform matrix: {s}"
    );
    assert!(s.contains("q\n") && s.contains("Q\n"), "{s}");
    // An upright run emits only the page's own transform, so the extra `cm` is genuinely the rotation.
    let upright = content_of(DrawOp::Text(TextRun {
        bounds: rpt_model::Rect {
            left: Twips(200),
            top: Twips(200),
            width: Twips(600),
            height: Twips(300),
        },
        text: "R".into(),
        font: FontSpec::default(),
        color: BLACK,
        align: TextAlign::Left,
        rotation: 0.0,
        metrics: None,
        character_spacing: Twips(0),
        source: None,
    }));
    assert!(
        cm_line(&upright).starts_with("1 0 0 -1"),
        "an upright run's transform is the plain page flip: {upright}"
    );
}

#[test]
fn justified_text_spreads_words_across_the_box() {
    let run = |align, width| {
        DrawOp::Text(TextRun {
            bounds: rpt_model::Rect {
                left: Twips(0),
                top: Twips(0),
                width: Twips(width),
                height: Twips(240),
            },
            text: "two words here".into(),
            font: FontSpec::default(),
            color: BLACK,
            align,
            rotation: 0.0,
            metrics: None,
            character_spacing: Twips(0),
            source: None,
        })
    };
    // The krilla writer justifies by drawing each word at its own pen position (it does not use the
    // `Tw` word-spacing operator), and each gap's space glyph is drawn at the pen too — so a
    // three-word run emits five text-positioning operators where the same run left-aligned emits one.
    let justified = content_of(run(TextAlign::Justified, 9000));
    let left = content_of(run(TextAlign::Left, 9000));
    let placements = |s: &str| s.matches(" Td\n").count() + s.matches(" Tm\n").count();
    assert_eq!(placements(&justified), 5, "{justified}");
    assert_eq!(placements(&left), 1, "{left}");
}

#[test]
fn justified_text_draws_its_inter_word_spaces() {
    // Read back what the document says its glyphs mean, through the font's own `/ToUnicode` — the
    // text an extractor recovers without inferring words from the gaps between them. A gap the pen
    // merely skips over leaves no character behind, and the line extracts as one run-together token.
    let drawn_text = |text: &str, spacing| {
        let page = one_op_page(DrawOp::Text(TextRun {
            bounds: rpt_model::Rect {
                left: Twips(0),
                top: Twips(0),
                width: Twips(9000),
                height: Twips(240),
            },
            text: text.into(),
            font: FontSpec::default(),
            color: BLACK,
            align: TextAlign::Justified,
            rotation: 0.0,
            metrics: None,
            character_spacing: Twips(spacing),
            source: None,
        }));
        rpt_test_support::pdf::structure(&render_pages(&[page])).pages[0]
            .shows
            .iter()
            .map(|s| s.text.clone())
            .collect::<String>()
    };
    assert_eq!(drawn_text("two words here", 0), "two words here");
    // Every gap is its own space, so a run of them survives as itself rather than collapsing, and a
    // spaced-out run keeps them too — the gap is stretched by the justification extra, not replaced.
    assert_eq!(drawn_text("two  words here", 0), "two  words here");
    assert_eq!(drawn_text("two words here", 40), "two words here");
}

/// Mid-grey — a gradient's midpoint stop in these tests, so a fallback to the representative color
/// would be visible as this exact `rg`.
const GREY: Color = Color {
    a: 255,
    r: 128,
    g: 128,
    b: 128,
};

const WHITE: Color = Color {
    a: 255,
    r: 255,
    g: 255,
    b: 255,
};

/// A 20×20 pt box (400 twips) filled with `fill`.
fn filled_rect(fill: Fill) -> DrawOp {
    DrawOp::Rect(RectOp {
        bounds: rpt_model::Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(400),
            height: Twips(400),
        },
        fill: Some(fill),
        stroke: None,
        corner_radius: Twips(0),
        source: None,
    })
}

/// A black→grey→white gradient at `angle_deg`.
fn gradient(angle_deg: f32) -> Fill {
    Fill::LinearGradient {
        stops: vec![(0.0, BLACK), (0.5, GREY), (1.0, WHITE)],
        angle_deg,
    }
}

/// The PDF's shading dictionaries are not compressed, so the axial coordinates the writer chose are
/// readable straight out of the bytes. Returns every `/Coords [...]` array in document order.
fn shading_coords(pdf: &[u8]) -> Vec<Vec<f64>> {
    let text = String::from_utf8_lossy(pdf);
    text.split("/Coords")
        .skip(1)
        .filter_map(|frag| {
            let inner = frag.split('[').nth(1)?.split(']').next()?;
            Some(
                inner
                    .split_whitespace()
                    .filter_map(|n| n.parse::<f64>().ok())
                    .collect(),
            )
        })
        .collect()
}

/// How many times `needle` occurs in the raw PDF bytes.
fn count(pdf: &[u8], needle: &str) -> usize {
    String::from_utf8_lossy(pdf).matches(needle).count()
}

#[test]
fn a_solid_fill_paints_a_rect() {
    // A solid fill is a closed path filled with a device-RGB color.
    let solid = content_of(filled_rect(GREY.into()));
    assert!(solid.contains(" rg\n") && solid.contains("f\n"), "{solid}");
}

#[test]
fn a_gradient_paints_an_axial_shading_rather_than_a_solid() {
    let pdf = render_pages(&[one_op_page(filled_rect(gradient(0.0)))]);
    // A real PDF type-2 (axial) shading, reached through a shading pattern.
    assert_eq!(count(&pdf, "/ShadingType 2"), 1, "no axial shading emitted");
    assert_eq!(count(&pdf, "/PatternType 2"), 1, "no shading pattern");
    // The content stream selects the pattern color space instead of painting device RGB.
    let content = content_of(filled_rect(gradient(0.0)));
    assert!(content.contains("/Pattern cs"), "{content}");
    assert!(content.contains("scn"), "{content}");
    assert!(
        !content.contains("0.502 0.502 0.502 rg"),
        "gradient still painting the representative mid-grey: {content}"
    );
}

/// The angle convention is only observable in the shading's own coordinates, and this is what pins
/// it: 0° left→right, 90° bottom→top (counter-clockwise on a y-down surface), 180° the reverse of 0°.
/// A backend that had the sign inverted would put 90°'s axis top→bottom and fail here.
#[test]
fn the_gradient_angle_maps_to_the_documented_axis() {
    // Compared to a thousandth of a point: the writer works in f32, so an exact match would be
    // asserting on rounding rather than on the axis.
    let axis_is = |angle: f32, expected: [f64; 4]| {
        let pdf = render_pages(&[one_op_page(filled_rect(gradient(angle)))]);
        let got = shading_coords(&pdf).first().cloned().expect("a shading");
        assert_eq!(got.len(), 4, "{angle}°: {got:?}");
        for (g, e) in got.iter().zip(&expected) {
            assert!(
                (g - e).abs() < 1e-3,
                "{angle}°: got {got:?}, want {expected:?}"
            );
        }
    };
    // The box is 20×20 pt at the page's top-left, so its centre is (10, 10).
    axis_is(0.0, [0.0, 10.0, 20.0, 10.0]); // left→right
    axis_is(90.0, [10.0, 20.0, 10.0, 0.0]); // bottom→top on a y-down surface
    axis_is(180.0, [20.0, 10.0, 0.0, 10.0]); // the reverse of 0°
}

#[test]
fn a_hatch_paints_a_tiling_pattern() {
    let pdf = render_pages(&[one_op_page(filled_rect(Fill::Hatch {
        fg: BLACK,
        bg: WHITE,
        pattern: rpt_pages::HatchPattern::ForwardDiagonal,
    }))]);
    assert_eq!(
        count(&pdf, "/PatternType 1"),
        1,
        "no tiling pattern emitted"
    );
    // The 8×8-device-pixel GDI cell at 96 dpi: 6 pt square, stepped by its own size.
    assert_eq!(count(&pdf, "/BBox[0 0 6 6]"), 1, "unexpected tile box");
    assert_eq!(count(&pdf, "/XStep 6"), 1, "unexpected tile step");
    assert_eq!(count(&pdf, "/YStep 6"), 1, "unexpected tile step");
}

#[test]
fn every_hatch_variant_renders_its_own_tiling_pattern() {
    for pattern in [
        rpt_pages::HatchPattern::Horizontal,
        rpt_pages::HatchPattern::Vertical,
        rpt_pages::HatchPattern::ForwardDiagonal,
        rpt_pages::HatchPattern::BackwardDiagonal,
        rpt_pages::HatchPattern::Cross,
        rpt_pages::HatchPattern::DiagonalCross,
    ] {
        let pdf = render_pages(&[one_op_page(filled_rect(Fill::Hatch {
            fg: BLACK,
            bg: WHITE,
            pattern,
        }))]);
        assert_eq!(count(&pdf, "/PatternType 1"), 1, "{pattern:?} drew no tile");
        let content = content_of(filled_rect(Fill::Hatch {
            fg: BLACK,
            bg: WHITE,
            pattern,
        }));
        assert!(content.contains("/Pattern cs"), "{pattern:?}: {content}");
    }
}

/// Identical hatches must collapse to one PDF pattern object; different ones must not share.
#[test]
fn identical_hatches_share_one_pattern_object() {
    let hatch = |pattern| Fill::Hatch {
        fg: BLACK,
        bg: WHITE,
        pattern,
    };
    let two_pages = |a: Fill, b: Fill| {
        let mut page = one_op_page(filled_rect(a));
        page.push(filled_rect(b));
        render_pages(&[page])
    };
    let same = two_pages(
        hatch(rpt_pages::HatchPattern::Cross),
        hatch(rpt_pages::HatchPattern::Cross),
    );
    assert_eq!(
        count(&same, "/PatternType 1"),
        1,
        "identical tiles not shared"
    );
    let different = two_pages(
        hatch(rpt_pages::HatchPattern::Cross),
        hatch(rpt_pages::HatchPattern::Vertical),
    );
    assert_eq!(
        count(&different, "/PatternType 1"),
        2,
        "distinct tiles collapsed into one"
    );
}

/// The one surviving path to a representative solid: a gradient with nothing to interpolate has no
/// axis to shade along, so it paints flat rather than emitting a degenerate shading.
#[test]
fn a_gradient_with_no_stops_falls_back_to_a_solid() {
    let pdf = render_pages(&[one_op_page(filled_rect(Fill::LinearGradient {
        stops: vec![],
        angle_deg: 0.0,
    }))]);
    assert_eq!(count(&pdf, "/ShadingType 2"), 0, "shaded an empty gradient");
}

/// The PDF names each embedded face in plain text (`/BaseFont /XXXXXX+ArialMT`), so the subset tag
/// aside, the family the writer actually resolved and subset is observable in the bytes. The tag is
/// stripped; every distinct name is returned in document order.
fn embedded_faces(pdf: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(pdf);
    let mut names: Vec<String> = Vec::new();
    for frag in text.split("/BaseFont").skip(1) {
        let name = frag
            .trim_start()
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .next()
            .unwrap_or_default();
        // `AAAAAA+Family` — krilla's per-subset tag, which is not part of the face's identity.
        let family = name.rsplit('+').next().unwrap_or(name).to_string();
        if !family.is_empty() && !names.contains(&family) {
            names.push(family);
        }
    }
    names
}

/// Render `sample()` (whose text is `FontSpec::default()`, i.e. Arial) through an explicit font
/// source — the seam under test.
fn render_sample_with_fonts(fonts: FontSource) -> Vec<u8> {
    render_family_with_fonts("Arial", fonts)
}

/// Render one text run in `family` through `fonts`.
fn render_family_with_fonts(family: &str, fonts: FontSource) -> Vec<u8> {
    let page = one_op_page(DrawOp::Text(TextRun {
        bounds: rpt_model::Rect {
            left: Twips(100),
            top: Twips(100),
            width: Twips(1800),
            height: Twips(320),
        },
        text: "Hermetic".into(),
        font: FontSpec {
            family: family.to_string(),
            size_pt: 12.0,
            ..FontSpec::default()
        },
        color: BLACK,
        align: TextAlign::Left,
        rotation: 0.0,
        metrics: None,
        character_spacing: Twips(0),
        source: None,
    }));
    try_render_pages_with_options(
        &[page],
        &BTreeMap::new(),
        &PdfOptions {
            fonts,
            ..PdfOptions::default()
        },
    )
    .expect("a one-text-run page serializes")
}

#[test]
fn the_default_font_source_is_the_bundled_library() {
    // Reproducibility over local fonts: an options-less caller must not embed whatever this machine
    // happens to have installed, so the default is the bundled set and reading the host library is the
    // explicit choice.
    assert_eq!(PdfOptions::default().fonts, FontSource::Bundled);
    let faces = embedded_faces(&render_pages(&[sample()]));
    // `all` holds vacuously on a render that embedded no font at all, which is the regression this
    // assertion is here to catch — so the face list must be non-empty first.
    assert!(!faces.is_empty(), "the default render embedded no face");
    assert!(
        faces.iter().all(|f| f.starts_with("Liberation")),
        "the default render must embed only bundled faces, got {faces:?}"
    );
    assert_eq!(
        render_pages(&[sample()]),
        try_render_pages_with_options(&[sample()], &BTreeMap::new(), &PdfOptions::default())
            .expect("the sample page serializes"),
        "the default options must render byte-identically to the options-less entry point"
    );
}

#[test]
fn the_bundled_font_source_embeds_a_bundled_face() {
    // The seam's whole point: with `Bundled`, the face the PDF embeds is one of this crate's own,
    // whatever the host has installed — the only thing a committed PDF baseline can be blessed
    // against. On a host with a real Arial (as here) the system render embeds `ArialMT` instead, so
    // this assertion is what catches the seam being ignored.
    let faces = embedded_faces(&render_sample_with_fonts(FontSource::Bundled));
    assert!(!faces.is_empty(), "the bundled render embedded no face");
    assert!(
        faces.iter().all(|f| f.starts_with("Liberation")),
        "bundled render must embed only bundled faces, got {faces:?}"
    );
    // And it must be reproducible, which is the property the baseline actually rests on.
    assert_eq!(
        render_sample_with_fonts(FontSource::Bundled),
        render_sample_with_fonts(FontSource::Bundled),
        "the bundled render must be byte-identical across renders"
    );
}

#[test]
fn the_font_source_picks_the_face_the_writer_subsets() {
    // Render a family this host resolves to a face the bundled set does not have, through both
    // sources: the embedded face name and the bytes must both change.
    let families = families_the_bundled_set_lacks();
    // A host with no such family cannot observe the seam at all, which would leave this test green
    // having rendered nothing — so it fails instead, naming what is missing.
    assert!(
        !families.is_empty(),
        "no installed family resolves to a face outside the bundled set, so `FontSource::System` is \
         indistinguishable from `Bundled` here and this test would assert nothing; install any font \
         that is not Liberation or DejaVu"
    );
    for family in &families {
        let system = render_family_with_fonts(family, FontSource::System);
        let bundled = render_family_with_fonts(family, FontSource::Bundled);
        assert_ne!(
            embedded_faces(&system),
            embedded_faces(&bundled),
            "{family}: the font source must decide which face is embedded"
        );
        assert_ne!(
            system, bundled,
            "{family}: a different face, different bytes"
        );
    }
}

/// Every family whose host-resolved face differs from what the bundled set resolves it to, found by
/// hashing both databases' resolved face bytes.
///
/// Enumerated from the host database's own inventory rather than from a fixed candidate list: a list
/// names the families one machine happened to have, and on a runner without them the whole check
/// evaporates. Capped so the caller renders a bounded number of PDFs.
fn families_the_bundled_set_lacks() -> Vec<String> {
    let system = rpt_text::FontDb::with_system_fonts();
    let bundled = rpt_text::FontDb::bundled();
    let face_hash = |db: &rpt_text::FontDb, family: &str| {
        let spec = FontSpec {
            family: family.to_string(),
            ..FontSpec::default()
        };
        db.query(&spec)
            .and_then(|id| db.with_face_data(id, |data, _| rpt_render_util::content_hash(data)))
    };
    let mut families: Vec<String> = system
        .inventory()
        .faces
        .into_iter()
        .map(|f| f.family)
        .collect();
    families.sort();
    families.dedup();
    families
        .into_iter()
        .filter(|family| match face_hash(&system, family) {
            Some(host) => Some(host) != face_hash(&bundled, family),
            None => false,
        })
        .take(4)
        .collect()
}

/// Render `sample()` against a conformance level, everything else at its default.
fn render_conforming(level: Conformance) -> Result<Vec<u8>, PdfError> {
    try_render_pages_with_options(
        &[sample()],
        &BTreeMap::new(),
        &PdfOptions {
            conformance: level,
            ..PdfOptions::default()
        },
    )
}

/// A one-page document whose only op is a half-transparent filled rect — the thing PDF/A-1 forbids
/// and PDF/A-2 allows.
fn translucent_page() -> Page {
    one_op_page(DrawOp::Rect(RectOp {
        bounds: rpt_model::Rect {
            left: Twips(100),
            top: Twips(100),
            width: Twips(1000),
            height: Twips(500),
        },
        fill: Some(
            Color {
                a: 128,
                r: 10,
                g: 20,
                b: 30,
            }
            .into(),
        ),
        stroke: None,
        corner_radius: Twips(0),
        source: None,
    }))
}

#[test]
fn conformance_is_off_by_default_and_costs_the_ordinary_render_nothing() {
    // The whole point of the default: asking for no standard must add nothing an ordinary render did
    // not already carry — no conformance claim, no output intent, and above all no date, which is the
    // one piece of metadata that could come from a clock.
    assert_eq!(PdfOptions::default().conformance, Conformance::None);
    assert_eq!(PdfOptions::default().created, None);
    let plain = render_pages(&[sample()]);
    assert_eq!(
        plain,
        render_conforming(Conformance::None).expect("the sample page serializes"),
        "an explicit `None` must render byte-identically to the options-less entry point"
    );
    assert!(
        !plain.windows(6).any(|w| w == b"pdfaid"),
        "an unflagged render must make no PDF/A claim"
    );
    assert!(
        !plain.windows(14).any(|w| w == b"/OutputIntents"),
        "an unflagged render must write no output intent"
    );
    assert!(
        !plain.windows(13).any(|w| w == b"/CreationDate"),
        "an unflagged render must write no creation date"
    );
}

/// Render `sample()` with an explicit producer, everything else at its default.
fn render_with_producer(producer: Producer) -> Vec<u8> {
    try_render_pages_with_options(
        &[sample()],
        &BTreeMap::new(),
        &PdfOptions {
            producer,
            ..PdfOptions::default()
        },
    )
    .expect("the sample page serializes")
}

#[test]
fn the_producer_identity_is_the_name_plus_the_workspace_version() {
    // One source: the version the crate was compiled from, so the tag, `--version` and the string
    // inside a rendered file cannot disagree.
    assert_eq!(
        RPT_RS_PRODUCER,
        format!("rpt-rs {}", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(Producer::default().as_str(), Some(RPT_RS_PRODUCER));
}

#[test]
fn every_render_names_the_engine_that_produced_it() {
    // Provenance by default: a file found on its own must say what wrote it, in both places a reader
    // looks — the info dictionary and the XMP packet.
    let pdf = render_pages(&[sample()]);
    let text = String::from_utf8_lossy(&pdf);
    assert!(
        text.contains(&format!("/Producer({RPT_RS_PRODUCER})")),
        "the info dictionary must name the producer"
    );
    assert!(
        text.contains(&format!("/Creator({RPT_RS_PRODUCER})")),
        "the info dictionary must name the creator"
    );
    assert!(
        text.contains(&format!("<pdf:Producer>{RPT_RS_PRODUCER}</pdf:Producer>")),
        "the XMP packet must name the producer"
    );
    assert!(
        text.contains(&format!(
            "<xmp:CreatorTool>{RPT_RS_PRODUCER}</xmp:CreatorTool>"
        )),
        "the XMP packet must name the creator tool"
    );
}

#[test]
fn naming_the_producer_costs_the_render_no_reproducibility() {
    // The identity is a build constant, not a clock or a host: the same pages must still render to
    // the same bytes, and the document must still carry no date it was never given.
    let first = render_pages(&[sample()]);
    assert_eq!(
        first,
        render_pages(&[sample()]),
        "two renders of the same pages must be byte-identical"
    );
    assert!(
        !first.windows(13).any(|w| w == b"/CreationDate"),
        "naming a producer must not date the document"
    );
}

#[test]
fn a_caller_can_rebrand_or_anonymize_the_document() {
    // The override an embedding application needs, and the opt-out for a caller who wants the file to
    // name nothing at all.
    let named = String::from_utf8_lossy(&render_with_producer(Producer::Named(
        "Acme Reports 9.1".into(),
    )))
    .into_owned();
    assert!(named.contains("/Producer(Acme Reports 9.1)"));
    assert!(named.contains("<pdf:Producer>Acme Reports 9.1</pdf:Producer>"));
    assert!(
        !named.contains("rpt-rs"),
        "a rebranded document must not also name this crate"
    );
    let anonymous = render_with_producer(Producer::Anonymous);
    let text = String::from_utf8_lossy(&anonymous);
    assert!(!text.contains("/Producer"), "no producer was asked for");
    assert!(!text.contains("/Creator"), "nor a creator");
    assert!(
        !text.contains("<pdf:Producer>"),
        "and none in the XMP packet either"
    );
}

#[test]
fn an_archival_render_names_its_producer_too() {
    // PDF/A wants a self-describing file, and the info dictionary and XMP packet must agree — krilla
    // writes both from the one identity, so agreement is structural rather than checked here.
    let pdf = render_conforming(Conformance::PdfA2b).expect("the sample page conforms to PDF/A-2b");
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains(&format!("/Producer({RPT_RS_PRODUCER})")));
    assert!(text.contains(&format!("<pdf:Producer>{RPT_RS_PRODUCER}</pdf:Producer>")));
    // Every archival level records file provenance; the creator is the software agent it cites.
    assert!(
        text.contains(&format!(
            "<stEvt:softwareAgent>{RPT_RS_PRODUCER}</stEvt:softwareAgent>"
        )),
        "the provenance record must name the converting software"
    );
}

#[test]
fn an_archival_render_states_the_standard_it_claims() {
    // The claim lives in the (uncompressed) XMP packet, as `pdfaid:part`/`pdfaid:conformance`; a
    // conforming file that does not say which standard it conforms to is not one.
    for (level, part) in [
        (Conformance::PdfA1b, "1"),
        (Conformance::PdfA2b, "2"),
        (Conformance::PdfA3b, "3"),
    ] {
        let pdf = render_conforming(level).unwrap_or_else(|e| panic!("{level}: {e}"));
        let text = String::from_utf8_lossy(&pdf);
        assert!(
            text.contains(&format!("<pdfaid:part>{part}</pdfaid:part>")),
            "{level}: the XMP packet must declare part {part}"
        );
        assert!(
            text.contains("<pdfaid:conformance>B</pdfaid:conformance>"),
            "{level}: the XMP packet must declare level B"
        );
    }
}

#[test]
fn an_archival_render_is_self_contained() {
    // The three properties an archival reader needs to reproduce the page without this machine: a
    // color space to interpret the numbers against, a date, and every face travelling with the file.
    let pdf = render_conforming(Conformance::PdfA2b).expect("the sample page conforms to PDF/A-2b");
    let text = String::from_utf8_lossy(&pdf);
    assert!(
        text.contains("/OutputIntents"),
        "an archival render must carry an output intent"
    );
    assert!(
        text.contains("/GTS_PDFA1"),
        "the output intent must be the PDF/A one"
    );
    assert!(
        text.contains("/CreationDate"),
        "every PDF/A level requires a document date"
    );
    let fonts = rpt_test_support::pdf::structure(&pdf);
    let fonts: Vec<_> = fonts.pages.iter().flat_map(|p| p.fonts.values()).collect();
    assert!(!fonts.is_empty(), "the sample page draws text");
    assert!(
        fonts.iter().all(|f| f.has_font_program),
        "every face must be embedded: {:?}",
        fonts.iter().map(|f| &f.base_font).collect::<Vec<_>>()
    );
}

#[test]
fn an_archival_render_is_reproducible() {
    // An archival file that differs run to run cannot be checksummed by the archive that keeps it.
    // The fallback date is what makes this hold without an explicit one.
    assert_eq!(
        render_conforming(Conformance::PdfA2b).expect("conforms"),
        render_conforming(Conformance::PdfA2b).expect("conforms"),
        "two archival renders of the same pages must be byte-identical"
    );
}

#[test]
fn an_explicit_date_reaches_the_document() {
    let pdf = try_render_pages_with_options(
        &[sample()],
        &BTreeMap::new(),
        &PdfOptions {
            conformance: Conformance::PdfA2b,
            created: Some(Timestamp {
                year: 2019,
                month: 7,
                day: 4,
                hour: 13,
                minute: 5,
                second: 6,
            }),
            ..PdfOptions::default()
        },
    )
    .expect("the sample page conforms to PDF/A-2b");
    let text = String::from_utf8_lossy(&pdf);
    assert!(
        text.contains("D:20190704130506"),
        "the caller's date must be the document's creation date"
    );
    assert!(
        text.contains("2019-07-04T13:05:06"),
        "and must reach the XMP packet too"
    );
    // The date is the only difference; without conformance it still writes, so it is an independent
    // option rather than a rider on the standard.
    let dated = try_render_pages_with_options(
        &[sample()],
        &BTreeMap::new(),
        &PdfOptions {
            created: Some(Timestamp::default()),
            ..PdfOptions::default()
        },
    )
    .expect("the sample page serializes");
    assert!(String::from_utf8_lossy(&dated).contains("D:19700101000000"));
}

#[test]
fn transparency_is_refused_by_pdf_a_1b_and_accepted_by_2b() {
    // PDF/A-1b is PDF 1.4, which has no transparency model — the one place where a level a report can
    // legitimately reach is unreachable, and the reason the option is not simply "archival: on".
    let err = try_render_pages_with_options(
        &[translucent_page()],
        &BTreeMap::new(),
        &PdfOptions {
            conformance: Conformance::PdfA1b,
            ..PdfOptions::default()
        },
    )
    .expect_err("a translucent fill cannot conform to PDF/A-1b");
    let PdfError::Conformance { level, reasons } = &err else {
        panic!("expected a conformance failure, got {err:?}");
    };
    assert_eq!(*level, Conformance::PdfA1b);
    assert!(
        reasons.iter().any(|r| r.contains("transparency")),
        "the failure must name the unmet requirement: {reasons:?}"
    );
    try_render_pages_with_options(
        &[translucent_page()],
        &BTreeMap::new(),
        &PdfOptions {
            conformance: Conformance::PdfA2b,
            ..PdfOptions::default()
        },
    )
    .expect("PDF/A-2b permits transparency");
}

#[test]
fn a_conformance_failure_still_yields_a_document_that_says_so() {
    // The infallible seam must not swallow the failure into an ordinary-looking file: it renders the
    // failure page, which makes no conformance claim of its own.
    use rpt_pages::PageBackend;
    let doc = rpt_pages::PagedDocument {
        pages: vec![translucent_page()],
        ..rpt_pages::PagedDocument::default()
    };
    let pdf = crate::PdfBackend.render(
        &doc,
        &PdfOptions {
            conformance: Conformance::PdfA1b,
            ..PdfOptions::default()
        },
    );
    let text = String::from_utf8_lossy(&pdf);
    assert!(
        !text.contains("pdfaid"),
        "the failure page must not claim conformance"
    );
    assert!(
        operator_listing(&pdf).contains("Tj") || operator_listing(&pdf).contains("TJ"),
        "the failure page states the reason in text"
    );
}

// ---------------------------------------------------------------------------
// Tagged PDF: the structure tree, and the levels that require one
// ---------------------------------------------------------------------------

/// A two-line wrapped field in a detail band, a footer in its own section, and a bordered box —
/// enough for a tree to have a paragraph, an artifact, and a section to classify.
fn tagging_sample() -> Page {
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
        fill: None,
        stroke: Some(Stroke {
            color: BLACK,
            width: Twips(15),
            style: LineStyle::Single,
        }),
        corner_radius: Twips(0),
        source: Some(
            ObjectRef::new("DetailSection1", ObjectKind::Box)
                .named("F1")
                .with_instance(1),
        ),
    }));
    for (i, line) in ["Customer", "Name"].iter().enumerate() {
        p.push(DrawOp::Text(TextRun {
            bounds: rpt_model::Rect {
                left: Twips(720),
                top: Twips(720 + i as i32 * 200),
                width: Twips(4000),
                height: Twips(200),
            },
            text: (*line).to_string(),
            font: FontSpec::default(),
            color: BLACK,
            align: TextAlign::Left,
            rotation: 0.0,
            metrics: None,
            character_spacing: Twips(0),
            source: Some(
                ObjectRef::new("DetailSection1", ObjectKind::Field)
                    .named("F1")
                    .with_instance(1),
            ),
        }));
    }
    p.push(DrawOp::Text(TextRun {
        bounds: rpt_model::Rect {
            left: Twips(720),
            top: Twips(15000),
            width: Twips(4000),
            height: Twips(200),
        },
        text: "Page 1".to_string(),
        font: FontSpec::default(),
        color: BLACK,
        align: TextAlign::Left,
        rotation: 0.0,
        metrics: None,
        character_spacing: Twips(0),
        source: Some(
            ObjectRef::new("PageFooterSection1", ObjectKind::Field)
                .named("PageN")
                .with_instance(2),
        ),
    }));
    p
}

/// Everything a tagging level needs, for a document that has no figures.
fn full_semantics() -> crate::Semantics {
    crate::Semantics {
        title: Some("Customer list".to_string()),
        language: Some("en-US".to_string()),
        alt_text: BTreeMap::new(),
        artifact_sections: Some(BTreeMap::from([(
            "PageFooterSection1".to_string(),
            crate::ArtifactRole::Footer,
        )])),
    }
}

fn render_tagged(opts: PdfOptions) -> Result<Vec<u8>, PdfError> {
    try_render_pages_with_options(&[tagging_sample()], &BTreeMap::new(), &opts)
}

/// [`tagging_sample`] as a whole document that classifies its own bands — what a real render passes,
/// and the state in which no caller classification is needed.
fn tagging_document() -> rpt_pages::PagedDocument {
    rpt_pages::PagedDocument {
        pages: vec![tagging_sample()],
        sections: BTreeMap::from([
            (
                "DetailSection1".to_string(),
                rpt_pages::SectionInfo {
                    band: rpt_model::AreaSectionKind::Detail,
                    group_level: None,
                },
            ),
            (
                "PageFooterSection1".to_string(),
                rpt_pages::SectionInfo {
                    band: rpt_model::AreaSectionKind::PageFooter,
                    group_level: None,
                },
            ),
        ]),
        ..Default::default()
    }
}

#[test]
fn tagging_is_off_by_default_and_costs_the_ordinary_render_nothing() {
    // The default must leave the bytes exactly as they were: a structure tree is only ever added on
    // request, so no existing render moves.
    assert!(!PdfOptions::default().tagged);
    let plain = render_tagged(PdfOptions::default()).expect("renders");
    // The structural half of the semantics is inert behind the flag too: an untagged render has no
    // tree to put an artifact or a figure description into, so naming them changes nothing.
    let with_structure = render_tagged(PdfOptions {
        semantics: crate::Semantics {
            title: None,
            language: None,
            ..full_semantics()
        },
        ..PdfOptions::default()
    })
    .expect("renders");
    assert_eq!(
        plain, with_structure,
        "artifact sections and alt text must not alter an untagged render"
    );
    let listing = operator_listing(&plain);
    assert!(!listing.contains("BDC"), "untagged output marks no content");
    assert!(
        !String::from_utf8_lossy(&plain).contains("/StructTreeRoot"),
        "untagged output carries no structure tree"
    );
}

#[test]
fn a_tagged_render_marks_content_as_content_and_decoration_as_an_artifact() {
    let pdf = render_tagged(PdfOptions {
        tagged: true,
        semantics: full_semantics(),
        ..PdfOptions::default()
    })
    .expect("renders");
    let listing = operator_listing(&pdf);
    // The bordered box is decoration, so it is an artifact rather than a leaf of the tree.
    assert!(
        listing.contains("/Artifact << /Type /Layout >> BDC"),
        "a border must be a layout artifact:\n{listing}"
    );
    // The classified footer is pagination furniture, not the document's text.
    assert!(
        listing.contains("/Subtype /Footer"),
        "a classified page footer must be a pagination artifact:\n{listing}"
    );
    // Each wrapped line is its own span, carrying the document's language.
    assert_eq!(
        listing.matches("/Lang (en-US)").count(),
        2,
        "one span per wrapped line:\n{listing}"
    );
    assert!(String::from_utf8_lossy(&pdf).contains("/StructTreeRoot"));
}

#[test]
fn a_wrapped_line_declares_the_space_the_break_consumed() {
    // Without this the two lines of one field extract as "CustomerName".
    let pdf = render_tagged(PdfOptions {
        tagged: true,
        semantics: full_semantics(),
        ..PdfOptions::default()
    })
    .expect("renders");
    let listing = operator_listing(&pdf);
    assert!(
        listing.contains("/ActualText (Customer )"),
        "the interior line must declare the space the break consumed:\n{listing}"
    );
    assert!(
        !listing.contains("/ActualText (Name"),
        "the last line of a paragraph needs none:\n{listing}"
    );
}

#[test]
fn a_tagging_level_is_refused_until_the_caller_supplies_what_the_page_ir_cannot() {
    // Nothing supplied: three separate things are missing, and the failure names all three rather
    // than one at a time.
    let err = render_tagged(PdfOptions {
        conformance: Conformance::PdfUa1,
        ..PdfOptions::default()
    })
    .expect_err("PDF/UA-1 cannot be claimed over an undescribed document");
    let PdfError::Conformance { level, reasons } = &err else {
        panic!("expected a conformance failure, got {err:?}");
    };
    assert_eq!(*level, Conformance::PdfUa1);
    let joined = reasons.join("\n");
    assert!(joined.contains("natural language"), "{joined}");
    assert!(joined.contains("title"), "{joined}");
    assert!(joined.contains("carries no sections"), "{joined}");

    // The level-A archival standards need the language and the classification, but no title.
    let err = render_tagged(PdfOptions {
        conformance: Conformance::PdfA2a,
        ..PdfOptions::default()
    })
    .expect_err("PDF/A-2a needs a language");
    let PdfError::Conformance { reasons, .. } = &err else {
        panic!("expected a conformance failure, got {err:?}");
    };
    assert!(!reasons.join("\n").contains("title"), "{reasons:?}");
}

/// A document that classifies its own bands earns the classification half of a tagging level without
/// the caller repeating it — and the furniture really is marked as furniture, not merely accepted.
#[test]
fn a_documents_own_bands_satisfy_the_classification_a_tagging_level_needs() {
    let semantics = crate::Semantics {
        artifact_sections: None,
        ..full_semantics()
    };
    let pdf = crate::try_render_document(
        &tagging_document(),
        &PdfOptions {
            conformance: Conformance::PdfUa1,
            semantics,
            ..PdfOptions::default()
        },
    )
    .expect("a classified document needs no caller override");
    let listing = operator_listing(&pdf);
    assert!(
        listing.contains("/Subtype /Footer"),
        "the document's own page footer must become a pagination artifact:\n{listing}"
    );

    // The same pages without the dictionary are still refused: the refusal is about the missing
    // classification, not about which entry point was used.
    let err = crate::try_render_document(
        &rpt_pages::PagedDocument {
            sections: BTreeMap::new(),
            ..tagging_document()
        },
        &PdfOptions {
            conformance: Conformance::PdfUa1,
            semantics: crate::Semantics {
                artifact_sections: None,
                ..full_semantics()
            },
            ..PdfOptions::default()
        },
    )
    .expect_err("an unclassified document cannot claim PDF/UA-1");
    let PdfError::Conformance { reasons, .. } = &err else {
        panic!("expected a conformance failure, got {err:?}");
    };
    assert!(
        reasons.join("\n").contains("carries no sections"),
        "{reasons:?}"
    );
}

#[test]
fn a_figure_without_alternate_text_fails_the_level_and_names_the_object() {
    let mut page = tagging_sample();
    page.push(DrawOp::Image(rpt_pages::ImageOp {
        bounds: rpt_model::Rect {
            left: Twips(720),
            top: Twips(2000),
            width: Twips(1000),
            height: Twips(1000),
        },
        image_id: "logo".to_string(),
        fit: Default::default(),
        source: Some(
            ObjectRef::new("DetailSection1", ObjectKind::Image)
                .named("Picture1")
                .with_instance(3),
        ),
    }));
    let opts = PdfOptions {
        conformance: Conformance::PdfUa1,
        semantics: full_semantics(),
        ..PdfOptions::default()
    };
    let err = try_render_pages_with_options(&[page.clone()], &BTreeMap::new(), &opts)
        .expect_err("an undescribed figure cannot be claimed accessible");
    let PdfError::Conformance { reasons, .. } = &err else {
        panic!("expected a conformance failure, got {err:?}");
    };
    assert!(
        reasons.iter().any(|r| r.contains("\"Picture1\"")),
        "the failure must name the object: {reasons:?}"
    );

    // Described, it conforms; the same page is now a figure the reader can be told about.
    let described = PdfOptions {
        semantics: crate::Semantics {
            alt_text: BTreeMap::from([("Picture1".to_string(), "Company logo".to_string())]),
            ..full_semantics()
        },
        ..opts
    };
    let pdf = try_render_pages_with_options(&[page], &BTreeMap::new(), &described)
        .expect("a described figure conforms");
    assert!(String::from_utf8_lossy(&pdf).contains("Company logo"));
}

#[test]
fn a_conforming_tagged_render_states_the_standard_it_claims() {
    // PDF/UA-1's claim lives in the XMP packet as `pdfuaid:part`, the same way the archival levels
    // write `pdfaid`.
    let pdf = render_tagged(PdfOptions {
        conformance: Conformance::PdfUa1,
        semantics: full_semantics(),
        ..PdfOptions::default()
    })
    .expect("the sample page conforms to PDF/UA-1");
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("pdfuaid:part>1"), "missing the PDF/UA claim");
    assert!(text.contains("/StructTreeRoot"));

    for (level, part) in [
        (Conformance::PdfA1a, "1"),
        (Conformance::PdfA2a, "2"),
        (Conformance::PdfA3a, "3"),
    ] {
        let pdf = render_tagged(PdfOptions {
            conformance: level,
            semantics: full_semantics(),
            ..PdfOptions::default()
        })
        .unwrap_or_else(|e| panic!("{level} should conform: {e}"));
        let text = String::from_utf8_lossy(&pdf);
        assert!(
            text.contains(&format!("<pdfaid:part>{part}</pdfaid:part>")),
            "{level} must state its part"
        );
        assert!(
            text.contains("<pdfaid:conformance>A</pdfaid:conformance>"),
            "{level} must state level A"
        );
    }
}

#[test]
fn tagging_without_a_conformance_level_claims_nothing_and_needs_nothing() {
    // A structure tree is useful on its own — to a screen reader and to text extraction — and
    // asserting no standard means none of the level's preconditions apply.
    let pdf = render_tagged(PdfOptions {
        tagged: true,
        ..PdfOptions::default()
    })
    .expect("a plain tagged render needs no semantics");
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/StructTreeRoot"));
    assert!(!text.contains("pdfuaid"), "no accessibility claim");
    assert!(!text.contains("pdfaid"), "no archival claim");
}
