//! `semantics_of` — what a tagged or accessible render can take from the report, and what it cannot.
//!
//! The line this draws is the whole point of the checked-claim design: a fact the file states is
//! filled in, and a fact it does not is left absent so the level is refused naming it. A test that
//! only asserted the filled-in half would let a future "helpful" default through.

use rpt_reader::model::{
    Area, ObjectFormat, PictureObject, ReportObject, ReportObjectKind, Section, Subreport,
    SummaryInfo,
};
use rpt_render::{semantics_of, Conformance, PdfOptions, Semantics};

/// A report object of `kind`, named, with `tooltip` as its stored `ToolTipText`.
fn figure(name: &str, kind: ReportObjectKind, tooltip: Option<&str>) -> ReportObject {
    ReportObject {
        name: name.to_string(),
        kind,
        format: ObjectFormat {
            tooltip_text: tooltip.map(str::to_string),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn report_with(title: &str, objects: Vec<ReportObject>) -> rpt_reader::model::Report {
    let mut report = rpt_reader::model::Report {
        summary_info: SummaryInfo {
            title: title.to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    report.report_definition.areas.push(Area {
        sections: vec![Section {
            objects,
            ..Default::default()
        }],
        ..Default::default()
    });
    report
}

/// The two facts the file states: the summary title and each figure's stored tooltip.
#[test]
fn the_report_supplies_its_title_and_its_figures_descriptions() {
    let report = report_with(
        "Quarterly Review",
        vec![
            figure(
                "Chart1",
                ReportObjectKind::Chart(Box::default()),
                Some("Revenue by quarter"),
            ),
            figure(
                "Picture1",
                ReportObjectKind::Picture(PictureObject::default()),
                Some("The company logo"),
            ),
        ],
    );
    let semantics = semantics_of(&report);
    assert_eq!(semantics.title.as_deref(), Some("Quarterly Review"));
    assert_eq!(
        semantics.alt_text.get("Chart1").map(String::as_str),
        Some("Revenue by quarter")
    );
    assert_eq!(
        semantics.alt_text.get("Picture1").map(String::as_str),
        Some("The company logo")
    );
}

/// What the file does not state is left absent, so the level is refused rather than claimed on a
/// guess: an empty summary title is not a title, a tooltip-less figure has no description, and
/// nothing in a `.rpt` records the language of its text.
#[test]
fn what_the_report_does_not_state_is_left_absent() {
    let report = report_with(
        "   ",
        vec![figure(
            "Chart1",
            ReportObjectKind::Chart(Box::default()),
            None,
        )],
    );
    let semantics = semantics_of(&report);
    assert_eq!(
        semantics.title, None,
        "a blank summary title is a field the author left alone, not a title"
    );
    assert!(
        semantics.alt_text.is_empty(),
        "an undescribed figure stays undescribed: {:?}",
        semantics.alt_text
    );
    assert_eq!(
        semantics.language, None,
        "no report states its language; deriving one would claim an accessibility it cannot support"
    );
    assert_eq!(
        semantics.artifact_sections, None,
        "band classification comes from the document, so the override stays unset"
    );
}

/// Only a picture or a chart is a figure. A text object's tooltip is not alternate text — it would
/// silently describe something that is already read as text.
#[test]
fn only_figures_contribute_alternate_text() {
    let report = report_with(
        "T",
        vec![figure(
            "Field1",
            ReportObjectKind::Text(Default::default()),
            Some("a hint for the mouse"),
        )],
    );
    assert!(semantics_of(&report).alt_text.is_empty());
}

/// A subreport's objects merge into the parent document under their own names, so their descriptions
/// have to come along; a name the two reports disagree about keeps the parent's, matching how the
/// producer merges a subreport's section dictionary.
#[test]
fn subreport_figures_are_described_too() {
    let mut report = report_with(
        "Parent",
        vec![figure(
            "Chart1",
            ReportObjectKind::Chart(Box::default()),
            Some("the parent's chart"),
        )],
    );
    let child = report_with(
        "Child",
        vec![
            figure(
                "Chart1",
                ReportObjectKind::Chart(Box::default()),
                Some("the child's chart"),
            ),
            figure(
                "Picture9",
                ReportObjectKind::Picture(PictureObject::default()),
                Some("only in the subreport"),
            ),
        ],
    );
    report.subreports.push(Subreport {
        name: "Sub1".to_string(),
        report: Box::new(child),
        links: Vec::new(),
    });

    let alt = semantics_of(&report).alt_text;
    assert_eq!(
        alt.get("Chart1").map(String::as_str),
        Some("the parent's chart")
    );
    assert_eq!(
        alt.get("Picture9").map(String::as_str),
        Some("only in the subreport")
    );
}

/// End to end on a committed report: what the facade derives is enough for PDF/UA-1 once the caller
/// adds the one thing no report states, and the render is refused — naming it — when they do not.
#[test]
fn a_derived_title_plus_a_caller_language_earns_pdf_ua_1() {
    let path = rpt_test_support::fixture("tests/fixtures/reports/worrall/AlphaISOsByCountry.rpt");
    let doc = rpt_render::ReportDocument::load(&path).expect("load");
    let pages = doc.render();

    let derived = semantics_of(doc.report());
    assert!(
        derived.title.is_some(),
        "this fixture's author filled in the summary title"
    );

    let refused = rpt_render::try_render_document(
        &pages,
        &PdfOptions {
            conformance: Conformance::PdfUa1,
            semantics: derived.clone(),
            ..Default::default()
        },
    )
    .expect_err("a level that needs a language must not be granted without one");
    assert!(
        refused.to_string().contains("natural language"),
        "{refused}"
    );

    let pdf = rpt_render::try_render_document(
        &pages,
        &PdfOptions {
            conformance: Conformance::PdfUa1,
            semantics: Semantics {
                language: Some("en-US".to_string()),
                ..derived
            },
            ..Default::default()
        },
    )
    .expect("with a language the claim is earned");
    assert!(pdf.starts_with(b"%PDF-"));
    assert!(
        String::from_utf8_lossy(&pdf).contains("/StructTreeRoot"),
        "a conforming document carries the structure tree the level requires"
    );
}
