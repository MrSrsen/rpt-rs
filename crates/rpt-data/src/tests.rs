//! Pipeline tests over hand-built rows and data definitions.
//!
//! The `rpt` model structs `DataDefinition`/`FieldDef`/`Group`/`Sort` are `#[non_exhaustive]`, so
//! they are built here via `Default` + field assignment (struct literals are disallowed
//! cross-crate); the small builders below keep the tests readable.
//!
//! `Default` + field assignment is exactly what `clippy::field_reassign_with_default` flags, but the
//! struct-literal form it wants is impossible for these cross-crate `#[non_exhaustive]` types — so
//! the lint is allowed for this test module.
#![allow(clippy::field_reassign_with_default)]

use crate::*;
use crystal_formula::eval::Value;
use rpt_model::{
    DataDefinition, FieldDef, FieldKindData, FieldValueType, FormulaField, Group,
    ResetConditionType, RunningTotalField, SavedData, Sort, SortDirection, SummaryField,
    SummaryOperation,
};
use rpt_test_support::saved_data as saved;

fn group(field: &str, dir: SortDirection) -> Group {
    let mut g = Group::default();
    g.condition_field = field.to_string();
    g.sort.direction = dir;
    g
}

fn sort(field: &str, dir: SortDirection) -> Sort {
    let mut s = Sort::default();
    s.field = field.to_string();
    s.direction = dir;
    s
}

fn summary_field(name: &str, op: SummaryOperation, field: &str) -> FieldDef {
    let mut f = FieldDef::default();
    f.name = name.to_string();
    f.kind = FieldKindData::Summary(SummaryField {
        operation: op,
        summarized_field: field.to_string(),
        ..SummaryField::default()
    });
    f
}

fn formula_field(name: &str, body: &str) -> FieldDef {
    let mut f = FieldDef::default();
    f.name = name.to_string();
    f.kind = FieldKindData::Formula(FormulaField {
        text: rpt_model::Formula(body.to_string()),
        ..FormulaField::default()
    });
    f
}

fn num(v: &Value) -> f64 {
    v.as_number()
        .unwrap_or_else(|| panic!("not a number: {v:?}"))
}

#[test]
fn flat_source_no_grouping() {
    let sd = saved(
        &[
            ("t.id", FieldValueType::Int32s),
            ("t.amt", FieldValueType::Number),
        ],
        &[&["1", "10"], &["2", "20"], &["3", "30"]],
    );
    let src = SavedDataSource::new(&sd);
    let ds = build_dataset(&src, &DataDefinition::default());
    assert_eq!(ds.row_count, 3);
    assert!(ds.groups.is_empty());
    assert_eq!(ds.details.len(), 3);
}

#[test]
fn record_selection_filters() {
    let sd = saved(
        &[("t.amt", FieldValueType::Number)],
        &[&["10"], &["20"], &["30"]],
    );
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.record_selection = Some(rpt_model::Formula("{t.amt} > 15".to_string()));
    let ds = build_dataset(&src, &dd);
    assert_eq!(ds.row_count, 2);
}

#[test]
fn sorting_descending() {
    let sd = saved(
        &[("t.n", FieldValueType::Number)],
        &[&["3"], &["1"], &["2"]],
    );
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.record_sorts = vec![sort("t.n", SortDirection::DescendingOrder)];
    let ds = build_dataset(&src, &dd);
    let seq: Vec<f64> = ds
        .iter_detail_rows()
        .iter()
        .map(|r| num(r.get("t.n").unwrap()))
        .collect();
    assert_eq!(seq, vec![3.0, 2.0, 1.0]);
}

