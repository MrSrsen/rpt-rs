//! The record pipeline: selection → sort → grouping → summaries, producing the [`Dataset`]
//! instance tree the layout engine iterates.

use crate::context::{DataContext, FormulaRegistry};
use crate::diagnostics::{DiagnosticKind, DiagnosticSink, EvalDiagnostic};
use crate::source::{Row, RowSource};
use crate::value_order::{compare_values, value_key};
use crate::{Dataset, GroupInstance, Summary, SummaryAccumulator};
use crystal_formula::eval::{vm, Date, Time, Value};
use crystal_formula::token::short_name;
use crystal_formula::{parse, Syntax};
use rpt_model::{
    DataDefinition, FieldKindData, Group, ResetConditionType, SortDirection, SummaryOperation,
};
use std::cmp::Ordering;

/// Build the [`Dataset`] for a report from a row source and its data definition.
///
/// The pipeline is **fail-open**: a formula or selection that errors is swallowed. Use
/// [`build_dataset_with_diagnostics`] to surface those swallowed failures.
pub fn build_dataset(source: &dyn RowSource, data_def: &DataDefinition) -> Dataset {
    build_dataset_inner(source, data_def, None)
}

/// Like [`build_dataset`], but reports every swallowed formula/selection failure to `sink` before
/// applying the fail-open fallback (dropping a row, keeping a group, resolving a `{@formula}` to
/// `Null`). The resulting [`Dataset`] is identical to [`build_dataset`]'s — the sink only observes.
pub fn build_dataset_with_diagnostics(
    source: &dyn RowSource,
    data_def: &DataDefinition,
    sink: &dyn DiagnosticSink,
) -> Dataset {
    build_dataset_inner(source, data_def, Some(sink))
}

fn build_dataset_inner(
    source: &dyn RowSource,
    data_def: &DataDefinition,
    sink: Option<&dyn DiagnosticSink>,
) -> Dataset {
    let formulas = compile_formulas(data_def);
    let mut rows = source.rows();

    // 1. Record selection — keep rows whose selection formula evaluates true (absent = keep all).
    if let Some(sel) = &data_def.record_selection {
        if !sel.0.trim().is_empty() {
            let (ast, _) = parse(&sel.0, Syntax::Crystal);
            let chunk = vm::compile(&ast);
            rows.retain(|row| {
                let mut ctx = DataContext::new(row, &formulas);
                if let Some(sink) = sink {
                    ctx = ctx.with_diagnostics(sink);
                }
                match vm::run(&chunk, &ctx) {
                    Ok(Value::Bool(true)) => true,
                    // A clean `false` is ordinary filtering, not a failure — never a diagnostic.
                    Ok(Value::Bool(false)) => false,
                    // An error or a non-boolean result drops the row (fail-open); report it first.
                    other => {
                        if let Some(sink) = sink {
                            sink.report(EvalDiagnostic {
                                kind: DiagnosticKind::RecordSelection,
                                detail: selection_detail(&other),
                                source: None,
                            });
                        }
                        false
                    }
                }
            });
        }
    }

    // Stamp read-order index (source order after selection, before sort) so the render pass can map
    // a printed record back to its read-order slot for evaluation-time scheduling.
    for (i, row) in rows.iter_mut().enumerate() {
        row.set_read_index(i as u64);
    }

    // 2. Record sort — stable sort by each record-sort field in order.
    for sort in data_def.record_sorts.iter().rev() {
        let field = sort.field.clone();
        let dir = sort.direction;
        rows.sort_by(|a, b| order_rows(a, b, &field, dir));
    }

    // 3. Summaries to compute at each level (declared summary fields).
    let summary_defs = collect_summaries(data_def);

    // 4. Grouping — nest by each Group in definition order; deepest level holds the detail rows.
    let groups = &data_def.groups;
    let (tree, grand) = if groups.is_empty() {
        (Vec::new(), summarize(&rows, &summary_defs))
    } else {
        let mut tree = build_groups(&rows, groups, 0, &summary_defs, &formulas, sink);
        // No-reset running totals accumulate across the top-level groups.
        apply_cumulative(&mut tree, &summary_defs);
        // Group selection (HAVING-like): drop groups the group-selection formula rejects.
        apply_group_selection(&mut tree, data_def, sink);
        let grand = summarize(&rows, &summary_defs);
        (tree, grand)
    };

    Dataset {
        columns: source.columns().to_vec(),
        row_count: rows.len(),
        details: if groups.is_empty() { rows } else { Vec::new() },
        groups: tree,
        grand_total: grand,
        params: Default::default(),
    }
}

/// Describe why a selection formula's result was not a clean boolean, for a diagnostic: an
/// evaluation error, or a non-boolean value where a boolean was expected.
fn selection_detail(result: &Result<Value, crystal_formula::eval::EvalError>) -> String {
    match result {
        Ok(value) => format!("selection formula returned a non-boolean value: {value:?}"),
        Err(err) => err.to_string(),
    }
}

