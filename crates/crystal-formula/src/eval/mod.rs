//! Formula **evaluator** — a tree-walking interpreter over the parsed [`Node`] AST.
//!
//! Design the native engine compiles formulas to
//! bytecode and caches values per evaluation context; a tree-walker is the equivalent, simpler
//! implementation — values are pulled through [`EvalContext`], which is where record/page state
//! and cross-formula resolution live. The evaluator itself is single-formula and stateless across
//! calls except for its variable store.
//!
//! Unimplemented builtins and constructs fail loudly ([`EvalError::Unsupported`]) — never
//! silently wrong. Null propagates through operators and most builtins (the engine's
//! "convert null values" report options are a later concern, handled by the caller/context).

mod builtins;
mod value;
pub mod vm;

pub use builtins::{is_print_state_special, is_record_nav};
pub use value::{format_number, Date, Time, Value};

use super::ast::{Node, VarKind, VarScope};
use super::token::{op, RefKind};
use std::collections::HashMap;

/// An evaluation failure. `Unsupported` marks known-but-unimplemented surface (the honest
/// failure mode); the others are genuine formula/runtime errors.
///
/// An `EvalError` carries **no source span**: the [`Node`] AST it is raised from is span-less (see
/// [`Node`'s span note](super::ast::Node#spans)), so an LSP/playground cannot yet underline the
/// offending sub-expression — only the message is available. This is safe today because evaluation
/// runs only on trusted, already-parsed stored formulas; threading spans is deferred until a
/// consumer needs node-level eval underlines.
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
    /// A bad argument (count, range, unparseable literal…).
    BadArg(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::Unsupported(s) => write!(f, "unsupported: {s}"),
            EvalError::UnknownName(s) => write!(f, "unknown name: {s}"),
            EvalError::TypeMismatch { what, got } => write!(f, "type mismatch in {what}: {got}"),
            EvalError::DivideByZero => write!(f, "division by zero"),
            EvalError::BadArg(s) => write!(f, "bad argument: {s}"),
        }
    }
}

impl std::error::Error for EvalError {}

