//! Special-field and embedded-reference resolution: date/time specials against a pinned as-of
//! instant, stored report facts, and the run-level substitution a text object needs.

use super::*;
use rpt_data::{build_dataset_with_params, normalize_param_name, DateTimeSpecials, Parameters};
use rpt_formula::eval::Value;
use rpt_model::{Group, Paragraph, SortDirection, TextRun};

/// A fixed as-of instant: 2023-11-14 22:13:20 UTC. Pinned so the expected strings below are stable.
const AS_OF_UNIX: i64 = 1_700_000_000;
const AS_OF_DATE: &str = "11/14/2023";

/// A fixed Windows `FILETIME` — 100-ns intervals since 1601-01-01 — for 2021-06-15 12:30:45 UTC.
/// Pinned so the expected strings below hold in any host timezone.
const FILE_TIME: u64 = 132_682_338_450_000_000;
const FILE_TIME_DATE: &str = "6/15/2021";
const FILE_TIME_TIME: &str = "12:30:45PM";

/// A field object bound to a special field, rendered through its stored (default) format.
fn special_object(name: &str, special: &str, top: i32) -> ReportObject {
    let mut o = ReportObject::default();
    o.name = name.to_string();
    o.bounds = Rect {
        left: Twips(100),
        top: Twips(top),
        width: Twips(3000),
        height: Twips(240),
    };
    let mut f = FieldObject::default();
    f.data_source = special.to_string();
    f.ref_kind = FieldRefKind::Special;
    o.kind = ReportObjectKind::Field(Box::new(f));
    o
}

/// A text object built from an explicit run list: `(text, field_ref)` per run, where `field_ref`
/// makes the run an embedded reference whose `text` is the engine's placeholder form.
fn run_text_object(name: &str, runs: &[(&str, Option<&str>)], top: i32) -> ReportObject {
    let mut o = ReportObject::default();
    o.name = name.to_string();
    o.bounds = Rect {
        left: Twips(100),
        top: Twips(top),
        width: Twips(8000),
        height: Twips(240),
    };
    let mut t = TextObject::default();
    let mut para = Paragraph::default();
    for (text, field_ref) in runs {
        let mut run = TextRun::default();
        run.text = (*text).to_string();
        run.field_ref = field_ref.map(str::to_string);
        para.runs.push(run);
        if let Some(r) = field_ref {
            t.embedded_fields.push((*r).to_string());
        }
    }
    t.display = para.runs.iter().map(|r| r.text.as_str()).collect();
    t.paragraphs = vec![para];
    o.kind = ReportObjectKind::Text(t);
    o
}

/// Lay `objects` out in a report header over one saved row, with the as-of instant pinned when
/// `as_of` is set. Returns the drawn text strings, in op order.
fn header_texts(
    report: &mut Report,
    objects: Vec<ReportObject>,
    as_of: Option<i64>,
) -> Vec<String> {
    header_texts_with_params(report, objects, as_of, &Parameters::new())
}

/// [`header_texts`] with report parameter values supplied, so a `{?Param}` reference resolves.
fn header_texts_with_params(
    report: &mut Report,
    objects: Vec<ReportObject>,
    as_of: Option<i64>,
    params: &Parameters,
) -> Vec<String> {
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![area(
        AreaSectionKind::ReportHeader,
        vec![section(AreaSectionKind::ReportHeader, "RH", 2000, objects)],
    )];
    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["1"]]);
    let mut ds = build_dataset_with_params(
        &SavedDataSource::new(&saved),
        &report.data_definition,
        params,
    );
    // The record pipeline uses the parameters but does not carry them on the Dataset; the render
    // facade attaches them for the layout pass, and so must this.
    ds.params = params.clone();
    let mut formulas = rpt_data::compile_formulas(&report.data_definition);
    if let Some(secs) = as_of {
        formulas = formulas.with_datetime(DateTimeSpecials::from_unix_seconds(secs));
    }
    let doc = layout(report, &ds, &formulas);
    doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

/// One report parameter value, keyed the way the pipeline keys them.
fn params_of(name: &str, value: Value) -> Parameters {
    Parameters::from([(normalize_param_name(name), value)])
}

/// A standalone `PrintDate` field resolves from the render's as-of instant — the same clock the
/// formula engine's `CurrentDate` reads.
#[test]
fn print_date_field_resolves_from_the_as_of_instant() {
    let mut report = Report::default();
    let texts = header_texts(
        &mut report,
        vec![special_object("PD", "PrintDate", 0)],
        Some(AS_OF_UNIX),
    );
    assert_eq!(texts, vec![AS_OF_DATE.to_string()]);
}