/// Parse every formula field's body once, keyed by lowercase name.
pub fn compile_formulas(data_def: &DataDefinition) -> FormulaRegistry {
    let mut reg = FormulaRegistry::new();
    for f in &data_def.field_definitions {
        if let FieldKindData::Formula(ff) = &f.kind {
            let syntax = match ff.syntax {
                rpt_model::FormulaSyntax::Basic => Syntax::Basic,
                _ => Syntax::Crystal,
            };
            let (ast, _) = parse(&ff.text.0, syntax);
            reg.insert(f.name.to_lowercase(), vm::compile(&ast));
        }
    }
    reg
}

/// The summary + running-total fields to compute at each level. A declared **summary** is keyed by
/// its summarized field; a **running total** (`#name`) is keyed by `#name` (how `{#name}` references
/// it) and aggregated over each group's rows — correct for a running total that resets on group
/// change (the common case, and what group charts plot). A running total with no reset condition
/// (`NoCondition`) accumulates across the top-level groups instead (see [`apply_cumulative`]).
fn collect_summaries(data_def: &DataDefinition) -> Vec<SummaryDef> {
    let mut defs = Vec::new();
    for f in &data_def.field_definitions {
        match &f.kind {
            FieldKindData::Summary(s) => defs.push(SummaryDef {
                operation: s.operation,
                field: s.summarized_field.clone(),
                key: s.summarized_field.clone(),
                cumulative: false,
                param: s.operation_parameter,
            }),
            FieldKindData::RunningTotal(rt) => defs.push(SummaryDef {
                operation: rt.operation,
                field: rt.summarized_field.clone(),
                key: format!("#{}", f.name),
                cumulative: rt.reset == ResetConditionType::NoCondition,
                param: rt.operation_parameter,
            }),
            _ => {}
        }
    }
    defs
}

#[derive(Clone)]
struct SummaryDef {
    operation: SummaryOperation,
    /// The field aggregated over (the summarized field).
    field: String,
    /// The name the resulting [`Summary`] is keyed by (the summarized field, or `#name` for a
    /// running total — so a `{#name}` reference / a chart binding resolves it).
    key: String,
    /// A running total with no reset: accumulate across the top-level groups (post-pass) rather than
    /// using the per-group aggregate.
    cumulative: bool,
    /// `ISummaryField.SummaryFieldOperationParameter`: the N argument for the parameterized ops —
    /// the percentile (`Percentile`), the rank N (`NthLargest`/`NthSmallest`/`NthMostFrequent`).
    /// Zero for the ops that take no parameter.
    param: i32,
}

/// Recursively group `rows` by `groups[level..]`, computing summaries at each level.
fn build_groups(
    rows: &[Row],
    groups: &[Group],
    level: usize,
    summaries: &[SummaryDef],
    formulas: &FormulaRegistry,
    sink: Option<&dyn DiagnosticSink>,
) -> Vec<GroupInstance> {
    let Some(group) = groups.get(level) else {
        return Vec::new();
    };
    // Partition rows by the group's condition-field value, preserving first-seen order.
    let mut order: Vec<String> = Vec::new();
    let mut buckets: std::collections::HashMap<String, (Value, Vec<Row>)> =
        std::collections::HashMap::new();
    for row in rows {
        let key_val = date_bucket(
            group_key(row, &group.condition_field, formulas, sink),
            group.date_condition.as_deref(),
        );
        let key_str = value_key(&key_val);
        buckets
            .entry(key_str.clone())
            .or_insert_with(|| {
                order.push(key_str.clone());
                (key_val, Vec::new())
            })
            .1
            .push(row.clone());
    }

    // Sort the group instances by the group's sort direction (on the key).
    order.sort_by(|a, b| {
        let ord = compare_values(&buckets[a].0, &buckets[b].0);
        match group.sort.direction {
            SortDirection::DescendingOrder => ord.reverse(),
            _ => ord,
        }
    });

    order
        .into_iter()
        .map(|key_str| {
            let (key, bucket) = buckets.remove(&key_str).unwrap();
            let subgroups = build_groups(&bucket, groups, level + 1, summaries, formulas, sink);
            let group_summaries = summarize(&bucket, summaries);
            // A leaf group owns its rows outright — move the bucket in rather than clone it.
            let details = if subgroups.is_empty() {
                bucket
            } else {
                Vec::new()
            };
            GroupInstance {
                level,
                condition_field: group.condition_field.clone(),
                key,
                summaries: group_summaries,
                subgroups,
                details,
            }
        })
        .collect()
}

