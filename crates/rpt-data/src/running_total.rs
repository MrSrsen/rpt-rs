//! Per-record running totals.
//!
//! The group-level running totals — the value shown in a group header/footer — are computed in
//! [`pipeline`](crate::pipeline) as summaries keyed `#name` (per-group aggregate for an
//! `OnChangeOfGroup` reset, cumulative across the top-level groups for `NoCondition`). This module
//! adds the *other* half: the value a `{#name}` field/text object shows **in a detail band**, which
//! is the running value accumulated up to and including the current record in **print order**.
//!
//! [`RunningTotals`] is a report-lifetime, interior-mutable accumulator (one instance per render,
//! threaded through [`DataContext`](crate::DataContext) like [`SharedState`](crate::SharedState)):
//! the layout engine calls [`RunningTotals::advance`] once per record as it prints them, and a
//! `resolve(RefKind::RunningTotal, name)` reads the current value back.
//!
//! Evaluate/reset conditions: the **reset** condition (`NoCondition` /
//! `OnChangeOfGroup` / `OnChangeOfField` / `OnFormula`) clears the accumulator before folding the
//! current record when its key changes; the **evaluate** condition governs whether the current
//! record is folded at all. Both are best-effort and fail-open — an unresolvable driver never resets
//! and always folds, so a running total can only ever *over*-accumulate, never drop data. Evaluation
//! is per record in print order (the common case, matching what the group-level aggregate plots).

use crate::value_order::value_key;
use crate::SummaryAccumulator;
use crystal_formula::eval::{EvalContext, Value};
use crystal_formula::RefKind;
use rpt_model::{
    DataDefinition, EvaluationConditionType, FieldKindData, ResetConditionType, RunningTotalField,
    SummaryOperation,
};
use std::cell::RefCell;

/// The report-lifetime print-order running-total accumulators, keyed by `#name`. Interior-mutable so
/// a `&RunningTotals` threads through the borrow-only render path (like [`SharedState`]).
///
/// [`SharedState`]: crate::SharedState
#[derive(Debug, Default)]
pub struct RunningTotals {
    entries: RefCell<Vec<RtEntry>>,
}

#[derive(Debug)]
struct RtEntry {
    /// The reference key (`#Name`) — how `{#name}` resolves it.
    key: String,
    operation: SummaryOperation,
    /// The summarized database/formula field (a `{@formula}` resolves through the registry).
    field: String,
    reset: ResetConditionType,
    evaluation: EvaluationConditionType,
    /// The driver of an `OnChangeOfField` / `OnFormula` evaluate-or-reset condition (`table.field`
    /// or `@formula`); empty otherwise.
    on_change_field: String,
    acc: SummaryAccumulator,
    /// Whether any record has been folded yet (so the first record never triggers a reset).
    started: bool,
    /// The last reset key seen (group signature / driver value), for change detection.
    last_reset_key: Option<String>,
    /// The last evaluate-driver value seen, for an `OnChangeOfField` evaluate condition.
    last_eval_key: Option<String>,
}

impl RunningTotals {
    /// Build the accumulators for a report's declared running-total fields (`#name`).
    pub fn from_data_def(dd: &DataDefinition) -> RunningTotals {
        let mut entries = Vec::new();
        for f in &dd.field_definitions {
            if let FieldKindData::RunningTotal(rt) = &f.kind {
                entries.push(RtEntry::new(&f.name, rt));
            }
        }
        RunningTotals {
            entries: RefCell::new(entries),
        }
    }

    /// Whether there are no running totals (lets the caller skip the per-record advance entirely).
    pub fn is_empty(&self) -> bool {
        self.entries.borrow().is_empty()
    }

    /// Advance every running total by the current record, evaluated through `ctx` (which resolves
    /// the summarized field and any `OnChangeOfField`/`OnFormula` driver). `group_signature` is the
    /// concatenation of the enclosing group keys, used by an `OnChangeOfGroup` reset — it changes
    /// exactly when the record's group path changes.
    pub fn advance(&self, ctx: &dyn EvalContext, group_signature: Option<&str>) {
        let mut entries = self.entries.borrow_mut();
        for e in entries.iter_mut() {
            e.advance(ctx, group_signature);
        }
    }

