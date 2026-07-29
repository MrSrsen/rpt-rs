use super::*;

#[test]
fn can_grow_text_wraps_into_multiple_lines_and_grows_band() {
    // A detail band with a narrow can-grow text object holding long text, followed by a second
    // detail row — the wrapped text must produce multiple runs and push the next row down.
    let mut wide = text_object(
        "Memo",
        "the quick brown fox jumps over the lazy dog again",
        0,
    );
    wide.bounds = Rect {
        left: Twips(100),
        top: Twips(0),
        width: Twips(1500),
        height: Twips(240),
    };
    if let ReportObjectKind::Text(t) = &mut wide.kind {
        t.text = "the quick brown fox jumps over the lazy dog again".into();
    }
    wide.format.can_grow = true;

    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(AreaSectionKind::Detail, "Details", 240, vec![wide])],
    )];
    let saved = saved_data(&[("t.x", FieldValueType::Number)], &[&["1"], &["2"]]);
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    let runs: Vec<&rpt_pages::TextRun> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t),
            _ => None,
        })
        .collect();
    // Two rows, each wrapping to >1 line → more than 2 runs total.
    assert!(
        runs.len() > 2,
        "expected wrapped multi-line runs, got {}",
        runs.len()
    );
    // Row 1's runs stack vertically (increasing tops).
    let tops: Vec<i32> = runs.iter().map(|r| r.bounds.top.0).collect();
    assert!(tops.windows(2).any(|w| w[1] > w[0]), "lines should stack");
    // The band grew: the two rows are separated by more than the 240-twip design height.
    let distinct_tops: std::collections::BTreeSet<i32> = tops.iter().copied().collect();
    assert!(
        *distinct_tops.iter().last().unwrap() > 240,
        "band grew past design height"
    );
}

#[test]
fn report_header_grows_with_can_grow_content() {
    // A Report Header is a flow section: a can-grow object wraps and the band grows, pushing the
    // detail below the header's designed height.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::ReportHeader,
            vec![section(
                AreaSectionKind::ReportHeader,
                "RH",
                240,
                vec![wrapping_can_grow("Memo")],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                240,
                vec![text_object("MARK", "MARK", 0)],
            )],
        ),
    ];
    let saved = empty_saved();
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    let runs = text_runs(&doc);
    // The header's can-grow text wrapped to more than one line.
    let memo_runs = runs.iter().filter(|r| r.text != "MARK").count();
    assert!(
        memo_runs > 1,
        "report header can-grow should wrap, got {memo_runs} run(s)"
    );
    // The detail marker sits below the grown header (past its 240-twip design height).
    let mark_top = runs
        .iter()
        .find(|r| r.text == "MARK")
        .expect("MARK run")
        .bounds
        .top
        .0;
    assert!(
        mark_top > 240,
        "detail should follow the grown header, MARK at {mark_top}"
    );
}

#[test]
fn page_header_does_not_grow_can_grow_is_inert() {
    // A Page Header is a fixed repeating band: can-grow is inert (native behavior), so its long
    // text stays a single (clipped) run and the detail is not pushed down.
    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![
        area(
            AreaSectionKind::PageHeader,
            vec![section(
                AreaSectionKind::PageHeader,
                "PH",
                240,
                vec![wrapping_can_grow("Memo")],
            )],
        ),
        area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                240,
                vec![text_object("MARK", "MARK", 0)],
            )],
        ),
    ];
    let saved = empty_saved();
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    let runs = text_runs(&doc);
    // Can-grow is inert in a page header: the long text is a single (clipped) run, not wrapped.
    let memo_runs = runs.iter().filter(|r| r.text != "MARK").count();
    assert_eq!(
        memo_runs, 1,
        "page header can-grow must be inert (no wrapping)"
    );
    // The detail is not pushed past the fixed 240-twip header height.
    let mark_top = runs
        .iter()
        .find(|r| r.text == "MARK")
        .expect("MARK run")
        .bounds
        .top
        .0;
    assert!(
        mark_top <= 240,
        "fixed page header must not push the detail down, MARK at {mark_top}"
    );
}