/// Resolves the names a formula pulls from its surroundings: `{...}` references and the 0-ary
/// print-state specials (`PageNumber`, `CurrentDate`, …).
///
/// Returning `None` from [`resolve`](EvalContext::resolve) means *unknown name* (an error);
/// a present-but-null field must return `Some(Value::Null)`.
pub trait EvalContext {
    /// Resolve a `{...}` reference to its current value, or `None` if the name is unknown.
    fn resolve(&self, kind: RefKind, name: &str) -> Option<Value>;
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

/// Evaluate a parsed formula against a context by compiling it to bytecode and running it on the
/// [`vm`]. This is the sole production path; the tree-walking `Evaluator` remains as the
/// differential-test reference (gated behind `cfg(test)` / the `differential` feature). For a
/// formula evaluated many times (once per row), compile once with
/// [`vm::compile`] and reuse the [`vm::Chunk`] rather than calling this per evaluation.
pub fn eval(node: &Node, ctx: &dyn EvalContext) -> Result<Value, EvalError> {
    vm::run(&vm::compile(node), ctx)
}

/// Apply a unary operator to an already-evaluated value. Shared by the tree-walker
/// (`Evaluator::eval_unary`) and the bytecode VM so both have identical semantics.
pub(super) fn apply_unary(code: u8, v: Value) -> Result<Value, EvalError> {
    if v.is_null() {
        return Ok(Value::Null);
    }
    match code {
        op::UNARY_MINUS => match v {
            Value::Number(n) => Ok(Value::Number(-n)),
            Value::Currency(n) => Ok(Value::Currency(-n)),
            v => Err(type_mismatch("unary `-`", &v)),
        },
        op::UNARY_PLUS => Ok(v),
        op::NOT => match v {
            Value::Bool(b) => Ok(Value::Bool(!b)),
            v => Err(type_mismatch("Not", &v)),
        },
        op::DOLLAR => match v.as_number() {
            Some(n) => Ok(Value::Currency(n)),
            None => Err(type_mismatch("`$`", &v)),
        },
        c => Err(EvalError::Unsupported(format!("unary operator 0x{c:02x}"))),
    }
}

/// Apply a binary operator to two already-evaluated values (both operands are eager — even `And`/
/// `Or`, matching the engine). Shared by the tree-walker and the bytecode VM.
pub(super) fn apply_binary(code: u8, l: Value, r: Value) -> Result<Value, EvalError> {
    // Ranges are built even from null bounds; everything else propagates Null.
    if !(op::RANGE_TO..=op::RANGE_BOTH_EXCL).contains(&code) && (l.is_null() || r.is_null()) {
        // Comparisons against Null are false (the engine's null comparisons never hold).
        if matches!(
            code,
            op::EQ
                | op::NE
                | op::LT
                | op::GT
                | op::GE
                | op::LE
                | op::IN
                | op::LIKE
                | op::STARTS_WITH
        ) {
            return Ok(Value::Bool(false));
        }
        return Ok(Value::Null);
    }
    match code {
        op::PLUS => add(l, r),
        op::MINUS => sub(l, r),
        op::STAR => numeric(l, r, "`*`", |a, b| Ok(a * b)),
        op::SLASH => numeric(l, r, "`/`", |a, b| {
            if b == 0.0 {
                Err(EvalError::DivideByZero)
            } else {
                Ok(a / b)
            }
        }),
        op::BACKSLASH => numeric(l, r, "`\\`", |a, b| {
            if b == 0.0 {
                Err(EvalError::DivideByZero)
            } else {
                Ok((a / b).trunc())
            }
        }),
        op::MOD => numeric(l, r, "Mod", |a, b| {
            if b == 0.0 {
                Err(EvalError::DivideByZero)
            } else {
                Ok(a % b)
            }
        }),
        op::CARET => numeric(l, r, "`^`", |a, b| Ok(a.powf(b))),
        // Binary `%`: x percent of y — `x % y` = 100 * x / y.
        op::PERCENT => numeric(l, r, "`%`", |a, b| {
            if b == 0.0 {
                Err(EvalError::DivideByZero)
            } else {
                Ok(a * 100.0 / b)
            }
        }),
        op::AMP => {
            let (a, b) = (coerce_text(&l)?, coerce_text(&r)?);
            Ok(Value::Str(a + &b))
        }
        op::EQ => Ok(Value::Bool(values_eq(&l, &r)?)),
        op::NE => Ok(Value::Bool(!values_eq(&l, &r)?)),
        op::LT | op::GT | op::GE | op::LE => {
            let ord = compare(&l, &r)?;
            Ok(Value::Bool(match code {
                op::LT => ord.is_lt(),
                op::GT => ord.is_gt(),
                op::GE => ord.is_ge(),
                _ => ord.is_le(),
            }))
        }
        // `To` ranges: the `_`-marked side is exclusive.
        op::RANGE_TO..=op::RANGE_BOTH_EXCL => Ok(Value::Range {
            lo: Box::new(l),
            hi: Box::new(r),
            lo_incl: code == op::RANGE_TO || code == op::RANGE_HI_EXCL,
            hi_incl: code == op::RANGE_TO || code == op::RANGE_LO_EXCL,
        }),
        op::IN => value_in(&l, &r),
        op::LIKE => match (&l, &r) {
            (Value::Str(s), Value::Str(pat)) => Ok(Value::Bool(like_match(s, pat))),
            _ => Err(type_mismatch("Like", &l)),
        },
        op::STARTS_WITH => match (&l, &r) {
            (Value::Str(s), Value::Str(p)) => Ok(Value::Bool(s.starts_with(p.as_str()))),
            _ => Err(type_mismatch("StartsWith", &l)),
        },
        op::AND..=op::IMP => {
            let (Value::Bool(a), Value::Bool(b)) = (&l, &r) else {
                return Err(type_mismatch("boolean operator", &l));
            };
            let (a, b) = (*a, *b);
            Ok(Value::Bool(match code {
                op::AND => a && b,
                op::OR => a || b,
                op::XOR => a ^ b,
                op::EQV => a == b,
                _ => !a || b, // Imp
            }))
        }
        c => Err(EvalError::Unsupported(format!("binary operator 0x{c:02x}"))),
    }
}

/// Apply a subscript `base[index]` to already-evaluated values (Crystal arrays are 1-based). Shared
/// by the tree-walker and the bytecode VM.
pub(super) fn apply_index(b: Value, i: Value) -> Result<Value, EvalError> {
    if b.is_null() || i.is_null() {
        return Ok(Value::Null);
    }
    let idx = i
        .as_number()
        .ok_or_else(|| type_mismatch("subscript", &i))?
        .trunc() as i64;
    match b {
        Value::Array(items) => items
            .get((idx - 1).max(0) as usize)
            .filter(|_| idx >= 1)
            .cloned()
            .ok_or_else(|| EvalError::BadArg(format!("subscript {idx} out of bounds"))),
        v => Err(type_mismatch("subscript base", &v)),
    }
}

/// The tree-walking evaluator — the **differential-test reference** for the bytecode [`vm`], which
/// is the sole production path (`eval` compiles + runs on the VM). It is gated behind `cfg(test)` /
/// the `differential` feature and is not compiled into a normal build.
///
/// Holds the variable store for a single evaluation: `Local`/`Global`/`Shared` are flattened into
/// one per-call scope, since this crate has no notion of a report's between-formulas lifetime —
/// cross-formula `Global`/`Shared` persistence is owned by the caller (`rpt-data`'s `SharedState`),
/// which threads values in/out through [`EvalContext`].
#[cfg(any(test, feature = "differential"))]
pub struct Evaluator<'c> {
    ctx: &'c dyn EvalContext,
    vars: HashMap<String, Value>,
    /// Set by an `Exit` and cleared by the innermost enclosing loop; while set, statement sequences
    /// stop advancing so the break unwinds to that loop.
    breaking: bool,
    /// Enclosing-loop nesting count — an `Exit` at depth 0 is an error rather than a break.
    loop_depth: usize,
}