/// With no as-of supplied (an inspection path that never set one) a date special stays blank rather
/// than reading the host clock, so a render is never non-deterministic by default.
#[test]
fn date_specials_render_blank_without_an_as_of() {
    let mut report = Report::default();
    let texts = header_texts(
        &mut report,
        vec![special_object("PD", "PrintDate", 0)],
        None,
    );
    assert_eq!(texts, vec![String::new()]);
}

/// A date special embedded in a text object renders its value, not its own placeholder name.
#[test]
fn embedded_special_run_renders_its_value_not_its_name() {
    let mut report = Report::default();
    let obj = run_text_object(
        "Caption",
        &[
            ("Printed Date: ", None),
            ("PrintDate", Some("Print Date")),
            (" / ", None),
            ("ReportTitle", Some("Report Title")),
        ],
        0,
    );
    report.summary_info.title = "Quarterly Sales".to_string();
    let texts = header_texts(&mut report, vec![obj], Some(AS_OF_UNIX));
    assert_eq!(
        texts,
        vec![format!("Printed Date: {AS_OF_DATE} / Quarterly Sales")]
    );
}

/// A special the model carries no stored fact for renders blank — never its placeholder name. The
/// report file's path is a property of where the file sits, not of its bytes, so nothing in the
/// model can supply it.
#[test]
fn unresolvable_special_run_renders_blank() {
    let mut report = Report::default();
    let obj = run_text_object(
        "Caption",
        &[("Saved as: ", None), ("FilePath", Some("File Path"))],
        0,
    );
    let texts = header_texts(&mut report, vec![obj], Some(AS_OF_UNIX));
    assert_eq!(texts, vec!["Saved as: ".to_string()]);
}

/// An embedded `{?Param}` renders the parameter's value: the embedded path resolves parameters
/// itself, rather than deferring to a layer that does not exist.
#[test]
fn embedded_parameter_run_renders_its_value() {
    let mut report = Report::default();
    let obj = run_text_object(
        "Caption",
        &[("Region: ", None), ("{?Region}", Some("?Region"))],
        0,
    );
    let texts = header_texts_with_params(
        &mut report,
        vec![obj],
        Some(AS_OF_UNIX),
        &params_of("Region", Value::Str("Nordwest".into())),
    );
    assert_eq!(texts, vec!["Region: Nordwest".to_string()]);
}

/// An embedded parameter and a placed parameter field object resolve through the same call, so they
/// cannot report different values for one parameter.
#[test]
fn embedded_and_placed_parameter_agree() {
    let mut report = Report::default();
    let mut placed = ReportObject::default();
    placed.name = "P".to_string();
    placed.bounds = Rect {
        left: Twips(100),
        top: Twips(400),
        width: Twips(3000),
        height: Twips(240),
    };
    let mut f = FieldObject::default();
    f.data_source = "{?Region}".to_string();
    f.ref_kind = FieldRefKind::Parameter;
    placed.kind = ReportObjectKind::Field(Box::new(f));

    let embedded = run_text_object("Caption", &[("{?Region}", Some("?Region"))], 0);
    let texts = header_texts_with_params(
        &mut report,
        vec![embedded, placed],
        Some(AS_OF_UNIX),
        &params_of("Region", Value::Str("Nordwest".into())),
    );
    assert_eq!(texts, vec!["Nordwest".to_string(), "Nordwest".to_string()]);
}

/// `ModificationDate` / `ModificationTime` resolve to the file's last-save `FILETIME`, split in UTC.
#[test]
fn modification_specials_resolve_from_the_last_save_time() {
    let mut report = Report::default();
    report.summary_info.last_saved = Some(FILE_TIME);
    let texts = header_texts(
        &mut report,
        vec![
            special_object("MD", "ModificationDate", 0),
            special_object("MT", "ModificationTime", 400),
        ],
        Some(AS_OF_UNIX),
    );
    assert_eq!(
        texts,
        vec![FILE_TIME_DATE.to_string(), FILE_TIME_TIME.to_string()]
    );
}

/// `FileCreationDate` reads the creation timestamp, not the last-save one.
#[test]
fn file_creation_date_resolves_from_the_creation_time() {
    let mut report = Report::default();
    report.summary_info.created = Some(FILE_TIME);
    report.summary_info.last_saved = Some(FILE_TIME + 86_400 * 10_000_000);
    let texts = header_texts(
        &mut report,
        vec![special_object("FC", "FileCreationDate", 0)],
        Some(AS_OF_UNIX),
    );
    assert_eq!(texts, vec![FILE_TIME_DATE.to_string()]);
}

