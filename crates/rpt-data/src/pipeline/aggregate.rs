//! Summary collection and aggregation: the [`SummaryDef`] set to compute, the per-group and grand
//! reducers, cumulative running totals, and the batch statistical operations.

use crate::context::{DataContext, FormulaRegistry, Parameters};
use crate::source::Row;
use crate::value_order::{compare_values, value_key};
use crate::{GroupInstance, Summary, SummaryAccumulator};
use rpt_formula::eval::Value;
use rpt_model::{DataDefinition, FieldKindData, ResetConditionType, SummaryOperation};
use std::cmp::Ordering;

use super::group::ctx_formula;

/// The summary + running-total fields to compute at each level. A declared **summary** is keyed by
/// its summarized field; a **running total** (`#name`) is keyed by `#name` (how `{#name}` references
/// it) and aggregated over each group's rows — correct for a running total that resets on group
/// change (the common case, and what group charts plot). A running total with no reset condition
/// (`NoCondition`) accumulates across the top-level groups instead (see [`apply_cumulative`]).
pub(super) fn collect_summaries(data_def: &DataDefinition) -> Vec<SummaryDef> {
    let mut defs = Vec::new();
    for f in &data_def.field_definitions {
        match &f.kind {
            // A summary definition is stored once per placement (group/report footer), so the same
            // (operation, field) appears several times; the value is computed at every level anyway,
            // so keep one per distinct summary.
            FieldKindData::Summary(s)
                if !defs.iter().any(|d: &SummaryDef| {
                    d.operation == s.operation && d.key == s.summarized_field
                }) =>
            {
                defs.push(SummaryDef {
                    operation: s.operation,
                    field: s.summarized_field.clone(),
                    secondary: non_empty(&s.secondary_summarized_field),
                    key: s.summarized_field.clone(),
                    cumulative: false,
                    param: s.operation_parameter,
                })
            }
            FieldKindData::Summary(_) => {}
            FieldKindData::RunningTotal(rt) => defs.push(SummaryDef {
                operation: rt.operation,
                field: rt.summarized_field.clone(),
                secondary: non_empty(&rt.secondary_summarized_field),
                key: format!("#{}", f.name),
                cumulative: rt.reset == ResetConditionType::NoCondition,
                param: rt.operation_parameter,
            }),
            _ => {}
        }
    }
    defs
}

/// `Some(field)` for a non-empty field reference, `None` for an empty one — the [`SummaryDef`]
/// secondary-field slot, absent for every single-field summary.
fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

#[derive(Clone)]
pub(super) struct SummaryDef {
    operation: SummaryOperation,
    /// The field aggregated over (the summarized field).
    field: String,
    /// The second operand field for a two-field operation (`WeightedAvg`/`Correlation`/`Covariance`):
    /// the weight field, or the paired field. `None` for every single-field operation.
    secondary: Option<String>,
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

/// Compute the declared summaries + running totals over a set of rows. `formulas`/`params` let a
/// summary over a formula field (`Avg of {@Formula}`) aggregate the formula's per-row value.
pub(super) fn summarize(
    rows: &[Row],
    defs: &[SummaryDef],
    formulas: &FormulaRegistry,
    params: &Parameters,
) -> Vec<Summary> {
    defs.iter()
        .map(|d| Summary {
            operation: d.operation,
            field: d.key.clone(),
            value: aggregate(
                rows,
                d.operation,
                &d.field,
                d.param,
                d.secondary.as_deref(),
                formulas,
                params,
            ),
        })
        .collect()
}

/// The value a summary aggregates for one row: a `{@formula}` summarized field is evaluated in a
/// stateless per-row context (no `Global`/`Shared` store, so a running-total side effect never
/// double-fires here); a plain database / SQL-expression column reads straight from the row.
fn summarized_value(
    row: &Row,
    field: &str,
    formulas: &FormulaRegistry,
    params: &Parameters,
) -> Value {
    if let Some(name) = field.strip_prefix('@') {
        let ctx = DataContext::new(row, formulas).with_params(params);
        ctx_formula(&ctx, name)
    } else {
        row.get(field).cloned().unwrap_or(Value::Null)
    }
}

/// Turn the per-group values of a no-reset running total into a running accumulation across the
/// top-level groups (in their sorted order): each group's value becomes the total up to and
/// including it. Additive for Sum/Count/Average-as-sum; Max/Min keep the running extremum; other
/// operations are left as their per-group value (a documented best-effort — running totals are
/// most commonly Sum/Count).
pub(super) fn apply_cumulative(groups: &mut [GroupInstance], defs: &[SummaryDef]) {
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

/// Apply one summary operation over a field across rows. `secondary` is the second-operand field for
/// the two-field operations (`WeightedAvg`/`Correlation`/`Covariance`); it is `None` for every
/// single-field operation. `formulas`/`params` evaluate a `{@formula}` summarized field per row.
#[allow(clippy::too_many_arguments)]
pub(super) fn aggregate(
    rows: &[Row],
    op: SummaryOperation,
    field: &str,
    param: i32,
    secondary: Option<&str>,
    formulas: &FormulaRegistry,
    params: &Parameters,
) -> Value {
    // Two-field operations pair `field` with `secondary` row-by-row; they cannot be folded one value
    // at a time (unlike the single-field ops), so they are handled here before the shared reducer.
    // Without a second field they are unavailable (`Null`) rather than a plausible-but-wrong number.
    if matches!(
        op,
        SummaryOperation::WeightedAvg
            | SummaryOperation::Correlation
            | SummaryOperation::Covariance
    ) {
        return match secondary {
            Some(sec) => two_field_aggregate(rows, op, field, sec, formulas, params),
            None => Value::Null,
        };
    }
    // Resolve each row's summarized value (a database column, or an evaluated `{@formula}`), keeping
    // only the non-null ones.
    let values: Vec<Value> = rows
        .iter()
        .map(|r| summarized_value(r, field, formulas, params))
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
            let refs: Vec<&Value> = values.iter().collect();
            nth_most_frequent(&refs, n)
        }
        // Any unrecognized op (`Other`) is Null so the pipeline stays total. The incremental ops
        // (Count / DistinctCount / Sum / Average / Max / Min) already returned above via the shared
        // accumulator, and the two-field ops (WeightedAvg / Correlation / Covariance) returned at the
        // top of `aggregate`, so neither reaches here.
        _ => Value::Null,
    }
}