#[cfg(any(test, feature = "differential"))]
impl std::fmt::Debug for Evaluator<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Evaluator")
            .field("vars", &self.vars)
            .finish()
    }
}

#[cfg(any(test, feature = "differential"))]
impl<'c> Evaluator<'c> {
    /// Create an evaluator that resolves references through `ctx`, with an empty variable store.
    pub fn new(ctx: &'c dyn EvalContext) -> Self {
        Evaluator {
            ctx,
            vars: HashMap::new(),
            breaking: false,
            loop_depth: 0,
        }
    }

    /// Evaluate an AST node to a [`Value`], threading variable state through the walk.
    pub fn eval(&mut self, node: &Node) -> Result<Value, EvalError> {
        match node {
            Node::Number(s) => s
                .trim()
                .parse::<f64>()
                .map(Value::Number)
                .map_err(|_| EvalError::BadArg(format!("number literal `{s}`"))),
            Node::Str(s) => Ok(Value::Str(s.clone())),
            Node::Bool(b) => Ok(Value::Bool(*b)),
            Node::DateLit(s) => parse_date_literal(s),
            Node::Reference { kind, name } => self
                .ctx
                .resolve(*kind, name)
                .ok_or_else(|| EvalError::UnknownName(format!("{{{name}}}"))),
            Node::Ident(name) => self.eval_ident(name),
            Node::Call { name, args } => self.eval_call(name, args),
            Node::Index { base, index } => self.eval_index(base, index),
            Node::Unary { op, expr } => self.eval_unary(*op, expr),
            Node::Binary { op, left, right } => self.eval_binary(*op, left, right),
            Node::Array(items) => Ok(Value::Array(
                items
                    .iter()
                    .map(|n| self.eval(n))
                    .collect::<Result<_, _>>()?,
            )),
            Node::If {
                cond,
                then,
                elifs,
                els,
            } => self.eval_if(cond, then, elifs, els.as_deref()),
            Node::Assign { name, value } => {
                let v = self.eval(value)?;
                self.vars.insert(name.to_lowercase(), v.clone());
                Ok(v)
            }
            Node::Declare {
                kind, names, init, ..
            } => self.eval_declare(*kind, names, init.as_deref()),
            Node::Seq(stmts) => {
                let mut last = Value::Null;
                for s in stmts {
                    last = self.eval(s)?;
                    // A pending break stops the sequence so it unwinds to the enclosing loop.
                    if self.breaking {
                        break;
                    }
                }
                Ok(last)
            }
            Node::Exit(_) => {
                if self.loop_depth == 0 {
                    return Err(exit_outside_loop());
                }
                self.breaking = true;
                Ok(Value::Null)
            }
            Node::While {
                cond,
                body,
                test_after,
            } => self.eval_while(cond, body, *test_after),
            Node::For {
                var,
                from,
                to,
                step,
                body,
            } => self.eval_for(var, from, to, step.as_deref(), body),
            Node::Unparsed(_) => Err(EvalError::Unsupported("unparsed construct".into())),
            Node::Error => Err(EvalError::Unsupported("parse error in formula".into())),
            Node::Empty => Ok(Value::Null),
        }
    }

