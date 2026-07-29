//! [`DataContext`] — the evaluation context a formula sees for one record.
//!
//! Implements [`EvalContext`]: `{field}` resolves from the current
//! [`Row`], `{@formula}` evaluates that formula's body **in this same context** (the "formulas are
//! a service" model), and the print-state specials (`RecordNumber`, `PageNumber`,
//! …) come from an injected snapshot. Formula recursion is cycle-guarded.

use crate::diagnostics::{DiagnosticKind, DiagnosticSink, EvalDiagnostic};
use crate::running_total::RunningTotals;
use crate::source::Row;
use rpt_formula::eval::vm::{self, Chunk};
use rpt_formula::eval::{Date, EvalContext, NullTreatment, Time, Value};
use rpt_formula::token::{split_reference, strip_braces};
use rpt_formula::{RefKind, VarScope};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// A compiled formula (parsed **and** compiled to bytecode once) plus how it treats a null field
/// operand. Run on the [`vm`] per record via [`vm::run_with`], threading [`null_treatment`] so a
/// formula authored with "default values for nulls" substitutes type defaults for null fields.
#[derive(Debug, Clone)]
pub struct CompiledFormula {
    /// The compiled bytecode.
    pub chunk: Chunk,
    /// The formula's per-formula null-treatment setting.
    pub null_treatment: NullTreatment,
}

/// The report-lifetime date/time specials (`CurrentDate`/`Today`, `CurrentDateTime`, `CurrentTime`)
/// resolved from a single "as-of" instant captured once per render. Injecting one fixed instant —
/// rather than reading the clock per cell — keeps a render deterministic and reproducible (frozen
/// Page-IR baselines) while still letting a report's date-relative formulas evaluate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTimeSpecials {
    date: Date,
    time: Time,
}

impl DateTimeSpecials {
    /// The specials for an explicit as-of calendar instant.
    pub fn new(date: Date, time: Time) -> DateTimeSpecials {
        DateTimeSpecials { date, time }
    }

    /// The specials for an instant given as whole seconds since the Unix epoch, interpreted as UTC
    /// (the caller captures the clock once at render start and passes the value in — the render core
    /// stays clock-free and WASM-safe).
    pub fn from_unix_seconds(secs: i64) -> DateTimeSpecials {
        DateTimeSpecials::new(
            Date::from_days(secs.div_euclid(86_400)),
            Time::from_seconds(secs),
        )
    }

    /// Resolve a data-time special by its lowercase name, or `None` if `name` is not one of them.
    /// `Today` aliases `CurrentDate`, matching the engine.
    pub(crate) fn resolve(&self, name: &str) -> Option<Value> {
        match name {
            "currentdate" | "today" => Some(Value::Date(self.date)),
            "currentdatetime" => Some(Value::DateTime(self.date, self.time)),
            "currenttime" => Some(Value::Time(self.time)),
            _ => None,
        }
    }
}

/// The compiled formulas of a (sub)report, keyed by lowercase name, plus the field type-defaults a
/// formula substitutes for a null field under [`NullTreatment::DefaultValue`]. Built once by
/// [`compile_formulas`](crate::compile_formulas) and shared across the record pass.
#[derive(Debug, Clone, Default)]
pub struct FormulaRegistry {
    formulas: HashMap<String, CompiledFormula>,
    /// Field name (full and short, lowercased) → the type-default [`Value`] to substitute for a null
    /// field under `DefaultValue` null-treatment.
    field_defaults: HashMap<String, Value>,
    /// The render's as-of date/time specials, injected once so every context built from this registry
    /// resolves `CurrentDate`/`Today`/`CurrentDateTime`/`CurrentTime`. `None` = not supplied (the
    /// offline/inspection paths that never set an as-of leave these specials unresolved).
    datetime: Option<DateTimeSpecials>,
}

impl FormulaRegistry {
    /// An empty registry.
    pub fn new() -> FormulaRegistry {
        FormulaRegistry::default()
    }

    /// Insert a compiled formula under its lowercase `name`.
    pub fn insert(&mut self, name: String, formula: CompiledFormula) {
        self.formulas.insert(name, formula);
    }

    /// The compiled formula for `name` (lowercase), if registered.
    pub fn get(&self, name: &str) -> Option<&CompiledFormula> {
        self.formulas.get(name)
    }

    /// Whether a formula named `name` (lowercase) is registered.
    pub fn contains_key(&self, name: &str) -> bool {
        self.formulas.contains_key(name)
    }

    /// The number of registered formulas.
    pub fn len(&self) -> usize {
        self.formulas.len()
    }

    /// Whether no formulas are registered.
    pub fn is_empty(&self) -> bool {
        self.formulas.is_empty()
    }