/// Apply the `group_selection` formula as a HAVING-like filter on the group tree:
/// evaluate it per leaf group against that group's summaries and drop the groups it rejects, pruning
/// ancestors that become empty. **Fail-open** — the grammar is untested against a corpus, so we only
/// filter when the selection references values we resolve reliably at the group level (running-total
/// `{#name}` summaries and group condition fields); any other reference, or a non-boolean / erroring
/// result, keeps the group. A running total or field summary can never wrongly drop data this way.
fn apply_group_selection(
    tree: &mut Vec<GroupInstance>,
    data_def: &DataDefinition,
    sink: Option<&dyn DiagnosticSink>,
) {
    let Some(sel) = &data_def.group_selection else {
        return;
    };
    let body = sel.0.trim();
    if body.is_empty() {
        return;
    }
    // Only filter when every reference is a running total or a group condition field — the values a
    // group instance can resolve group-constantly. Otherwise fail open (keep every group).
    use crystal_formula::{references, RefKind};
    let cond_fields: std::collections::HashSet<String> = data_def
        .groups
        .iter()
        .map(|g| short_name(&g.condition_field))
        .collect();
    let refs: Vec<_> = references(body).collect();
    let safe = !refs.is_empty()
        && refs.iter().all(|r| match r.kind {
            RefKind::RunningTotal => true,
            RefKind::Field => cond_fields.contains(&short_name(&r.name)),
            _ => false,
        });
    if !safe {
        return;
    }
    let (ast, _) = parse(body, Syntax::Crystal);
    let chunk = vm::compile(&ast);
    filter_group_tree(tree, &chunk, sink);
}

/// Recursively retain groups the selection keeps: a leaf group is dropped only when the selection
/// cleanly evaluates to `false`; a parent is dropped when all its subgroups were dropped.
fn filter_group_tree(
    groups: &mut Vec<GroupInstance>,
    chunk: &vm::Chunk,
    sink: Option<&dyn DiagnosticSink>,
) {
    groups.retain_mut(|g| {
        if g.subgroups.is_empty() {
            keep_group(g, chunk, sink)
        } else {
            filter_group_tree(&mut g.subgroups, chunk, sink);
            !g.subgroups.is_empty()
        }
    });
}

/// Whether a leaf group survives the group-selection formula. Keeps the group on anything but a clean
/// `false` (fail-open); an error or non-boolean result keeps the group but is reported to `sink`.
fn keep_group(g: &GroupInstance, chunk: &vm::Chunk, sink: Option<&dyn DiagnosticSink>) -> bool {
    let ctx = GroupFilterContext {
        row: g.details.first(),
        summaries: &g.summaries,
    };
    match vm::run(chunk, &ctx) {
        // A clean `false` drops the group (ordinary HAVING filtering); a clean `true` keeps it.
        Ok(Value::Bool(false)) => false,
        Ok(Value::Bool(true)) => true,
        // An error or a non-boolean result keeps the group (fail-open); report it first.
        other => {
            if let Some(sink) = sink {
                sink.report(EvalDiagnostic {
                    kind: DiagnosticKind::GroupSelection,
                    detail: selection_detail(&other),
                    source: None,
                });
            }
            true
        }
    }
}

/// A minimal [`EvalContext`] for evaluating a group-selection formula against one group: `{field}`
/// resolves group-constantly from the group's first row (only group condition fields are permitted,
/// so this is representative), and `{#name}` resolves from the group's computed `#name` summary.
struct GroupFilterContext<'a> {
    row: Option<&'a Row>,
    summaries: &'a [Summary],
}

impl crystal_formula::eval::EvalContext for GroupFilterContext<'_> {
    fn resolve(&self, kind: crystal_formula::RefKind, name: &str) -> Option<Value> {
        use crystal_formula::RefKind;
        match kind {
            RefKind::Field => self.row.and_then(|r| r.get(name).cloned()),
            RefKind::RunningTotal => {
                let want = name.trim_start_matches('#');
                self.summaries
                    .iter()
                    .find(|s| s.field.trim_start_matches('#').eq_ignore_ascii_case(want))
                    .map(|s| s.value.clone())
            }
            _ => None,
        }
    }
}

/// Collapse a date/datetime group key into its period bucket per the group's `date_condition`
/// (`"daily"`/`"weekly"`/`"monthly"`), so a date group partitions rows *by period* rather than by the
/// raw timestamp — e.g. a monthly group on a `DateTime` field puts every row of a calendar month in
/// one bucket instead of one bucket per distinct timestamp. The bucket is the period's start date
/// (the day itself, the week-start, or the first of the month), which also becomes the group's
/// `GroupName` value. A non-date key or an absent/unknown condition passes through unchanged.
pub fn date_bucket(val: Value, condition: Option<&str>) -> Value {
    let Some(cond) = condition else { return val };
    // Time-of-day periods bucket the time component, keeping the date (a `DateTime` stays a
    // `DateTime`; a bare `Time` stays a `Time`).
    if let Some(time) = time_of(&val) {
        if let Some(bucket) = time_bucket(time, cond) {
            return match val {
                Value::DateTime(d, _) => Value::DateTime(d, bucket),
                _ => Value::Time(bucket),
            };
        }
    }
    // Calendar periods bucket the date component (a DateTime collapses to its bucketed day).
    let date = match &val {
        Value::Date(d) => *d,
        Value::DateTime(d, _) => *d,
        _ => return val,
    };
    match date_period_bucket(date, cond) {
        Some(bucket) => Value::Date(bucket),
        None => val,
    }
}

