//! Structure-planning tests: what a run of ops means, where the band and object boundaries fall, and
//! what order the tree reads in. All of it decided without krilla, which is why it can be asserted
//! directly rather than through a serialized PDF.

use super::*;
use crate::{ArtifactRole, Semantics};
use rpt_model::{AreaSectionKind, Color, Rect, Twips};
use rpt_pages::{
    FontSpec, ImageOp, LineOp, LineStyle, ObjectKind, ObjectRef, PageSize, Point, RectOp,
    SectionInfo, Stroke, TextAlign, TextRun,
};
use std::collections::BTreeMap;

fn page(ops: Vec<DrawOp>) -> Page {
    let mut page = Page::new(
        1,
        PageSize {
            width: Twips(12240),
            height: Twips(15840),
        },
    );
    page.extend(ops);
    page
}

fn src(section: &str, name: &str, kind: ObjectKind, instance: u32) -> Option<ObjectRef> {
    Some(
        ObjectRef::new(section, kind)
            .named(name)
            .with_instance(instance),
    )
}

fn text(section: &str, name: &str, instance: u32, top: i32, left: i32, s: &str) -> DrawOp {
    DrawOp::Text(TextRun {
        bounds: Rect {
            left: Twips(left),
            top: Twips(top),
            width: Twips(1000),
            height: Twips(200),
        },
        text: s.to_string(),
        font: FontSpec::default(),
        color: Color::default(),
        align: TextAlign::Left,
        rotation: 0.0,
        metrics: None,
        character_spacing: Twips(0),
        source: src(section, name, ObjectKind::Field, instance),
    })
}

fn border(section: &str, name: &str, instance: u32, top: i32) -> DrawOp {
    DrawOp::Rect(RectOp {
        bounds: Rect {
            left: Twips(0),
            top: Twips(top),
            width: Twips(1000),
            height: Twips(200),
        },
        fill: None,
        stroke: Some(Stroke {
            color: Color::default(),
            width: Twips(15),
            style: LineStyle::Single,
        }),
        corner_radius: Twips(0),
        source: src(section, name, ObjectKind::Box, instance),
    })
}

fn background(section: &str, top: i32) -> DrawOp {
    DrawOp::Rect(RectOp {
        bounds: Rect {
            left: Twips(0),
            top: Twips(top),
            width: Twips(12240),
            height: Twips(400),
        },
        fill: Some(Color::WHITE.into()),
        stroke: None,
        corner_radius: Twips(0),
        source: Some(ObjectRef::new(section, ObjectKind::Section)),
    })
}

fn picture(section: &str, name: &str) -> DrawOp {
    DrawOp::Image(ImageOp {
        bounds: Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(500),
            height: Twips(500),
        },
        image_id: "img".to_string(),
        fit: Default::default(),
        source: src(section, name, ObjectKind::Image, 1),
    })
}

/// The units of a plan, flattened, so a test can read the classification straight through. The
/// document carries no band dictionary, so the classification is the caller's alone.
fn kinds(page: &Page, semantics: &Semantics) -> Vec<UnitKind> {
    kinds_of(page, &BTreeMap::new(), semantics)
}

/// [`kinds`] over a document that classifies its own bands.
fn kinds_of(
    page: &Page,
    sections: &BTreeMap<String, SectionInfo>,
    semantics: &Semantics,
) -> Vec<UnitKind> {
    plan(page, &artifact_roles(sections, semantics), semantics)
        .into_iter()
        .flat_map(|b| b.units)
        .map(|u| u.kind)
        .collect()
}

#[test]
fn every_op_is_covered_exactly_once() {
    // The invariant the writer depends on: the plan partitions the page's ops, so drawing straight
    // through the units draws every op once and none twice. A gap would silently lose content.
    let p = page(vec![
        background("Detail", 0),
        border("Detail", "F1", 1, 0),
        text("Detail", "F1", 1, 0, 0, "a"),
        text("Detail", "F1", 1, 200, 0, "b"),
        picture("Detail", "P1"),
    ]);
    let mut covered: Vec<usize> = plan(&p, &BTreeMap::new(), &Semantics::default())
        .into_iter()
        .flat_map(|b| b.units)
        .flat_map(|u| u.ops)
        .collect();
    covered.sort_unstable();
    assert_eq!(covered, (0..p.ops.len()).collect::<Vec<_>>());
}

