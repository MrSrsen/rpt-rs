//! Bytecode VM for Crystal formulas — a flat `Vec<Op>` compiled from the AST and run on a stack
//! machine. Pure safe Rust; no runtime/JIT. It reuses the tree-walker's value-level
//! operations (`apply_binary`/`apply_unary`/`apply_index`),
//! its builtin dispatch, and its reference resolution, so it produces byte-identical results — the
//! whole AST is compiled (no per-formula fallback to the tree-walker).
//!
//! Control flow lowers to jumps: `If` uses a *null-guard* stack (a later true branch wins; if no
//! branch fires but some condition was Null, the result is Null), `IIf`/`Switch` are conditional
//! chains, and `Choose` is a runtime jump table — matching the tree-walker's laziness exactly.

use std::collections::HashMap;

use super::builtins;
use super::{apply_binary, apply_index, apply_unary};
use crate::ast::{Node, VarKind, VarScope};
use crate::eval::{EvalContext, EvalError, Value};
use crate::token::{op, RefKind};

/// How a conditional treats a `Null` condition value.
#[derive(Debug, Clone, Copy)]
enum NullMode {
    /// `Switch`: skip this branch (fall through).
    Skip,
    /// `If`: remember a null was seen (set the top null-guard), fall through.
    Guard,
    /// `IIf`: the whole expression yields `Null` (push Null, jump to `end`).
    ToNull(usize),
}

/// One VM instruction. Jump targets are op indices, back-patched after compilation.
#[derive(Debug, Clone)]
enum Op {
    /// Push a constant.
    Push(Value),
    /// Resolve a `{ref}` (field/formula/param) and push it.
    LoadRef(RefKind, String),
    /// Resolve a bare identifier: a variable, else a no-arg builtin/constant.
    LoadIdent(String),
    /// Push a variable's current value.
    LoadVar(String),
    /// Pop a value, store it in a variable, push it back (assignment is an expression).
    StoreVar(String),
    /// Bring a declared variable into scope with a default if absent (no stack effect).
    DeclareDefault(String, Value),
    /// Pop `argc` args (in order) and call a builtin.
    Call(String, usize),
    /// Pop rhs, lhs; push `apply_binary`.
    Bin(u8),
    /// Pop operand; push `apply_unary`.
    Un(u8),
    /// Pop index, base; push `apply_index`.
    Index,
    /// Pop `n` values and push them as an array.
    MakeArray(usize),
    /// Unconditional jump.
    Jump(usize),
    /// Pop a condition: `Bool(true)` → jump; `Bool(false)` → fall through; `Null` → per [`NullMode`];
    /// otherwise a type error.
    CondJump(usize, NullMode),
    /// Pop a loop condition: `Bool(false)` or `Null` → jump (exit); `Bool(true)` → fall through
    /// (continue); otherwise a type error.
    CondJumpFalse(usize),
    /// Push a fresh null-guard (false).
    PushGuard,
    /// Pop the null-guard; if it was set (a null condition was seen), jump.
    GuardJump(usize),
    /// Pop the null-guard and discard (a branch was taken).
    PopGuard,
    /// Pop a 1-based index; jump to the matching branch target, or (on a `Null` index) push `Null`
    /// and jump to the end target — matching the tree-walker's null-propagation. A non-numeric,
    /// non-null index or an out-of-range one raises an error (`Choose`).
    ChooseJump { branches: Vec<usize>, end: usize },
    /// Discard the top of stack.
    Pop,
    /// Enter a loop: record the value/guard-stack depths and the loop's exit target (patched) so a
    /// `Break` can unwind to it.
    LoopEnter(usize),
    /// Leave a loop normally: pop the innermost loop frame.
    LoopExit,
    /// `Exit`: unwind the value/guard stacks to the innermost loop frame and jump to its exit. With
    /// no enclosing loop frame it is an error (an `Exit` outside any loop).
    Break,
    /// Raise a fixed error at runtime (unparsed / parse-error nodes).
    Fail(String),
}

/// A live loop's unwind target: the value- and guard-stack depths at loop entry and the op index to
/// jump to when the loop exits (used by [`Op::Break`]). `iters` counts this loop's back-edges so the
/// VM aborts a runaway loop at the same per-loop [`LOOP_LIMIT`](super::LOOP_LIMIT) as the tree-walker.
#[derive(Debug, Clone, Copy)]
struct LoopFrame {
    exit: usize,
    stack: usize,
    guards: usize,
    iters: usize,
}

