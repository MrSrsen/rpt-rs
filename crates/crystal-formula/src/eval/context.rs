//! The evaluator's public error and context surface: [`EvalError`]/[`SpannedEvalError`], the
//! [`NullTreatment`] setting, the [`EvalContext`] resolution trait, and its two ready-made
//! implementations ([`EmptyContext`]/[`MapContext`]).

use std::collections::HashMap;

use crate::ast::VarScope;
use crate::eval::Value;
use crate::token::{RefKind, Span};

/// An evaluation failure. `Unsupported` marks known-but-unimplemented surface (the honest
/// failure mode); the others are genuine formula/runtime errors.
///
/// An `EvalError` carries only its message. A consumer that needs to underline the failing
/// sub-expression (an LSP/playground) evaluates through
/// [`eval_spanned`](crate::eval::eval_spanned)/[`vm::run_spanned`](crate::eval::vm::run_spanned),
/// which pair the same error with the [`Span`](crate::token::Span) of the node that raised it in a
/// [`SpannedEvalError`] — the plain [`eval`](crate::eval::eval)/[`vm::run`](crate::eval::vm::run)
/// path is unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// A recognised builtin/construct the evaluator does not implement yet.
    Unsupported(String),
    /// An unresolved identifier or reference.
    UnknownName(String),
    /// An operator/builtin applied to the wrong value type.
    TypeMismatch {
        /// The operator or builtin that rejected the operand.
        what: String,
        /// The offending value's type name.
        got: String,
    },
    /// Division (or modulo) by zero.
    DivideByZero,
    /// A builtin called with the wrong number of arguments (distinct from a per-argument
    /// [`TypeMismatch`](EvalError::TypeMismatch) or a value-level [`BadArg`](EvalError::BadArg)).
    Arity {
        /// The called builtin (lowercased).
        name: String,
        /// The expected argument-count shape (e.g. `"3 arguments"`, `"at least 1 argument"`).
        expected: String,
        /// The number of arguments actually supplied.
        got: usize,
    },
    /// A bad argument (count, range, unparseable literal…).
    BadArg(String),
    /// An internal invariant of the evaluator was violated — a bug in this crate, not in the formula.
    ///
    /// Reported rather than panicked because formula text comes from an arbitrary `.rpt` and this
    /// crate is meant to be embeddable in an LSP, a validator, or a WASM sandbox, where a panic takes
    /// down the host instead of producing a diagnostic. A `debug_assert!` at the same site still fails
    /// loudly in development.
    Internal(&'static str),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::Unsupported(s) => write!(f, "unsupported: {s}"),
            EvalError::UnknownName(s) => write!(f, "unknown name: {s}"),
            EvalError::TypeMismatch { what, got } => write!(f, "type mismatch in {what}: {got}"),
            EvalError::DivideByZero => write!(f, "division by zero"),
            EvalError::Arity {
                name,
                expected,
                got,
            } => write!(
                f,
                "wrong number of arguments to `{name}`: expected {expected}, got {got}"
            ),
            EvalError::BadArg(s) => write!(f, "bad argument: {s}"),
            EvalError::Internal(s) => {
                write!(f, "internal evaluator error: {s} (please report this)")
            }
        }
    }
}

impl std::error::Error for EvalError {}

/// An [`EvalError`] paired with the source [`Span`] of the node that raised it — the failing
/// sub-expression an editor can underline. Produced by
/// [`eval_spanned`](crate::eval::eval_spanned)/[`vm::run_spanned`](crate::eval::vm::run_spanned); it
/// is a transparent wrapper (the `error` and `span` are public), so a caller that wants only the
/// error reads `.error`. The span is `0..0` when the failing op has no source origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpannedEvalError {
    /// The underlying evaluation failure.
    pub error: EvalError,
    /// The source span of the node whose evaluation failed.
    pub span: Span,
}

impl std::fmt::Display for SpannedEvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (at {}..{})",
            self.error, self.span.start, self.span.end
        )
    }
}

impl std::error::Error for SpannedEvalError {}

impl From<SpannedEvalError> for EvalError {
    fn from(s: SpannedEvalError) -> EvalError {
        s.error
    }
}

/// How a formula treats a **null** database-field operand — Crystal's per-formula "default values for
/// nulls" vs. "exceptions for nulls" editor setting (SDK `CrFormulaNullTreatmentEnum`).
///
/// This crate is independent of the `.rpt` model, so it carries its own copy of the setting; a
/// consumer maps its model enum onto this at the evaluation site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NullTreatment {
    /// A null field propagates (comparisons are false, arithmetic yields null) — the engine default.
    #[default]
    Exception,
    /// A null field is replaced by its type's default value (`0` / `""` / `False` / the zero date)
    /// before the formula runs, so the computation continues over the substituted value.
    DefaultValue,
}