#[test]
fn left_indent_shifts_run_and_reduces_width() {
    let runs = runs_for_object(indented_text("Lbl", "ID", 72, 0, 0));
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].bounds.left.0,
        100 + 72,
        "run shifts right by left_indent"
    );
    assert_eq!(
        runs[0].bounds.width.0,
        3000 - 72,
        "usable width reduced by left_indent"
    );
}

#[test]
fn right_indent_reduces_width_only() {
    let runs = runs_for_object(indented_text("Lbl", "ID", 0, 100, 0));
    assert_eq!(
        runs[0].bounds.left.0, 100,
        "left edge unchanged by right_indent"
    );
    assert_eq!(
        runs[0].bounds.width.0,
        3000 - 100,
        "usable width reduced by right_indent"
    );
}

#[test]
fn first_line_indent_offsets_only_first_line_of_paragraph() {
    // A can-grow paragraph that wraps to several lines: only the first line carries the first-line
    // indent; continuation lines sit at the object's left edge.
    let mut o = indented_text(
        "Memo",
        "the quick brown fox jumps over the lazy dog again",
        0,
        0,
        200,
    );
    o.bounds.width = Twips(1500);
    o.format.can_grow = true;
    let runs = runs_for_object(o);
    assert!(runs.len() > 1, "text should wrap, got {}", runs.len());
    assert_eq!(
        runs[0].bounds.left.0,
        100 + 200,
        "first line carries first_line_indent"
    );
    for r in &runs[1..] {
        assert_eq!(
            r.bounds.left.0, 100,
            "continuation lines are not first-line-indented"
        );
    }
}

#[test]
fn first_line_indent_narrows_the_first_line_wrap() {
    // A large first-line indent must break the first line earlier (fewer words) than the
    // continuation lines, which keep the paragraph's full width.
    let long = "the quick brown fox jumps over the lazy dog again";

    let mut base = indented_text("Base", long, 0, 0, 0);
    base.bounds.width = Twips(1500);
    base.format.can_grow = true;
    let base_runs = runs_for_object(base);

    let mut ind = indented_text("Ind", long, 0, 0, 900);
    ind.bounds.width = Twips(1500);
    ind.format.can_grow = true;
    let ind_runs = runs_for_object(ind);

    assert!(
        base_runs.len() > 1 && ind_runs.len() > 1,
        "both should wrap"
    );
    let base_first = base_runs[0].text.split_whitespace().count();
    let ind_first = ind_runs[0].text.split_whitespace().count();
    assert!(
        ind_first < base_first,
        "first-line indent should break the first line earlier: {ind_first} vs {base_first} words \
         ({:?} vs {:?})",
        ind_runs[0].text,
        base_runs[0].text
    );
    // First line shifted right and narrowed; the continuation line uses the full box.
    assert_eq!(ind_runs[0].bounds.left.0, 100 + 900);
    assert_eq!(ind_runs[0].bounds.width.0, 1500 - 900);
    assert_eq!(ind_runs[1].bounds.left.0, 100);
    assert_eq!(ind_runs[1].bounds.width.0, 1500);
}

#[test]
fn indent_is_a_no_op_when_zero() {
    // A paragraph with all-zero indents places exactly as an object with no paragraph tree at all.
    let with_para = runs_for_object(indented_text("A", "hello", 0, 0, 0));
    let plain = runs_for_object(text_object("B", "hello", 0));
    assert_eq!(with_para[0].bounds.left.0, plain[0].bounds.left.0);
    assert_eq!(with_para[0].bounds.width.0, plain[0].bounds.width.0);
    assert_eq!(with_para[0].bounds.left.0, 100);
    assert_eq!(with_para[0].bounds.width.0, 3000);
}

#[test]
fn rtl_default_align_reads_right() {
    use rpt_model::ReadingOrder;
    let mut o = text_object("R", "abc", 0);
    if let ReportObjectKind::Text(t) = &mut o.kind {
        t.reading_order = ReadingOrder::RightToLeft;
    }
    let runs = runs_for_object(o);
    assert_eq!(
        runs[0].align,
        rpt_pages::TextAlign::Right,
        "RTL default-align reads flush right"
    );
    // An LTR object at the default alignment stays left.
    let ltr = runs_for_object(text_object("L", "abc", 0));
    assert_eq!(ltr[0].align, rpt_pages::TextAlign::Left);
}