/// A compiled formula: a flat instruction stream plus the scope of each non-`Local` variable it
/// declares. `Global`/`Shared` variables route through the [`EvalContext`]'s persistent store so
/// they retain their value across records and formulas; names absent from `scopes` are `Local` and
/// live in the VM's per-run map.
#[derive(Debug, Clone)]
pub struct Chunk {
    ops: Vec<Op>,
    scopes: HashMap<String, VarScope>,
}

/// Compile an AST to a [`Chunk`]. Total — every node compiles (unsupported nodes become a runtime
/// `Fail` op mirroring the tree-walker's error).
pub fn compile(node: &Node) -> Chunk {
    let mut c = Compiler {
        ops: Vec::new(),
        scopes: HashMap::new(),
        loop_seq: 0,
    };
    c.node(node);
    Chunk {
        ops: c.ops,
        scopes: c.scopes,
    }
}

struct Compiler {
    ops: Vec<Op>,
    /// Lowercased variable name → declared scope, for the non-`Local` declarations in this formula.
    scopes: HashMap<String, VarScope>,
    /// Counter for the hidden per-`For`-loop bookkeeping variables (limit/step/direction).
    loop_seq: usize,
}

impl Compiler {
    fn emit(&mut self, op: Op) -> usize {
        self.ops.push(op);
        self.ops.len() - 1
    }

    /// The index the next emitted op will occupy (a forward jump label once code is appended).
    fn here(&self) -> usize {
        self.ops.len()
    }

    /// Patch a previously-emitted jump/branch op to target `to`.
    fn patch(&mut self, at: usize, to: usize) {
        match &mut self.ops[at] {
            Op::Jump(t)
            | Op::CondJump(t, _)
            | Op::GuardJump(t)
            | Op::CondJumpFalse(t)
            | Op::LoopEnter(t) => *t = to,
            _ => panic!("patch of non-jump op"),
        }
    }

    fn node(&mut self, node: &Node) {
        match node {
            Node::Number(s) => match s.trim().parse::<f64>() {
                Ok(n) => {
                    self.emit(Op::Push(Value::Number(n)));
                }
                Err(_) => {
                    self.emit(Op::Fail(format!("number literal `{s}`")));
                }
            },
            Node::Str(s) => {
                self.emit(Op::Push(Value::Str(s.clone())));
            }
            Node::Bool(b) => {
                self.emit(Op::Push(Value::Bool(*b)));
            }
            Node::DateLit(s) => match super::parse_date_literal(s) {
                Ok(v) => {
                    self.emit(Op::Push(v));
                }
                Err(e) => {
                    self.emit(Op::Fail(format!("{e}")));
                }
            },
            Node::Reference { kind, name } => {
                self.emit(Op::LoadRef(*kind, name.clone()));
            }
            Node::Ident(name) => {
                self.emit(Op::LoadIdent(name.clone()));
            }
            Node::Unary { op, expr } => {
                self.node(expr);
                self.emit(Op::Un(*op));
            }
            Node::Binary { op, left, right } => {
                self.node(left);
                self.node(right);
                self.emit(Op::Bin(*op));
            }
            Node::Index { base, index } => {
                self.node(base);
                self.node(index);
                self.emit(Op::Index);
            }
            Node::Array(items) => {
                for it in items {
                    self.node(it);
                }
                self.emit(Op::MakeArray(items.len()));
            }
            Node::Call { name, args } => self.call(name, args),
            Node::If {
                cond,
                then,
                elifs,
                els,
            } => self.compile_if(cond, then, elifs, els.as_deref()),
            Node::Assign { name, value } => {
                self.node(value);
                self.emit(Op::StoreVar(name.to_lowercase()));
            }
            Node::Declare {
                scope,
                kind,
                names,
                init,
                ..
            } => self.compile_declare(*scope, *kind, names, init.as_deref()),
            Node::Seq(stmts) => {
                if stmts.is_empty() {
                    self.emit(Op::Push(Value::Null));
                } else {
                    for (i, s) in stmts.iter().enumerate() {
                        if i > 0 {
                            self.emit(Op::Pop);
                        }
                        self.node(s);
                    }
                }
            }
            Node::While {
                cond,
                body,
                test_after,
            } => self.compile_while(cond, body, *test_after),
            Node::For {
                var,
                from,
                to,
                step,
                body,
            } => self.compile_for(var, from, to, step.as_deref(), body),
            Node::Exit(_) => {
                // Every loop leaves a Null on the stack; `Break` unwinds to that point at runtime,
                // erroring if there is no enclosing loop frame.
                self.emit(Op::Break);
            }
            Node::Empty => {
                self.emit(Op::Push(Value::Null));
            }
            Node::Unparsed(_) => {
                self.emit(Op::Fail("unparsed construct".into()));
            }
            Node::Error => {
                self.emit(Op::Fail("parse error in formula".into()));
            }
        }
    }