/// A `ModificationDate` embedded in a text object resolves the same fact the placed field does.
#[test]
fn embedded_modification_date_run_renders_the_save_date() {
    let mut report = Report::default();
    report.summary_info.last_saved = Some(FILE_TIME);
    let obj = run_text_object(
        "Caption",
        &[
            ("Last modified: ", None),
            ("ModificationDate", Some("Modification Date")),
        ],
        0,
    );
    let texts = header_texts(&mut report, vec![obj], Some(AS_OF_UNIX));
    assert_eq!(texts, vec![format!("Last modified: {FILE_TIME_DATE}")]);
}

/// A file timestamp the summary set does not carry — absent, or the zero `FILETIME` the engine
/// writes for "never printed" — renders blank rather than the 1601 epoch.
#[test]
fn absent_or_zero_file_timestamps_render_blank() {
    for stored in [None, Some(0)] {
        let mut report = Report::default();
        report.summary_info.last_saved = stored;
        report.summary_info.created = stored;
        let texts = header_texts(
            &mut report,
            vec![
                special_object("MD", "ModificationDate", 0),
                special_object("MT", "ModificationTime", 400),
                special_object("FC", "FileCreationDate", 800),
            ],
            Some(AS_OF_UNIX),
        );
        assert_eq!(texts, vec![String::new(), String::new(), String::new()]);
    }
}

/// Two region groups (`t.region`) with a group footer carrying `objects`, over four detail rows.
fn grouped_report(objects: Vec<ReportObject>) -> (Report, SavedData) {
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    let mut g = Group::default();
    g.condition_field = "t.region".into();
    g.sort.direction = SortDirection::AscendingOrder;
    report.data_definition.groups = vec![g];
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                300,
                vec![text_object("Row", "line", 0)],
            )],
        ),
        area(
            AreaSectionKind::GroupFooter,
            vec![section(AreaSectionKind::GroupFooter, "GF", 300, objects)],
        ),
    ];
    let saved = saved_data(
        &[
            ("t.region", FieldValueType::String),
            ("t.x", FieldValueType::Number),
        ],
        &[&["A", "1"], &["A", "2"], &["B", "3"], &["B", "4"]],
    );
    (report, saved)
}

fn drawn_texts(report: &Report, saved: &SavedData) -> Vec<String> {
    let ds = build_dataset(&SavedDataSource::new(saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    layout(report, &ds, &formulas)
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

/// A group-name reference embedded in a text object renders the group's key — the whole
/// `GroupName ({cond})` placeholder is consumed, not just its inner argument.
#[test]
fn embedded_group_name_run_renders_the_key_without_its_syntax() {
    let obj = run_text_object(
        "Caption",
        &[
            ("Total for ", None),
            ("GroupName ({t.region})", Some("Group #1 Name")),
            (":", None),
        ],
        0,
    );
    let (report, saved) = grouped_report(vec![obj]);
    let texts = drawn_texts(&report, &saved);
    assert!(
        texts.contains(&"Total for A:".to_string()) && texts.contains(&"Total for B:".to_string()),
        "group captions: {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.contains("GroupName")),
        "the placeholder syntax reached the page: {texts:?}"
    );
}

/// A placed group-name field object resolves the level its condition field names.
#[test]
fn group_name_field_object_resolves_the_named_level() {
    let mut o = ReportObject::default();
    o.name = "GN".to_string();
    o.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(3000),
        height: Twips(240),
    };
    let mut f = FieldObject::default();
    f.data_source = "GroupName ({t.region})".to_string();
    f.ref_kind = FieldRefKind::GroupName;
    o.kind = ReportObjectKind::Field(Box::new(f));

    let (report, saved) = grouped_report(vec![o]);
    let texts = drawn_texts(&report, &saved);
    assert!(
        texts.contains(&"A".to_string()) && texts.contains(&"B".to_string()),
        "group keys: {texts:?}"
    );
}

/// `GroupName ({cond})` called from *inside a formula body* resolves to the same key a placed
/// group-name field prints, so the formula can format it however it likes. The group condition here
/// is a String column holding an ISO date, which the formula parses and re-formats — the shape a
/// report uses to print a group header the stored date format cannot express.
#[test]
fn a_formula_body_resolves_group_name_to_the_group_key() {
    use rpt_model::{FieldDef, FieldKindData, Formula, FormulaField};

    let mut o = ReportObject::default();
    o.name = "Caption".to_string();
    o.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(4000),
        height: Twips(240),
    };
    let mut f = FieldObject::default();
    f.data_source = "{@GroupDate}".to_string();
    f.ref_kind = FieldRefKind::Formula;
    o.kind = ReportObjectKind::Field(Box::new(f));

    let mut def = FieldDef::default();
    def.name = "GroupDate".into();
    def.value_type = FieldValueType::String;
    def.kind = FieldKindData::Formula(FormulaField {
        text: Formula(r#"ToText(CDate(GroupName({t.day})), "yyyy, MMMM dd")"#.into()),
        ..FormulaField::default()
    });

    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.data_definition.field_definitions = vec![def];
    let mut g = Group::default();
    g.condition_field = "t.day".into();
    g.sort.direction = SortDirection::AscendingOrder;
    report.data_definition.groups = vec![g];
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                300,
                vec![text_object("Row", "line", 0)],
            )],
        ),
        area(
            AreaSectionKind::GroupFooter,
            vec![section(AreaSectionKind::GroupFooter, "GF", 300, vec![o])],
        ),
    ];
    let saved = saved_data(
        &[("t.day", FieldValueType::String)],
        &[&["2019-01-01"], &["2019-01-01"], &["1832-01-28"]],
    );

    let texts = drawn_texts(&report, &saved);
    assert!(
        texts.contains(&"2019, January 01".to_string())
            && texts.contains(&"1832, January 28".to_string()),
        "group captions: {texts:?}"
    );
}