/// The start date of the calendar period `cond` containing `date` — also the group's `GroupName`
/// value. `None` for a non-calendar (time-of-day) or unknown period.
fn date_period_bucket(date: Date, cond: &str) -> Option<Date> {
    let bucket = match cond {
        // One bucket per calendar day.
        "daily" => date,
        // Week-start. `day_of_week` is 1 = Sunday, Crystal's default first day of week; a
        // locale-specific first day of week is not modelled.
        "weekly" => week_start(date),
        // A two-week period, aligned on week boundaries: fortnights align to even week-indices
        // from the civil epoch (an arbitrary but stable anchor — biweekly grouping has no
        // canonical starting date).
        "biweekly" => {
            let ws = week_start(date);
            let even_week = ws.to_days().div_euclid(7).rem_euclid(2);
            Date::from_days(ws.to_days() - even_week * 7)
        }
        // Two buckets per month: the 1st (days 1–15) and the 16th (days 16–end).
        "semimonthly" => Date::new(date.year, date.month, if date.day <= 15 { 1 } else { 16 }),
        // First of the month.
        "monthly" => Date::new(date.year, date.month, 1),
        // First day of the calendar quarter.
        "quarterly" => Date::new(date.year, (date.month - 1) / 3 * 3 + 1, 1),
        // First day of the half-year (Jan 1 or Jul 1).
        "semiannually" => Date::new(date.year, if date.month <= 6 { 1 } else { 7 }, 1),
        // Jan 1 of the year.
        "annually" => Date::new(date.year, 1, 1),
        _ => return None,
    };
    Some(bucket)
}

/// The start of `date`'s week (Sunday, matching Crystal's default first day of week).
fn week_start(date: Date) -> Date {
    Date::from_days(date.to_days() - i64::from(date.day_of_week() - 1))
}

/// The start time of the time-of-day period `cond` containing `time`. `None` for a non-time period.
fn time_bucket(time: Time, cond: &str) -> Option<Time> {
    let bucket = match cond {
        "bysecond" => time,
        "byminute" => Time::new(time.hour, time.minute, 0),
        "byhour" => Time::new(time.hour, 0, 0),
        // Two buckets per day: AM (before noon → 00:00) and PM (noon on → 12:00).
        "byampm" => Time::new(if time.hour < 12 { 0 } else { 12 }, 0, 0),
        _ => return None,
    };
    Some(bucket)
}

/// The time component of a `Time` or `DateTime` value, else `None`.
fn time_of(val: &Value) -> Option<Time> {
    match val {
        Value::Time(t) => Some(*t),
        Value::DateTime(_, t) => Some(*t),
        _ => None,
    }
}

/// The value a row groups by (a `{@formula}` condition field resolves through the registry).
fn group_key(
    row: &Row,
    field: &str,
    formulas: &FormulaRegistry,
    sink: Option<&dyn DiagnosticSink>,
) -> Value {
    if let Some(name) = field.strip_prefix('@') {
        let mut ctx = DataContext::new(row, formulas);
        if let Some(sink) = sink {
            ctx = ctx.with_diagnostics(sink);
        }
        return ctx_formula(&ctx, name);
    }
    row.get(field).cloned().unwrap_or(Value::Null)
}

fn ctx_formula(ctx: &DataContext, name: &str) -> Value {
    use crystal_formula::eval::EvalContext;
    ctx.resolve(crystal_formula::RefKind::Formula, name)
        .unwrap_or(Value::Null)
}

/// Compute the declared summaries + running totals over a set of rows.
fn summarize(rows: &[Row], defs: &[SummaryDef]) -> Vec<Summary> {
    defs.iter()
        .map(|d| Summary {
            operation: d.operation,
            field: d.key.clone(),
            value: aggregate(rows, d.operation, &d.field, d.param),
        })
        .collect()
}