    fn call(&mut self, name: &str, args: &[Node]) {
        match name.to_lowercase().as_str() {
            // Lazy forms compiled to jumps (only the selected branch runs).
            "iif" if args.len() == 3 => {
                // cond ? a : b ; Null -> Null
                self.node(&args[0]);
                let to_a = self.emit(Op::CondJump(0, NullMode::ToNull(0)));
                self.node(&args[2]); // b
                let over = self.emit(Op::Jump(0));
                let a_at = self.here();
                self.node(&args[1]); // a
                let end = self.here();
                // Patch: CondJump true->a_at, its ToNull end; Jump over->end.
                self.ops[to_a] = Op::CondJump(a_at, NullMode::ToNull(end));
                self.patch(over, end);
            }
            "switch" => self.compile_switch(args),
            "choose" if !args.is_empty() => self.compile_choose(args),
            _ => {
                for a in args {
                    self.node(a);
                }
                self.emit(Op::Call(name.to_lowercase(), args.len()));
            }
        }
    }

    fn compile_if(&mut self, cond: &Node, then: &Node, elifs: &[(Node, Node)], els: Option<&Node>) {
        self.emit(Op::PushGuard);
        let mut then_jumps: Vec<(usize, &Node)> = Vec::new(); // (CondJump idx, body)
        let branches = std::iter::once((cond, then)).chain(elifs.iter().map(|(c, v)| (c, v)));
        for (c, body) in branches {
            self.node(c);
            let cj = self.emit(Op::CondJump(0, NullMode::Guard));
            then_jumps.push((cj, body));
        }
        // Fell through (no branch true): if a null was seen, yield Null; else els/default.
        let gj = self.emit(Op::GuardJump(0));
        match els {
            Some(e) => self.node(e),
            None => {
                self.emit(Op::Push(super::branch_default(then)));
            }
        }
        let over_null = self.emit(Op::Jump(0));
        let null_at = self.here();
        self.emit(Op::Push(Value::Null));
        // Bodies: taken branches pop the guard, then jump to end.
        let mut body_ends: Vec<usize> = Vec::new();
        let mut body_starts: Vec<usize> = Vec::new();
        for (_, body) in &then_jumps {
            body_starts.push(self.here());
            self.node(body);
            self.emit(Op::PopGuard);
            body_ends.push(self.emit(Op::Jump(0)));
        }
        let end = self.here();
        self.patch(gj, null_at);
        self.patch(over_null, end);
        for (i, (cj, _)) in then_jumps.iter().enumerate() {
            self.patch(*cj, body_starts[i]);
        }
        for j in body_ends {
            self.patch(j, end);
        }
    }

    fn compile_switch(&mut self, args: &[Node]) {
        let mut cond_jumps: Vec<usize> = Vec::new();
        let mut default: Option<&Node> = None;
        let mut i = 0;
        while i < args.len() {
            if i + 1 < args.len() {
                self.node(&args[i]);
                cond_jumps.push(self.emit(Op::CondJump(0, NullMode::Skip)));
                i += 2;
            } else {
                default = Some(&args[i]); // trailing odd arg = default
                i += 1;
            }
        }
        match default {
            Some(d) => self.node(d),
            None => {
                self.emit(Op::Push(Value::Null));
            }
        }
        let over = self.emit(Op::Jump(0));
        let mut val_starts: Vec<usize> = Vec::new();
        let mut val_ends: Vec<usize> = Vec::new();
        let mut j = 0;
        while j + 1 < args.len() {
            val_starts.push(self.here());
            self.node(&args[j + 1]);
            val_ends.push(self.emit(Op::Jump(0)));
            j += 2;
        }
        let end = self.here();
        self.patch(over, end);
        for (k, cj) in cond_jumps.iter().enumerate() {
            self.patch(*cj, val_starts[k]);
        }
        for e in val_ends {
            self.patch(e, end);
        }
    }

