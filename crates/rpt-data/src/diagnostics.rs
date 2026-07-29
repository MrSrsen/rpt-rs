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
    /// The record selection kept **no rows at all** out of a non-empty input. Reported once, as a
    /// summary. Not itself a failure — a selection is allowed to exclude everything — but it is the
    /// explanation for an empty report, which otherwise looks like "there was no data".
    AllRowsExcluded,
    /// The group-selection formula errored or returned a non-boolean — the group was **kept**.
    GroupSelection,
    /// A `{@formula}` field failed to evaluate — it resolved to `Null`.
    Formula,
    /// A column's cells would not parse as their declared type, so a different type was substituted.
    /// Reported once per column, not per row.
    TypeCoercion,
    /// A formula did not **parse**. It was compiled from the parser's partial recovery AST and
    /// evaluated anyway, so whatever it produced is meaningless. Reported once per formula at compile
    /// time, not per row — and unlike a runtime error, this is one the user can fix by editing the
    /// report.
    FormulaParse,
    /// A group's grouping condition ([`GroupCondition`](rpt_model::GroupCondition)) is an unrecognized
    /// ordinal the pipeline can neither bucket as a date/time period nor treat as one of the six
    /// boolean conditions — rows are grouped by the field's **raw** value instead. (The date/time
    /// periods bucket by period and the boolean conditions group sequentially; only an unknown ordinal
    /// falls back and raises this.)
    UnsupportedGroupCondition,
}

/// One swallowed evaluation failure the pipeline would otherwise hide.
///
/// `record_index` and `span` locate the failure. Both are best-effort and never fabricated: a
/// per-formula failure has no record, and a failure the evaluator reported without a span has no
/// span. A consumer renders only what is present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalDiagnostic {
    /// Which fail-open site produced it.
    pub kind: DiagnosticKind,
    /// The underlying failure: an [`EvalError`](rpt_formula::eval::EvalError) message, or a note
    /// that the result was a non-boolean value.
    pub detail: String,
    /// The formula/selection involved: a formula field's name for [`DiagnosticKind::Formula`], or
    /// `None` for a selection formula (which has no name of its own).
    pub source: Option<String>,
    /// 0-based index of the record being evaluated, for a per-row failure. The same formula runs on
    /// every row, so without this a user cannot tell one bad row from a systematically broken formula.
    pub record_index: Option<u64>,
    /// Byte range within the formula text the failure points at, when the evaluator reported one.
    pub span: Option<std::ops::Range<usize>>,
}

impl EvalDiagnostic {
    /// A diagnostic of `kind` described by `detail`, with no location yet.
    pub fn new(kind: DiagnosticKind, detail: impl Into<String>) -> EvalDiagnostic {
        EvalDiagnostic {
            kind,
            detail: detail.into(),
            source: None,
            record_index: None,
            span: None,
        }
    }

    /// Name the formula or selection involved.
    pub fn from_source(mut self, source: impl Into<String>) -> EvalDiagnostic {
        self.source = Some(source.into());
        self
    }

    /// Note the record being evaluated when this failed.
    pub fn at_record(mut self, record_index: u64) -> EvalDiagnostic {
        self.record_index = Some(record_index);
        self
    }

    /// Note the byte range within the formula text this points at.
    pub fn at_span(mut self, span: std::ops::Range<usize>) -> EvalDiagnostic {
        self.span = Some(span);
        self
    }
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

/// Report a formula's parse diagnostics to `sink`, if any and if there is a sink.
///
/// Called once per formula at compile time. A parse error is worth saying out loud even though the
/// pipeline carries on with the recovery AST: the resulting value is meaningless, and unlike a runtime
/// type error it is something the report's author can actually fix.
///
/// `label` names the formula ("the record selection", a formula field's name); `src` is its body,
/// used to quote the offending text.
pub fn report_parse_diagnostics(
    sink: Option<&dyn DiagnosticSink>,
    label: &str,
    src: &str,
    diagnostics: &[rpt_formula::Diagnostic],
) {
    let Some(sink) = sink else { return };
    for d in diagnostics {
        let detail = match excerpt(src, d.start, d.end) {
            Some(quoted) => format!(
                "{label}: {} at byte {} (near `{quoted}`); the formula was evaluated from a partial \
                 parse, so its value is not meaningful",
                d.message, d.start
            ),
            None => format!(
                "{label}: {} at byte {}; the formula was evaluated from a partial parse, so its \
                 value is not meaningful",
                d.message, d.start
            ),
        };
        sink.report(
            EvalDiagnostic::new(DiagnosticKind::FormulaParse, detail)
                .from_source(label)
                .at_span(d.start..d.end),
        );
    }
}

/// The source text a diagnostic's span covers, trimmed and capped, or `None` when the span is empty
/// or does not land on character boundaries. Formula bodies are short, so quoting the offending text
/// is more use than a line/column pair.
fn excerpt(src: &str, start: usize, end: usize) -> Option<String> {
    const MAX: usize = 40;
    let end = end.min(src.len());
    if start >= end || !src.is_char_boundary(start) || !src.is_char_boundary(end) {
        return None;
    }
    let text = src[start..end].trim();
    if text.is_empty() {
        return None;
    }
    Some(match text.char_indices().nth(MAX) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    })
}