    fn eval_ident(&mut self, name: &str) -> Result<Value, EvalError> {
        let lname = name.to_lowercase();
        if let Some(v) = self.vars.get(&lname) {
            return Ok(v.clone());
        }
        builtins::resolve(name, &[], self.ctx)
    }

    fn eval_call(&mut self, name: &str, args: &[Node]) -> Result<Value, EvalError> {
        let lname = name.to_lowercase();
        // Lazy forms: only the selected branch is evaluated (`IIf(x=0, 0, y/x)` must not run
        // the division when x=0 — the engine is lazy here too).
        match lname.as_str() {
            "iif" => {
                let [cond, a, b] = args else {
                    return Err(EvalError::BadArg("IIf takes 3 arguments".into()));
                };
                return match self.eval(cond)? {
                    Value::Bool(true) => self.eval(a),
                    Value::Bool(false) => self.eval(b),
                    Value::Null => Ok(Value::Null),
                    v => Err(type_mismatch("IIf condition", &v)),
                };
            }
            "switch" => {
                for pair in args.chunks(2) {
                    match pair {
                        [c, v] => match self.eval(c)? {
                            Value::Bool(true) => return self.eval(v),
                            Value::Bool(false) | Value::Null => {}
                            v => return Err(type_mismatch("Switch condition", &v)),
                        },
                        // Odd trailing arg = the default.
                        [d] => return self.eval(d),
                        _ => unreachable!(),
                    }
                }
                return Ok(Value::Null);
            }
            "choose" => {
                let Some((idx, rest)) = args.split_first() else {
                    return Err(EvalError::BadArg("Choose needs an index".into()));
                };
                let i = match self.eval(idx)? {
                    Value::Null => return Ok(Value::Null),
                    v => v
                        .as_number()
                        .ok_or_else(|| type_mismatch("Choose index", &v))?,
                };
                let i = i.trunc() as i64;
                if i < 1 || i as usize > rest.len() {
                    return Err(EvalError::BadArg(format!("Choose index {i} out of range")));
                }
                return self.eval(&rest[i as usize - 1]);
            }
            _ => {}
        }
        let vals: Vec<Value> = args
            .iter()
            .map(|n| self.eval(n))
            .collect::<Result<_, _>>()?;
        builtins::resolve(name, &vals, self.ctx)
    }

    fn eval_index(&mut self, base: &Node, index: &Node) -> Result<Value, EvalError> {
        let b = self.eval(base)?;
        let i = self.eval(index)?;
        apply_index(b, i)
    }

    fn eval_unary(&mut self, code: u8, expr: &Node) -> Result<Value, EvalError> {
        let v = self.eval(expr)?;
        apply_unary(code, v)
    }

    fn eval_binary(&mut self, code: u8, left: &Node, right: &Node) -> Result<Value, EvalError> {
        let l = self.eval(left)?;
        let r = self.eval(right)?;
        apply_binary(code, l, r)
    }

    fn eval_if(
        &mut self,
        cond: &Node,
        then: &Node,
        elifs: &[(Node, Node)],
        els: Option<&Node>,
    ) -> Result<Value, EvalError> {
        let mut branches = std::iter::once((cond, then)).chain(elifs.iter().map(|(c, v)| (c, v)));
        let mut any_null = false;
        for (c, v) in &mut branches {
            match self.eval(c)? {
                Value::Bool(true) => return self.eval(v),
                Value::Bool(false) => {}
                Value::Null => any_null = true,
                v => return Err(type_mismatch("If condition", &v)),
            }
        }
        if any_null {
            return Ok(Value::Null);
        }
        match els {
            Some(e) => self.eval(e),
            // No Else: the engine yields the branch type's default value.
            None => Ok(branch_default(then)),
        }
    }