    fn compile_choose(&mut self, args: &[Node]) {
        // args[0] = index; args[1..] = options (1-based).
        self.node(&args[0]);
        let dispatch = self.emit(Op::ChooseJump {
            branches: Vec::new(),
            end: 0,
        });
        let mut starts: Vec<usize> = Vec::new();
        let mut ends: Vec<usize> = Vec::new();
        for opt in &args[1..] {
            starts.push(self.here());
            self.node(opt);
            ends.push(self.emit(Op::Jump(0)));
        }
        let end = self.here();
        self.ops[dispatch] = Op::ChooseJump {
            branches: starts,
            end,
        };
        for e in ends {
            self.patch(e, end);
        }
    }

    /// Lower a loop to jumps. Every loop leaves a single `Null` on the stack (a statement value).
    /// `LoopEnter`/`LoopExit` bracket the body so an inner `Exit` (`Op::Break`) can unwind to the
    /// exit point.
    fn compile_while(&mut self, cond: &Node, body: &Node, test_after: bool) {
        let enter = self.emit(Op::LoopEnter(0));
        if test_after {
            let start = self.here();
            self.node(body);
            self.emit(Op::Pop);
            self.node(cond);
            // Loop back on true; false/null falls through (exit).
            self.emit(Op::CondJump(start, NullMode::Skip));
        } else {
            let start = self.here();
            self.node(cond);
            let exit = self.emit(Op::CondJumpFalse(0));
            self.node(body);
            self.emit(Op::Pop);
            self.emit(Op::Jump(start));
            let end = self.here();
            self.patch(exit, end);
        }
        let end = self.here();
        self.patch(enter, end);
        self.emit(Op::LoopExit);
        self.emit(Op::Push(Value::Null));
    }

    /// Lower a `For` loop. The limit and step are evaluated once into hidden locals, and the loop
    /// direction follows the step's sign so a negative step counts down.
    fn compile_for(&mut self, var: &str, from: &Node, to: &Node, step: Option<&Node>, body: &Node) {
        let id = self.loop_seq;
        self.loop_seq += 1;
        let v = var.to_lowercase();
        // `#`-prefixed names cannot collide with a real identifier (alnum + underscore only).
        let to_var = format!("#for_to_{id}");
        let step_var = format!("#for_step_{id}");
        let up_var = format!("#for_up_{id}");

        let enter = self.emit(Op::LoopEnter(0));
        self.store_expr(from, &v);
        self.store_expr(to, &to_var);
        match step {
            Some(s) => self.store_expr(s, &step_var),
            None => {
                self.emit(Op::Push(Value::Number(1.0)));
                self.emit(Op::StoreVar(step_var.clone()));
                self.emit(Op::Pop);
            }
        }
        // up = step >= 0
        self.emit(Op::LoadVar(step_var.clone()));
        self.emit(Op::Push(Value::Number(0.0)));
        self.emit(Op::Bin(op::GE));
        self.emit(Op::StoreVar(up_var.clone()));
        self.emit(Op::Pop);

        let start = self.here();
        // continue = (up And v <= to) Or ((Not up) And v >= to)
        self.emit(Op::LoadVar(up_var.clone()));
        self.emit(Op::LoadVar(v.clone()));
        self.emit(Op::LoadVar(to_var.clone()));
        self.emit(Op::Bin(op::LE));
        self.emit(Op::Bin(op::AND));
        self.emit(Op::LoadVar(up_var.clone()));
        self.emit(Op::Un(op::NOT));
        self.emit(Op::LoadVar(v.clone()));
        self.emit(Op::LoadVar(to_var.clone()));
        self.emit(Op::Bin(op::GE));
        self.emit(Op::Bin(op::AND));
        self.emit(Op::Bin(op::OR));
        let exit = self.emit(Op::CondJumpFalse(0));

        self.node(body);
        self.emit(Op::Pop);
        // v = v + step
        self.emit(Op::LoadVar(v.clone()));
        self.emit(Op::LoadVar(step_var.clone()));
        self.emit(Op::Bin(op::PLUS));
        self.emit(Op::StoreVar(v.clone()));
        self.emit(Op::Pop);
        self.emit(Op::Jump(start));

        let end = self.here();
        self.patch(exit, end);
        self.patch(enter, end);
        self.emit(Op::LoopExit);
        self.emit(Op::Push(Value::Null));
    }