/// Turn the per-group values of a no-reset running total into a running accumulation across the
/// top-level groups (in their sorted order): each group's value becomes the total up to and
/// including it. Additive for Sum/Count/Average-as-sum; Max/Min keep the running extremum; other
/// operations are left as their per-group value (a documented best-effort — running totals are
/// most commonly Sum/Count).
fn apply_cumulative(groups: &mut [GroupInstance], defs: &[SummaryDef]) {
    for d in defs.iter().filter(|d| d.cumulative) {
        let mut acc: Option<Value> = None;
        for g in groups.iter_mut() {
            let Some(s) = g.summaries.iter_mut().find(|s| s.field == d.key) else {
                continue;
            };
            let combined = accumulate(acc.as_ref(), &s.value, d.operation);
            s.value = combined.clone();
            acc = Some(combined);
        }
    }
}

/// Combine a running accumulator with the next per-group value for a no-reset running total.
fn accumulate(acc: Option<&Value>, next: &Value, op: SummaryOperation) -> Value {
    let Some(acc) = acc else {
        return next.clone();
    };
    match op {
        SummaryOperation::Sum
        | SummaryOperation::Count
        | SummaryOperation::DistinctCount
        | SummaryOperation::Average => match (acc.as_number(), next.as_number()) {
            (Some(a), Some(b)) => {
                let sum = a + b;
                if matches!(acc, Value::Currency(_)) || matches!(next, Value::Currency(_)) {
                    Value::Currency(sum)
                } else {
                    Value::Number(sum)
                }
            }
            _ => next.clone(),
        },
        SummaryOperation::Maximum => {
            if compare_values(next, acc).is_gt() {
                next.clone()
            } else {
                acc.clone()
            }
        }
        SummaryOperation::Minimum => {
            if compare_values(next, acc).is_lt() {
                next.clone()
            } else {
                acc.clone()
            }
        }
        _ => next.clone(),
    }
}

/// Apply one summary operation over a field across rows.
fn aggregate(rows: &[Row], op: SummaryOperation, field: &str, param: i32) -> Value {
    let values: Vec<&Value> = rows
        .iter()
        .filter_map(|r| r.get(field))
        .filter(|v| !v.is_null())
        .collect();
    // Count / DistinctCount / Sum / Average / WeightedAvg / Max / Min share the one reducer with the
    // running totals and the cross-tab cells; only the batch-only ops below fall through.
    let mut acc = SummaryAccumulator::new();
    for v in &values {
        acc.fold(v);
    }
    if let Some(result) = acc.value(op) {
        return result;
    }
    match op {
        // Dispersion: variance / standard deviation, sample (÷ n-1) and population (÷ n) forms.
        SummaryOperation::SampleVariance
        | SummaryOperation::PopVariance
        | SummaryOperation::SampleStandardDeviation
        | SummaryOperation::PopStandardDeviation => {
            let nums: Vec<f64> = values.iter().filter_map(|v| v.as_number()).collect();
            let sample = matches!(
                op,
                SummaryOperation::SampleVariance | SummaryOperation::SampleStandardDeviation
            );
            match variance(&nums, sample) {
                Some(var) => {
                    let stddev = matches!(
                        op,
                        SummaryOperation::SampleStandardDeviation
                            | SummaryOperation::PopStandardDeviation
                    );
                    Value::Number(if stddev { var.sqrt() } else { var })
                }
                None => Value::Null,
            }
        }
        // Order statistics: sort the numeric values ascending, then index into them.
        SummaryOperation::Median
        | SummaryOperation::Percentile
        | SummaryOperation::NthLargest
        | SummaryOperation::NthSmallest => {
            let mut nums: Vec<f64> = values.iter().filter_map(|v| v.as_number()).collect();
            if nums.is_empty() {
                return Value::Null;
            }
            nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            match op {
                SummaryOperation::Median => Value::Number(percentile(&nums, 50.0)),
                SummaryOperation::Percentile => Value::Number(percentile(&nums, param as f64)),
                // Nth is 1-based; NthLargest(1) = the maximum, NthSmallest(1) = the minimum.
                SummaryOperation::NthSmallest => nth(&nums, param, false),
                _ => nth(&nums, param, true),
            }
        }
        // Frequency: the (Nth) most-frequently-occurring value. Mode == the 1st most frequent.
        SummaryOperation::Mode | SummaryOperation::NthMostFrequent => {
            let n = if op == SummaryOperation::Mode {
                1
            } else {
                param
            };
            nth_most_frequent(&values, n)
        }
        // Any unrecognized op (`Other`) is Null so the pipeline stays total. The incremental ops
        // (Count / DistinctCount / Sum / Average / Max / Min) and the two-field ops (WeightedAvg /
        // Correlation / Covariance, which resolve to Null for want of their second field) already
        // returned above via the shared accumulator and never reach here.
        _ => Value::Null,
    }
}

/// Sample (`÷ n-1`) or population (`÷ n`) variance of `nums`. `None` when there is too little data
/// (empty, or a single value for the sample form).
fn variance(nums: &[f64], sample: bool) -> Option<f64> {
    let n = nums.len();
    if n == 0 || (sample && n < 2) {
        return None;
    }
    let mean = nums.iter().sum::<f64>() / n as f64;
    let ss: f64 = nums.iter().map(|x| (x - mean).powi(2)).sum();
    Some(ss / if sample { (n - 1) as f64 } else { n as f64 })
}