    fn eval_while(
        &mut self,
        cond: &Node,
        body: &Node,
        test_after: bool,
    ) -> Result<Value, EvalError> {
        let mut iters = 0usize;
        self.loop_depth += 1;
        loop {
            if !test_after {
                match self.loop_cond(cond)? {
                    true => {}
                    false => break,
                }
            }
            self.eval(body)?;
            if self.breaking {
                self.breaking = false;
                break;
            }
            iters += 1;
            if iters > LOOP_LIMIT {
                self.loop_depth -= 1;
                return Err(loop_limit());
            }
            if test_after && !self.loop_cond(cond)? {
                break;
            }
        }
        self.loop_depth -= 1;
        Ok(Value::Null)
    }

    /// A loop condition: `Null` exits the loop (matching the engine's null-is-false treatment).
    fn loop_cond(&mut self, cond: &Node) -> Result<bool, EvalError> {
        match self.eval(cond)? {
            Value::Bool(b) => Ok(b),
            Value::Null => Ok(false),
            v => Err(type_mismatch("loop condition", &v)),
        }
    }

    fn eval_for(
        &mut self,
        var: &str,
        from: &Node,
        to: &Node,
        step: Option<&Node>,
        body: &Node,
    ) -> Result<Value, EvalError> {
        let from_v = self.eval(from)?;
        let mut cur = from_v
            .as_number()
            .ok_or_else(|| type_mismatch("For start", &from_v))?;
        let to_v = self.eval(to)?;
        let limit = to_v
            .as_number()
            .ok_or_else(|| type_mismatch("For limit", &to_v))?;
        let step_n = match step {
            Some(s) => {
                let sv = self.eval(s)?;
                sv.as_number()
                    .ok_or_else(|| type_mismatch("For step", &sv))?
            }
            None => 1.0,
        };
        let up = step_n >= 0.0;
        let lname = var.to_lowercase();
        let mut iters = 0usize;
        self.loop_depth += 1;
        while if up { cur <= limit } else { cur >= limit } {
            self.vars.insert(lname.clone(), Value::Number(cur));
            self.eval(body)?;
            if self.breaking {
                self.breaking = false;
                break;
            }
            cur += step_n;
            iters += 1;
            if iters > LOOP_LIMIT {
                self.loop_depth -= 1;
                return Err(loop_limit());
            }
        }
        self.loop_depth -= 1;
        Ok(Value::Null)
    }

    fn eval_declare(
        &mut self,
        kind: VarKind,
        names: &[String],
        init: Option<&Node>,
    ) -> Result<Value, EvalError> {
        if let Some(init) = init {
            let v = self.eval(init)?;
            if let Some(name) = names.first() {
                self.vars.insert(name.to_lowercase(), v.clone());
            }
            return Ok(v);
        }
        let mut last = Value::Null;
        for name in names {
            let lname = name.to_lowercase();
            // A re-declaration does not reset an existing variable (Crystal semantics: the
            // declaration brings the name into scope; the value persists).
            let v = self
                .vars
                .entry(lname)
                .or_insert_with(|| var_default(kind))
                .clone();
            last = v;
        }
        Ok(last)
    }
}

/// The maximum iterations a single loop runs before evaluation aborts — a guard against a
/// pathological or non-terminating formula hanging the evaluator. Shared by both evaluators (the
/// tree-walker here and the [`vm`]) so a formula aborts at the same point in each.
pub(crate) const LOOP_LIMIT: usize = 5_000_000;

pub(crate) fn loop_limit() -> EvalError {
    EvalError::Unsupported("loop iteration limit exceeded".into())
}

/// The error an `Exit` outside any loop raises (both evaluators emit it identically).
fn exit_outside_loop() -> EvalError {
    EvalError::BadArg("`Exit` outside a loop".into())
}

/// The default value of a freshly declared variable.
fn var_default(kind: VarKind) -> Value {
    match kind {
        VarKind::Number => Value::Number(0.0),
        VarKind::Currency => Value::Currency(0.0),
        VarKind::Boolean => Value::Bool(false),
        VarKind::String => Value::Str(String::new()),
        // Date/time variables start out null-ish; the engine errors on use-before-set.
        VarKind::Date | VarKind::Time | VarKind::DateTime => Value::Null,
    }
}