#[test]
fn multi_column_flows_records_across_columns() {
    use rpt_model::MultiColumn;
    // A detail band with one object at left=100, laid out in 3 columns of pitch 3000.
    let obj = text_object("Cell", "X", 0);
    let mut report = Report::default();
    report.print_options.content_width = Twips(12000);
    report.print_options.content_height = Twips(20000);
    report.print_options.multi_column = Some(MultiColumn {
        columns: 3,
        column_width: Twips(3000),
        gap_h: Twips(0),
        gap_v: Twips(0),
        across_then_down: true,
    });
    report.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(AreaSectionKind::Detail, "Details", 300, vec![obj])],
    )];
    let saved = SavedData {
        record_count: 6,
        columns: vec![SavedColumn {
            name: "t.x".into(),
            value_type: FieldValueType::Number,
        }],
        rows: (0..6).map(|_| vec![Some("1".into())]).collect(),
    };
    let src = SavedDataSource::new(&saved);
    let ds = build_dataset(&src, &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    // Collect the 6 detail cells' (left, top).
    let cells: Vec<(i32, i32)> = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) if t.text == "X" => Some((t.bounds.left.0, t.bounds.top.0)),
            _ => None,
        })
        .collect();
    assert_eq!(cells.len(), 6, "6 records rendered");
    // content_left = 0 (zero margins); object left = 100; pitch = 3000 → columns at 100/3100/6100.
    let lefts: Vec<i32> = cells.iter().map(|c| c.0).collect();
    assert_eq!(
        lefts,
        vec![100, 3100, 6100, 100, 3100, 6100],
        "3 columns, wrap after 3"
    );
    // Rows 0-2 share a top; rows 3-5 share a lower top.
    assert_eq!(cells[0].1, cells[1].1, "first row of columns aligned");
    assert!(cells[3].1 > cells[0].1, "second column-row is lower");
}

#[test]
fn multi_column_down_then_across_fills_columns_vertically() {
    use rpt_model::MultiColumn;
    // 2 columns, pitch 3000, one 300-twip object; a short body so only 2 records fit per column.
    // Down-then-across fills column 0 top-to-bottom, then column 1.
    let obj = text_object("Cell", "X", 0);
    let mut report = Report::default();
    report.print_options.content_width = Twips(12000);
    report.print_options.content_height = Twips(700);
    report.print_options.multi_column = Some(MultiColumn {
        columns: 2,
        column_width: Twips(3000),
        gap_h: Twips(0),
        gap_v: Twips(0),
        across_then_down: false,
    });
    report.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(AreaSectionKind::Detail, "Details", 300, vec![obj])],
    )];
    let saved = SavedData {
        record_count: 4,
        columns: vec![SavedColumn {
            name: "t.x".into(),
            value_type: FieldValueType::Number,
        }],
        rows: (0..4).map(|_| vec![Some("1".into())]).collect(),
    };
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    // (left, top) of every cell, in record (print) order — all should land on page 1.
    let cells: Vec<(i32, i32)> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t) if t.text == "X" => Some((t.bounds.left.0, t.bounds.top.0)),
            _ => None,
        })
        .collect();
    assert_eq!(cells.len(), 4, "4 records on one page");
    // Records 0,1 stack down column 0 (same left); 2,3 stack down column 1 (left + pitch 3000).
    assert_eq!(cells[0].0, cells[1].0, "records 0,1 in the same column");
    assert_eq!(cells[2].0, cells[3].0, "records 2,3 in the same column");
    assert_eq!(
        cells[2].0 - cells[0].0,
        3000,
        "second column is one pitch to the right"
    );
    // Vertical stacking within a column, restarting at the top of the next column.
    assert!(cells[1].1 > cells[0].1, "record 1 sits below record 0");
    assert_eq!(cells[1].1 - cells[0].1, 300, "stacked by the detail height");
    assert_eq!(cells[2].1, cells[0].1, "column 1 restarts at the top");
    assert_eq!(
        cells[3].1, cells[1].1,
        "column 1 second record aligns with column 0's"
    );
}