    /// Emit `<expr>; StoreVar name; Pop` — evaluate `expr` and stash it in `name` with no net stack
    /// effect.
    fn store_expr(&mut self, expr: &Node, name: &str) {
        self.node(expr);
        self.emit(Op::StoreVar(name.to_string()));
        self.emit(Op::Pop);
    }

    fn compile_declare(
        &mut self,
        scope: VarScope,
        kind: VarKind,
        names: &[String],
        init: Option<&Node>,
    ) {
        // Record the scope of every non-`Local` name so the runtime routes its loads/stores to the
        // persistent store. `Local` names are left absent (the default), keeping them per-run.
        if scope != VarScope::Local {
            for name in names {
                self.scopes.insert(name.to_lowercase(), scope);
            }
        }
        if let Some(init) = init {
            self.node(init);
            if let Some(first) = names.first() {
                self.emit(Op::StoreVar(first.to_lowercase()));
            }
            return;
        }
        let default = super::var_default(kind);
        for name in names {
            self.emit(Op::DeclareDefault(name.to_lowercase(), default.clone()));
        }
        match names.last() {
            Some(last) => {
                self.emit(Op::LoadVar(last.to_lowercase()));
            }
            None => {
                self.emit(Op::Push(Value::Null));
            }
        }
    }
}

/// Run a compiled [`Chunk`] against an evaluation context.
pub fn run(chunk: &Chunk, ctx: &dyn EvalContext) -> Result<Value, EvalError> {
    run_with_loop_limit(chunk, ctx, super::LOOP_LIMIT)
}