#[test]
fn decoration_is_an_artifact_and_text_is_content() {
    let p = page(vec![
        background("Detail", 0),
        border("Detail", "F1", 1, 0),
        text("Detail", "F1", 1, 0, 0, "value"),
        DrawOp::Line(LineOp {
            from: Point::new(0, 300),
            to: Point::new(9000, 300),
            stroke: Stroke {
                color: Color::default(),
                width: Twips(15),
                style: LineStyle::Single,
            },
            source: src("Detail", "Line1", ObjectKind::Line, 2),
        }),
    ]);
    assert_eq!(
        kinds(&p, &Semantics::default()),
        vec![
            UnitKind::Artifact(ArtifactKind::Layout),
            UnitKind::Artifact(ArtifactKind::Layout),
            UnitKind::Paragraph,
            UnitKind::Artifact(ArtifactKind::Layout),
        ]
    );
}

#[test]
fn a_wrapped_field_is_one_paragraph_of_lines() {
    // Lines of one placed object share an identity, so they become one unit — one `P` of `Span`s,
    // rather than three unrelated paragraphs.
    let p = page(vec![
        text("Detail", "F1", 1, 0, 0, "one"),
        text("Detail", "F1", 1, 200, 0, "two"),
        text("Detail", "F1", 1, 400, 0, "three"),
        text("Detail", "F2", 2, 0, 2000, "other"),
    ]);
    let units: Vec<_> = plan(&p, &BTreeMap::new(), &Semantics::default())
        .into_iter()
        .flat_map(|b| b.units)
        .collect();
    assert_eq!(units.len(), 2);
    assert_eq!(units[0].ops, 0..3);
    assert_eq!(units[1].ops, 3..4);
}

#[test]
fn a_picture_or_chart_is_one_figure_however_many_marks_it_draws() {
    // A chart draws its interior as paths and its labels as text, all under one identity. Splitting
    // that into "the labels read, the paths do not" would announce the axis numbers as prose.
    let chart = |op: DrawOp| op;
    let p = page(vec![
        chart(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(0),
                top: Twips(0),
                width: Twips(5000),
                height: Twips(4000),
            },
            fill: Some(Color::WHITE.into()),
            stroke: None,
            corner_radius: Twips(0),
            source: src("Detail", "Graph1", ObjectKind::Chart, 1),
        })),
        chart(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(100),
                top: Twips(100),
                width: Twips(500),
                height: Twips(200),
            },
            text: "40".to_string(),
            font: FontSpec::default(),
            color: Color::default(),
            align: TextAlign::Left,
            rotation: 0.0,
            metrics: None,
            character_spacing: Twips(0),
            source: src("Detail", "Graph1", ObjectKind::Chart, 1),
        })),
    ]);
    let units: Vec<_> = plan(&p, &BTreeMap::new(), &Semantics::default())
        .into_iter()
        .flat_map(|b| b.units)
        .collect();
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].ops, 0..2);
    assert!(matches!(
        &units[0].kind,
        UnitKind::Figure { object, alt } if object == "Graph1" && alt.is_none()
    ));
}

#[test]
fn alternate_text_reaches_the_figure_and_an_empty_one_makes_it_decorative() {
    let p = page(vec![picture("Detail", "Logo")]);
    let described = Semantics {
        alt_text: BTreeMap::from([("Logo".to_string(), "Company logo".to_string())]),
        ..Semantics::default()
    };
    assert!(matches!(
        &kinds(&p, &described)[0],
        UnitKind::Figure { alt: Some(alt), .. } if alt == "Company logo"
    ));
    // The HTML `alt=""` convention: the caller looked, and it carries no information.
    let decorative = Semantics {
        alt_text: BTreeMap::from([("Logo".to_string(), String::new())]),
        ..Semantics::default()
    };
    assert_eq!(
        kinds(&p, &decorative),
        vec![UnitKind::Artifact(ArtifactKind::Layout)]
    );
}

