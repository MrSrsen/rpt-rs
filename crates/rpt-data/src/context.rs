//! [`DataContext`] — the evaluation context a formula sees for one record.
//!
//! Implements [`EvalContext`]: `{field}` resolves from the current
//! [`Row`], `{@formula}` evaluates that formula's body **in this same context** (the "formulas are
//! a service" model), and the print-state specials (`RecordNumber`, `PageNumber`,
//! …) come from an injected snapshot. Formula recursion is cycle-guarded.

use crate::diagnostics::{DiagnosticKind, DiagnosticSink, EvalDiagnostic};
use crate::running_total::RunningTotals;
use crate::source::Row;
use crystal_formula::eval::vm::{self, Chunk};
use crystal_formula::eval::{EvalContext, Value};
use crystal_formula::{RefKind, VarScope};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// A compiled formula (parsed **and** compiled to bytecode once), keyed by lowercase name. Run on
/// the [`vm`] per record.
pub type FormulaRegistry = HashMap<String, Chunk>;

/// The report-lifetime store for Crystal `Global`/`Shared` variables.
///
/// Crystal's `Global` (default) and `Shared` variables retain their value across **every formula
/// and record** of a report run — this is what makes running totals and `WhilePrintingRecords`
/// counters accumulate. The formula VM keeps only `Local` variables per evaluation; it routes a
/// `Global`/`Shared` load/store through the active [`DataContext`] into this store, so one instance
/// shared across the record pass lets those variables build up in record order. Create one per
/// render; interior-mutable so a `&SharedState` threads through the borrow-only evaluation path.
///
/// `Global` and `Shared` are kept in separate maps: within a single (sub)report they behave
/// identically, but `Shared` additionally crosses into subreports — a distinction preserved by
/// [`SharedState::child`], which gives a subreport its own `Global` map while sharing the parent's
/// `Shared` map (the engine shares `Shared` variables across the main↔subreport boundary). The
/// `Shared` map is therefore reference-counted so a parent and its subreports can hold the same
/// storage.
#[derive(Debug, Default)]
pub struct SharedState {
    globals: RefCell<HashMap<String, Value>>,
    shared: Rc<RefCell<HashMap<String, Value>>>,
}

impl SharedState {
    /// A fresh state with empty `Global` and `Shared` variable scopes.
    pub fn new() -> SharedState {
        SharedState::default()
    }

    /// A child (sub)report state: a **fresh** `Global` scope (running totals / global counters reset
    /// per subreport, matching the engine) but the **same** `Shared` scope as `self`, so `Shared`
    /// variables set in the parent are visible in the subreport and vice-versa.
    pub fn child(&self) -> SharedState {
        SharedState {
            globals: RefCell::new(HashMap::new()),
            shared: Rc::clone(&self.shared),
        }
    }

    fn map(&self, scope: VarScope) -> &RefCell<HashMap<String, Value>> {
        match scope {
            VarScope::Shared => &self.shared,
            // `Global` is the default; `Local` never reaches here (the VM keeps it per-run).
            _ => &self.globals,
        }
    }

    /// Current value of a persistent variable (`None` if never assigned/declared).
    pub fn get(&self, scope: VarScope, name: &str) -> Option<Value> {
        self.map(scope).borrow().get(name).cloned()
    }

    /// Store a persistent variable's value.
    pub fn set(&self, scope: VarScope, name: &str, value: Value) {
        self.map(scope).borrow_mut().insert(name.to_string(), value);
    }
}

/// Report parameter values, keyed by [`normalize_param_name`] of the parameter name. Supplied by the
/// caller (CLI/API); a formula's `{?Name}` reference resolves against this.
pub type Parameters = HashMap<String, Value>;

/// Normalize a parameter name for matching: drop surrounding `{}`, a leading `?`, and lowercase — so
/// `{?DocKey@}`, `?DocKey@`, and `dockey@` all key the same value.
pub fn normalize_param_name(name: &str) -> String {
    name.trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim_start_matches('?')
        .to_lowercase()
}

