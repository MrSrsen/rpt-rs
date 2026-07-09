//! Semantic **validation** of a parsed formula — the diagnostics the [`parser`](super::parser)
//! doesn't produce.
//!
//! The parser only reports *syntactic* recovery (an unexpected token). This pass walks a
//! (successfully or partially) parsed [`Node`] tree and reports *semantic* problems an editor or
//! LSP wants to surface:
//!
//! 1. **Unknown / misspelled built-in functions** — a call whose name is in neither the built-in
//!    function table nor the caller-supplied custom-function set (with a nearest-name suggestion).
//! 2. **Function arity** — a built-in called with a structurally wrong number of arguments, for the
//!    functions whose argument shape the type system encodes (`IIf`/`Switch`/`Choose` and the
//!    aggregate/argument-copying families). The engine's per-function signatures aren't tabulated,
//!    so functions with a fixed/opaque return rule are not arity-checked.
//! 3. **Operator type errors** — a binary/unary operator applied to statically-known incompatible
//!    operand types (e.g. arithmetic on a `String`), via [`deduce_type`].
//! 4. **Unknown references** — a `{field}` / `{?param}` / `{@formula}` / `{#rt}` / `{%sql}` whose
//!    name isn't in the corresponding [`ValidationContext`] set. Checked only when the caller
//!    supplies that set, so the crate stays standalone (with no context, only the intrinsic checks
//!    1–3 run).
//!
//! This is an additive, parity-neutral pass: it produces diagnostics and touches neither the
//! reference-counting ([`refs`](super::refs)) nor the evaluation paths.
//!
//! ## Spans
//!
//! The [`Node`] tree carries no source spans, so precise spans come from the token stream:
//! [`validate_str`] tokenizes the source and points each name/reference diagnostic at the exact
//! offending token. The AST-only [`validate`] entry cannot recover offsets and reports every
//! diagnostic against a whole-formula span — use it when only an AST is available; prefer
//! [`validate_str`] (which an LSP always can, since it has the source) for editor-quality spans.
//! Operator diagnostics use a whole-formula span in both entries (the AST doesn't identify which
//! operator token produced the type error).

use std::collections::HashSet;

use super::ast::Node;
use super::parser::{Diagnostic, Severity};
use super::token::{op, RefKind, Syntax, TokenKind};
use super::types::{deduce_type, func_id, ResultKind};

/// The known-symbol sets a caller supplies so cross-reference checks can run. Every set is optional:
/// a `None` set disables the corresponding unknown-reference check (so an empty, `default()` context
/// runs only the intrinsic function/arity/operator checks). Names are matched case-insensitively.
#[derive(Debug, Clone, Default)]
pub struct ValidationContext {
    fields: Option<HashSet<String>>,
    parameters: Option<HashSet<String>>,
    formulas: Option<HashSet<String>>,
    running_totals: Option<HashSet<String>>,
    sql_expressions: Option<HashSet<String>>,
    functions: Option<HashSet<String>>,
}

fn lower_set<I, S>(names: I) -> HashSet<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    names
        .into_iter()
        .map(|n| n.into().to_ascii_lowercase())
        .collect()
}

impl ValidationContext {
    /// An empty context: unknown-reference checks are all disabled; only the intrinsic checks run.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare the known database-field names (`{table.field}`).
    pub fn with_fields<I: IntoIterator<Item = S>, S: Into<String>>(mut self, names: I) -> Self {
        self.fields = Some(lower_set(names));
        self
    }

    /// Declare the known parameter names (`{?name}`).
    pub fn with_parameters<I: IntoIterator<Item = S>, S: Into<String>>(mut self, names: I) -> Self {
        self.parameters = Some(lower_set(names));
        self
    }

    /// Declare the known formula names (`{@name}`).
    pub fn with_formulas<I: IntoIterator<Item = S>, S: Into<String>>(mut self, names: I) -> Self {
        self.formulas = Some(lower_set(names));
        self
    }

    /// Declare the known running-total names (`{#name}`).
    pub fn with_running_totals<I: IntoIterator<Item = S>, S: Into<String>>(
        mut self,
        names: I,
    ) -> Self {
        self.running_totals = Some(lower_set(names));
        self
    }

    /// Declare the known SQL-expression names (`{%name}`).
    pub fn with_sql_expressions<I: IntoIterator<Item = S>, S: Into<String>>(
        mut self,
        names: I,
    ) -> Self {
        self.sql_expressions = Some(lower_set(names));
        self
    }

    /// Declare known **custom** function names (in addition to the built-ins). When set, a call to a
    /// name that is neither a built-in nor in this set is a hard error; when unset, such a call is a
    /// warning (it may be a custom function the caller didn't declare).
    pub fn with_functions<I: IntoIterator<Item = S>, S: Into<String>>(mut self, names: I) -> Self {
        self.functions = Some(lower_set(names));
        self
    }