#[test]
fn a_classified_page_footer_becomes_a_pagination_artifact() {
    let p = page(vec![
        text("Detail", "F1", 1, 0, 0, "row"),
        text("PageFooterSection1", "PageN", 2, 15000, 0, "Page 1 of 42"),
    ]);
    // Unclassified, the footer reads as content on every page — degraded, but never a false claim.
    assert_eq!(
        kinds(&p, &Semantics::default()),
        vec![UnitKind::Paragraph, UnitKind::Paragraph]
    );
    let classified = Semantics {
        artifact_sections: Some(BTreeMap::from([(
            "PageFooterSection1".to_string(),
            ArtifactRole::Footer,
        )])),
        ..Semantics::default()
    };
    assert_eq!(
        kinds(&p, &classified),
        vec![
            UnitKind::Paragraph,
            UnitKind::Artifact(ArtifactKind::Pagination(ArtifactRole::Footer)),
        ]
    );
}

/// A section dictionary, for a document that classified its own bands.
fn sections(entries: &[(&str, AreaSectionKind)]) -> BTreeMap<String, SectionInfo> {
    entries
        .iter()
        .map(|&(name, band)| {
            (
                name.to_string(),
                SectionInfo {
                    band,
                    group_level: None,
                },
            )
        })
        .collect()
}

/// The document's own band dictionary classifies the furniture, with no caller involvement: the page
/// header and footer repeat on every page and become the artifacts PDF names, while the report header
/// prints once and reads like any other content.
#[test]
fn the_documents_bands_decide_the_artifact_roles() {
    let p = page(vec![
        text("TSection7", "Title", 1, 0, 0, "Sales"),
        text("Section2", "Logo", 2, 400, 0, "ACME"),
        text("Section3", "F1", 3, 800, 0, "row"),
        text("Section5", "PageN", 4, 15000, 0, "Page 1 of 42"),
    ]);
    let dict = sections(&[
        ("TSection7", AreaSectionKind::ReportHeader),
        ("Section2", AreaSectionKind::PageHeader),
        ("Section3", AreaSectionKind::Detail),
        ("Section5", AreaSectionKind::PageFooter),
    ]);
    assert_eq!(
        kinds_of(&p, &dict, &Semantics::default()),
        vec![
            UnitKind::Paragraph,
            UnitKind::Artifact(ArtifactKind::Pagination(ArtifactRole::Header)),
            UnitKind::Paragraph,
            UnitKind::Artifact(ArtifactKind::Pagination(ArtifactRole::Footer)),
        ]
    );
}

/// A band the dictionary does not mention, or names as one this backend has no rule for, reads as
/// content — missing information must never delete content from the reading order.
#[test]
fn an_unlisted_or_unrecognised_band_stays_content() {
    let p = page(vec![
        text("Section3", "F1", 1, 0, 0, "row"),
        text("Section9", "X", 2, 400, 0, "?"),
    ]);
    let dict = sections(&[("Section9", AreaSectionKind::Other(42))]);
    assert_eq!(
        kinds_of(&p, &dict, &Semantics::default()),
        vec![UnitKind::Paragraph, UnitKind::Paragraph]
    );
}

/// The caller's classification is an override, not a supplement: supplying one replaces the
/// document's wholesale, so a caller can both add furniture the document did not mark and take back
/// furniture it did.
#[test]
fn a_callers_classification_replaces_the_documents() {
    let p = page(vec![
        text("Section2", "Logo", 1, 0, 0, "ACME"),
        text("Section5", "PageN", 2, 15000, 0, "Page 1 of 42"),
    ]);
    let dict = sections(&[
        ("Section2", AreaSectionKind::PageHeader),
        ("Section5", AreaSectionKind::PageFooter),
    ]);
    let disagrees = Semantics {
        artifact_sections: Some(BTreeMap::from([(
            "Section5".to_string(),
            ArtifactRole::Pagination,
        )])),
        ..Semantics::default()
    };
    assert_eq!(
        kinds_of(&p, &dict, &disagrees),
        vec![
            UnitKind::Paragraph,
            UnitKind::Artifact(ArtifactKind::Pagination(ArtifactRole::Pagination)),
        ]
    );
    // An empty override says "classified, and nothing is furniture" — the document's own marks go.
    let none_is_furniture = Semantics {
        artifact_sections: Some(BTreeMap::new()),
        ..Semantics::default()
    };
    assert_eq!(
        kinds_of(&p, &dict, &none_is_furniture),
        vec![UnitKind::Paragraph, UnitKind::Paragraph]
    );
}