    /// Register a field's type-default value under both its full and short (post-last-`.`) names.
    pub fn set_field_default(&mut self, name: &str, default: Value) {
        let lname = name.to_lowercase();
        let short = short_name(&lname);
        if short != lname {
            self.field_defaults
                .entry(short)
                .or_insert_with(|| default.clone());
        }
        self.field_defaults.insert(lname, default);
    }

    /// Attach the render's as-of date/time specials (chainable), so every [`DataContext`] built from
    /// this registry resolves `CurrentDate`/`Today`/`CurrentDateTime`/`CurrentTime`.
    pub fn with_datetime(mut self, datetime: DateTimeSpecials) -> FormulaRegistry {
        self.datetime = Some(datetime);
        self
    }

    /// The as-of date/time specials attached to this registry, if any. A child (sub)report registry
    /// is built with the same instant so its date-relative formulas share the parent's as-of.
    pub fn datetime(&self) -> Option<DateTimeSpecials> {
        self.datetime
    }

    /// Resolve a data-time special (`CurrentDate`/…) from the attached as-of instant, or `None` when
    /// `name` is not a data-time special or no as-of was supplied.
    fn datetime_special(&self, name: &str) -> Option<Value> {
        self.datetime.and_then(|d| d.resolve(name))
    }

    /// The type-default for a null field, if known (see [`EvalContext::null_default`]).
    fn field_default(&self, name: &str) -> Option<Value> {
        let lname = name.to_lowercase();
        self.field_defaults
            .get(&lname)
            .or_else(|| self.field_defaults.get(&short_name(&lname)))
            .cloned()
    }
}

/// The bare field name after the last `.` (`countries.id` → `id`).
fn short_name(name: &str) -> String {
    name.rsplit('.').next().unwrap_or(name).to_string()
}

/// Lower-case `name` into `buf` (cleared first), matching [`str::to_lowercase`]. An ASCII name — the
/// common case on the record-eval hot path — lower-cases in place with no intermediate allocation; a
/// non-ASCII name (rare) falls back to [`str::to_lowercase`] for full Unicode correctness.
fn lower_into(buf: &mut String, name: &str) {
    buf.clear();
    if name.is_ascii() {
        buf.push_str(name);
        buf.make_ascii_lowercase();
    } else {
        buf.push_str(&name.to_lowercase());
    }
}

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

/// Resolves a summary-function reference (`Sum({field}[, {group}])` inside a formula body) to the
/// report's computed group/grand-total summary value. The record pipeline computes the summary tree
/// but the *in-scope* summaries at a print position live in the layout's resolve state, so the layout
/// injects an implementor into the [`DataContext`]; a formula's summary function then resolves to the
/// same value a placed summary object would. `op` is the lowercased operation as written, `field` the
/// summarized field (with a formula's `@` restored), `group` the group scope for the 2-argument form.
pub trait SummaryScope: std::fmt::Debug {
    /// The summary value for `op` over `field` in scope `group`, or `Value::Null` when no matching
    /// summary is in scope (the facility exists, so this is never `None`).
    fn resolve_summary(&self, op: &str, field: &str, group: Option<&str>) -> Value;
}

/// Resolves a group-name reference (`GroupName({condition field})` inside a formula body) to the
/// named group's key. Injected by the layout for the same reason as [`SummaryScope`]: the record
/// pipeline builds the group tree, but *which* group is in scope at a print position is layout state,
/// so a formula's `GroupName` resolves to the value a placed Group Name field would print beside it.
pub trait GroupNameScope: std::fmt::Debug {
    /// The key of the group whose condition is `field`, or `Value::Null` when no group in scope has
    /// that condition (the facility exists, so this is never `None`).
    fn group_name(&self, field: &str) -> Value;
}

