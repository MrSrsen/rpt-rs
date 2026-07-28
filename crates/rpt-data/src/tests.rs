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
fn record_selection_resolves_parameters() {
    // A parameter-filtered selection: without the parameter values every row's selection formula
    // errors on the unresolved `{?Customer}`/`{?MinAmt}` and is dropped fail-open (empty dataset);
    // with them, the parameters select the matching rows.
    let sd = saved(
        &[
            ("t.cust", FieldValueType::Int32s),
            ("t.amt", FieldValueType::Number),
        ],
        &[&["374", "50"], &["374", "5"], &["999", "50"]],
    );
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.record_selection = Some(rpt_model::Formula(
        "({t.cust} IN {?Customer}) AND ({t.amt} >= {?MinAmt})".to_string(),
    ));

    // No parameters supplied: the unresolved references drop every row.
    assert_eq!(build_dataset(&src, &dd).row_count, 0);

    // Parameters supplied: only cust 374 with amt >= 10 survives (the first row).
    let mut params = Parameters::new();
    params.insert(
        "customer".to_string(),
        Value::Array(vec![Value::Number(374.0)]),
    );
    params.insert("minamt".to_string(), Value::Number(10.0));
    let ds = build_dataset_with_params(&src, &dd, &params);
    assert_eq!(ds.row_count, 1);
    assert_eq!(num(ds.details[0].get("t.amt").unwrap()), 50.0);
}

#[test]
fn build_dataset_with_link_filter_and_param() {
    // The per-instance subreport path: a parent link value reaches the subreport either as a merged
    // parameter (its selection formula filters on `{?param}`) or as a structural `FieldFilter`.
    let sd = saved(
        &[
            ("invoice.customer_id", FieldValueType::Int32s),
            ("invoice.total", FieldValueType::Number),
        ],
        &[&["1", "10"], &["1", "40"], &["2", "99"]],
    );
    let src = SavedDataSource::new(&sd);

    // Parameter-routed link: selection `{invoice.customer_id} = {?p}` keeps only customer 1's rows.
    let mut dd = DataDefinition::default();
    dd.record_selection = Some(rpt_model::Formula(
        "{invoice.customer_id} = {?p}".to_string(),
    ));
    let mut params = Parameters::new();
    params.insert("p".to_string(), Value::Number(1.0));
    let ds = build_dataset_with(&src, &dd, &params, &[]);
    assert_eq!(ds.row_count, 2);

    // Direct field link: a structural equality filter, no selection formula, keeps only customer 2.
    let filters = [FieldFilter {
        field: "invoice.customer_id".to_string(),
        value: Value::Number(2.0),
    }];
    let ds = build_dataset_with(
        &src,
        &DataDefinition::default(),
        &Parameters::new(),
        &filters,
    );
    assert_eq!(ds.row_count, 1);
    assert_eq!(num(ds.details[0].get("invoice.total").unwrap()), 99.0);

    // Filter and selection AND together: filter to customer 1, then keep total >= 40 → one row.
    let mut dd = DataDefinition::default();
    dd.record_selection = Some(rpt_model::Formula("{invoice.total} >= 40".to_string()));
    let filters = [FieldFilter {
        field: "invoice.customer_id".to_string(),
        value: Value::Number(1.0),
    }];
    let ds = build_dataset_with(&src, &dd, &Parameters::new(), &filters);
    assert_eq!(ds.row_count, 1);
    assert_eq!(num(ds.details[0].get("invoice.total").unwrap()), 40.0);
}

