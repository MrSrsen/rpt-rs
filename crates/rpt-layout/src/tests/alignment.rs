//! Horizontal alignment resolution: what `Alignment::DefaultAlign` means at paint time.

use super::*;
use rpt_model::{Alignment, FieldDef, FieldKindData, Formula, FormulaField, ReadingOrder};
use rpt_pages::TextAlign;

/// A field object with the given bound value type and stored alignment, at (left=100, width=3000).
fn field_at(vt: FieldValueType, align: Alignment) -> ReportObject {
    let mut o = db_field_object("F", "{t.x}", 0);
    o.format.horizontal_alignment = align;
    if let ReportObjectKind::Field(f) = &mut o.kind {
        f.value_type = vt;
    }
    o
}

#[test]
fn default_align_resolves_right_for_numeric_value_types() {
    use FieldValueType as V;
    let report = Report::default();
    for vt in [
        V::Int8s,
        V::Int16s,
        V::Int32s,
        V::Int32u,
        V::Number,
        V::Currency,
    ] {
        let obj = field_at(vt, Alignment::DefaultAlign);
        assert_eq!(
            crate::resolved_align(&report, &obj, ReadingOrder::LeftToRight),
            TextAlign::Right,
            "{vt:?} is numeric, so its column lines up on the right"
        );
    }
}

#[test]
fn default_align_resolves_left_for_non_numeric_value_types() {
    use FieldValueType as V;
    let report = Report::default();
    for vt in [
        V::String,
        V::PersistentMemo,
        V::Blob,
        V::Date,
        V::Time,
        V::DateTime,
        V::Boolean,
        V::Unknown,
    ] {
        let obj = field_at(vt, Alignment::DefaultAlign);
        assert_eq!(
            crate::resolved_align(&report, &obj, ReadingOrder::LeftToRight),
            TextAlign::Left,
            "{vt:?} is not numeric, so it stays flush left"
        );
    }
}

#[test]
fn an_explicit_alignment_overrides_the_numeric_default() {
    let report = Report::default();
    for (stored, want) in [
        (Alignment::LeftAlign, TextAlign::Left),
        (Alignment::HorizontalCenterAlign, TextAlign::Center),
        (Alignment::RightAlign, TextAlign::Right),
        (Alignment::Justified, TextAlign::Justified),
    ] {
        let obj = field_at(FieldValueType::Currency, stored);
        assert_eq!(
            crate::resolved_align(&report, &obj, ReadingOrder::LeftToRight),
            want,
            "a stored {stored:?} wins over the value-type default"
        );
        // The same explicit choice on a string field resolves identically — the value type only ever
        // decides the *default*.
        let s = field_at(FieldValueType::String, stored);
        assert_eq!(
            crate::resolved_align(&report, &s, ReadingOrder::LeftToRight),
            want
        );
    }
}

#[test]
fn default_align_keys_on_the_declared_type_not_the_drawn_text() {
    // A formula field object carries no bound value type; the type comes from its *definition*.
    // A formula declared `Currency` right-aligns even though nothing has been evaluated, and one
    // declared `String` stays left even when it would print digits.
    let mut o = db_field_object("F", "{@amount}", 0);
    o.format.horizontal_alignment = Alignment::DefaultAlign;
    if let ReportObjectKind::Field(f) = &mut o.kind {
        f.data_source = "{@amount}".into();
        f.ref_kind = FieldRefKind::Formula;
        f.value_type = FieldValueType::Unknown;
    }
    let declared = |vt: FieldValueType| {
        let mut def = FieldDef::default();
        def.name = "amount".into();
        def.value_type = vt;
        def.kind = FieldKindData::Formula(FormulaField {
            text: Formula("ToText({t.x})".into()),
            ..FormulaField::default()
        });
        let mut report = Report::default();
        report.data_definition.field_definitions = vec![def];
        crate::resolved_align(&report, &o, ReadingOrder::LeftToRight)
    };
    assert_eq!(declared(FieldValueType::Currency), TextAlign::Right);
    assert_eq!(declared(FieldValueType::String), TextAlign::Left);
}

#[test]
fn the_numeric_default_is_a_field_object_rule_only() {
    // A text object's default alignment follows its reading order, not its content: a text object
    // whose literal happens to be a number is still flush left.
    let report = Report::default();
    let t = text_object("T", "1234.50", 0);
    assert_eq!(
        crate::resolved_align(&report, &t, ReadingOrder::LeftToRight),
        TextAlign::Left
    );
    assert_eq!(
        crate::resolved_align(&report, &t, ReadingOrder::RightToLeft),
        TextAlign::Right
    );
}