#[test]
fn grouping_with_summaries() {
    let sd = saved(
        &[
            ("t.region", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[
            &["West", "10"],
            &["East", "5"],
            &["West", "20"],
            &["East", "15"],
        ],
    );
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.groups = vec![group("t.region", SortDirection::AscendingOrder)];
    dd.field_definitions = vec![summary_field("Sum_amt", SummaryOperation::Sum, "t.amt")];
    let ds = build_dataset(&src, &dd);

    assert_eq!(ds.groups.len(), 2);
    assert_eq!(ds.groups[0].key, Value::Str("East".to_string()));
    assert_eq!(num(&ds.groups[0].summaries[0].value), 20.0); // 5 + 15
    assert_eq!(ds.groups[1].key, Value::Str("West".to_string()));
    assert_eq!(num(&ds.groups[1].summaries[0].value), 30.0); // 10 + 20
    assert_eq!(num(&ds.grand_total[0].value), 50.0);
    assert_eq!(ds.iter_detail_rows().len(), 4);
}

#[test]
fn formula_field_resolves_in_context() {
    let sd = saved(&[("t.amt", FieldValueType::Number)], &[&["10"], &["100"]]);
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.record_selection = Some(rpt_model::Formula("{@Big}".to_string()));
    dd.field_definitions = vec![formula_field("Big", "{t.amt} >= 50")];
    let ds = build_dataset(&src, &dd);
    assert_eq!(ds.row_count, 1);
}

#[test]
fn count_and_max_summaries() {
    let sd = saved(
        &[("t.v", FieldValueType::Number)],
        &[&["4"], &["9"], &["2"]],
    );
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.field_definitions = vec![
        summary_field("Cnt", SummaryOperation::Count, "t.v"),
        summary_field("Mx", SummaryOperation::Maximum, "t.v"),
    ];
    let ds = build_dataset(&src, &dd);
    assert_eq!(num(&ds.grand_total[0].value), 3.0);
    assert_eq!(num(&ds.grand_total[1].value), 9.0);
}

#[test]
fn multi_key_record_sort_reproduces_deterministic_order() {
    // The pipeline's record-sort pass reproduces the engine's rowset order
    // when wired to the report's sort fields. Primary region ascending, secondary amount descending
    // — a stored order that is neither, so only a correct multi-key stable sort yields this result.
    let sd = saved(
        &[
            ("t.region", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[
            &["West", "10"],
            &["East", "5"],
            &["West", "20"],
            &["East", "15"],
            &["East", "5"],
        ],
    );
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.record_sorts = vec![
        sort("t.region", SortDirection::AscendingOrder),
        sort("t.amt", SortDirection::DescendingOrder),
    ];
    let ds = build_dataset(&src, &dd);
    let seq: Vec<(String, f64)> = ds
        .iter_detail_rows()
        .iter()
        .map(|r| {
            (
                match r.get("t.region").unwrap() {
                    Value::Str(s) => s.clone(),
                    _ => String::new(),
                },
                num(r.get("t.amt").unwrap()),
            )
        })
        .collect();
    assert_eq!(
        seq,
        vec![
            ("East".into(), 15.0),
            ("East".into(), 5.0),
            ("East".into(), 5.0),
            ("West".into(), 20.0),
            ("West".into(), 10.0),
        ]
    );
}

#[test]
fn global_variable_accumulates_across_records_with_shared_state() {
    use crystal_formula::eval::EvalContext;
    use crystal_formula::RefKind;

    // A Global running total: `Global NumberVar t; t := t + {t.amt}; t`.
    let mut dd = DataDefinition::default();
    dd.field_definitions = vec![formula_field(
        "RunTotal",
        "Global NumberVar t; t := t + {t.amt}; t",
    )];
    let formulas = compile_formulas(&dd);
    let state = SharedState::new();

    // One context per record, all sharing the report-lifetime state — the print pass shape.
    let mut seen = Vec::new();
    for a in [10.0, 20.0, 5.0] {
        let mut row = Row::default();
        row.insert("t.amt", Value::Number(a));
        let ctx = DataContext::new(&row, &formulas).with_state(&state);
        seen.push(num(&ctx.resolve(RefKind::Formula, "RunTotal").unwrap()));
    }
    // The Global persists across records → a genuine running total.
    assert_eq!(seen, vec![10.0, 30.0, 35.0]);
}

#[test]
fn without_shared_state_global_resets_each_record() {
    use crystal_formula::eval::EvalContext;
    use crystal_formula::RefKind;

    // Same formula, but no SharedState attached: the VM keeps the variable per-evaluation, so it
    // cannot accumulate — the state-less (test/default) path's behavior.
    let mut dd = DataDefinition::default();
    dd.field_definitions = vec![formula_field(
        "RunTotal",
        "Global NumberVar t; t := t + {t.amt}; t",
    )];
    let formulas = compile_formulas(&dd);

    let mut seen = Vec::new();
    for a in [10.0, 20.0, 5.0] {
        let mut row = Row::default();
        row.insert("t.amt", Value::Number(a));
        let ctx = DataContext::new(&row, &formulas);
        seen.push(num(&ctx.resolve(RefKind::Formula, "RunTotal").unwrap()));
    }
    assert_eq!(seen, vec![10.0, 20.0, 5.0]);
}

#[test]
fn per_record_cache_evaluates_formula_once() {
    use crystal_formula::eval::EvalContext;
    use crystal_formula::RefKind;

    // A Global counter that increments on every evaluation.
    let mut dd = DataDefinition::default();
    dd.field_definitions = vec![formula_field(
        "Counter",
        "Global NumberVar c; c := c + 1; c",
    )];
    let formulas = compile_formulas(&dd);
    let state = SharedState::new();
    let row = Row::default();

    // Two references within one record's context: the cache returns the same value and the Global
    // increments exactly once — side effect fires once per record, not per reference.
    let ctx = DataContext::new(&row, &formulas).with_state(&state);
    let a = num(&ctx.resolve(RefKind::Formula, "Counter").unwrap());
    let b = num(&ctx.resolve(RefKind::Formula, "Counter").unwrap());
    assert_eq!((a, b), (1.0, 1.0));

    // A fresh context (the next record) increments once more.
    let ctx2 = DataContext::new(&row, &formulas).with_state(&state);
    assert_eq!(
        num(&ctx2.resolve(RefKind::Formula, "Counter").unwrap()),
        2.0
    );
}

#[test]
fn shared_scope_persists_and_is_distinct_from_global() {
    use crystal_formula::eval::EvalContext;
    use crystal_formula::RefKind;

    // `Shared` and `Global` variables of the same name are distinct stores; both persist.
    let mut dd = DataDefinition::default();
    dd.field_definitions = vec![
        formula_field("G", "Global NumberVar v; v := v + 1; v"),
        formula_field("S", "Shared NumberVar v; v := v + 10; v"),
    ];
    let formulas = compile_formulas(&dd);
    let state = SharedState::new();
    let row = Row::default();

    let mut g = Vec::new();
    let mut s = Vec::new();
    for _ in 0..3 {
        let ctx = DataContext::new(&row, &formulas).with_state(&state);
        g.push(num(&ctx.resolve(RefKind::Formula, "G").unwrap()));
        s.push(num(&ctx.resolve(RefKind::Formula, "S").unwrap()));
    }
    assert_eq!(g, vec![1.0, 2.0, 3.0]);
    assert_eq!(s, vec![10.0, 20.0, 30.0]);
}

#[test]
fn parameters_resolve_in_data_context() {
    use crystal_formula::eval::Evaluator;
    use crystal_formula::{parse, Syntax};

    let row = Row::default();
    let formulas = FormulaRegistry::new();
    let mut params = Parameters::new();
    // Stored under the normalized key; the ref `{?DocKey@}` must find it.
    params.insert(normalize_param_name("{?DocKey@}"), Value::Number(42.0));
    let ctx = DataContext::new(&row, &formulas).with_params(&params);

    let eval = |src: &str| {
        let (ast, _) = parse(src, Syntax::Crystal);
        Evaluator::new(&ctx).eval(&ast).unwrap()
    };
    // Parameter resolves and participates in arithmetic.
    assert_eq!(eval("{?DocKey@} + 1").as_number(), Some(43.0));
    // Brace/case-insensitive match for the same parameter.
    assert_eq!(eval("{?dockey@}"), Value::Number(42.0));
    // Without params supplied, an unresolved parameter ref errors (rendering catches this as Null
    // via unwrap_or) — unchanged default behavior.
    let bare = DataContext::new(&row, &formulas);
    let (ast, _) = parse("{?DocKey@}", Syntax::Crystal);
    assert!(Evaluator::new(&bare).eval(&ast).is_err());
}

/// A `#name` running-total field: `op` of `field`, resetting per `reset`.
fn running_total_field(
    name: &str,
    op: SummaryOperation,
    field: &str,
    reset: ResetConditionType,
) -> FieldDef {
    let mut rt = RunningTotalField::default();
    rt.operation = op;
    rt.summarized_field = field.to_string();
    rt.reset = reset;
    let mut f = FieldDef::default();
    f.name = name.to_string();
    f.kind = FieldKindData::RunningTotal(rt);
    f
}

fn saved_region_amt() -> SavedData {
    saved(
        &[
            ("t.region", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[
            &["West", "10"],
            &["East", "5"],
            &["West", "20"],
            &["East", "15"],
        ],
    )
}

/// A running total that resets on group change is the per-group aggregate, keyed `#name`.
#[test]
fn running_total_reset_on_group_is_per_group_aggregate() {
    let sd = saved_region_amt();
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.groups = vec![group("t.region", SortDirection::AscendingOrder)];
    dd.field_definitions = vec![running_total_field(
        "RT",
        SummaryOperation::Sum,
        "t.amt",
        ResetConditionType::OnChangeOfGroup,
    )];
    let ds = build_dataset(&src, &dd);

    let rt = |g: &GroupInstance| num(&g.summaries.iter().find(|s| s.field == "#RT").unwrap().value);
    assert_eq!(ds.groups[0].key, Value::Str("East".into()));
    assert_eq!(rt(&ds.groups[0]), 20.0); // 5 + 15
    assert_eq!(ds.groups[1].key, Value::Str("West".into()));
    assert_eq!(rt(&ds.groups[1]), 30.0); // 10 + 20
}

/// Group selection drops the groups its formula rejects, keeping the group tree HAVING-like.
/// Here the running total `#RT` is each region's `Sum(amt)`; the selection keeps
/// only regions whose total exceeds 25.
#[test]
fn group_selection_filters_groups_by_summary() {
    let sd = saved_region_amt();
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.groups = vec![group("t.region", SortDirection::AscendingOrder)];
    dd.field_definitions = vec![running_total_field(
        "RT",
        SummaryOperation::Sum,
        "t.amt",
        ResetConditionType::OnChangeOfGroup,
    )];
    dd.group_selection = Some(rpt_model::Formula("{#RT} > 25".to_string()));
    let ds = build_dataset(&src, &dd);

    // East totals 20 (dropped), West totals 30 (kept).
    assert_eq!(ds.groups.len(), 1);
    assert_eq!(ds.groups[0].key, Value::Str("West".into()));
}

/// A group selection that references values we can't resolve group-constantly is fail-open — every
/// group is kept rather than risk dropping data.
#[test]
fn group_selection_fails_open_on_unresolvable_reference() {
    let sd = saved_region_amt();
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.groups = vec![group("t.region", SortDirection::AscendingOrder)];
    // `{t.amt}` is a detail field, not a group condition field or running total → not filtered.
    dd.group_selection = Some(rpt_model::Formula("{t.amt} > 1000000".to_string()));
    let ds = build_dataset(&src, &dd);
    assert_eq!(
        ds.groups.len(),
        2,
        "unresolvable selection keeps all groups"
    );
}

/// `Shared`-scope variables cross the main↔subreport boundary while `Global` stays per-report:
/// a child state shares the parent's `Shared` map but gets a fresh `Global` map.
#[test]
fn child_state_shares_shared_scope_isolates_global() {
    use crystal_formula::VarScope;

    let parent = SharedState::new();
    parent.set(VarScope::Shared, "s", Value::Number(1.0));
    parent.set(VarScope::Global, "g", Value::Number(1.0));

    let child = parent.child();
    // The child sees the parent's Shared value, but not its Global.
    assert_eq!(child.get(VarScope::Shared, "s"), Some(Value::Number(1.0)));
    assert_eq!(child.get(VarScope::Global, "g"), None);

    // A Shared write in the child is visible in the parent (one shared store)…
    child.set(VarScope::Shared, "s", Value::Number(9.0));
    assert_eq!(parent.get(VarScope::Shared, "s"), Some(Value::Number(9.0)));
    // …but a Global write in the child stays local to it.
    child.set(VarScope::Global, "g2", Value::Number(5.0));
    assert_eq!(parent.get(VarScope::Global, "g2"), None);
}

/// A running total with no reset accumulates across the top-level groups.
#[test]
fn running_total_no_reset_accumulates_across_groups() {
    let sd = saved_region_amt();
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.groups = vec![group("t.region", SortDirection::AscendingOrder)];
    dd.field_definitions = vec![running_total_field(
        "RT",
        SummaryOperation::Sum,
        "t.amt",
        ResetConditionType::NoCondition,
    )];
    let ds = build_dataset(&src, &dd);

    let rt = |g: &GroupInstance| num(&g.summaries.iter().find(|s| s.field == "#RT").unwrap().value);
    // Sorted East(20) then West(30) → cumulative 20, then 50.
    assert_eq!(rt(&ds.groups[0]), 20.0);
    assert_eq!(rt(&ds.groups[1]), 50.0);
}

// --- Optional diagnostics for the fail-open sites ---------------------------------------------

#[test]
fn record_selection_error_is_reported_yet_still_fail_open() {
    let sd = saved(&[("t.amt", FieldValueType::Number)], &[&["10"], &["20"]]);
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    // References a field the rows do not carry → the selection errors on every row.
    dd.record_selection = Some(rpt_model::Formula("{t.missing} > 0".to_string()));

    // Default path: fail-open silently drops every row.
    let ds = build_dataset(&src, &dd);
    assert_eq!(ds.row_count, 0);

    // With a sink: identical result, but each swallowed failure is captured (one per row).
    let sink = CollectingSink::new();
    let ds2 = build_dataset_with_diagnostics(&src, &dd, &sink);
    assert_eq!(ds2.row_count, 0);
    let diags = sink.diagnostics();
    assert_eq!(diags.len(), 2);
    assert!(diags
        .iter()
        .all(|d| d.kind == DiagnosticKind::RecordSelection));
    assert!(diags.iter().all(|d| d.source.is_none()));
    assert!(diags.iter().all(|d| !d.detail.is_empty()));
}

#[test]
fn valid_selection_reports_nothing() {
    // A clean `false` is ordinary filtering, not a failure — it must never produce a diagnostic.
    let sd = saved(&[("t.amt", FieldValueType::Number)], &[&["10"], &["20"]]);
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.record_selection = Some(rpt_model::Formula("{t.amt} > 15".to_string()));

    let sink = CollectingSink::new();
    let ds = build_dataset_with_diagnostics(&src, &dd, &sink);
    assert_eq!(ds.row_count, 1);
    assert!(sink.is_empty());
}

#[test]
fn formula_error_during_grouping_is_reported() {
    let sd = saved(&[("t.amt", FieldValueType::Number)], &[&["10"], &["20"]]);
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    // A formula that errors at eval (references an absent field).
    dd.field_definitions = vec![formula_field("Broken", "{t.missing} + 1")];
    // Group by the broken formula → grouping resolves it per row.
    dd.groups = vec![group("@Broken", SortDirection::AscendingOrder)];

    // Default path: the formula resolves to Null; both rows fall into one Null group.
    let ds = build_dataset(&src, &dd);
    assert_eq!(ds.groups.len(), 1);

    let sink = CollectingSink::new();
    let ds2 = build_dataset_with_diagnostics(&src, &dd, &sink);
    assert_eq!(ds2.groups.len(), 1);
    let diags = sink.diagnostics();
    assert!(!diags.is_empty());
    assert!(diags.iter().all(|d| d.kind == DiagnosticKind::Formula));
    assert!(diags.iter().all(|d| d.source.as_deref() == Some("Broken")));
}

#[test]
fn group_selection_non_boolean_is_reported_yet_group_kept() {
    let sd = saved(&[("t.cat", FieldValueType::Number)], &[&["1"], &["2"]]);
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.groups = vec![group("t.cat", SortDirection::AscendingOrder)];
    // References the group condition field (so it passes the group-selection safety gate) but
    // returns a number, not a boolean → fail-open keeps every group.
    dd.group_selection = Some(rpt_model::Formula("{t.cat}".to_string()));

    // Default path: both groups kept, silently.
    let ds = build_dataset(&src, &dd);
    assert_eq!(ds.groups.len(), 2);

    let sink = CollectingSink::new();
    let ds2 = build_dataset_with_diagnostics(&src, &dd, &sink);
    assert_eq!(ds2.groups.len(), 2);
    let diags = sink.diagnostics();
    assert_eq!(diags.len(), 2);
    assert!(diags
        .iter()
        .all(|d| d.kind == DiagnosticKind::GroupSelection));
    assert!(diags.iter().all(|d| d.detail.contains("non-boolean")));
}