    /// The current running value of `#name` (the `#` is optional). `None` if there is no such
    /// running total (or it has not folded any record yet).
    pub fn get(&self, name: &str) -> Option<Value> {
        let want = name.trim_start_matches('#');
        self.entries
            .borrow()
            .iter()
            .find(|e| e.key.trim_start_matches('#').eq_ignore_ascii_case(want))
            // Parameterized ops (percentile/median/nth/…) aren't computed incrementally; a running
            // total falls back to the last folded value, as before.
            .map(|e| e.acc.value(e.operation).unwrap_or_else(|| e.acc.last()))
    }
}

impl RtEntry {
    fn new(name: &str, rt: &RunningTotalField) -> RtEntry {
        RtEntry {
            key: format!("#{name}"),
            operation: rt.operation,
            field: rt.summarized_field.clone(),
            reset: rt.reset,
            evaluation: rt.evaluation,
            on_change_field: rt.on_change_field.clone(),
            acc: SummaryAccumulator::new(),
            started: false,
            last_reset_key: None,
            last_eval_key: None,
        }
    }

    fn advance(&mut self, ctx: &dyn EvalContext, group_signature: Option<&str>) {
        // Reset first: a change in the reset key clears the accumulator *before* the current record
        // is folded (so the record starts the new run). The first record never resets.
        let reset_key = self.reset_key(ctx, group_signature);
        if self.started && reset_key != self.last_reset_key {
            self.acc = SummaryAccumulator::new();
        }
        self.last_reset_key = reset_key;

        // Evaluate condition: whether to fold this record. Default (`NoCondition`) folds every
        // record; a driver-based condition folds only when its trigger fires (fail-open: an
        // unresolvable driver folds).
        if self.should_evaluate(ctx) {
            let value = self.resolve_field(ctx);
            self.acc.fold(&value);
        }
        self.started = true;
    }

    /// The reset key for the current record, or `None` when the running total never resets.
    fn reset_key(&self, ctx: &dyn EvalContext, group_signature: Option<&str>) -> Option<String> {
        match self.reset {
            ResetConditionType::NoCondition => None,
            ResetConditionType::OnChangeOfGroup => {
                Some(group_signature.unwrap_or_default().to_string())
            }
            ResetConditionType::OnChangeOfField | ResetConditionType::OnFormula => {
                Some(driver_key(ctx, &self.on_change_field))
            }
            _ => None,
        }
    }

    /// Whether the current record should be folded, per the evaluate condition.
    fn should_evaluate(&mut self, ctx: &dyn EvalContext) -> bool {
        match self.evaluation {
            EvaluationConditionType::OnFormula => {
                // Fold when the condition formula is true; fail-open (unresolvable ⇒ fold).
                match driver_value(ctx, &self.on_change_field) {
                    Some(Value::Bool(b)) => b,
                    _ => true,
                }
            }
            EvaluationConditionType::OnChangeOfField => {
                // Fold once per distinct driver value (fold when it changes).
                let key = driver_key(ctx, &self.on_change_field);
                let changed = self.last_eval_key.as_deref() != Some(key.as_str());
                self.last_eval_key = Some(key);
                changed
            }
            // `NoCondition` / `OnChangeOfGroup` evaluate ⇒ every record.
            _ => true,
        }
    }

    /// Resolve the summarized field's value for the current record.
    fn resolve_field(&self, ctx: &dyn EvalContext) -> Value {
        resolve_ref(ctx, &self.field)
    }
}

/// The string key of a driver reference (`table.field` / `@formula`) for change detection.
fn driver_key(ctx: &dyn EvalContext, reference: &str) -> String {
    value_key(&driver_value(ctx, reference).unwrap_or(Value::Null))
}