    /// The declared name set for a reference kind, if the caller supplied one.
    fn set_for(&self, kind: RefKind) -> Option<&HashSet<String>> {
        match kind {
            RefKind::Field => self.fields.as_ref(),
            RefKind::Parameter => self.parameters.as_ref(),
            RefKind::Formula => self.formulas.as_ref(),
            RefKind::RunningTotal => self.running_totals.as_ref(),
            RefKind::SqlExpr => self.sql_expressions.as_ref(),
        }
    }
}

/// Which token a diagnostic points at, resolved against the source in [`validate_str`].
#[derive(Debug, Clone)]
enum Locate {
    /// The next identifier token whose text matches (case-insensitively) — a call's callee name.
    Ident(String),
    /// The next reference token with this kind and name.
    Reference(RefKind, String),
    /// No specific token; the whole formula.
    Whole,
}

/// An un-located diagnostic: its message/severity plus the token to point at.
#[derive(Debug, Clone)]
struct Raw {
    message: String,
    severity: Severity,
    locate: Locate,
}

/// Validate a parsed formula against `ctx`, returning semantic diagnostics.
///
/// The AST carries no spans, so every diagnostic here is reported against a whole-formula span
/// (`0..0`). Prefer [`validate_str`] when the source text is available — it points name and
/// reference diagnostics at their exact tokens. See the [module docs](self#spans).
pub fn validate(node: &Node, ctx: &ValidationContext) -> Vec<Diagnostic> {
    let mut raws = Vec::new();
    Walker {
        ctx,
        out: &mut raws,
    }
    .visit(node);
    raws.into_iter()
        .map(|r| Diagnostic {
            message: r.message,
            start: 0,
            end: 0,
            severity: r.severity,
        })
        .collect()
}

/// Parse `src` under `syntax` and validate it against `ctx`, with precise token spans.
///
/// This is the LSP-facing entry: it returns the parser's syntactic diagnostics followed by the
/// semantic diagnostics from [`validate`], each name/reference diagnostic located at its exact
/// source token (operator diagnostics keep a whole-formula span; see the [module docs](self#spans)).
pub fn validate_str(src: &str, syntax: Syntax, ctx: &ValidationContext) -> Vec<Diagnostic> {
    let (node, mut diags) = super::parse(src, syntax);
    let mut raws = Vec::new();
    Walker {
        ctx,
        out: &mut raws,
    }
    .visit(&node);
    let toks = super::tokenize(src, syntax);
    let mut consumed = vec![false; toks.len()];
    let whole = (0usize, src.len());
    for r in raws {
        let (start, end) = match &r.locate {
            Locate::Whole => whole,
            Locate::Ident(name) => find_token(&toks, &mut consumed, whole, |t| {
                matches!(t.kind, TokenKind::Ident) && t.text.eq_ignore_ascii_case(name)
            }),
            Locate::Reference(kind, name) => find_token(&toks, &mut consumed, whole, |t| {
                matches!(t.kind, TokenKind::Reference(k) if k == *kind)
                    && t.text.eq_ignore_ascii_case(name)
            }),
        };
        diags.push(Diagnostic {
            message: r.message,
            start,
            end,
            severity: r.severity,
        });
    }
    diags
}

/// Find the first not-yet-consumed token satisfying `pred`, mark it consumed, and return its span;
/// fall back to `whole` when none is left (keeps occurrences of the same name in source order).
fn find_token(
    toks: &[super::token::Token],
    consumed: &mut [bool],
    whole: (usize, usize),
    pred: impl Fn(&super::token::Token) -> bool,
) -> (usize, usize) {
    for (i, t) in toks.iter().enumerate() {
        if !consumed[i] && pred(t) {
            consumed[i] = true;
            return (t.start, t.end);
        }
    }
    whole
}

/// The recursive AST walk collecting [`Raw`] diagnostics.
struct Walker<'a> {
    ctx: &'a ValidationContext,
    out: &'a mut Vec<Raw>,
}