/// Resolves the names a formula pulls from its surroundings: `{...}` references and the 0-ary
/// print-state specials (`PageNumber`, `CurrentDate`, …).
///
/// Returning `None` from [`resolve`](EvalContext::resolve) means *unknown name* (an error);
/// a present-but-null field must return `Some(Value::Null)`.
pub trait EvalContext {
    /// Resolve a `{...}` reference to its current value, or `None` if the name is unknown.
    fn resolve(&self, kind: RefKind, name: &str) -> Option<Value>;

    /// The type-default value substituted for a **null** `{...}` field when the running formula uses
    /// [`NullTreatment::DefaultValue`] — `0` / `""` / `False` / the zero date, chosen from the field's
    /// declared type. Returns `None` when this context cannot supply a typed default (the field then
    /// stays [`Value::Null`]). Only consulted under `DefaultValue`; the default returns `None`.
    fn null_default(&self, kind: RefKind, name: &str) -> Option<Value> {
        let _ = (kind, name);
        None
    }
    /// A print-state special by lowercase name (`"pagenumber"`, `"currentdate"`, …).
    fn special(&self, name: &str) -> Option<Value> {
        let _ = name;
        None
    }

    /// Read a persistent (`Global`/`Shared`) variable's current value, or `None` if it is unset or
    /// this context keeps no persistent store. Crystal's `Global`/`Shared` variables retain their
    /// value across every formula and record of the report run; a report-lifetime context (the data
    /// pipeline's `DataContext`) overrides this so running variables accumulate across the record
    /// pass. The default `None` preserves the pre-persistence behavior — the VM then
    /// keeps the variable in its per-evaluation locals, identical to a single flattened scope.
    fn var_get(&self, scope: VarScope, name: &str) -> Option<Value> {
        let _ = (scope, name);
        None
    }
    /// Write a persistent (`Global`/`Shared`) variable. Returns `true` when a persistent store took
    /// it; `false` (the default) means no store, so the VM keeps the value in its per-run locals.
    fn var_set(&self, scope: VarScope, name: &str, value: Value) -> bool {
        let _ = (scope, name, value);
        false
    }

    /// Resolve a summary-function reference — `Op({field}[, {group}])` where the operand is a `{...}`
    /// reference rather than an array literal — to the report's already-computed group/grand-total
    /// summary value. This is how the engine treats a summary function in a formula body: a reference
    /// to an existing subtotal, not an inline reduction of the current row's scalar.
    ///
    /// `op` is the lowercased operation name as written (`"sum"`, `"count"`, `"avg"`, …); `field` the
    /// summarized field (`table.field`, or `@formula` with its sigil restored); `group` the group
    /// scope's condition field, present only for the 2-argument (group-scoped) form. Returns `None`
    /// when this context has no summary facility at all (the caller then reports the record-set form
    /// as unsupported); a facility that simply finds no matching summary returns `Some(Value::Null)`.
    fn resolve_summary(&self, op: &str, field: &str, group: Option<&str>) -> Option<Value> {
        let _ = (op, field, group);
        None
    }
}

/// A context that resolves nothing — for formulas over literals only.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyContext;

impl EvalContext for EmptyContext {
    fn resolve(&self, _kind: RefKind, _name: &str) -> Option<Value> {
        None
    }
}

/// A map-backed context: field values keyed by `(RefKind, lowercase name)`, specials by
/// lowercase name. The workhorse for tests and row-driven evaluation.
#[derive(Debug, Clone, Default)]
pub struct MapContext {
    /// Field/reference values, keyed by `(RefKind, lowercase name)`.
    pub fields: HashMap<(RefKind, String), Value>,
    /// Print-state special values, keyed by lowercase name.
    pub specials: HashMap<String, Value>,
}

impl MapContext {
    /// Add a reference value, returning `self` for chaining.
    pub fn with_field(mut self, kind: RefKind, name: &str, value: Value) -> Self {
        self.fields.insert((kind, name.to_lowercase()), value);
        self
    }
    /// Add a print-state special value, returning `self` for chaining.
    pub fn with_special(mut self, name: &str, value: Value) -> Self {
        self.specials.insert(name.to_lowercase(), value);
        self
    }
}

impl EvalContext for MapContext {
    fn resolve(&self, kind: RefKind, name: &str) -> Option<Value> {
        self.fields.get(&(kind, name.to_lowercase())).cloned()
    }
    fn special(&self, name: &str) -> Option<Value> {
        self.specials.get(name).cloned()
    }
}