/// A two-field summary over the rows where **both** `x_field` and `y_field` hold a number:
///
/// * `WeightedAvg(X, W)` = Σ(Xᵢ·Wᵢ) / Σ(Wᵢ) — `x_field` is the value X, `y_field` the weight W.
/// * `Covariance(X, Y)`  = Σ((Xᵢ−X̄)(Yᵢ−Ȳ)) / (n−1).
/// * `Correlation(X, Y)` = Σ((Xᵢ−X̄)(Yᵢ−Ȳ)) / √(Σ(Xᵢ−X̄)²·Σ(Yᵢ−Ȳ)²).
///
/// `Null` when there is no usable data (no paired rows; a zero total weight; `Covariance` with fewer
/// than two pairs; `Correlation` with a zero-variance field).
///
/// `Covariance`'s divisor (n−1 vs n, unconfirmed) defaults to the sample form, by analogy with
/// `SampleVariance`/`SampleStandardDeviation` — `SummaryOperation` has no Pop/Sample split for
/// covariance to pin it exactly. `Correlation` is invariant to the choice (the divisor cancels), so
/// only a standalone `Covariance` magnitude depends on it.
fn two_field_aggregate(
    rows: &[Row],
    op: SummaryOperation,
    x_field: &str,
    y_field: &str,
    formulas: &FormulaRegistry,
    params: &Parameters,
) -> Value {
    let pairs: Vec<(f64, f64)> = rows
        .iter()
        .filter_map(|r| {
            Some((
                summarized_value(r, x_field, formulas, params).as_number()?,
                summarized_value(r, y_field, formulas, params).as_number()?,
            ))
        })
        .collect();
    let n = pairs.len();
    match op {
        SummaryOperation::WeightedAvg => {
            let weight_sum: f64 = pairs.iter().map(|(_, w)| w).sum();
            if weight_sum == 0.0 {
                return Value::Null;
            }
            let weighted: f64 = pairs.iter().map(|(x, w)| x * w).sum();
            Value::Number(weighted / weight_sum)
        }
        SummaryOperation::Covariance => {
            if n < 2 {
                return Value::Null;
            }
            let (mx, my) = pair_means(&pairs, n);
            let cross: f64 = pairs.iter().map(|(x, y)| (x - mx) * (y - my)).sum();
            // Sample divisor (n−1); see the doc comment on the Pop/Sample choice above.
            Value::Number(cross / (n as f64 - 1.0))
        }
        SummaryOperation::Correlation => {
            if n < 2 {
                return Value::Null;
            }
            let (mx, my) = pair_means(&pairs, n);
            let cross: f64 = pairs.iter().map(|(x, y)| (x - mx) * (y - my)).sum();
            let var_x: f64 = pairs.iter().map(|(x, _)| (x - mx).powi(2)).sum();
            let var_y: f64 = pairs.iter().map(|(_, y)| (y - my).powi(2)).sum();
            let denom = (var_x * var_y).sqrt();
            if denom == 0.0 {
                return Value::Null;
            }
            Value::Number(cross / denom)
        }
        // Only the three two-field ops route here (from `aggregate`); nothing else can.
        _ => Value::Null,
    }
}

/// The means of the X and Y components of `pairs` (`n` = `pairs.len()`, assumed non-zero).
fn pair_means(pairs: &[(f64, f64)], n: usize) -> (f64, f64) {
    let sx: f64 = pairs.iter().map(|(x, _)| x).sum();
    let sy: f64 = pairs.iter().map(|(_, y)| y).sum();
    (sx / n as f64, sy / n as f64)
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