impl Walker<'_> {
    fn push(&mut self, message: impl Into<String>, severity: Severity, locate: Locate) {
        self.out.push(Raw {
            message: message.into(),
            severity,
            locate,
        });
    }

    fn visit(&mut self, node: &Node) {
        match node {
            Node::Call { name, args } => {
                self.check_call(name, args);
                for a in args {
                    self.visit(a);
                }
            }
            Node::Reference { kind, name } => self.check_reference(*kind, name),
            Node::Binary { op, left, right } => {
                if let Some(msg) = check_binary(*op, left, right) {
                    self.push(msg, Severity::Error, Locate::Whole);
                }
                self.visit(left);
                self.visit(right);
            }
            Node::Unary { op, expr } => {
                if let Some(msg) = check_unary(*op, expr) {
                    self.push(msg, Severity::Error, Locate::Whole);
                }
                self.visit(expr);
            }
            Node::Index { base, index } => {
                self.visit(base);
                self.visit(index);
            }
            Node::Array(items) | Node::Seq(items) | Node::Unparsed(items) => {
                for n in items {
                    self.visit(n);
                }
            }
            Node::If {
                cond,
                then,
                elifs,
                els,
            } => {
                self.visit(cond);
                self.visit(then);
                for (c, v) in elifs {
                    self.visit(c);
                    self.visit(v);
                }
                if let Some(e) = els {
                    self.visit(e);
                }
            }
            Node::Assign { value, .. } => self.visit(value),
            Node::Declare { init: Some(i), .. } => self.visit(i),
            Node::While { cond, body, .. } => {
                self.visit(cond);
                self.visit(body);
            }
            Node::For {
                from,
                to,
                step,
                body,
                ..
            } => {
                self.visit(from);
                self.visit(to);
                if let Some(s) = step {
                    self.visit(s);
                }
                self.visit(body);
            }
            // Leaves and value-less nodes: nothing to check or descend into.
            Node::Number(_)
            | Node::Str(_)
            | Node::Bool(_)
            | Node::DateLit(_)
            | Node::Ident(_)
            | Node::Declare { init: None, .. }
            | Node::Exit(_)
            | Node::Error
            | Node::Empty => {}
        }
    }

    /// Check a `name(args…)` call: unknown function (with suggestion) and structural arity.
    fn check_call(&mut self, name: &str, args: &[Node]) {
        match func_id(name) {
            None => {
                let known_custom = self
                    .ctx
                    .functions
                    .as_ref()
                    .is_some_and(|s| s.contains(&name.to_ascii_lowercase()));
                if known_custom {
                    return;
                }
                let mut message = format!("unknown function `{name}`");
                if let Some(suggestion) = nearest_builtin(name) {
                    message.push_str(&format!(" (did you mean `{suggestion}`?)"));
                }
                // With no declared custom-function set, an unknown call may be a legitimate custom
                // function — warn rather than error.
                let severity = if self.ctx.functions.is_some() {
                    Severity::Error
                } else {
                    message.push_str(" (not a built-in; may be a custom function)");
                    Severity::Warning
                };
                self.push(message, severity, Locate::Ident(name.to_string()));
            }
            Some(id) => {
                if let Some(msg) = arity_error(name, id, args.len()) {
                    self.push(msg, Severity::Error, Locate::Ident(name.to_string()));
                }
            }
        }
    }

    /// Check a `{…}` reference against the caller's declared name set for its kind, if any.
    fn check_reference(&mut self, kind: RefKind, name: &str) {
        if let Some(set) = self.ctx.set_for(kind) {
            if !set.contains(&name.to_ascii_lowercase()) {
                let what = ref_kind_noun(kind);
                self.push(
                    format!("unknown {what} `{}{name}`", ref_sigil(kind)),
                    Severity::Error,
                    Locate::Reference(kind, name.to_string()),
                );
            }
        }
    }
}

