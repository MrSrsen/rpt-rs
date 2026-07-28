//! A formula that does not parse must say so.
//!
//! The parser recovers from a syntax error and returns a partial AST; the pipeline compiles and
//! evaluates it regardless. That is the right behaviour — one broken formula must not abort a render —
//! but the value it produces is meaningless, so a user seeing a blank field cannot tell "the formula
//! is broken" from "the data is null" from "we don't implement that builtin". These assert the
//! diagnostic exists, names the formula, and carries the byte span.
//!
//! Uses a synthesized report rather than a fixture: the point is a *deliberately* malformed formula,
//! and no committed fixture has one (the corpus is valid reports).

use rpt_data::{build_dataset_opts, CollectingSink, DatasetOptions, DiagnosticKind, EmptySource};
use rpt_model::{DataDefinition, FieldDef, FieldKindData, FieldValueType, Formula, FormulaField};

/// A data definition with one formula field whose body is `text`.
fn with_formula(name: &str, text: &str) -> DataDefinition {
    let mut dd = DataDefinition::default();
    dd.field_definitions.push(FieldDef {
        name: name.to_string(),
        value_type: FieldValueType::Number,
        kind: FieldKindData::Formula(FormulaField {
            text: Formula(text.to_string()),
            ..Default::default()
        }),
        ..Default::default()
    });
    dd
}

fn diagnose(dd: &DataDefinition) -> Vec<rpt_data::EvalDiagnostic> {
    let sink = CollectingSink::new();
    build_dataset_opts(
        &EmptySource,
        dd,
        DatasetOptions {
            sink: Some(&sink),
            ..Default::default()
        },
    );
    sink.into_diagnostics()
}

#[test]
fn a_malformed_formula_field_is_reported_with_its_name_and_span() {
    // Unbalanced parenthesis: the parser recovers, the evaluator runs the partial AST.
    let diags = diagnose(&with_formula("Order Total", "Sum({o.amount} * 2"));
    let parse: Vec<_> = diags
        .iter()
        .filter(|d| d.kind == DiagnosticKind::FormulaParse)
        .collect();
    assert!(
        !parse.is_empty(),
        "a formula that does not parse produced no diagnostic: {diags:?}"
    );
    let d = parse[0];
    assert!(
        d.detail.contains("Order Total"),
        "the diagnostic must name the formula: {}",
        d.detail
    );
    assert!(
        d.span.is_some(),
        "the diagnostic must carry the byte span: {d:?}"
    );
    // And it must say that the value cannot be trusted — that is the actionable part.
    assert!(
        d.detail.contains("partial parse"),
        "the diagnostic must say the value is not meaningful: {}",
        d.detail
    );
}

#[test]
fn a_well_formed_formula_is_silent() {
    // The corollary: a valid formula must never produce a parse diagnostic, or the signal is noise.
    let diags = diagnose(&with_formula("Doubled", "{o.amount} * 2"));
    assert!(
        !diags.iter().any(|d| d.kind == DiagnosticKind::FormulaParse),
        "a valid formula produced a parse diagnostic: {diags:?}"
    );
}

#[test]
fn a_malformed_record_selection_is_reported_too() {
    let dd = DataDefinition {
        record_selection: Some(Formula("{o.amount} > ".to_string())),
        ..Default::default()
    };
    let diags = diagnose(&dd);
    let d = diags
        .iter()
        .find(|d| d.kind == DiagnosticKind::FormulaParse)
        .unwrap_or_else(|| panic!("no parse diagnostic for a broken selection: {diags:?}"));
    assert!(
        d.detail.contains("record-selection"),
        "the diagnostic must name which formula: {}",
        d.detail
    );
}
