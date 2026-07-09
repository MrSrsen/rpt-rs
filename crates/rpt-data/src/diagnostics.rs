//! Optional diagnostics for the otherwise fail-open pipeline.
//!
//! The record pipeline is deliberately **fail-open**: a formula or selection that errors (or returns
//! an unexpected type) is swallowed — a detail row is dropped, a group is kept, a `{@formula}`
//! resolves to `Null` — so one broken formula never aborts a whole render. The cost is that a broken
//! formula is invisible. A caller that wants to *see* those failures (a `--strict` CLI, an LSP/
//! validator) supplies a [`DiagnosticSink`]; each fail-open site reports the swallowed failure to it
//! **before** applying the fallback. With no sink the behavior is byte-identical to before.

use std::cell::{Ref, RefCell};

/// Which fail-open site produced a swallowed failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    /// The record-selection formula errored or returned a non-boolean — the row was **dropped**.
    RecordSelection,
    /// The group-selection formula errored or returned a non-boolean — the group was **kept**.
    GroupSelection,
    /// A `{@formula}` field failed to evaluate — it resolved to `Null`.
    Formula,
}

/// One swallowed evaluation failure the pipeline would otherwise hide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalDiagnostic {
    /// Which fail-open site produced it.
    pub kind: DiagnosticKind,
    /// The underlying failure: an [`EvalError`](crystal_formula::eval::EvalError) message, or a note
    /// that the result was a non-boolean value.
    pub detail: String,
    /// The formula/selection involved: a formula field's name for [`DiagnosticKind::Formula`], or
    /// `None` for a selection formula (which has no name of its own).
    pub source: Option<String>,
}

/// A collector the pipeline reports swallowed failures to. Interior-mutable on the impl side, so a
/// shared `&dyn DiagnosticSink` threads through the borrow-only evaluation path.
pub trait DiagnosticSink: std::fmt::Debug {
    /// Record one swallowed failure.
    fn report(&self, diagnostic: EvalDiagnostic);
}

/// A simple [`DiagnosticSink`] that collects every reported diagnostic into a `Vec`, in report order.
#[derive(Debug, Default)]
pub struct CollectingSink {
    diagnostics: RefCell<Vec<EvalDiagnostic>>,
}

impl CollectingSink {
    /// A new, empty collector.
    pub fn new() -> CollectingSink {
        CollectingSink::default()
    }

    /// Borrow the diagnostics collected so far, in report order.
    pub fn diagnostics(&self) -> Ref<'_, Vec<EvalDiagnostic>> {
        self.diagnostics.borrow()
    }

    /// Consume the collector, returning the diagnostics in report order.
    pub fn into_diagnostics(self) -> Vec<EvalDiagnostic> {
        self.diagnostics.into_inner()
    }

    /// The number of diagnostics collected so far.
    pub fn len(&self) -> usize {
        self.diagnostics.borrow().len()
    }

    /// Whether no diagnostic has been reported yet.
    pub fn is_empty(&self) -> bool {
        self.diagnostics.borrow().is_empty()
    }
}

impl DiagnosticSink for CollectingSink {
    fn report(&self, diagnostic: EvalDiagnostic) {
        self.diagnostics.borrow_mut().push(diagnostic);
    }
}