/// The `p`th percentile (0–100) of an already-ascending-sorted slice, by linear interpolation
/// between the two nearest ranks. `p` is clamped to `[0, 100]`; `nums` must be non-empty.
fn percentile(nums: &[f64], p: f64) -> f64 {
    let p = p.clamp(0.0, 100.0);
    if nums.len() == 1 {
        return nums[0];
    }
    let rank = p / 100.0 * (nums.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let frac = rank - lo as f64;
    nums[lo] + (nums[hi] - nums[lo]) * frac
}

/// The 1-based Nth largest (`largest`) or Nth smallest value of an ascending-sorted slice. Out-of-
/// range N yields `Null`.
fn nth(nums: &[f64], n: i32, largest: bool) -> Value {
    if n < 1 || n as usize > nums.len() {
        return Value::Null;
    }
    let i = n as usize - 1;
    let idx = if largest { nums.len() - 1 - i } else { i };
    Value::Number(nums[idx])
}

/// The value with the Nth-highest occurrence count (1-based). Ties break toward the value that
/// sorts first, so the result is deterministic. `Null` when N exceeds the number of distinct values.
fn nth_most_frequent(values: &[&Value], n: i32) -> Value {
    if n < 1 || values.is_empty() {
        return Value::Null;
    }
    let mut counts: std::collections::HashMap<String, (usize, &Value)> =
        std::collections::HashMap::new();
    let mut order = Vec::new();
    for v in values {
        let k = value_key(v);
        let e = counts.entry(k.clone()).or_insert_with(|| {
            order.push(k.clone());
            (0, *v)
        });
        e.0 += 1;
    }
    let mut ranked: Vec<(usize, &Value)> = order.iter().map(|k| counts[k]).collect();
    // Highest count first; ties resolved by value order (compare_values) for determinism.
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| compare_values(a.1, b.1)));
    ranked
        .get(n as usize - 1)
        .map(|(_, v)| (*v).clone())
        .unwrap_or(Value::Null)
}

fn order_rows(a: &Row, b: &Row, field: &str, dir: SortDirection) -> Ordering {
    let av = a.get(field).cloned().unwrap_or(Value::Null);
    let bv = b.get(field).cloned().unwrap_or(Value::Null);
    let ord = compare_values(&av, &bv);
    match dir {
        SortDirection::DescendingOrder => ord.reverse(),
        _ => ord,
    }
}

#[cfg(test)]
mod agg_tests {
    use super::*;

    fn rows(field: &str, vals: &[f64]) -> Vec<Row> {
        vals.iter()
            .map(|&n| {
                let mut r = Row::default();
                r.insert(field, Value::Number(n));
                r
            })
            .collect()
    }

    fn num(v: Value) -> f64 {
        match v {
            Value::Number(n) | Value::Currency(n) => n,
            other => panic!("expected number, got {other:?}"),
        }
    }