/// The per-record evaluation context.
#[derive(Debug)]
pub struct DataContext<'a> {
    row: &'a Row,
    formulas: &'a FormulaRegistry,
    /// Report parameter values (`{?Name}`), if provided by the caller.
    params: Option<&'a Parameters>,
    /// Print-state specials by lowercase name (`recordnumber`, `pagenumber`, …).
    specials: HashMap<String, Value>,
    /// Names currently being evaluated, to break `{@a}` → `{@b}` → `{@a}` cycles.
    in_progress: RefCell<HashSet<String>>,
    /// Report-lifetime `Global`/`Shared` variable store. When present, running
    /// variables accumulate across records; when absent, the VM keeps them per-evaluation (`Local`).
    state: Option<&'a SharedState>,
    /// Report-lifetime print-order running-total accumulators. When present,
    /// `resolve(RefKind::RunningTotal, name)` returns the value accumulated up to the current record.
    running: Option<&'a RunningTotals>,
    /// Pre-scheduled `BeforeReadingRecords` formula values, keyed by lowercase name.
    /// A formula listed here returns this value instead of re-evaluating — its side-effects already
    /// fired in the scheduled pre-pass, so they fire exactly once.
    scheduled_before: Option<&'a HashMap<String, Value>>,
    /// Pre-scheduled `WhileReadingRecords` formula values for **this record**, keyed
    /// by lowercase name. Same single-fire contract as [`scheduled_before`](Self::scheduled_before).
    scheduled_row: Option<&'a HashMap<String, Value>>,
    /// Per-record memoization of formula values, keyed by lowercase formula name.
    /// A formula is computed once per context and read many times — matching the native engine's
    /// per-object value cache, and, crucially, firing any `Global`/`Shared`
    /// side-effect (a running-total assignment) exactly once per record.
    cache: RefCell<HashMap<String, Value>>,
    /// Optional diagnostics sink. When present, a `{@formula}` that errors reports the failure here
    /// before the fail-open fallback to `Null`; when absent, the failure is silently swallowed.
    sink: Option<&'a dyn DiagnosticSink>,
}

impl<'a> DataContext<'a> {
    /// A context evaluating `{@formula}`s against `row`, resolving names via `formulas`.
    pub fn new(row: &'a Row, formulas: &'a FormulaRegistry) -> DataContext<'a> {
        DataContext {
            row,
            formulas,
            params: None,
            specials: HashMap::new(),
            in_progress: RefCell::new(HashSet::new()),
            state: None,
            running: None,
            scheduled_before: None,
            scheduled_row: None,
            cache: RefCell::new(HashMap::new()),
            sink: None,
        }
    }

    /// Attach a diagnostics sink so a `{@formula}` that fails to evaluate reports the underlying
    /// error before resolving to `Null` (chainable). With no sink the failure is silently swallowed.
    pub fn with_diagnostics(mut self, sink: &'a dyn DiagnosticSink) -> Self {
        self.sink = Some(sink);
        self
    }

    /// Supply report parameter values so `{?Name}` references resolve (chainable).
    pub fn with_params(mut self, params: &'a Parameters) -> Self {
        self.params = Some(params);
        self
    }

    /// Attach the report-lifetime [`SharedState`] so `Global`/`Shared` variables persist across the
    /// record pass (chainable). All record contexts of one render must share the same instance.
    pub fn with_state(mut self, state: &'a SharedState) -> Self {
        self.state = Some(state);
        self
    }

    /// Attach the report-lifetime [`RunningTotals`] so `{#name}` resolves to the value accumulated up
    /// to the current record in print order (chainable).
    pub fn with_running_totals(mut self, running: &'a RunningTotals) -> Self {
        self.running = Some(running);
        self
    }

    /// Attach the pre-scheduled formula values: `before` for `BeforeReadingRecords`
    /// (report-level), `row` for this record's `WhileReadingRecords`. A formula found in either
    /// returns the recorded value rather than re-evaluating, so its side-effects fire once (chainable).
    pub fn with_scheduled(
        mut self,
        before: Option<&'a HashMap<String, Value>>,
        row: Option<&'a HashMap<String, Value>>,
    ) -> Self {
        self.scheduled_before = before;
        self.scheduled_row = row;
        self
    }

    /// Set a print-state special (chainable).
    pub fn with_special(mut self, name: &str, value: Value) -> Self {
        self.specials.insert(name.to_lowercase(), value);
        self
    }

    /// Set the standard record-position specials at once.
    pub fn with_record_number(self, record_number: i64) -> Self {
        self.with_special("recordnumber", Value::Number(record_number as f64))
    }
}

