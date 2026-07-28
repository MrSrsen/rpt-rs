//! Semantic **validation** of a parsed formula — the diagnostics the [`parser`](super::parser)
//! doesn't produce.
//!
//! The parser only reports *syntactic* recovery (an unexpected token). This pass walks a
//! (successfully or partially) parsed [`Node`] tree and reports *semantic* problems an editor or
//! LSP wants to surface:
//!
//! 1. **Unknown / misspelled built-in functions** — a call whose name is in neither the built-in
//!    function table nor the caller-supplied custom-function set (with a nearest-name suggestion).
//! 2. **Function arity** — a built-in called with a structurally wrong number of arguments, driven
//!    by the funcID-keyed signature table (`types::sig`). The dispatched built-ins and the
//!    conditional forms (`IIf`/`Switch`/`Choose`) carry a real bound; a name with no confidently
//!    known signature is left unchecked rather than risk rejecting a valid call.
//! 3. **Operator type errors** — a binary/unary operator applied to statically-known incompatible
//!    operand types (e.g. arithmetic on a `String`), via [`deduce_type`].
//! 4. **Unknown references** — a `{field}` / `{?param}` / `{@formula}` / `{#rt}` / `{%sql}` whose
//!    name isn't in the corresponding [`ValidationContext`] set. Checked only when the caller
//!    supplies that set, so the crate stays standalone (with no context, only the intrinsic checks
//!    1–3 run).
//!
//! This is an additive pass: it produces diagnostics and touches neither the reference-extraction
//! ([`refs`](super::refs)) nor the evaluation paths.
//!
//! ## Spans
//!
//! Every [`Node`] carries its source [`Span`], so each diagnostic is reported at the exact
//! sub-expression it concerns — the offending call, reference, or operator expression — with no
//! re-tokenization. The AST-only [`validate`] entry and the source-level [`validate_str`] entry
//! produce identical spans; `validate_str` additionally prepends the parser's syntactic diagnostics.

use std::collections::HashSet;

use super::ast::{Node, NodeKind};
use super::parser::{Diagnostic, Severity};
use super::token::{op, RefKind, Span, Syntax};
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

/// Validate a parsed formula against `ctx`, returning semantic diagnostics.
///
/// Each diagnostic is located at its node's [`Span`] — the offending call,
/// reference, or operator sub-expression — so occurrences of the same name are pinpointed
/// independently. See the [module docs](self#spans).
pub fn validate(node: &Node, ctx: &ValidationContext) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    Walker { ctx, out: &mut out }.visit(node);
    out
}

/// Parse `src` under `syntax` and validate it against `ctx`.
///
/// This is the LSP-facing entry: it returns the parser's syntactic diagnostics followed by the
/// semantic diagnostics from [`validate`], each located at its exact source span.
pub fn validate_str(src: &str, syntax: Syntax, ctx: &ValidationContext) -> Vec<Diagnostic> {
    let (node, mut diags) = super::parse(src, syntax);
    diags.extend(validate(&node, ctx));
    diags
}

/// The recursive AST walk emitting span-located [`Diagnostic`]s.
struct Walker<'a> {
    ctx: &'a ValidationContext,
    out: &'a mut Vec<Diagnostic>,
}

impl Walker<'_> {
    fn push(&mut self, message: impl Into<String>, severity: Severity, span: Span) {
        self.out.push(Diagnostic {
            message: message.into(),
            start: span.start,
            end: span.end,
            severity,
        });
    }

    fn visit(&mut self, node: &Node) {
        match &node.kind {
            NodeKind::Call { name, args } => {
                self.check_call(name, args, node.span);
                for a in args {
                    self.visit(a);
                }
            }
            NodeKind::Reference { kind, name } => self.check_reference(*kind, name, node.span),
            NodeKind::Binary { op, left, right } => {
                if let Some(msg) = check_binary(*op, left, right) {
                    self.push(msg, Severity::Error, node.span);
                }
                self.visit(left);
                self.visit(right);
            }
            NodeKind::Unary { op, expr } => {
                if let Some(msg) = check_unary(*op, expr) {
                    self.push(msg, Severity::Error, node.span);
                }
                self.visit(expr);
            }
            NodeKind::Index { base, index } => {
                self.visit(base);
                self.visit(index);
            }
            NodeKind::Array(items) | NodeKind::Seq(items) | NodeKind::Unparsed(items) => {
                for n in items {
                    self.visit(n);
                }
            }
            NodeKind::If {
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
            NodeKind::Assign { value, .. } => self.visit(value),
            NodeKind::Declare { init: Some(i), .. } => self.visit(i),
            NodeKind::While { cond, body, .. } => {
                self.visit(cond);
                self.visit(body);
            }
            NodeKind::For {
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
            NodeKind::Number(_)
            | NodeKind::Str(_)
            | NodeKind::Bool(_)
            | NodeKind::DateLit(_)
            | NodeKind::Ident(_)
            | NodeKind::Declare { init: None, .. }
            | NodeKind::Exit(_)
            | NodeKind::Error
            | NodeKind::Empty => {}
        }
    }

    /// Check a `name(args…)` call: unknown function (with suggestion) and structural arity. `span` is
    /// the call node's span; the callee-name diagnostics anchor at just the name (the call's first
    /// token), not the whole `name(args)`.
    fn check_call(&mut self, name: &str, args: &[Node], span: Span) {
        // The callee name is the call's leading token, so its span runs from the node start over the
        // name's source bytes (the stored name is the verbatim identifier text).
        let name_span = Span::new(span.start, span.start + name.len());
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
                self.push(message, severity, name_span);
            }
            Some(id) => {
                if let Some(msg) = arity_error(name, id, args.len()) {
                    self.push(msg, Severity::Error, name_span);
                }
            }
        }
    }

    /// Check a `{…}` reference against the caller's declared name set for its kind, if any. `span` is
    /// the reference node's span.
    fn check_reference(&mut self, kind: RefKind, name: &str, span: Span) {
        if let Some(set) = self.ctx.set_for(kind) {
            if !set.contains(&name.to_ascii_lowercase()) {
                let what = ref_kind_noun(kind);
                self.push(
                    format!("unknown {what} `{}{name}`", ref_sigil(kind)),
                    Severity::Error,
                    span,
                );
            }
        }
    }
}

/// A structural arity error message for a built-in, or `None` when the call's argument count is
/// accepted (or the funcID carries no bound). Driven by the funcID-keyed signature table
/// ([`super::types::sig`]).
fn arity_error(name: &str, id: u16, n: usize) -> Option<String> {
    let sig = super::types::sig(id);
    (!sig.accepts(n)).then(|| format!("`{name}` expects {}, got {n}", sig.expected()))
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
