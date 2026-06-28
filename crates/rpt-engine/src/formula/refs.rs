//! Reference extraction for `UseCount` / parameter-usage counting.
//!
//! Driven by the **token stream** (not the AST parser) so it can never fail on an unparseable
//! construct — every reference is found even if the full parse is partial. Each reference carries the
//! enclosing call's function name and argument index, so the aggregation "group-by argument"
//! exclusion is applied structurally rather than by a comma heuristic.

use super::lexer::tokenize;
use super::token::{op, RefKind, Syntax, TokenKind};

/// One reference occurrence found in a formula body, with its enclosing-call context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    /// What the `{...}` refers to (field / parameter / formula / running total / SQL expr).
    pub kind: RefKind,
    /// The sigil-stripped inner name (e.g. `Command.drug_name`, the formula/param name).
    pub name: String,
    /// The function identifier immediately before the innermost enclosing `(`, if any
    /// (skipping whitespace/newlines/comments). `None` when the reference is not inside a call.
    pub enclosing_fn: Option<String>,
    /// Number of top-level commas of the innermost enclosing call seen before this reference
    /// (so `0` = first argument, `>= 1` = a later argument).
    pub arg_index: usize,
}

impl Ref {
    /// Whether this occurrence is the **2nd-or-later argument of an aggregation call** and so
    /// contributes **0** to `UseCount`. The engine resolves an aggregation's group-by argument to an
    /// existing group and creates no persistent field reference.
    pub fn is_aggregation_group_arg(&self) -> bool {
        self.arg_index >= 1
            && self
                .enclosing_fn
                .as_deref()
                .is_some_and(is_aggregation_function)
    }

    /// Whether this occurrence is the argument of a `GroupName(...)` call. `GroupName({field})`
    /// resolves `{field}` to its group and returns the group's name; the engine treats it as a group
    /// selector. When the field is *also* referenced as a displayed value the selector creates no new
    /// persistent field reference (so it contributes 0), but when the field's only value use is via
    /// `GroupName` it does count — the caller decides using the field's value-reference set.
    pub fn is_group_name_arg(&self) -> bool {
        self.enclosing_fn
            .as_deref()
            .is_some_and(|f| f.eq_ignore_ascii_case("groupname"))
    }
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
    // Per enclosing-paren frame: (function-name-before-`(`, top-level comma count so far).
    struct Frame {
        fn_name: Option<String>,
        commas: usize,
    }
    let mut stack: Vec<Frame> = Vec::new();
    // The last identifier seen, skipping whitespace/newlines/comments — the candidate function
    // name when the next significant token is `(`.
    let mut prev_ident: Option<String> = None;
    for t in &toks {
        match &t.kind {
            TokenKind::Op(op::LPAREN) => {
                stack.push(Frame {
                    fn_name: prev_ident.take(),
                    commas: 0,
                });
            }
            TokenKind::Op(op::RPAREN) => {
                stack.pop();
                prev_ident = None;
            }
            TokenKind::Op(op::COMMA) => {
                if let Some(f) = stack.last_mut() {
                    f.commas += 1;
                }
                prev_ident = None;
            }
            TokenKind::Reference(rk) => {
                let (enclosing_fn, arg_index) = stack
                    .last()
                    .map(|f| (f.fn_name.clone(), f.commas))
                    .unwrap_or((None, 0));
                out.push(Ref {
                    kind: *rk,
                    name: t.text.clone(),
                    enclosing_fn,
                    arg_index,
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

/// Crystal summary/aggregation functions (case-insensitive). A field that appears as the
/// 2nd-or-later argument of one of these is the group selector, not a value dependency; the first
/// argument (the summarized field) counts.
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

/// The names of every formula (`{@name}`) referenced in `body` (Crystal syntax). Used to drive
/// formula-liveness: a `{@name}` token inside a string literal or `//` comment is correctly
/// *not* yielded.
pub fn formula_names(body: &str) -> impl Iterator<Item = String> + '_ {
    references(body)
        .into_iter()
        .filter(|r| r.kind == RefKind::Formula)
        .map(|r| r.name)
}

/// The names of every parameter (`{?name}`) referenced in `body` (Crystal syntax).
pub fn parameter_names(body: &str) -> impl Iterator<Item = String> + '_ {
    references(body)
        .into_iter()
        .filter(|r| r.kind == RefKind::Parameter)
        .map(|r| r.name)
}