/// [`run`] with an explicit per-loop iteration cap (so tests can trip the guard cheaply). Production
/// callers use [`run`], which passes the shared [`LOOP_LIMIT`](super::LOOP_LIMIT).
pub(crate) fn run_with_loop_limit(
    chunk: &Chunk,
    ctx: &dyn EvalContext,
    loop_limit: usize,
) -> Result<Value, EvalError> {
    let mut stack: Vec<Value> = Vec::new();
    let mut guards: Vec<bool> = Vec::new();
    let mut loops: Vec<LoopFrame> = Vec::new();
    let mut vars: HashMap<String, Value> = HashMap::new();
    let mut ip = 0;
    let underflow = || EvalError::Unsupported("VM stack underflow".into());

    while ip < chunk.ops.len() {
        let mut next = ip + 1;
        match &chunk.ops[ip] {
            Op::Push(v) => stack.push(v.clone()),
            Op::LoadRef(kind, name) => {
                let v = ctx
                    .resolve(*kind, name)
                    .ok_or_else(|| EvalError::UnknownName(format!("{{{name}}}")))?;
                stack.push(v);
            }
            Op::LoadIdent(name) => {
                let lname = name.to_lowercase();
                let scoped = chunk
                    .scopes
                    .get(&lname)
                    .and_then(|&s| ctx.var_get(s, &lname));
                if let Some(v) = scoped {
                    stack.push(v);
                } else if let Some(v) = vars.get(&lname) {
                    stack.push(v.clone());
                } else {
                    stack.push(builtins::resolve(name, &[], ctx)?);
                }
            }
            Op::LoadVar(name) => {
                let scoped = chunk.scopes.get(name).and_then(|&s| ctx.var_get(s, name));
                stack.push(
                    scoped
                        .or_else(|| vars.get(name).cloned())
                        .unwrap_or(Value::Null),
                );
            }
            Op::StoreVar(name) => {
                let v = stack.last().ok_or_else(underflow)?.clone();
                // A `Global`/`Shared` write goes to the persistent store; if there is none (default
                // context), it falls back to the per-run locals — identical to the old behavior.
                let persisted = match chunk.scopes.get(name) {
                    Some(&scope) => ctx.var_set(scope, name, v.clone()),
                    None => false,
                };
                if !persisted {
                    vars.insert(name.clone(), v);
                }
            }
            Op::DeclareDefault(name, default) => {
                // A persistent var initialises only when unset, so an accumulated value survives
                // re-declaration on the next record/formula; a `Local` (or store-less) var defaults
                // in the per-run map.
                match chunk.scopes.get(name) {
                    Some(&scope) if ctx.var_get(scope, name).is_some() => {}
                    Some(&scope) if ctx.var_set(scope, name, default.clone()) => {}
                    _ => {
                        vars.entry(name.clone()).or_insert_with(|| default.clone());
                    }
                }
            }
            Op::Call(name, argc) => {
                if stack.len() < *argc {
                    return Err(underflow());
                }
                let args = stack.split_off(stack.len() - argc);
                stack.push(builtins::resolve(name, &args, ctx)?);
            }
            Op::Bin(code) => {
                let r = stack.pop().ok_or_else(underflow)?;
                let l = stack.pop().ok_or_else(underflow)?;
                stack.push(apply_binary(*code, l, r)?);
            }
            Op::Un(code) => {
                let v = stack.pop().ok_or_else(underflow)?;
                stack.push(apply_unary(*code, v)?);
            }
            Op::Index => {
                let i = stack.pop().ok_or_else(underflow)?;
                let b = stack.pop().ok_or_else(underflow)?;
                stack.push(apply_index(b, i)?);
            }
            Op::MakeArray(n) => {
                if stack.len() < *n {
                    return Err(underflow());
                }
                let items = stack.split_off(stack.len() - n);
                stack.push(Value::Array(items));
            }
            Op::Jump(t) => next = *t,
            Op::CondJump(t, null_mode) => {
                let cond = stack.pop().ok_or_else(underflow)?;
                match cond {
                    Value::Bool(true) => next = *t,
                    Value::Bool(false) => {}
                    Value::Null => match null_mode {
                        NullMode::Skip => {}
                        NullMode::Guard => {
                            if let Some(g) = guards.last_mut() {
                                *g = true;
                            }
                        }
                        NullMode::ToNull(end) => {
                            stack.push(Value::Null);
                            next = *end;
                        }
                    },
                    // Match the tree-walker's per-construct label so both evaluators report
                    // the same `what` on a non-Bool condition.
                    v => {
                        let what = match null_mode {
                            NullMode::Guard => "If condition",
                            NullMode::ToNull(_) => "IIf condition",
                            NullMode::Skip => "Switch condition",
                        };
                        return Err(super::type_mismatch(what, &v));
                    }
                }
            }
            Op::CondJumpFalse(t) => {
                let cond = stack.pop().ok_or_else(underflow)?;
                match cond {
                    Value::Bool(true) => {}
                    Value::Bool(false) | Value::Null => next = *t,
                    v => return Err(super::type_mismatch("loop condition", &v)),
                }
            }
            Op::PushGuard => guards.push(false),
            Op::GuardJump(t) => {
                if guards.pop().ok_or_else(underflow)? {
                    next = *t;
                }
            }
            Op::PopGuard => {
                guards.pop();
            }
            Op::ChooseJump { branches, end } => {
                let idx = stack.pop().ok_or_else(underflow)?;
                // A null index propagates to null (the whole Choose is null), matching the walker.
                if idx.is_null() {
                    stack.push(Value::Null);
                    next = *end;
                } else {
                    let i = idx
                        .as_number()
                        .ok_or_else(|| super::type_mismatch("Choose index", &idx))?
                        .trunc() as i64;
                    if i < 1 || i as usize > branches.len() {
                        return Err(EvalError::BadArg(format!("Choose index {i} out of range")));
                    }
                    next = branches[i as usize - 1];
                }
            }
            Op::Pop => {
                stack.pop();
            }
            Op::LoopEnter(exit) => loops.push(LoopFrame {
                exit: *exit,
                stack: stack.len(),
                guards: guards.len(),
                iters: 0,
            }),
            Op::LoopExit => {
                loops.pop();
            }
            Op::Break => match loops.last() {
                Some(frame) => {
                    stack.truncate(frame.stack);
                    guards.truncate(frame.guards);
                    next = frame.exit;
                }
                None => return Err(super::exit_outside_loop()),
            },
            Op::Fail(msg) => return Err(EvalError::Unsupported(msg.clone())),
        }
        // A backward jump is a loop's back-edge — the only way `next` moves up. Count it against the
        // innermost loop's per-iteration budget, matching the tree-walker's per-loop `LOOP_LIMIT`
        // (each loop resets its own count, so N sequential loops are independent).
        if next < ip {
            if let Some(frame) = loops.last_mut() {
                frame.iters += 1;
                if frame.iters > loop_limit {
                    return Err(super::loop_limit());
                }
            }
        }
        ip = next;
    }
    Ok(stack.pop().unwrap_or(Value::Null))
}