#[test]
fn null_selection_formula_excludes_record() {
    // A selection formula that evaluates to Null (rather than a clean boolean `true`) excludes the
    // record — the engine keeps a record only when its selection is true. An empty Number cell reads
    // as Null; `IIf(IsNull({t.x}), {t.x}, True)` then returns Null for that row (and `True` for a
    // valued row), so only the valued row survives.
    let sd = saved(&[("t.x", FieldValueType::Number)], &[&[""], &["7"]]);
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.record_selection = Some(rpt_model::Formula(
        "IIf(IsNull({t.x}), {t.x}, True)".to_string(),
    ));
    let ds = build_dataset(&src, &dd);
    assert_eq!(ds.row_count, 1);
    assert_eq!(num(ds.details[0].get("t.x").unwrap()), 7.0);
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
fn summary_over_a_formula_field_aggregates_the_evaluated_value() {
    // A summary whose summarized field is a `{@formula}` must aggregate the formula's per-row value,
    // not read a (non-existent) raw column — otherwise every group/grand total is Null.
    let sd = saved(
        &[
            ("t.region", FieldValueType::String),
            ("t.qty", FieldValueType::Number),
            ("t.price", FieldValueType::Number),
        ],
        &[
            &["West", "2", "10"],
            &["East", "1", "5"],
            &["West", "3", "10"],
        ],
    );
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.groups = vec![group("t.region", SortDirection::AscendingOrder)];
    dd.field_definitions = vec![
        formula_field("LineTotal", "{t.qty} * {t.price}"),
        // Sum over the formula field (its summarized field is `@LineTotal`).
        summary_field("Sum_line", SummaryOperation::Sum, "@LineTotal"),
        // A raw-column sum alongside it still works.
        summary_field("Sum_qty", SummaryOperation::Sum, "t.qty"),
    ];
    let ds = build_dataset(&src, &dd);

    // East: 1*5 = 5.  West: 2*10 + 3*10 = 50.  Grand: 55.
    assert_eq!(ds.groups[0].key, Value::Str("East".to_string()));
    assert_eq!(num(&ds.groups[0].summaries[0].value), 5.0);
    assert_eq!(ds.groups[1].key, Value::Str("West".to_string()));
    assert_eq!(num(&ds.groups[1].summaries[0].value), 50.0);
    assert_eq!(num(&ds.grand_total[0].value), 55.0);
    // The parallel raw-field summary is unaffected: qty 1 / (2+3) / grand 6.
    assert_eq!(num(&ds.groups[0].summaries[1].value), 1.0);
    assert_eq!(num(&ds.groups[1].summaries[1].value), 5.0);
    assert_eq!(num(&ds.grand_total[1].value), 6.0);
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
fn multi_key_sort_is_stable_and_matches_field_order() {
    // Two sort fields (cat asc, sub desc) plus a tag that ties on both keys: the single-pass
    // composite sort must order by cat then sub, and preserve input order among rows equal on both
    // keys (stability) — the guarantee the prior per-field reverse-order passes gave.
    let sd = saved(
        &[
            ("t.cat", FieldValueType::String),
            ("t.sub", FieldValueType::Number),
            ("t.tag", FieldValueType::String),
        ],
        &[
            &["B", "1", "b1"],
            &["A", "2", "a2first"],
            &["A", "2", "a2second"],
            &["A", "1", "a1"],
            &["B", "1", "b1again"],
        ],
    );
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.record_sorts = vec![
        sort("t.cat", SortDirection::AscendingOrder),
        sort("t.sub", SortDirection::DescendingOrder),
    ];
    let ds = build_dataset(&src, &dd);
    let seq: Vec<String> = ds
        .iter_detail_rows()
        .iter()
        .map(|r| match r.get("t.tag").unwrap() {
            Value::Str(s) => s.clone(),
            _ => String::new(),
        })
        .collect();
    // A/sub2 before A/sub1; the two A/2 rows keep input order; B rows last, both sub1, input order.
    assert_eq!(seq, vec!["a2first", "a2second", "a1", "b1", "b1again"]);
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

/// A group-scoped summary function in the group-selection formula filters at the level its group
/// argument names, not the innermost leaf. Here two levels (category → carrier) and a per-carrier
/// `DistinctCount({shipment}, {carrier}) > 1`: a carrier with a single distinct shipment is dropped,
/// and a category left with no surviving carrier is pruned.
#[test]
fn group_selection_by_group_scoped_summary_filters_at_named_level() {
    let sd = saved(
        &[
            ("t.cat", FieldValueType::String),
            ("t.carrier", FieldValueType::String),
            ("t.shipment", FieldValueType::Int32s),
        ],
        &[
            &["A", "X", "1"],
            &["A", "X", "2"], // carrier X: 2 distinct shipments → kept
            &["A", "Y", "3"],
            &["A", "Y", "3"], // carrier Y: 1 distinct shipment → dropped
            &["B", "Z", "4"], // carrier Z: 1 distinct shipment → dropped → category B pruned
        ],
    );
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.groups = vec![
        group("t.cat", SortDirection::AscendingOrder),
        group("t.carrier", SortDirection::AscendingOrder),
    ];
    dd.field_definitions = vec![summary_field(
        "dc",
        SummaryOperation::DistinctCount,
        "t.shipment",
    )];
    dd.group_selection = Some(rpt_model::Formula(
        "DistinctCount({t.shipment}, {t.carrier}) > 1".to_string(),
    ));
    let ds = build_dataset(&src, &dd);

    // Only category A survives, with only carrier X under it.
    assert_eq!(ds.groups.len(), 1);
    assert_eq!(ds.groups[0].key, Value::Str("A".into()));
    assert_eq!(ds.groups[0].subgroups.len(), 1);
    assert_eq!(ds.groups[0].subgroups[0].key, Value::Str("X".into()));
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

    // With a sink: identical result, but each swallowed failure is captured (one per row) plus a
    // summary, because "every row failed" is a different problem from "one row failed" and the per-row
    // diagnostics alone do not say which it was.
    let sink = CollectingSink::new();
    let ds2 = build_dataset_with_diagnostics(&src, &dd, &sink);
    assert_eq!(ds2.row_count, 0);
    let diags = sink.diagnostics();
    assert_eq!(diags.len(), 3, "2 per-row failures + 1 summary");
    assert!(diags
        .iter()
        .all(|d| d.kind == DiagnosticKind::RecordSelection));
    assert!(diags.iter().all(|d| d.source.is_none()));
    assert!(diags.iter().all(|d| !d.detail.is_empty()));

    // The per-row ones carry their row index, in order; the summary carries none (it is about the set).
    let per_row: Vec<_> = diags.iter().filter(|d| d.record_index.is_some()).collect();
    assert_eq!(per_row.len(), 2);
    assert_eq!(per_row[0].record_index, Some(0));
    assert_eq!(per_row[1].record_index, Some(1));

    // The summary states the counts and points at the failure, so an empty report explains itself.
    let summary = diags
        .iter()
        .find(|d| d.record_index.is_none())
        .expect("a summary diagnostic");
    assert!(
        summary.detail.contains("0 of 2 row(s) kept"),
        "{}",
        summary.detail
    );
    assert!(summary.detail.contains("FAILED on 2"), "{}", summary.detail);
}

/// A selection that cleanly excludes every row is not a failure — but it is still the answer to "why
/// is my report empty?", so it is reported, distinguishably.
#[test]
fn a_selection_that_cleanly_excludes_every_row_is_reported_as_such() {
    let sd = saved(&[("t.amt", FieldValueType::Number)], &[&["10"], &["20"]]);
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.record_selection = Some(rpt_model::Formula("{t.amt} > 999".to_string()));

    let sink = CollectingSink::new();
    let ds = build_dataset_with_diagnostics(&src, &dd, &sink);
    assert_eq!(ds.row_count, 0);
    let diags = sink.diagnostics();
    // Exactly one: the summary. No per-row diagnostics, because no row *failed*.
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].kind, DiagnosticKind::AllRowsExcluded);
    assert!(
        diags[0].detail.contains("excluded every row"),
        "{}",
        diags[0].detail
    );
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

// ---- formula null treatment ----

#[test]
fn formula_null_treatment_controls_null_field_default() {
    use crystal_formula::eval::EvalContext;
    use crystal_formula::RefKind;

    // A null number field read by two identical formulas that differ only in null-treatment.
    let mut amt = FieldDef::default();
    amt.name = "t.amt".to_string();
    amt.value_type = FieldValueType::Number;

    let except = formula_field("Except", "{t.amt} + 1");
    let mut deflt = formula_field("Deflt", "{t.amt} + 1");
    if let FieldKindData::Formula(ff) = &mut deflt.kind {
        ff.null_treatment = rpt_model::FormulaNullTreatment::DefaultValue;
    }

    let mut dd = DataDefinition::default();
    dd.field_definitions = vec![amt, except, deflt];
    let formulas = compile_formulas(&dd);

    let mut row = Row::default();
    row.insert("t.amt", Value::Null);
    let ctx = DataContext::new(&row, &formulas);

    // Exception (engine default): the null propagates, so the whole formula is null.
    assert_eq!(ctx.resolve(RefKind::Formula, "Except"), Some(Value::Null));
    // DefaultValue: the null number becomes 0, so the formula yields 1.
    assert_eq!(
        ctx.resolve(RefKind::Formula, "Deflt"),
        Some(Value::Number(1.0))
    );
}

// ---- Top N group sort DiscardOthers ----

fn topn_group(field: &str, summed: &str, n: u16, discard: bool) -> Group {
    let mut g = group(field, SortDirection::TopNOrder);
    g.sort.field = format!("Sum ({{{summed}}}, {{{field}}})");
    let mut topn = rpt_model::TopBottomNSort::default();
    topn.number_of_groups = n;
    topn.discard_others = discard;
    topn.not_in_topn_name = "Others".to_string();
    g.sort.topn = Some(topn);
    g
}

fn topn_dataset(discard: bool) -> Dataset {
    // Three groups whose sums rank East(50) > West(30) > North(9): Top 2 keeps East and West.
    let sd = saved(
        &[
            ("t.region", FieldValueType::String),
            ("t.amt", FieldValueType::Number),
        ],
        &[
            &["West", "10"],
            &["East", "20"],
            &["West", "20"],
            &["East", "30"],
            &["North", "9"],
        ],
    );
    let src = SavedDataSource::new(&sd);
    let mut dd = DataDefinition::default();
    dd.groups = vec![topn_group("t.region", "t.amt", 2, discard)];
    dd.field_definitions = vec![summary_field("Sum_amt", SummaryOperation::Sum, "t.amt")];
    build_dataset(&src, &dd)
}

#[test]
fn topn_group_sort_discards_others() {
    let ds = topn_dataset(true);
    // Only the top 2 groups survive, ranked by descending sum; the "North" group is dropped.
    assert_eq!(ds.groups.len(), 2);
    assert_eq!(ds.groups[0].key, Value::Str("East".to_string()));
    assert_eq!(num(&ds.groups[0].summaries[0].value), 50.0);
    assert_eq!(ds.groups[1].key, Value::Str("West".to_string()));
    assert_eq!(num(&ds.groups[1].summaries[0].value), 30.0);
    // Discarded rows are gone from the printed detail set.
    assert_eq!(ds.iter_detail_rows().len(), 4);
}

#[test]
fn topn_group_sort_collapses_others_when_not_discarded() {
    let ds = topn_dataset(false);
    // Top 2 plus a trailing "Others" group collapsing the remaining group(s).
    assert_eq!(ds.groups.len(), 3);
    assert_eq!(ds.groups[0].key, Value::Str("East".to_string()));
    assert_eq!(ds.groups[1].key, Value::Str("West".to_string()));
    let others = &ds.groups[2];
    assert_eq!(others.key, Value::Str("Others".to_string()));
    assert_eq!(num(&others.summaries[0].value), 9.0);
    // The Others group carries the collapsed detail rows (North's single row).
    assert_eq!(others.details.len(), 1);
}

/// The date/time specials (`CurrentDate`/`Today`/`CurrentDateTime`/`CurrentTime`) resolve from the
/// registry's injected as-of instant — so a formula reading them evaluates against one fixed value.
#[test]
fn datetime_specials_resolve_from_injected_as_of() {
    use crystal_formula::eval::{Date, EvalContext, Time};
    use crystal_formula::RefKind;

    let dt = DateTimeSpecials::new(Date::new(2021, 6, 15), Time::new(12, 30, 45));
    let mut dd = DataDefinition::default();
    dd.field_definitions = vec![
        formula_field("Cd", "CurrentDate"),
        formula_field("Td", "Today"),
        formula_field("Ct", "CurrentTime"),
        formula_field("Cdt", "CurrentDateTime"),
    ];
    let formulas = compile_formulas(&dd).with_datetime(dt);
    let row = Row::default();
    let ctx = DataContext::new(&row, &formulas);

    assert_eq!(
        ctx.resolve(RefKind::Formula, "Cd"),
        Some(Value::Date(Date::new(2021, 6, 15)))
    );
    // `Today` is the engine alias for `CurrentDate` — same value.
    assert_eq!(
        ctx.resolve(RefKind::Formula, "Td"),
        Some(Value::Date(Date::new(2021, 6, 15)))
    );
    assert_eq!(
        ctx.resolve(RefKind::Formula, "Ct"),
        Some(Value::Time(Time::new(12, 30, 45)))
    );
    assert_eq!(
        ctx.resolve(RefKind::Formula, "Cdt"),
        Some(Value::DateTime(
            Date::new(2021, 6, 15),
            Time::new(12, 30, 45)
        ))
    );
}

/// The aging-bucket shape from the corpus: `DateDiff("d", {due}, CurrentDate)` buckets a row by its
/// age relative to the injected as-of date — previously null because `CurrentDate` was never supplied.
#[test]
fn aging_bucket_uses_injected_current_date() {
    use crystal_formula::eval::{Date, EvalContext, Time};
    use crystal_formula::RefKind;

    let as_of = Date::new(2021, 6, 15);
    let dt = DateTimeSpecials::new(as_of, Time::new(0, 0, 0));
    let mut dd = DataDefinition::default();
    dd.field_definitions = vec![formula_field(
        "AgingBucket",
        "Select DateDiff(\"d\", {t.due}, CurrentDate) \
         Case 0 To 30 : \"0-30\" \
         Case 31 To 60 : \"31-60\" \
         Case 61 To 90 : \"61-90\" \
         Default : \"90+\"",
    )];
    let formulas = compile_formulas(&dd).with_datetime(dt);

    let mut recent = Row::default();
    recent.insert("t.due", Value::Date(Date::new(2021, 6, 1))); // 14 days old
    let ctx = DataContext::new(&recent, &formulas);
    assert_eq!(
        ctx.resolve(RefKind::Formula, "AgingBucket"),
        Some(Value::Str("0-30".to_string()))
    );

    let mut old = Row::default();
    old.insert("t.due", Value::Date(Date::new(2021, 3, 1))); // > 90 days old
    let ctx = DataContext::new(&old, &formulas);
    assert_eq!(
        ctx.resolve(RefKind::Formula, "AgingBucket"),
        Some(Value::Str("90+".to_string()))
    );
}

/// With no as-of injected (the default registry), the date/time specials stay unresolved — matching
/// the offline/inspection paths that never supply a render instant.
#[test]
fn datetime_specials_absent_without_as_of() {
    use crystal_formula::eval::EvalContext;

    let formulas = compile_formulas(&DataDefinition::default());
    let row = Row::default();
    let ctx = DataContext::new(&row, &formulas);
    assert_eq!(ctx.special("currentdate"), None);
}

/// `from_unix_seconds` splits an epoch instant into its UTC calendar date and time-of-day.
#[test]
fn datetime_specials_from_unix_epoch() {
    use crystal_formula::eval::{Date, Time};

    let dt = DateTimeSpecials::from_unix_seconds(0);
    assert_eq!(
        dt.resolve("currentdate"),
        Some(Value::Date(Date::new(1970, 1, 1)))
    );
    assert_eq!(
        dt.resolve("currenttime"),
        Some(Value::Time(Time::new(0, 0, 0)))
    );

    // 2021-06-15T12:30:45Z = 1_623_760_245 seconds since the epoch.
    let dt = DateTimeSpecials::from_unix_seconds(1_623_760_245);
    assert_eq!(
        dt.resolve("currentdate"),
        Some(Value::Date(Date::new(2021, 6, 15)))
    );
    assert_eq!(
        dt.resolve("currenttime"),
        Some(Value::Time(Time::new(12, 30, 45)))
    );
}
