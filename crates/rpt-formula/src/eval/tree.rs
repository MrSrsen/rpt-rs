//! The tree-walking evaluator — the **differential-test reference** for the bytecode
//! [`vm`](crate::eval::vm), which is the sole production path. It shares its value-level semantics
//! ([`ops`](crate::eval::ops)) and lazy/summary classification ([`lazy`](crate::eval::lazy)) with
//! the VM, so both produce byte-identical results. The whole module is gated behind `cfg(test)` /
//! the `differential` feature and is not compiled into a normal build.

use std::collections::HashMap;

use crate::ast::{Node, NodeKind, VarKind};
use crate::eval::{EvalContext, EvalError, Value};

use super::builtins;
use super::lazy::{
    group_name_needs_context, summary_needs_context, GroupNameCall, LazyForm, SummaryCall,
};
use super::ops::{
    apply_binary, apply_index, apply_unary, branch_default, exit_outside_loop, loop_limit,
    parse_date_literal, type_mismatch, var_default, LOOP_LIMIT,
};

/// The tree-walking evaluator — the **differential-test reference** for the bytecode
/// [`vm`](crate::eval::vm), which is the sole production path (`eval` compiles + runs on the VM).
///
/// Holds the variable store for a single evaluation: `Local`/`Global`/`Shared` are flattened into
/// one per-call scope, since this crate has no notion of a report's between-formulas lifetime —
/// cross-formula `Global`/`Shared` persistence is owned by the caller (`rpt-data`'s `SharedState`),
/// which threads values in/out through [`EvalContext`].
pub struct Evaluator<'c> {
    ctx: &'c dyn EvalContext,
    vars: HashMap<String, Value>,
    /// Set by an `Exit` and cleared by the innermost enclosing loop; while set, statement sequences
    /// stop advancing so the break unwinds to that loop.
    breaking: bool,
    /// Enclosing-loop nesting count — an `Exit` at depth 0 is an error rather than a break.
    loop_depth: usize,
}

impl std::fmt::Debug for Evaluator<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Evaluator")
            .field("vars", &self.vars)
            .finish()
    }
}

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
    ///
    /// # Errors
    ///
    /// [`EvalError`] on any evaluation failure — see [`eval`](crate::eval::eval).
    pub fn eval(&mut self, node: &Node) -> Result<Value, EvalError> {
        match &node.kind {
            NodeKind::Number(s) => s
                .trim()
                .parse::<f64>()
                .map(Value::Number)
                .map_err(|_| EvalError::BadArg(format!("number literal `{s}`"))),
            NodeKind::Str(s) => Ok(Value::Str(s.clone())),
            NodeKind::Bool(b) => Ok(Value::Bool(*b)),
            NodeKind::DateLit(s) => parse_date_literal(s),
            NodeKind::Reference { kind, name } => self
                .ctx
                .resolve(*kind, name)
                .ok_or_else(|| EvalError::UnknownName(format!("{{{name}}}"))),
            NodeKind::Ident(name) => self.eval_ident(name),
            NodeKind::Call { name, args } => self.eval_call(name, args),
            NodeKind::Index { base, index } => self.eval_index(base, index),
            NodeKind::Unary { op, expr } => self.eval_unary(*op, expr),
            NodeKind::Binary { op, left, right } => self.eval_binary(*op, left, right),
            NodeKind::Array(items) => Ok(Value::Array(
                items
                    .iter()
                    .map(|n| self.eval(n))
                    .collect::<Result<_, _>>()?,
            )),
            NodeKind::If {
                cond,
                then,
                elifs,
                els,
            } => self.eval_if(cond, then, elifs, els.as_deref()),
            NodeKind::Assign { name, value } => {
                let v = self.eval(value)?;
                self.vars.insert(name.to_lowercase(), v.clone());
                Ok(v)
            }
            NodeKind::Declare {
                kind, names, init, ..
            } => self.eval_declare(*kind, names, init.as_deref()),
            NodeKind::Seq(stmts) => {
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
            NodeKind::Exit(_) => {
                if self.loop_depth == 0 {
                    return Err(exit_outside_loop());
                }
                self.breaking = true;
                Ok(Value::Null)
            }
            NodeKind::While {
                cond,
                body,
                test_after,
            } => self.eval_while(cond, body, *test_after),
            NodeKind::For {
                var,
                from,
                to,
                step,
                body,
            } => self.eval_for(var, from, to, step.as_deref(), body),
            NodeKind::Unparsed(_) => Err(EvalError::Unsupported("unparsed construct".into())),
            NodeKind::Error => Err(EvalError::Unsupported("parse error in formula".into())),
            NodeKind::Empty => Ok(Value::Null),
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
        // A summary function over a `{...}` reference resolves against the report's computed
        // summaries, not by reducing the current row's scalar value.
        if let Some(sc) = SummaryCall::from_call(name, args) {
            return self
                .ctx
                .resolve_summary(&sc.op, &sc.field, sc.group.as_deref())
                .ok_or_else(|| summary_needs_context(&sc));
        }
        // A group-name reference names a group by its condition field and resolves against the
        // report's group state, so its operand is never evaluated as a row value either.
        if let Some(gc) = GroupNameCall::from_call(name, args) {
            return self
                .ctx
                .group_name(&gc.field)
                .ok_or_else(|| group_name_needs_context(&gc.field));
        }
        // Lazy forms: only the selected branch is evaluated (`IIf(x=0, 0, y/x)` must not run
        // the division when x=0 — the engine is lazy here too).
        match LazyForm::from_name(name) {
            Some(LazyForm::IIf) => {
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
            Some(LazyForm::Switch) => {
                for pair in args.chunks(2) {
                    match pair {
                        [c, v] => match self.eval(c)? {
                            Value::Bool(true) => return self.eval(v),
                            Value::Bool(false) | Value::Null => {}
                            v => return Err(type_mismatch("Switch condition", &v)),
                        },
                        // Odd trailing arg = the default.
                        [d] => return self.eval(d),
                        // `chunks(2)` yields only 1- or 2-element slices, never empty or longer.
                        _ => {
                            debug_assert!(false, "chunks(2) yielded {} items", pair.len());
                            return Err(EvalError::Internal("Switch argument chunking"));
                        }
                    }
                }
                return Ok(Value::Null);
            }
            Some(LazyForm::Choose) => {
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
            None => {}
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