/// Normalize a parameter name for matching: drop surrounding `{}`, a leading `?`, and lowercase — so
/// `{?DocKey@}`, `?DocKey@`, and `dockey@` all key the same value.
pub fn normalize_param_name(name: &str) -> String {
    split_reference(strip_braces(name)).1.to_lowercase()
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
    /// Optional in-scope summary resolver. When present, a summary function in a formula body
    /// (`Count({f}, {g})`) resolves to the report's computed summary; when absent, the record-set
    /// summary form is reported as unsupported (a bare evaluation with no summaries).
    summaries: Option<&'a dyn SummaryScope>,
    /// Optional in-scope group resolver. When present, `GroupName({cond})` in a formula body resolves
    /// to the named group's key; when absent, the reference is reported as unsupported (a bare
    /// evaluation with no report grouping).
    group_names: Option<&'a dyn GroupNameScope>,
    /// Reusable lower-casing buffer for name-keyed lookups. A formula/parameter reference lower-cases
    /// its name into this buffer and probes the caches with a borrowed key, so a cache **hit** on the
    /// record-eval hot path allocates nothing; only a miss clones an owned key for the caches.
    scratch: RefCell<String>,
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
            summaries: None,
            group_names: None,
            scratch: RefCell::new(String::new()),
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

    /// Attach the in-scope summary resolver so a summary function in a formula body
    /// (`Count({f}, {g})`) resolves against the report's computed summaries (chainable).
    pub fn with_summaries(mut self, summaries: &'a dyn SummaryScope) -> Self {
        self.summaries = Some(summaries);
        self
    }

    /// Attach the in-scope group resolver so `GroupName({cond})` in a formula body resolves to the
    /// named group's key (chainable).
    pub fn with_group_names(mut self, group_names: &'a dyn GroupNameScope) -> Self {
        self.group_names = Some(group_names);
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
                // Lower-case into the reusable scratch buffer and probe every cache with the borrowed
                // key, so a hit (the common case — a formula referenced N times per record) allocates
                // nothing. Only the miss path below clones an owned key for the caches.
                let mut scratch = self.scratch.borrow_mut();
                lower_into(&mut scratch, name);
                let key: &str = scratch.as_str();
                // Pre-scheduled value: a `BeforeReading`/`WhileReading` formula was
                // already evaluated (side-effects fired) in the scheduled pre-pass — return its
                // recorded value without re-evaluating, so its side-effects fire exactly once.
                if let Some(v) = self.scheduled_before.and_then(|m| m.get(key)) {
                    return Some(v.clone());
                }
                if let Some(v) = self.scheduled_row.and_then(|m| m.get(key)) {
                    return Some(v.clone());
                }
                // Per-record cache: return the already-computed value, so a formula
                // referenced N times in a record evaluates once — and its running-variable writes
                // apply once per record, not once per reference.
                if let Some(v) = self.cache.borrow().get(key) {
                    return Some(v.clone());
                }
                // Cycle guard: a formula referencing itself (directly or transitively) resolves to
                // Null rather than recursing forever.
                if self.in_progress.borrow().contains(key) {
                    return Some(Value::Null);
                }
                let compiled = self.formulas.get(key)?;
                // Cache miss: take the one owned key the in-progress set and cache need, then release
                // the scratch borrow before evaluating (the VM re-enters `resolve` and reuses it).
                let key = key.to_string();
                drop(scratch);
                self.in_progress.borrow_mut().insert(key.clone());
                // Fail-open: a formula that errors resolves to Null. When a sink is attached, report
                // the underlying error first so a strict caller can surface the broken formula. The
                // formula's null-treatment decides whether a null field it reads propagates or is
                // replaced by its type default.
                let result = match vm::run_with(&compiled.chunk, self, compiled.null_treatment) {
                    Ok(value) => value,
                    Err(err) => {
                        if let Some(sink) = self.sink {
                            // No record index here: the evaluation context holds the row, not its
                            // position in the set.
                            sink.report(
                                EvalDiagnostic::new(DiagnosticKind::Formula, err.to_string())
                                    .from_source(name),
                            );
                        }
                        Value::Null
                    }
                };
                self.in_progress.borrow_mut().remove(&key);
                self.cache.borrow_mut().insert(key, result.clone());
                Some(result)
            }
            RefKind::Parameter => {
                let params = self.params?;
                // Same normalization as `normalize_param_name`, but lower-cased into the scratch
                // buffer so the lookup probes with a borrowed key instead of a fresh String.
                let raw = split_reference(strip_braces(name)).1;
                let mut scratch = self.scratch.borrow_mut();
                lower_into(&mut scratch, raw);
                params.get(scratch.as_str()).cloned()
            }
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

    fn null_default(&self, kind: RefKind, name: &str) -> Option<Value> {
        // Only a database field (or a server-evaluated SQL expression, fetched as a field) is
        // null-converted; a null formula/parameter/running-total is left as-is.
        match kind {
            RefKind::Field | RefKind::SqlExpr => self.formulas.field_default(name),
            _ => None,
        }
    }

    fn special(&self, name: &str) -> Option<Value> {
        // Per-position specials (record/page number) take precedence; the report-lifetime data-time
        // specials (`CurrentDate`/…) come from the registry, so every context resolves them without a
        // per-site injection.
        self.specials
            .get(name)
            .cloned()
            .or_else(|| self.formulas.datetime_special(name))
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

    fn resolve_summary(&self, op: &str, field: &str, group: Option<&str>) -> Option<Value> {
        // With an attached scope, a missing summary resolves to Null (the scope answers `Some`); with
        // no scope, `None` lets the evaluator report the record-set form as unsupported.
        self.summaries.map(|s| s.resolve_summary(op, field, group))
    }

    fn group_name(&self, field: &str) -> Option<Value> {
        // Same contract as `resolve_summary`: an attached scope always answers, and its absence is
        // what tells the evaluator the reference is unresolvable here.
        self.group_names.map(|g| g.group_name(field))
    }
}