/// A text object whose runs carry no field reference still flattens to exactly its `display`.
#[test]
fn literal_runs_flatten_unchanged() {
    let mut report = Report::default();
    let obj = run_text_object("Label", &[("Numeric", None), (" Code", None)], 0);
    let texts = header_texts(&mut report, vec![obj], Some(AS_OF_UNIX));
    assert_eq!(texts, vec!["Numeric Code".to_string()]);
}

/// A `Page N of M` reference embedded in a page-footer text object is a forward reference like the
/// placed field is: every page must end up carrying the final total, not the provisional one the
/// single layout pass had when that page was closed.
#[test]
fn embedded_page_total_run_is_patched_with_the_final_count() {
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(1200);
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                300,
                vec![text_object("Row", "line", 0)],
            )],
        ),
        area(
            AreaSectionKind::PageFooter,
            vec![section(
                AreaSectionKind::PageFooter,
                "PF",
                300,
                vec![run_text_object(
                    "Foot",
                    &[("-- ", None), ("PageNofM", Some("Page N of M"))],
                    0,
                )],
            )],
        ),
    ];
    let saved = saved_data(
        &[("t.x", FieldValueType::Number)],
        &[&["1"], &["2"], &["3"], &["4"], &["5"], &["6"]],
    );
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);
    let total = doc.pages.len();
    assert!(total > 1, "expected a multi-page render, got {total}");
    let footers: Vec<String> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) if t.text.starts_with("-- ") => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    let expected: Vec<String> = (1..=total)
        .map(|n| format!("-- Page {n} of {total}"))
        .collect();
    assert_eq!(footers, expected);
}

/// Patching the final page count into a run re-measures its text, and that measurement must stay in
/// the same advance model the run was emitted under: natural advances **plus** the paragraph's rigid
/// character spacing. Measuring the rewritten text without the spacing under-reports the width, and a
/// centre/right-aligned footer anchors off that advance — so it would re-anchor to the wrong x.
#[test]
fn rewritten_page_total_run_keeps_character_spacing_in_its_advance() {
    use rpt_pages::{ApproxLayout, TextLayout};
    const SPACING: i32 = 40;

    let mut foot = run_text_object(
        "Foot",
        &[("-- ", None), ("PageNofM", Some("Page N of M"))],
        0,
    );
    foot.format.horizontal_alignment = rpt_model::Alignment::RightAlign;
    if let ReportObjectKind::Text(t) = &mut foot.kind {
        t.paragraphs[0].runs[0].character_spacing = Twips(SPACING);
    }

    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(1200);
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                300,
                vec![text_object("Row", "line", 0)],
            )],
        ),
        area(
            AreaSectionKind::PageFooter,
            vec![section(AreaSectionKind::PageFooter, "PF", 300, vec![foot])],
        ),
    ];
    let saved = saved_data(
        &[("t.x", FieldValueType::Number)],
        &[&["1"], &["2"], &["3"], &["4"], &["5"], &["6"]],
    );
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);
    let total = doc.pages.len();
    assert!(total > 1, "expected a multi-page render, got {total}");

    let footers: Vec<&rpt_pages::TextRun> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) if t.text.starts_with("-- ") => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(footers.len(), total, "one patched footer run per page");
    for (i, run) in footers.iter().enumerate() {
        assert_eq!(run.character_spacing, Twips(SPACING));
        let natural = ApproxLayout.width_twips(&run.text, &run.font) as i32;
        let spaced = natural + SPACING * run.text.chars().count() as i32;
        assert!(
            spaced > natural,
            "the spaced and natural widths must differ for the assertion below to discriminate"
        );
        assert_eq!(
            run.metrics.unwrap().advance,
            Twips(spaced),
            "page {} footer advance must include its character spacing",
            i + 1
        );
    }
}