#[test]
fn a_default_aligned_numeric_detail_field_draws_flush_right() {
    // End to end: a Currency field and a String field side by side in a detail band, both stored at
    // `DefaultAlign`. The currency run must carry `Right` and the string run `Left`, so the currency
    // column lines up on the box's right edge.
    let mut amount = db_field_object("Amount", "{t.amt}", 0);
    amount.bounds = Rect {
        left: Twips(4000),
        top: Twips(0),
        width: Twips(2000),
        height: Twips(240),
    };
    if let ReportObjectKind::Field(f) = &mut amount.kind {
        f.value_type = FieldValueType::Currency;
    }
    let name = db_field_object("Name", "{t.name}", 0);

    let mut report = Report::default();
    report.print_options.content_width = Twips(12240);
    report.print_options.content_height = Twips(15840);
    report.report_definition.areas = vec![area(
        AreaSectionKind::Detail,
        vec![section(
            AreaSectionKind::Detail,
            "Details",
            300,
            vec![name, amount],
        )],
    )];

    let saved = saved_data(
        &[
            ("t.name", FieldValueType::String),
            ("t.amt", FieldValueType::Currency),
        ],
        &[&["ab", "10"], &["cd", "1000"]],
    );
    let ds = build_dataset(&SavedDataSource::new(&saved), &report.data_definition);
    let formulas = rpt_data::compile_formulas(&report.data_definition);
    let doc = layout(&report, &ds, &formulas);

    let runs = text_runs(&doc);
    let by_name = |n: &str| -> Vec<&rpt_pages::TextRun> {
        runs.iter()
            .copied()
            .filter(|r| {
                r.source
                    .as_ref()
                    .is_some_and(|s| s.object_name.as_deref() == Some(n))
            })
            .collect()
    };
    let amounts = by_name("Amount");
    assert_eq!(amounts.len(), 2, "one currency run per row");
    for r in &amounts {
        assert_eq!(
            r.align,
            TextAlign::Right,
            "a default-aligned currency field"
        );
        assert_eq!(
            r.bounds.left.0 + r.bounds.width.0,
            6000,
            "right edge is the box's"
        );
    }
    for r in by_name("Name") {
        assert_eq!(r.align, TextAlign::Left, "a default-aligned string field");
    }
}

/// A field heading stored at `DefaultAlign` sits over its column the way the column itself does:
/// it takes the headed field's explicit alignment when the field has one, and otherwise the same
/// numeric/non-numeric rule the field would resolve to.
#[test]
fn a_default_aligned_heading_follows_the_field_it_heads() {
    fn heading_over(field: ReportObject) -> TextAlign {
        let mut h = ReportObject::default();
        h.name = "H".into();
        h.format.horizontal_alignment = Alignment::DefaultAlign;
        h.kind = ReportObjectKind::FieldHeading(rpt_model::FieldHeadingObject {
            text: "Amount".into(),
            field_object_name: field.name.clone(),
            ..Default::default()
        });
        let mut report = Report::default();
        report.report_definition.areas = vec![area(
            AreaSectionKind::Detail,
            vec![section(
                AreaSectionKind::Detail,
                "Details",
                300,
                vec![field, h],
            )],
        )];
        let h = report.objects().find(|o| o.name == "H").unwrap().clone();
        crate::resolved_align(&report, &h, ReadingOrder::LeftToRight)
    }

    // The headed field is itself default-aligned: the heading inherits the value-type rule.
    assert_eq!(
        heading_over(field_at(FieldValueType::Currency, Alignment::DefaultAlign)),
        TextAlign::Right
    );
    assert_eq!(
        heading_over(field_at(FieldValueType::String, Alignment::DefaultAlign)),
        TextAlign::Left
    );
    // The headed field carries an explicit alignment: the heading takes that instead, even where
    // it contradicts the value type.
    assert_eq!(
        heading_over(field_at(FieldValueType::Currency, Alignment::LeftAlign)),
        TextAlign::Left
    );
    assert_eq!(
        heading_over(field_at(
            FieldValueType::String,
            Alignment::HorizontalCenterAlign
        )),
        TextAlign::Center
    );
}

/// A heading whose link no longer names a live field is decoded as a plain text object, so it falls
/// through to the reading-order default rather than to any field rule.
#[test]
fn a_heading_over_a_missing_field_takes_the_reading_order_default() {
    let mut h = ReportObject::default();
    h.name = "H".into();
    h.format.horizontal_alignment = Alignment::DefaultAlign;
    h.kind = ReportObjectKind::FieldHeading(rpt_model::FieldHeadingObject {
        text: "Amount".into(),
        field_object_name: "NoSuchField".into(),
        ..Default::default()
    });
    let report = Report::default();
    assert_eq!(
        crate::resolved_align(&report, &h, ReadingOrder::LeftToRight),
        TextAlign::Left
    );
    assert_eq!(
        crate::resolved_align(&report, &h, ReadingOrder::RightToLeft),
        TextAlign::Right
    );
}