/// A structural arity error message for a built-in whose argument shape the type system encodes,
/// or `None` when the function isn't arity-constrained here. The per-function signatures aren't
/// tabulated, so only the functions with a structured return rule (`IIf`/`Switch`/`Choose` and the
/// aggregate/argument families) are checked.
fn arity_error(name: &str, id: u16, n: usize) -> Option<String> {
    use super::types::ReturnRule as R;
    let too_few = |min: usize| {
        (n < min).then(|| {
            format!(
                "`{name}` expects at least {min} argument{}, got {n}",
                plural(min)
            )
        })
    };
    match super::types::return_rule(id) {
        R::Iif => (n != 3).then(|| format!("`{name}` expects 3 arguments, got {n}")),
        R::Switch => (n < 2 || n % 2 != 0)
            .then(|| format!("`{name}` expects an even number of arguments (≥2), got {n}")),
        R::Choose => too_few(2),
        R::AggNumeric | R::MaxMinArg | R::DeArray | R::CopyArg => too_few(1),
        R::Fixed(_) | R::Complex => None,
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// A binary-operator type error message, or `None` if the operand types are compatible or not
/// statically known. Operands are collapsed to their element scalar first; an `Unknown`/opaque
/// operand suppresses the check (no false positives on unresolved references or variables).
fn check_binary(code: u8, left: &Node, right: &Node) -> Option<String> {
    let l = family(left);
    let r = family(right);
    let (l, r) = (l?, r?); // skip if either operand type is unknown
    use Fam::*;
    let ok = match code {
        op::AMP => true, // concat coerces anything
        op::STAR | op::SLASH | op::BACKSLASH | op::MOD | op::CARET | op::PERCENT => {
            l == Numeric && r == Numeric
        }
        op::PLUS => {
            (l == Numeric && r == Numeric)
                || (l == StringF && r == StringF)
                || (l == Temporal && r == Numeric)
                || (l == Numeric && r == Temporal)
        }
        op::MINUS => {
            (l == Numeric && r == Numeric)
                || (l == Temporal && r == Temporal)
                || (l == Temporal && r == Numeric)
        }
        op::LT | op::GT | op::GE | op::LE => {
            (l == Numeric && r == Numeric)
                || (l == StringF && r == StringF)
                || (l == Temporal && r == Temporal)
        }
        op::EQ | op::NE => l == r,
        op::AND | op::OR | op::XOR | op::EQV | op::IMP => {
            matches!(l, Boolean | Numeric) && matches!(r, Boolean | Numeric)
        }
        op::LIKE | op::STARTS_WITH => l == StringF && r == StringF,
        _ => true, // `In`, ranges, and anything else: not checked
    };
    if ok {
        None
    } else {
        Some(format!(
            "operator `{}` cannot be applied to {} and {}",
            op::symbol(code),
            l.noun(),
            r.noun()
        ))
    }
}

/// A unary-operator type error message, or `None` if compatible / not statically known.
fn check_unary(code: u8, expr: &Node) -> Option<String> {
    let f = family(expr)?;
    use Fam::*;
    let ok = match code {
        op::NOT => matches!(f, Boolean | Numeric),
        op::UNARY_MINUS | op::UNARY_PLUS | op::DOLLAR => matches!(f, Numeric),
        _ => true,
    };
    if ok {
        None
    } else {
        Some(format!(
            "operator `{}` cannot be applied to {}",
            op::symbol(code),
            f.noun()
        ))
    }
}

/// A coarse operand type family for operator checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fam {
    Numeric,
    Temporal,
    StringF,
    Boolean,
}

impl Fam {
    fn noun(self) -> &'static str {
        match self {
            Fam::Numeric => "a number",
            Fam::Temporal => "a date/time",
            Fam::StringF => "a string",
            Fam::Boolean => "a boolean",
        }
    }
}

/// The operand's type family, or `None` for a statically-unknown / opaque type (skip the check).
fn family(node: &Node) -> Option<Fam> {
    // References resolve to `Unknown` here (the validator injects no type map), so operator checks
    // fire only on statically-typed operands — literals and typed built-ins — never fields.
    match deduce_type(node, &|_, _| None).to_scalar() {
        ResultKind::Number | ResultKind::Currency => Some(Fam::Numeric),
        ResultKind::Date | ResultKind::Time | ResultKind::DateTime => Some(Fam::Temporal),
        ResultKind::String => Some(Fam::StringF),
        ResultKind::Boolean => Some(Fam::Boolean),
        _ => None,
    }
}

/// The nearest built-in name within edit distance 2, for a suggestion. Skips very short names
/// (where a distance-2 edit is meaningless) and requires a strictly closest single candidate.
fn nearest_builtin(name: &str) -> Option<String> {
    let lname = name.to_ascii_lowercase();
    if lname.len() < 3 {
        return None;
    }
    let mut best: Option<(usize, &str)> = None;
    for (cand, _) in super::types::NAME_FUNCID {
        let d = edit_distance(&lname, cand, 2);
        if let Some(d) = d {
            match best {
                Some((bd, _)) if bd <= d => {}
                _ => best = Some((d, cand)),
            }
        }
    }
    best.map(|(_, c)| c.to_string())
}

/// Levenshtein distance between `a` and `b`, or `None` if it exceeds `max` (bounded for speed).
fn edit_distance(a: &str, b: &str, max: usize) -> Option<usize> {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        let mut row_min = cur[0];
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
            row_min = row_min.min(cur[j + 1]);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let d = prev[b.len()];
    (d <= max).then_some(d)
}

fn ref_kind_noun(kind: RefKind) -> &'static str {
    match kind {
        RefKind::Field => "field",
        RefKind::Parameter => "parameter",
        RefKind::Formula => "formula",
        RefKind::RunningTotal => "running total",
        RefKind::SqlExpr => "SQL expression",
    }
}

fn ref_sigil(kind: RefKind) -> &'static str {
    match kind {
        RefKind::Field => "",
        RefKind::Parameter => "?",
        RefKind::Formula => "@",
        RefKind::RunningTotal => "#",
        RefKind::SqlExpr => "%",
    }
}