    #[test]
    fn sql_expression_field_is_never_compiled_as_a_crystal_formula() {
        use rpt_model::{FieldDef, FieldKindData, FormulaField, SqlExpressionField};
        // A SQL Expression field carries a SQL body (server-evaluated), NOT a Crystal formula. Its
        // text must never reach the crystal-formula parser/VM; the DB computes it and it arrives as a
        // fetched column. `compile_formulas` proves this: it registers only Formula fields.
        // A body that is valid SQL but not valid Crystal — if it were parsed as Crystal it would be
        // mangled; it must be passed through opaquely instead.
        let sql_field = FieldDef {
            name: "SqlExpr1".to_string(),
            kind: FieldKindData::SqlExpression(SqlExpressionField {
                text: "CASE WHEN amount > 0 THEN 'pos' ELSE 'neg' END".to_string(),
            }),
            ..FieldDef::default()
        };
        let formula = FieldDef {
            name: "Formula1".to_string(),
            kind: FieldKindData::Formula(FormulaField {
                text: rpt_model::Formula("1 + 1".to_string()),
                ..FormulaField::default()
            }),
            ..FieldDef::default()
        };

        let data_def = DataDefinition {
            field_definitions: vec![sql_field, formula],
            ..DataDefinition::default()
        };

        let reg = compile_formulas(&data_def);
        // The Crystal formula is compiled; the SQL Expression field is not present in the registry,
        // so it is never handed to crystal-formula.
        assert!(reg.contains_key("formula1"));
        assert!(!reg.contains_key("sqlexpr1"));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn date_group_buckets_a_datetime_key_by_period() {
        use crystal_formula::eval::Time;
        let noon = Time::new(12, 0, 0);
        let dt = |y, m, d| Value::DateTime(Date::new(y, m, d), noon);

        // Monthly: every day of a month collapses to the first of that month; distinct months differ.
        assert_eq!(
            date_bucket(dt(2024, 1, 3), Some("monthly")),
            Value::Date(Date::new(2024, 1, 1))
        );
        assert_eq!(
            date_bucket(dt(2024, 1, 28), Some("monthly")),
            Value::Date(Date::new(2024, 1, 1))
        );
        assert_eq!(
            date_bucket(dt(2024, 2, 1), Some("monthly")),
            Value::Date(Date::new(2024, 2, 1))
        );

        // Daily: a DateTime collapses to its calendar day (time dropped), regardless of the time.
        assert_eq!(
            date_bucket(dt(2024, 1, 3), Some("daily")),
            Value::Date(Date::new(2024, 1, 3))
        );
        assert_eq!(
            date_bucket(
                Value::DateTime(Date::new(2024, 1, 3), Time::new(23, 59, 59)),
                Some("daily")
            ),
            Value::Date(Date::new(2024, 1, 3))
        );

        // Weekly: keyed by the week-start Sunday. 2024-01-03 is a Wednesday → the prior Sunday
        // 2023-12-31; the following Sunday 2024-01-07 keys itself.
        assert_eq!(
            date_bucket(dt(2024, 1, 3), Some("weekly")),
            Value::Date(Date::new(2023, 12, 31))
        );
        assert_eq!(
            date_bucket(dt(2024, 1, 7), Some("weekly")),
            Value::Date(Date::new(2024, 1, 7))
        );

        // No condition, or a non-date key, passes through unchanged.
        assert_eq!(date_bucket(dt(2024, 1, 3), None), dt(2024, 1, 3));
        assert_eq!(
            date_bucket(Value::Number(5.0), Some("monthly")),
            Value::Number(5.0)
        );
    }

    #[test]
    fn date_bucket_extended_calendar_periods() {
        let d = |y, m, dd| Value::Date(Date::new(y, m, dd));
        // Semimonthly: 1st–15th → the 1st; 16th–end → the 16th.
        assert_eq!(
            date_bucket(d(2024, 3, 15), Some("semimonthly")),
            d(2024, 3, 1)
        );
        assert_eq!(
            date_bucket(d(2024, 3, 16), Some("semimonthly")),
            d(2024, 3, 16)
        );
        // Quarterly: first day of the calendar quarter.
        assert_eq!(
            date_bucket(d(2024, 2, 29), Some("quarterly")),
            d(2024, 1, 1)
        );
        assert_eq!(
            date_bucket(d(2024, 8, 10), Some("quarterly")),
            d(2024, 7, 1)
        );
        // Semiannually: Jan 1 or Jul 1.
        assert_eq!(
            date_bucket(d(2024, 6, 30), Some("semiannually")),
            d(2024, 1, 1)
        );
        assert_eq!(
            date_bucket(d(2024, 7, 1), Some("semiannually")),
            d(2024, 7, 1)
        );
        // Annually: Jan 1 of the year.
        assert_eq!(date_bucket(d(2024, 11, 5), Some("annually")), d(2024, 1, 1));
        // Biweekly aligns to a week-start and is stable within a fortnight (the fortnight starting
        // Sun 2024-01-07 spans through Sat 2024-01-20; the next fortnight starts 2024-01-21).
        let bw = |v| date_bucket(v, Some("biweekly"));
        assert_eq!(bw(d(2024, 1, 7)), bw(d(2024, 1, 20))); // same fortnight
        assert_ne!(bw(d(2024, 1, 7)), bw(d(2024, 1, 21))); // next fortnight
    }

    #[test]
    fn date_bucket_time_of_day_periods() {
        let dt = |h, mi, s| Value::DateTime(Date::new(2024, 1, 3), Time::new(h, mi, s));
        let day = Date::new(2024, 1, 3);
        // Time periods keep the date and truncate the time; a DateTime stays a DateTime.
        assert_eq!(
            date_bucket(dt(9, 41, 30), Some("byhour")),
            Value::DateTime(day, Time::new(9, 0, 0))
        );
        assert_eq!(
            date_bucket(dt(9, 41, 30), Some("byminute")),
            Value::DateTime(day, Time::new(9, 41, 0))
        );
        assert_eq!(
            date_bucket(dt(9, 41, 30), Some("byampm")),
            Value::DateTime(day, Time::new(0, 0, 0))
        );
        assert_eq!(
            date_bucket(dt(14, 5, 0), Some("byampm")),
            Value::DateTime(day, Time::new(12, 0, 0))
        );
        // A bare Time value stays a Time.
        assert_eq!(
            date_bucket(Value::Time(Time::new(14, 5, 30)), Some("byhour")),
            Value::Time(Time::new(14, 0, 0))
        );
    }

    #[test]
    fn variance_and_stddev_sample_vs_population() {
        // {2,4,4,4,5,5,7,9}: pop variance 4, sample variance 32/7.
        let rs = rows("x", &[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
        let pv = num(aggregate(&rs, SummaryOperation::PopVariance, "x", 0));
        assert!((pv - 4.0).abs() < 1e-9, "pop variance {pv}");
        let ps = num(aggregate(
            &rs,
            SummaryOperation::PopStandardDeviation,
            "x",
            0,
        ));
        assert!((ps - 2.0).abs() < 1e-9, "pop stddev {ps}");
        let sv = num(aggregate(&rs, SummaryOperation::SampleVariance, "x", 0));
        assert!((sv - 32.0 / 7.0).abs() < 1e-9, "sample variance {sv}");
        let ss = num(aggregate(
            &rs,
            SummaryOperation::SampleStandardDeviation,
            "x",
            0,
        ));
        assert!(
            (ss - (32.0f64 / 7.0).sqrt()).abs() < 1e-9,
            "sample stddev {ss}"
        );
    }

    #[test]
    fn sample_variance_needs_two_values() {
        let rs = rows("x", &[5.0]);
        assert_eq!(
            aggregate(&rs, SummaryOperation::SampleVariance, "x", 0),
            Value::Null
        );
        // Population variance of one value is 0.
        assert_eq!(
            num(aggregate(&rs, SummaryOperation::PopVariance, "x", 0)),
            0.0
        );
    }

    #[test]
    fn median_odd_and_even() {
        assert_eq!(
            num(aggregate(
                &rows("x", &[3.0, 1.0, 2.0]),
                SummaryOperation::Median,
                "x",
                0
            )),
            2.0
        );
        // Even count → mean of the two middle values (2 and 3 → 2.5).
        assert_eq!(
            num(aggregate(
                &rows("x", &[1.0, 2.0, 3.0, 4.0]),
                SummaryOperation::Median,
                "x",
                0
            )),
            2.5
        );
    }

    #[test]
    fn percentile_interpolates() {
        let rs = rows("x", &[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(
            num(aggregate(&rs, SummaryOperation::Percentile, "x", 0)),
            1.0
        );
        assert_eq!(
            num(aggregate(&rs, SummaryOperation::Percentile, "x", 100)),
            4.0
        );
        assert_eq!(
            num(aggregate(&rs, SummaryOperation::Percentile, "x", 50)),
            2.5
        );
    }

    #[test]
    fn nth_largest_smallest_are_one_based() {
        let rs = rows("x", &[10.0, 30.0, 20.0, 40.0]);
        assert_eq!(
            num(aggregate(&rs, SummaryOperation::NthLargest, "x", 1)),
            40.0
        );
        assert_eq!(
            num(aggregate(&rs, SummaryOperation::NthLargest, "x", 2)),
            30.0
        );
        assert_eq!(
            num(aggregate(&rs, SummaryOperation::NthSmallest, "x", 1)),
            10.0
        );
        assert_eq!(
            num(aggregate(&rs, SummaryOperation::NthSmallest, "x", 2)),
            20.0
        );
        // Out of range → Null.
        assert_eq!(
            aggregate(&rs, SummaryOperation::NthLargest, "x", 9),
            Value::Null
        );
    }

    #[test]
    fn mode_and_nth_most_frequent() {
        // 20 appears 3x, 10 twice, 30 once.
        let rs = rows("x", &[10.0, 20.0, 20.0, 30.0, 10.0, 20.0]);
        assert_eq!(num(aggregate(&rs, SummaryOperation::Mode, "x", 0)), 20.0);
        assert_eq!(
            num(aggregate(&rs, SummaryOperation::NthMostFrequent, "x", 2)),
            10.0
        );
        assert_eq!(
            num(aggregate(&rs, SummaryOperation::NthMostFrequent, "x", 3)),
            30.0
        );
    }

    #[test]
    fn two_field_ops_are_null() {
        // WeightedAvg / Correlation / Covariance need a second field the summary model does not carry,
        // so they aggregate to Null (unavailable) — never a plausible-but-wrong number. WeightedAvg in
        // particular must NOT silently return the plain Average (here 2.0) of the single field.
        let rs = rows("x", &[1.0, 2.0, 3.0]);
        assert_eq!(
            aggregate(&rs, SummaryOperation::Average, "x", 0),
            Value::Number(2.0)
        );
        assert_eq!(
            aggregate(&rs, SummaryOperation::WeightedAvg, "x", 0),
            Value::Null
        );
        assert_eq!(
            aggregate(&rs, SummaryOperation::Correlation, "x", 0),
            Value::Null
        );
        assert_eq!(
            aggregate(&rs, SummaryOperation::Covariance, "x", 0),
            Value::Null
        );
    }
}