/// ObjectFormat.VerticalAlignment offsets the text block within a fixed box taller than its text:
/// Top pins to the box top, VerticalCenter to the middle, Bottom to the box bottom.
#[test]
fn vertical_alignment_offsets_text_block() {
    // Single 240-twip line (10pt → 10 × 20 × 1.2) in a 720-twip box: 480 twips of slack.
    fn top_of(valign: VerticalAlignment) -> i32 {
        let mut o = ReportObject::default();
        o.name = "V".into();
        o.bounds = Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(3000),
            height: Twips(720),
        };
        let mut t = TextObject::default();
        t.text = "one line".into();
        t.font_color.font.size_pt = 10.0;
        o.kind = ReportObjectKind::Text(t);
        o.format.vertical_alignment = valign;
        let mut report = tiny_report(15840);
        // The detail band must be tall enough to hold the 720-twip box.
        report.report_definition.areas[1].sections[0].height = Twips(720);
        report.report_definition.areas[1].sections[0].objects = vec![o];
        rendered(&report, &numeric_rows(1)).pages[0]
            .ops
            .iter()
            .find_map(|op| match op {
                DrawOp::Text(t) if t.text == "one line" => Some(t.bounds.top.0),
                _ => None,
            })
            .expect("the aligned text run")
    }
    // The detail band starts below the 300-twip page header.
    let base = 300;
    assert_eq!(
        top_of(VerticalAlignment::Top),
        base,
        "top pins to the box top"
    );
    assert_eq!(
        top_of(VerticalAlignment::VerticalCenter),
        base + 240,
        "centre offsets by half the 480-twip slack"
    );
    assert_eq!(
        top_of(VerticalAlignment::Bottom),
        base + 480,
        "bottom offsets by the full slack"
    );
}

/// Each paragraph renders at its own font size — a 12pt paragraph followed by a 20pt one emits two
/// runs at those sizes, and the larger paragraph advances the next line by its taller pitch.
#[test]
fn per_paragraph_font_size_applies() {
    let single = rpt_model::LineSpacing::default();
    let runs = runs_for_object(multi_para_object(
        "Mixed",
        &[(single, 12.0, "first"), (single, 20.0, "second")],
    ));
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].font.size_pt, 12.0);
    assert_eq!(runs[1].font.size_pt, 20.0);
    // The 12pt line's pitch is 12 * 20 * 1.2 = 288 twips (ApproxLayout), so the 20pt line sits there.
    assert_eq!(runs[1].bounds.top.0 - runs[0].bounds.top.0, 288);
    // The 20pt run carries the taller line height (20 * 20 * 1.2 = 480).
    assert_eq!(runs[1].metrics.unwrap().line_height.0, 480);
}

/// Line spacing scales each line's pitch: double spacing doubles the line height and the gap to the
/// next paragraph; exact spacing pins the pitch to its twip value regardless of font size.
#[test]
fn line_spacing_scales_line_pitch() {
    use rpt_model::{LineSpacing, LineSpacingType};
    let single = LineSpacing::default();
    let double = LineSpacing {
        spacing_type: LineSpacingType::Multiple,
        raw: 0x0002_0000,
    };
    let s = runs_for_object(multi_para_object(
        "S",
        &[(single, 10.0, "a"), (single, 10.0, "b")],
    ));
    let d = runs_for_object(multi_para_object(
        "D",
        &[(double, 10.0, "a"), (double, 10.0, "b")],
    ));
    let sh = s[0].metrics.unwrap().line_height.0;
    let dh = d[0].metrics.unwrap().line_height.0;
    assert_eq!(sh, 240); // 10pt * 20 * 1.2
    assert_eq!(dh, 480); // doubled
    assert_eq!(d[1].bounds.top.0 - d[0].bounds.top.0, dh);

    let exact = LineSpacing {
        spacing_type: LineSpacingType::Exact,
        raw: 360,
    };
    let e = runs_for_object(multi_para_object(
        "E",
        &[(exact, 10.0, "a"), (exact, 10.0, "b")],
    ));
    assert_eq!(e[0].metrics.unwrap().line_height.0, 360);
    assert_eq!(e[1].bounds.top.0 - e[0].bounds.top.0, 360);
}