impl EvalContext for DataContext<'_> {
    fn resolve(&self, kind: RefKind, name: &str) -> Option<Value> {
        match kind {
            RefKind::Field => self.row.get(name).cloned(),
            RefKind::Formula => {
                let key = name.to_lowercase();
                // Pre-scheduled value: a `BeforeReading`/`WhileReading` formula was
                // already evaluated (side-effects fired) in the scheduled pre-pass — return its
                // recorded value without re-evaluating, so its side-effects fire exactly once.
                if let Some(v) = self.scheduled_before.and_then(|m| m.get(&key)) {
                    return Some(v.clone());
                }
                if let Some(v) = self.scheduled_row.and_then(|m| m.get(&key)) {
                    return Some(v.clone());
                }
                // Per-record cache: return the already-computed value, so a formula
                // referenced N times in a record evaluates once — and its running-variable writes
                // apply once per record, not once per reference.
                if let Some(v) = self.cache.borrow().get(&key) {
                    return Some(v.clone());
                }
                // Cycle guard: a formula referencing itself (directly or transitively) resolves to
                // Null rather than recursing forever.
                if self.in_progress.borrow().contains(&key) {
                    return Some(Value::Null);
                }
                let chunk = self.formulas.get(&key)?;
                self.in_progress.borrow_mut().insert(key.clone());
                // Fail-open: a formula that errors resolves to Null. When a sink is attached, report
                // the underlying error first so a strict caller can surface the broken formula.
                let result = match vm::run(chunk, self) {
                    Ok(value) => value,
                    Err(err) => {
                        if let Some(sink) = self.sink {
                            sink.report(EvalDiagnostic {
                                kind: DiagnosticKind::Formula,
                                detail: err.to_string(),
                                source: Some(name.to_string()),
                            });
                        }
                        Value::Null
                    }
                };
                self.in_progress.borrow_mut().remove(&key);
                self.cache.borrow_mut().insert(key, result.clone());
                Some(result)
            }
            RefKind::Parameter => self
                .params
                .and_then(|p| p.get(&normalize_param_name(name)).cloned()),
            // A SQL expression (`{%name}`) is evaluated by the database server, so its value arrives
            // as a column in the result set (live fetch: aliased by rpt-query; saved data: stored
            // under the field name). Resolve it like any fetched field.
            RefKind::SqlExpr => self.row.get(name).cloned(),
            // A running total (`{#name}`) resolves to the print-order value accumulated up to the
            // current record. The group-level value (a group header/footer total) is
            // resolved separately by the layout from the group's `#name` summary.
            RefKind::RunningTotal => self.running.and_then(|r| r.get(name)),
        }
    }

    fn special(&self, name: &str) -> Option<Value> {
        self.specials.get(name).cloned()
    }

    fn var_get(&self, scope: VarScope, name: &str) -> Option<Value> {
        self.state.and_then(|s| s.get(scope, name))
    }

    fn var_set(&self, scope: VarScope, name: &str, value: Value) -> bool {
        match self.state {
            Some(s) => {
                s.set(scope, name, value);
                true
            }
            None => false,
        }
    }
}