/// Resolve a driver reference to its current value, or `None` when it is empty/unresolvable.
fn driver_value(ctx: &dyn EvalContext, reference: &str) -> Option<Value> {
    if reference.is_empty() {
        return None;
    }
    Some(resolve_ref(ctx, reference))
}

/// Resolve a bare reference (`table.field` or `@formula`) to a [`Value`] through `ctx`.
fn resolve_ref(ctx: &dyn EvalContext, reference: &str) -> Value {
    let r = reference.trim().trim_matches(['{', '}']);
    if let Some(name) = r.strip_prefix('@') {
        ctx.resolve(RefKind::Formula, name).unwrap_or(Value::Null)
    } else {
        ctx.resolve(RefKind::Field, r).unwrap_or(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DataContext, FormulaRegistry, Row};

    fn row(field: &str, n: f64) -> Row {
        let mut r = Row::default();
        r.insert(field, Value::Number(n));
        r
    }

    fn rt_field(
        name: &str,
        op: SummaryOperation,
        field: &str,
        reset: ResetConditionType,
    ) -> RtEntry {
        let rt = RunningTotalField {
            operation: op,
            summarized_field: field.to_string(),
            reset,
            ..Default::default()
        };
        RtEntry::new(name, &rt)
    }

    #[test]
    fn per_record_sum_accumulates_in_order() {
        let formulas = FormulaRegistry::new();
        let rts = RunningTotals {
            entries: RefCell::new(vec![rt_field(
                "T",
                SummaryOperation::Sum,
                "t.amt",
                ResetConditionType::NoCondition,
            )]),
        };
        let mut seen = Vec::new();
        for n in [10.0, 20.0, 5.0] {
            let r = row("t.amt", n);
            let ctx = DataContext::new(&r, &formulas);
            rts.advance(&ctx, None);
            seen.push(rts.get("T").unwrap().as_number().unwrap());
        }
        assert_eq!(seen, vec![10.0, 30.0, 35.0]);
    }

    #[test]
    fn running_max_min_over_dates_orders_chronologically() {
        use crystal_formula::eval::Date;
        let formulas = FormulaRegistry::new();
        let rts = RunningTotals {
            entries: RefCell::new(vec![
                rt_field(
                    "MX",
                    SummaryOperation::Maximum,
                    "t.d",
                    ResetConditionType::NoCondition,
                ),
                rt_field(
                    "MN",
                    SummaryOperation::Minimum,
                    "t.d",
                    ResetConditionType::NoCondition,
                ),
            ]),
        };
        // Dates within one year, folded out of order. The old Debug-string key ranked "month: 12"
        // below "month: 2", so a running Max wrongly picked Feb over Dec.
        for d in [
            Date::new(2024, 2, 1),
            Date::new(2024, 12, 1),
            Date::new(2024, 6, 15),
        ] {
            let mut r = Row::default();
            r.insert("t.d", Value::Date(d));
            let ctx = DataContext::new(&r, &formulas);
            rts.advance(&ctx, None);
        }
        assert_eq!(rts.get("MX").unwrap(), Value::Date(Date::new(2024, 12, 1)));
        assert_eq!(rts.get("MN").unwrap(), Value::Date(Date::new(2024, 2, 1)));
    }

    #[test]
    fn reset_on_group_change_restarts_the_run() {
        let formulas = FormulaRegistry::new();
        let rts = RunningTotals {
            entries: RefCell::new(vec![rt_field(
                "T",
                SummaryOperation::Sum,
                "t.amt",
                ResetConditionType::OnChangeOfGroup,
            )]),
        };
        // Two records in group "A", then two in group "B": the total resets at the boundary.
        let seq = [("A", 10.0), ("A", 20.0), ("B", 3.0), ("B", 4.0)];
        let mut seen = Vec::new();
        for (g, n) in seq {
            let r = row("t.amt", n);
            let ctx = DataContext::new(&r, &formulas);
            rts.advance(&ctx, Some(g));
            seen.push(rts.get("T").unwrap().as_number().unwrap());
        }
        assert_eq!(seen, vec![10.0, 30.0, 3.0, 7.0]);
    }
}