/// Justified alignment stretches every wrapped line except a paragraph's last, which stays flush-left
/// (typography never stretches the final line). A single-line paragraph is that last line.
#[test]
fn justified_marks_non_last_lines_left() {
    use rpt_model::Alignment;
    use rpt_pages::TextAlign;
    let mut o = text_object(
        "J",
        "the quick brown fox jumps over the lazy dog again and again over the hill",
        0,
    );
    o.bounds.width = Twips(1500);
    o.format.can_grow = true;
    o.format.horizontal_alignment = Alignment::Justified;
    let runs = runs_for_object(o);
    assert!(runs.len() > 1, "text should wrap into several lines");
    for r in &runs[..runs.len() - 1] {
        assert_eq!(r.align, TextAlign::Justified, "interior lines justify");
    }
    assert_eq!(
        runs.last().unwrap().align,
        TextAlign::Left,
        "the last line stays ragged-left"
    );

    // A single (last) line is never stretched.
    let mut one = text_object("J1", "short", 0);
    one.format.horizontal_alignment = Alignment::Justified;
    assert_eq!(runs_for_object(one)[0].align, TextAlign::Left);
}

/// A quarter-turn rotation is emitted as the run's rotation angle and lays the lines out as columns:
/// a 90° run anchors at the box bottom, a 270° run at the box top. Upright text stays angle 0.
#[test]
fn text_rotation_angle_emitted_and_columns_anchored() {
    use rpt_model::TextRotationAngle;
    let mut o = text_object("R90", "abc", 0);
    o.format.text_rotation = TextRotationAngle::Rotate90;
    let r90 = runs_for_object(o);
    assert_eq!(r90[0].rotation, 90.0);
    // The box is at top 0, left 100, 3000×240. 90°: the run anchors at the box bottom (top + height)
    // so its text reads upward within the box; the first column keeps the box left.
    assert_eq!(r90[0].bounds.top.0, 240);
    assert_eq!(r90[0].bounds.left.0, 100);

    let mut o2 = text_object("R270", "abc", 0);
    o2.format.text_rotation = TextRotationAngle::Rotate270;
    let r270 = runs_for_object(o2);
    assert_eq!(r270[0].rotation, 270.0);
    // 270°: the run anchors at the box top-right (left + width), reading downward.
    assert_eq!(r270[0].bounds.top.0, 0);
    assert_eq!(r270[0].bounds.left.0, 100 + 3000);

    assert_eq!(runs_for_object(text_object("U", "abc", 0))[0].rotation, 0.0);
}

/// A horizontal tab is a layout advance, not a glyph: the run splits at the tab and the text after it
/// starts at the next quarter-inch stop from the object's left edge, so no control character reaches
/// a backend's shaper (which would paint it as `.notdef`).
#[test]
fn a_tab_advances_to_the_next_stop_instead_of_reaching_a_shaper() {
    let runs = runs_for_object(text_object("T", "ab\tcd", 0));
    assert_eq!(runs.len(), 2, "the tab splits the line into two runs");
    assert!(
        runs.iter().all(|r| !r.text.contains('\t')),
        "no tab survives into the Page IR"
    );
    assert_eq!(runs[0].text, "ab");
    assert_eq!(runs[1].text, "cd");
    // The object is at left 100: the second run sits one 360-twip stop past it.
    assert_eq!(runs[0].bounds.left.0, 100);
    assert_eq!(runs[1].bounds.left.0, 100 + 360);
    // A trailing tab advances the pen and draws nothing.
    let trailing = runs_for_object(text_object("T2", "ab\t", 0));
    assert_eq!(trailing.len(), 1);
    assert_eq!(trailing[0].text, "ab");
}

/// A text object whose single paragraph carries `spacing` twips of rigid character spacing.
fn spaced_object(name: &str, text: &str, spacing: i32) -> ReportObject {
    use rpt_model::Paragraph;
    let mut o = text_object(name, text, 0);
    if let ReportObjectKind::Text(t) = &mut o.kind {
        t.display = text.to_string();
        t.paragraphs = vec![Paragraph {
            runs: vec![rpt_model::TextRun {
                text: text.to_string(),
                character_spacing: Twips(spacing),
                ..Default::default()
            }],
            ..Default::default()
        }];
    }
    o
}