/// The default an `If` without `Else` yields, from the then-branch's statically deduced type.
fn branch_default(then: &Node) -> Value {
    use super::types::ResultKind as K;
    match super::types::deduce_type(then, &|_, _| None) {
        K::String => Value::Str(String::new()),
        K::Number => Value::Number(0.0),
        K::Currency => Value::Currency(0.0),
        K::Boolean => Value::Bool(false),
        _ => Value::Null,
    }
}

fn type_mismatch(what: &str, got: &Value) -> EvalError {
    EvalError::TypeMismatch {
        what: what.to_string(),
        got: got.type_name().to_string(),
    }
}

/// Coerce an operand of `&` to text (Null → empty string).
fn coerce_text(v: &Value) -> Result<String, EvalError> {
    v.to_text_default()
        .ok_or_else(|| type_mismatch("text coercion", v))
}

/// Numeric binary operator with Currency promotion (Currency if either side is).
fn numeric(
    l: Value,
    r: Value,
    what: &str,
    f: impl Fn(f64, f64) -> Result<f64, EvalError>,
) -> Result<Value, EvalError> {
    let (a, b) = match (l.as_number(), r.as_number()) {
        (Some(a), Some(b)) => (a, b),
        _ => {
            return Err(type_mismatch(
                what,
                if l.as_number().is_none() { &l } else { &r },
            ))
        }
    };
    let n = f(a, b)?;
    if matches!(l, Value::Currency(_)) || matches!(r, Value::Currency(_)) {
        Ok(Value::Currency(n))
    } else {
        Ok(Value::Number(n))
    }
}

/// A DateTime as fractional civil days.
fn dt_to_f(d: Date, t: Time) -> f64 {
    d.to_days() as f64 + t.to_seconds() as f64 / 86_400.0
}

fn f_to_dt(f: f64) -> (Date, Time) {
    let days = f.floor() as i64;
    let secs = ((f - f.floor()) * 86_400.0).round() as i64;
    // A rounded-up full day carries into the date.
    let (days, secs) = if secs >= 86_400 {
        (days + 1, 0)
    } else {
        (days, secs)
    };
    (Date::from_days(days), Time::from_seconds(secs))
}

