//! Reference extraction: every `{...}` a formula body names, with its enclosing-call context.
//!
//! Driven by the **token stream** (not the AST parser) so it can never fail on an unparseable
//! construct — every reference is found even if the full parse is partial. Each reference carries
//! the name of the call it sits inside, so a caller can tell `{f}` used as a value from `{f}`
//! handed to an aggregation.

use super::lexer::tokenize;
use super::token::{op, RefKind, Syntax, TokenKind};

/// One reference occurrence found in a formula body, with its enclosing-call context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    /// What the `{...}` refers to (field / parameter / formula / running total / SQL expr).
    pub kind: RefKind,
    /// The sigil-stripped inner name (e.g. `Command.some_field`, the formula/param name).
    pub name: String,
    /// The function identifier immediately before the innermost enclosing `(`, if any
    /// (skipping whitespace/newlines/comments). `None` when the reference is not inside a call.
    pub enclosing_fn: Option<String>,
}

/// Extract every reference in `body` (Crystal syntax — the engine treats all stored bodies as
/// Crystal for counting). See [`references_with_syntax`] to choose Basic.
pub fn references(body: &str) -> Vec<Ref> {
    references_with_syntax(body, Syntax::Crystal)
}

/// Extract every reference in `body` under the given `syntax`.
pub fn references_with_syntax(body: &str, syntax: Syntax) -> Vec<Ref> {
    let toks = tokenize(body, syntax);
    let mut out = Vec::new();
    // One frame per open paren, holding the function name that preceded it (if any).
    let mut stack: Vec<Option<String>> = Vec::new();
    // The last identifier seen, skipping whitespace/newlines/comments — the candidate function
    // name when the next significant token is `(`.
    let mut prev_ident: Option<String> = None;
    for t in &toks {
        match &t.kind {
            TokenKind::Op(op::LPAREN) => stack.push(prev_ident.take()),
            TokenKind::Op(op::RPAREN) => {
                stack.pop();
                prev_ident = None;
            }
            TokenKind::Reference(rk) => {
                out.push(Ref {
                    kind: *rk,
                    name: t.text.clone(),
                    enclosing_fn: stack.last().cloned().flatten(),
                });
                prev_ident = None;
            }
            TokenKind::Ident => prev_ident = Some(t.text.clone()),
            // Whitespace-equivalent tokens between an identifier and `(` must not break the
            // function-name association (the engine skips them too).
            TokenKind::Newline | TokenKind::Comment => {}
            _ => prev_ident = None,
        }
    }
    out
}

/// Whether `body` invokes a summary/aggregation function over a `{...}` reference — `Sum({field})`,
/// `Count({field}, {group})`, etc. Such a formula reads a report subtotal, which only exists during
/// the print pass, so its evaluation time must be `WhilePrintingRecords` (see
/// `rpt_data::classify_eval_time`). The array-literal aggregate form (`Sum([1, 2, 3])`) holds no
/// reference and so is not a summary. Token-driven, so an unparseable body still classifies.
pub fn has_summary_function(body: &str) -> bool {
    references(body).iter().any(|r| {
        r.enclosing_fn
            .as_deref()
            .is_some_and(is_aggregation_function)
    })
}

/// Crystal summary/aggregation functions (case-insensitive). The first argument is the summarized
/// field; a 2nd-or-later `{...}` argument is the group selector, not a value dependency.
pub fn is_aggregation_function(name: &str) -> bool {
    AGGREGATION_FUNCTIONS
        .iter()
        .any(|f| name.eq_ignore_ascii_case(f))
}

const AGGREGATION_FUNCTIONS: &[&str] = &[
    "sum",
    "average",
    "count",
    "distinctcount",
    "maximum",
    "minimum",
    "stddev",
    "populationstddev",
    "variance",
    "populationvariance",
    "median",
    "mode",
    "nthlargest",
    "nthsmallest",
    "nthmostfrequent",
    "percentofsum",
    "percentofaverage",
    "percentofcount",
    "percentofmaximum",
    "percentofminimum",
    "percentofdistinctcount",
    "correlation",
    "covariance",
    "weightedaverage",
];