/// A paragraph's character spacing travels onto the emitted run and is already counted in the
/// advance the run reports — extra after **every** Unicode scalar, the trailing one included.
#[test]
fn character_spacing_travels_with_the_run_and_is_in_its_advance() {
    use rpt_pages::{ApproxLayout, TextLayout};
    let text = "abcdef";
    let spaced = runs_for_object(spaced_object("S", text, 40));
    assert_eq!(spaced.len(), 1);
    assert_eq!(spaced[0].character_spacing, Twips(40));

    let natural = ApproxLayout.width_twips(text, &spaced[0].font);
    let expected = natural as i32 + 40 * text.chars().count() as i32;
    assert_eq!(spaced[0].metrics.unwrap().advance, Twips(expected));

    // Zero spacing is inert: the field is absent from the run and the advance is the natural width.
    let plain = runs_for_object(spaced_object("P", text, 0));
    assert_eq!(plain[0].character_spacing, Twips(0));
    assert_eq!(plain[0].metrics.unwrap().advance, Twips(natural as i32));
}

/// THE CONTRACT between the producer's advance model and any backend that re-shapes the run: summing
/// the backend's rule — natural cluster advance plus `character_spacing × the scalars in that
/// cluster` — over the run's clusters must reproduce `metrics.advance`, or measurement and drawing
/// disagree and the visible symptom is a wrong wrap on an earlier page.
///
/// The unit is the Unicode scalar, never the glyph: a cluster that shapes several scalars into one
/// ligature glyph still owes spacing for each of them. The per-glyph reading is asserted *not* to
/// reconcile, so this test fails if a backend ever adopts it.
#[test]
fn a_backends_per_cluster_spacing_sums_to_the_measured_advance() {
    use rpt_pages::{ApproxLayout, TextLayout};
    const SPACING: i32 = 40;
    let text = "off ice";
    let run = &runs_for_object(spaced_object("L", text, SPACING))[0];
    let advance = run.metrics.unwrap().advance.0;

    // A plausible shaping of the run: "ff" is one ligature glyph covering two scalars, the rest map
    // one glyph per scalar. Byte ranges into `text`, exactly as a shaper reports cluster indices.
    let clusters: Vec<&str> = vec!["o", "ff", " ", "i", "c", "e"];
    assert_eq!(clusters.concat(), text);

    let natural = |s: &str| ApproxLayout.width_twips(s, &run.font);
    let per_scalar: f64 = clusters
        .iter()
        .map(|c| natural(c) + f64::from(SPACING) * c.chars().count() as f64)
        .sum();
    assert!(
        (per_scalar - f64::from(advance)).abs() <= 1.0,
        "drawn {per_scalar} vs measured {advance}"
    );

    // Charging the spacing once per glyph loses it for every extra scalar the ligature covers.
    let per_glyph: f64 = clusters
        .iter()
        .map(|c| natural(c) + f64::from(SPACING))
        .sum();
    assert_eq!(
        (f64::from(advance) - per_glyph).round() as i32,
        SPACING,
        "the per-glyph rule must not reconcile — that is the bug this field prevents"
    );
}

/// Spacing widens the text, so it must also decide where the text breaks. Wrapping against the
/// unspaced width would break in the wrong place — the failure shows up as wrong text on the page
/// before the spaced one, not as visibly wrong spacing.
#[test]
fn character_spacing_moves_the_wrap_point() {
    let text = "the quick brown fox jumps over the lazy dog";
    let mut plain = spaced_object("W0", text, 0);
    plain.format.can_grow = true;
    plain.bounds.width = Twips(2400);
    let mut spaced = spaced_object("W1", text, 60);
    spaced.format.can_grow = true;
    spaced.bounds.width = Twips(2400);

    let plain_lines = runs_for_object(plain).len();
    let spaced_lines = runs_for_object(spaced).len();
    assert!(plain_lines > 1, "the box must already wrap unspaced");
    assert!(
        spaced_lines > plain_lines,
        "spacing must break earlier: {spaced_lines} vs {plain_lines} lines"
    );
}
