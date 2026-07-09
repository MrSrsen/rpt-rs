//! Evaluation-time scheduling.
//!
//! [`classify_eval_time`](crate::classify_eval_time) sorts every formula into a
//! [`EvalTime`](crate::EvalTime) class; this module *acts* on that classification:
//!
//! - **`BeforeReadingRecords`** — evaluated once, before any record, so its (constant) side-effects
//!   fire exactly once and its value is reused by every record.
//! - **`WhileReadingRecords`** — evaluated per record in **read order** (the source order after
//!   record selection, before sort/group), so a running `Global`/`Shared` variable accumulates in
//!   the order the engine read the rows, not the order it later printed them.
//! - **`WhilePrintingRecords`** — left to the print pass, which evaluates it lazily per record in
//!   **print order** (page/record specials, running totals, `Previous`/`Next`).
//!
//! The pre-pass ([`EvalSchedule::run`]) records each scheduled formula's value (report-level for
//! `BeforeReading`, per read-order record for `WhileReading`) into [`ScheduledValues`]; the render
//! pass hands those back to each record's [`DataContext`](crate::DataContext), which returns the
//! recorded value instead of re-evaluating — so a formula's side-effects fire **once per class**,
//! never twice.
//!
//! Conservative scope: only formulas whose body references fields/parameters directly (no
//! `{@formula}` chaining, no running totals) are pre-scheduled; anything that chains another formula
//! is left to lazy print-time evaluation (its transitive class can't be settled without full
//! dependency analysis, and evaluating it early risks reading a print-time value before it exists).

use crate::context::{DataContext, FormulaRegistry, Parameters, SharedState};
use crate::eval_time::{classify_eval_time, EvalTime};
use crate::source::Row;
use crystal_formula::eval::{EvalContext, Value};
use crystal_formula::{references, RefKind};
use rpt_model::{DataDefinition, FieldKindData};
use std::collections::HashMap;

/// The formula names (lowercase) to run in each scheduled pass. Formulas not listed here are
/// evaluated lazily by the print pass (`WhilePrintingRecords`, and any formula that chains another).
#[derive(Debug, Default, Clone)]
pub struct EvalSchedule {
    /// `BeforeReadingRecords` formulas — run once, report-level.
    before: Vec<String>,
    /// `WhileReadingRecords` formulas — run per record in read order.
    while_reading: Vec<String>,
}

/// The values recorded by the scheduled pre-pass, handed to each record's context so it returns them
/// instead of re-evaluating (single-fire).
#[derive(Debug, Default, Clone)]
pub struct ScheduledValues {
    /// `BeforeReadingRecords` formula values (report-level, constant across records).
    pub before: HashMap<String, Value>,
    /// `WhileReadingRecords` formula values per record, keyed by the row's read index.
    pub per_record: HashMap<u64, HashMap<String, Value>>,
}

impl ScheduledValues {
    /// Whether nothing was scheduled (the caller can skip threading these through).
    pub fn is_empty(&self) -> bool {
        self.before.is_empty() && self.per_record.is_empty()
    }

    /// The recorded values for the record with read index `idx`, if any.
    pub fn record(&self, idx: u64) -> Option<&HashMap<String, Value>> {
        self.per_record.get(&idx)
    }
}

impl EvalSchedule {
    /// Classify a report's formula fields into the scheduled passes.
    pub fn classify(dd: &DataDefinition) -> EvalSchedule {
        let mut sched = EvalSchedule::default();
        for f in &dd.field_definitions {
            let FieldKindData::Formula(ff) = &f.kind else {
                continue;
            };
            let body = &ff.text.0;
            // A formula that chains another formula, or reads a running total, is left to lazy
            // print-time evaluation (its true class needs the chained formula's class).
            let chains = references(body)
                .any(|r| matches!(r.kind, RefKind::Formula | RefKind::RunningTotal));
            if chains {
                continue;
            }
            match classify_eval_time(body) {
                EvalTime::BeforeReadingRecords => sched.before.push(f.name.to_lowercase()),
                EvalTime::WhileReadingRecords => sched.while_reading.push(f.name.to_lowercase()),
                EvalTime::WhilePrintingRecords => {}
            }
        }
        sched
    }

    /// Whether there is nothing to pre-schedule.
    pub fn is_empty(&self) -> bool {
        self.before.is_empty() && self.while_reading.is_empty()
    }