fn add(l: Value, r: Value) -> Result<Value, EvalError> {
    match (&l, &r) {
        (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
        (Value::Date(d), _) | (_, Value::Date(d)) if other(&l, &r).as_number().is_some() => {
            let n = other(&l, &r).as_number().unwrap();
            Ok(Value::Date(Date::from_days(d.to_days() + n.trunc() as i64)))
        }
        (Value::DateTime(d, t), _) | (_, Value::DateTime(d, t))
            if other(&l, &r).as_number().is_some() =>
        {
            let n = other(&l, &r).as_number().unwrap();
            let (nd, nt) = f_to_dt(dt_to_f(*d, *t) + n);
            Ok(Value::DateTime(nd, nt))
        }
        (Value::Time(t), _) | (_, Value::Time(t)) if other(&l, &r).as_number().is_some() => {
            let n = other(&l, &r).as_number().unwrap();
            Ok(Value::Time(Time::from_seconds(
                t.to_seconds() + n.trunc() as i64,
            )))
        }
        _ => numeric(l, r, "`+`", |a, b| Ok(a + b)),
    }
}

/// The non-temporal operand of a mixed temporal/number pair.
fn other<'v>(l: &'v Value, r: &'v Value) -> &'v Value {
    if matches!(l, Value::Date(_) | Value::DateTime(..) | Value::Time(_)) {
        r
    } else {
        l
    }
}

fn sub(l: Value, r: Value) -> Result<Value, EvalError> {
    match (&l, &r) {
        (Value::Date(a), Value::Date(b)) => Ok(Value::Number((a.to_days() - b.to_days()) as f64)),
        (Value::DateTime(ad, at), Value::DateTime(bd, bt)) => {
            Ok(Value::Number(dt_to_f(*ad, *at) - dt_to_f(*bd, *bt)))
        }
        (Value::Time(a), Value::Time(b)) => {
            Ok(Value::Number((a.to_seconds() - b.to_seconds()) as f64))
        }
        (Value::Date(d), _) if r.as_number().is_some() => Ok(Value::Date(Date::from_days(
            d.to_days() - r.as_number().unwrap().trunc() as i64,
        ))),
        (Value::DateTime(d, t), _) if r.as_number().is_some() => {
            let (nd, nt) = f_to_dt(dt_to_f(*d, *t) - r.as_number().unwrap());
            Ok(Value::DateTime(nd, nt))
        }
        (Value::Time(t), _) if r.as_number().is_some() => Ok(Value::Time(Time::from_seconds(
            t.to_seconds() - r.as_number().unwrap().trunc() as i64,
        ))),
        _ => numeric(l, r, "`-`", |a, b| Ok(a - b)),
    }
}

/// Equality across the scalar types (Number/Currency compare numerically; strings are
/// case-sensitive, matching the formula language).
fn values_eq(l: &Value, r: &Value) -> Result<bool, EvalError> {
    match (l, r) {
        (Value::Bool(a), Value::Bool(b)) => Ok(a == b),
        _ => Ok(compare(l, r)?.is_eq()),
    }
}

fn compare(l: &Value, r: &Value) -> Result<std::cmp::Ordering, EvalError> {
    use std::cmp::Ordering;
    match (l, r) {
        (Value::Str(a), Value::Str(b)) => Ok(a.as_str().cmp(b.as_str())),
        (Value::Date(a), Value::Date(b)) => Ok(a.cmp(b)),
        (Value::Time(a), Value::Time(b)) => Ok(a.cmp(b)),
        (Value::DateTime(ad, at), Value::DateTime(bd, bt)) => Ok((ad, at).cmp(&(bd, bt))),
        (Value::Bool(a), Value::Bool(b)) => Ok(a.cmp(b)),
        _ => match (l.as_number(), r.as_number()) {
            (Some(a), Some(b)) => a
                .partial_cmp(&b)
                .ok_or_else(|| EvalError::BadArg("NaN comparison".into())),
            _ => Err(EvalError::TypeMismatch {
                what: "comparison".into(),
                got: format!("{} vs {}", l.type_name(), r.type_name()),
            }),
        },
    }
    .map(|o: Ordering| o)
}

/// `x In y`: substring for strings, membership for arrays, bounds for ranges.
fn value_in(l: &Value, r: &Value) -> Result<Value, EvalError> {
    match r {
        Value::Str(hay) => match l {
            Value::Str(needle) => Ok(Value::Bool(hay.contains(needle.as_str()))),
            v => Err(type_mismatch("In (string)", v)),
        },
        Value::Array(items) => {
            for item in items {
                // An array of ranges tests range membership per element.
                if let Value::Range { .. } = item {
                    if let Value::Bool(true) = value_in(l, item)? {
                        return Ok(Value::Bool(true));
                    }
                } else if values_eq(l, item)? {
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        Value::Range {
            lo,
            hi,
            lo_incl,
            hi_incl,
        } => {
            let lo_ok = match compare(l, lo)? {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Equal => *lo_incl,
                std::cmp::Ordering::Less => false,
            };
            let hi_ok = match compare(l, hi)? {
                std::cmp::Ordering::Less => true,
                std::cmp::Ordering::Equal => *hi_incl,
                std::cmp::Ordering::Greater => false,
            };
            Ok(Value::Bool(lo_ok && hi_ok))
        }
        v => Err(type_mismatch("In", v)),
    }
}

/// VB-style `Like`: `*` = any run, `?` = any one character (case-sensitive).
fn like_match(s: &str, pat: &str) -> bool {
    fn rec(s: &[char], p: &[char]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some(('*', rest)) => (0..=s.len()).any(|i| rec(&s[i..], rest)),
            Some(('?', rest)) => s.split_first().is_some_and(|(_, srest)| rec(srest, rest)),
            Some((c, rest)) => s
                .split_first()
                .is_some_and(|(sc, srest)| sc == c && rec(srest, rest)),
        }
    }
    let sc: Vec<char> = s.chars().collect();
    let pc: Vec<char> = pat.chars().collect();
    rec(&sc, &pc)
}

/// Parse a `#...#` date/time literal. Forms: numeric `#m/d/yyyy#` / `#yyyy-m-d#`, the textual
/// `#Month d, yyyy#` (full or abbreviated English month name), an optional `hh:mm[:ss] [AM|PM]`
/// time tail, or a bare time.
pub(crate) fn parse_date_literal(src: &str) -> Result<Value, EvalError> {
    let inner = src.trim().trim_matches('#').trim();
    let bad = || EvalError::BadArg(format!("date literal `{src}`"));
    // Split off a trailing AM/PM designator (attached or spaced).
    let lower = inner.to_ascii_lowercase();
    let (body, pm) = if let Some(stripped) = lower.strip_suffix("pm") {
        (stripped.trim_end(), Some(true))
    } else if let Some(stripped) = lower.strip_suffix("am") {
        (stripped.trim_end(), Some(false))
    } else {
        (lower.as_str(), None)
    };
    let mut date: Option<Date> = None;
    let mut time: Option<Time> = None;
    // Textual `Month d, yyyy` accumulators: a month name plus the bare integers (day then year)
    // that follow it. Populated only when a month name appears.
    let mut month_name: Option<u8> = None;
    let mut bare_nums: Vec<i32> = Vec::new();
    for part in body.split_whitespace() {
        if part.contains(':') {
            let nums: Vec<&str> = part.split(':').collect();
            if nums.len() < 2 || nums.len() > 3 {
                return Err(bad());
            }
            let mut hour: u8 = nums[0].parse().map_err(|_| bad())?;
            let minute: u8 = nums[1].parse().map_err(|_| bad())?;
            let second: u8 = nums
                .get(2)
                .map_or(Ok(0), |s| s.parse())
                .map_err(|_| bad())?;
            match pm {
                Some(true) if hour < 12 => hour += 12,
                Some(false) if hour == 12 => hour = 0,
                _ => {}
            }
            time = Some(Time::new(hour, minute, second));
        } else if part.contains('/') || part.contains('-') {
            let sep = if part.contains('/') { '/' } else { '-' };
            let nums: Vec<&str> = part.split(sep).collect();
            if nums.len() != 3 {
                return Err(bad());
            }
            // `yyyy-m-d` when the first component is 4 digits, else US `m/d/y`.
            let (y, m, d) = if nums[0].len() == 4 {
                (nums[0], nums[1], nums[2])
            } else {
                (nums[2], nums[0], nums[1])
            };
            date = Some(Date::new(
                y.parse().map_err(|_| bad())?,
                m.parse().map_err(|_| bad())?,
                d.parse().map_err(|_| bad())?,
            ));
        } else if let Some(m) = month_from_name(part) {
            if month_name.is_some() {
                return Err(bad());
            }
            month_name = Some(m);
        } else if let Ok(n) = part.trim_end_matches(',').parse::<i32>() {
            // A bare integer (day / year) of the textual form; the trailing comma after the day
            // (`March 1, 2024`) is optional.
            bare_nums.push(n);
        } else {
            return Err(bad());
        }
    }
    // Assemble the textual `Month d, yyyy` date, if a month name was seen.
    if let Some(m) = month_name {
        if date.is_some() || bare_nums.len() != 2 {
            return Err(bad());
        }
        let day = u8::try_from(bare_nums[0]).map_err(|_| bad())?;
        date = Some(Date::new(bare_nums[1], m, day));
    } else if !bare_nums.is_empty() {
        // Bare integers with no month name are not a valid date form.
        return Err(bad());
    }
    match (date, time) {
        (Some(d), Some(t)) => Ok(Value::DateTime(d, t)),
        (Some(d), None) => Ok(Value::Date(d)),
        (None, Some(t)) => Ok(Value::Time(t)),
        (None, None) => Err(bad()),
    }
}

/// Map a full or three-letter-abbreviated English month name (already lowercased) to its 1-based
/// number, for the textual `#Month d, yyyy#` literal form.
fn month_from_name(s: &str) -> Option<u8> {
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    MONTHS
        .iter()
        .position(|full| *full == s || (s.len() == 3 && full.starts_with(s)))
        .map(|i| i as u8 + 1)
}