#[test]
fn a_repeating_band_splits_into_one_group_per_occurrence() {
    // Two detail rows are one section run but two bands: the section background opens each.
    let p = page(vec![
        background("Detail", 0),
        text("Detail", "F1", 1, 0, 0, "row 1"),
        background("Detail", 400),
        text("Detail", "F1", 2, 400, 0, "row 2"),
    ]);
    assert_eq!(plan(&p, &BTreeMap::new(), &Semantics::default()).len(), 2);
    // …and it still splits when the band draws no background, on the object name coming round again.
    let p = page(vec![
        text("Detail", "F1", 1, 0, 0, "row 1"),
        text("Detail", "F1", 2, 400, 0, "row 2"),
    ]);
    assert_eq!(plan(&p, &BTreeMap::new(), &Semantics::default()).len(), 2);
}

#[test]
fn reading_order_is_by_row_then_left_to_right() {
    // Paint order here is definition order — the rightmost field first, and the second row's field
    // before the first row's last. Reading order must recover rows top-down, left-to-right.
    let p = page(vec![
        text("H", "C", 1, 10, 4000, "third"),
        text("H", "A", 2, 0, 0, "first"),
        text("H", "B", 3, 5, 2000, "second"),
    ]);
    let bands = plan(&p, &BTreeMap::new(), &Semantics::default());
    assert_eq!(bands.len(), 1);
    let order = bands[0].reading_order();
    let texts: Vec<&str> = order
        .iter()
        .map(|&i| match &p.ops[bands[0].units[i].ops.start] {
            DrawOp::Text(t) => t.text.as_str(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(texts, vec!["first", "second", "third"]);
}

#[test]
fn a_taller_field_still_shares_a_row_with_its_neighbours() {
    // A 14 pt field in a 10 pt row starts higher and ends lower; sorting on `top` alone would read it
    // as its own row, ahead of everything to its left.
    let mut tall = text("H", "Big", 1, 300, 4000, "tall");
    if let DrawOp::Text(t) = &mut tall {
        t.bounds.top = Twips(280);
        t.bounds.height = Twips(260);
    }
    let p = page(vec![tall, text("H", "Small", 2, 300, 0, "small")]);
    let bands = plan(&p, &BTreeMap::new(), &Semantics::default());
    let order = bands[0].reading_order();
    let texts: Vec<&str> = order
        .iter()
        .map(|&i| match &p.ops[bands[0].units[i].ops.start] {
            DrawOp::Text(t) => t.text.as_str(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(texts, vec!["small", "tall"]);
}

#[test]
fn artifacts_are_left_out_of_the_reading_order() {
    let p = page(vec![
        background("Detail", 0),
        border("Detail", "F1", 1, 0),
        text("Detail", "F1", 1, 0, 0, "value"),
    ]);
    let bands = plan(&p, &BTreeMap::new(), &Semantics::default());
    assert_eq!(bands[0].reading_order().len(), 1);
}

#[test]
fn a_wrapped_line_declares_the_space_the_wrapper_consumed() {
    let run = |text: &str, align| TextRun {
        bounds: Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(1000),
            height: Twips(200),
        },
        text: text.to_string(),
        font: FontSpec::default(),
        color: Color::default(),
        align,
        rotation: 0.0,
        metrics: None,
        character_spacing: Twips(0),
        source: None,
    };
    // An interior line: the break ate a space, so the extracted text would otherwise concatenate.
    assert_eq!(
        actual_text(&run("field", TextAlign::Left), false).as_deref(),
        Some("field ")
    );
    // The last line of a left-aligned paragraph needs nothing — the glyphs already say it.
    assert_eq!(actual_text(&run("name", TextAlign::Left), true), None);
    // A justified line draws a real space glyph in every gap, so its final line needs nothing either
    // — declaring the words would only replace an extraction that is already right.
    assert_eq!(actual_text(&run("a b c", TextAlign::Justified), true), None);
}