    /// Run the scheduled passes against `rows` in **read order**, firing each scheduled formula's
    /// side-effects into `state` and recording its value into the returned [`ScheduledValues`].
    /// `BeforeReading` formulas run once first; `WhileReading` formulas then run per record.
    pub fn run(
        &self,
        rows: &[Row],
        formulas: &FormulaRegistry,
        state: &SharedState,
        params: &Parameters,
    ) -> ScheduledValues {
        let mut out = ScheduledValues::default();
        if self.is_empty() {
            return out;
        }

        // BeforeReadingRecords: once, over an empty record (constants / parameter-only).
        if !self.before.is_empty() {
            let empty = Row::default();
            let ctx = DataContext::new(&empty, formulas)
                .with_params(params)
                .with_state(state);
            for name in &self.before {
                let v = ctx.resolve(RefKind::Formula, name).unwrap_or(Value::Null);
                out.before.insert(name.clone(), v);
            }
        }

        // WhileReadingRecords: per record in read order.
        if !self.while_reading.is_empty() {
            for row in rows {
                let Some(idx) = row.read_index() else {
                    continue;
                };
                let ctx = DataContext::new(row, formulas)
                    .with_params(params)
                    .with_state(state)
                    .with_scheduled(Some(&out.before), None);
                let mut values = HashMap::new();
                for name in &self.while_reading {
                    let v = ctx.resolve(RefKind::Formula, name).unwrap_or(Value::Null);
                    values.insert(name.clone(), v);
                }
                out.per_record.insert(idx, values);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::compile_formulas;
    use rpt_model::{FieldDef, FieldKindData, FormulaField};

    fn formula_field(name: &str, body: &str) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            kind: FieldKindData::Formula(FormulaField {
                text: rpt_model::Formula(body.to_string()),
                ..FormulaField::default()
            }),
            ..Default::default()
        }
    }

    fn row(field: &str, n: f64, idx: u64) -> Row {
        let mut r = Row::default();
        r.insert(field, Value::Number(n));
        r.set_read_index(idx);
        r
    }

    #[test]
    fn classify_splits_by_eval_time() {
        let dd = DataDefinition {
            field_definitions: vec![
                formula_field("Const", "2 + 2"),
                formula_field("Read", "{t.amt} * 2"),
                formula_field("Print", "PageNumber + 1"),
                formula_field("Chained", "{@Read} + 1"),
            ],
            ..Default::default()
        };
        let s = EvalSchedule::classify(&dd);
        assert_eq!(s.before, vec!["const"]);
        assert_eq!(s.while_reading, vec!["read"]);
        // Print-time and formula-chaining formulas are not pre-scheduled.
        assert!(!s.before.contains(&"print".to_string()));
        assert!(!s.while_reading.contains(&"chained".to_string()));
    }

    #[test]
    fn while_reading_accumulates_in_read_order() {
        // A Global running sum classified WhileReading; the pre-pass folds it in read order and
        // records the per-record value.
        let dd = DataDefinition {
            field_definitions: vec![formula_field(
                "RunTotal",
                "Global NumberVar t; t := t + {t.amt}; t",
            )],
            ..Default::default()
        };
        let formulas = compile_formulas(&dd);
        let sched = EvalSchedule::classify(&dd);
        assert_eq!(sched.while_reading, vec!["runtotal"]);

        // Read order: amounts 3, 1, 2 at read indices 0, 1, 2.
        let rows = vec![
            row("t.amt", 3.0, 0),
            row("t.amt", 1.0, 1),
            row("t.amt", 2.0, 2),
        ];
        let state = SharedState::new();
        let params = Parameters::new();
        let vals = sched.run(&rows, &formulas, &state, &params);

        let at = |i: u64| vals.record(i).unwrap()["runtotal"].as_number().unwrap();
        assert_eq!(at(0), 3.0);
        assert_eq!(at(1), 4.0);
        assert_eq!(at(2), 6.0);
    }

    #[test]
    fn before_reading_runs_once() {
        let dd = DataDefinition {
            field_definitions: vec![formula_field("K", "10 + 5")],
            ..Default::default()
        };
        let formulas = compile_formulas(&dd);
        let sched = EvalSchedule::classify(&dd);
        let state = SharedState::new();
        let params = Parameters::new();
        let vals = sched.run(&[], &formulas, &state, &params);
        assert_eq!(vals.before["k"], Value::Number(15.0));
    }
}
