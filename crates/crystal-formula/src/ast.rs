//! Formula AST.
//!
//! Node kind = enum variant; operator nodes carry the operator's token code in `op`. Result-type
//! deduction is a separate pass ([`super::types`]); the AST itself carries no result type.

use super::token::RefKind;

/// The scope keyword of a variable declaration. Crystal's default (no keyword) is `Global`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VarScope {
    /// `Local` — visible only within the formula that declares it.
    Local,
    /// `Global` (the default) — shared across formulas in the same report/subreport pass.
    #[default]
    Global,
    /// `Shared` — shared across the whole report, including between main report and subreports.
    Shared,
}

/// Which loop keyword an `Exit` names (`Exit For` / `Exit While` / `Exit Do`). Retained for AST
/// fidelity; evaluation breaks the innermost enclosing loop regardless of kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitKind {
    /// `Exit For`.
    For,
    /// `Exit While`.
    While,
    /// `Exit Do`.
    Do,
}

/// The declared type of a variable (`NumberVar`, `StringVar`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    /// `NumberVar`.
    Number,
    /// `CurrencyVar`.
    Currency,
    /// `BooleanVar`.
    Boolean,
    /// `DateVar`.
    Date,
    /// `TimeVar`.
    Time,
    /// `DateTimeVar`.
    DateTime,
    /// `StringVar`.
    String,
}

/// A parsed formula node. `Error`/`Empty` keep the tree total so the parser never panics and an
/// LSP can still walk a partial parse.
///
/// # Spans
///
/// A `Node` carries **no source span**. Syntactic diagnostics recover exact offsets from the token
/// stream ([`parser::Diagnostic`](super::parser::Diagnostic) carries `start`/`end`, and
/// [`validate::validate_str`](super::validate::validate_str) re-tokenizes to point each diagnostic
/// at its token); an [`EvalError`](super::eval::EvalError), however, cannot be underlined at its
/// originating node because the span is not threaded here. Threading an `Option<Span>` onto every
/// variant is deferred until an LSP/playground consumer needs node-level eval underlines — today
/// evaluation runs only on trusted, already-parsed stored formulas, where the error text suffices.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// Numeric literal (number or currency).
    Number(String),
    /// String literal.
    Str(String),
    /// Boolean literal.
    Bool(bool),
    /// `#...#` date/time literal (internals deferred).
    DateLit(String),
    /// A `{...}` reference.
    Reference {
        /// Which reference class the braces denote (field, parameter, formula, …).
        kind: RefKind,
        /// The reference's raw inner text (e.g. `Table.Field`, `?Param`, `@Formula`).
        name: String,
    },
    /// A bare identifier / variable / 0-ary built-in.
    Ident(String),
    /// A function/built-in call `name(args...)`.
    Call {
        /// The function or built-in name.
        name: String,
        /// The positional argument expressions.
        args: Vec<Node>,
    },
    /// Postfix subscript `base[index]`.
    Index {
        /// The array/collection expression being subscripted.
        base: Box<Node>,
        /// The 1-based index expression.
        index: Box<Node>,
    },
    /// Unary prefix operator; `op` is the operator token code.
    Unary {
        /// The operator's token code (see [`op`](super::token::op)).
        op: u8,
        /// The operand expression.
        expr: Box<Node>,
    },
    /// Binary operator; `op` is the operator token code.
    Binary {
        /// The operator's token code (see [`op`](super::token::op)).
        op: u8,
        /// The left-hand operand.
        left: Box<Node>,
        /// The right-hand operand.
        right: Box<Node>,
    },
    /// Array literal `[a, b, ...]`.
    Array(Vec<Node>),
    /// Crystal `If cond Then a [Else If...] [Else b]` — an expression.
    If {
        /// The `If` condition.
        cond: Box<Node>,
        /// The `Then` branch.
        then: Box<Node>,
        /// Zero or more `Else If (cond, branch)` pairs, in source order.
        elifs: Vec<(Node, Node)>,
        /// The optional trailing `Else` branch.
        els: Option<Box<Node>>,
    },
    /// Assignment `name := value` (Crystal) / `name = value` (Basic).
    Assign {
        /// The target variable name.
        name: String,
        /// The assigned value expression.
        value: Box<Node>,
    },
    /// Variable declaration `[Local|Global|Shared] <Type>Var [Array] name[, name…] [:= init]`.
    /// The declaration is itself an expression; its value is the (single) initialised variable's
    /// value, or the type default when uninitialised.
    Declare {
        /// The declared scope (`Local`/`Global`/`Shared`).
        scope: VarScope,
        /// The declared variable type.
        kind: VarKind,
        /// Whether the declaration is an `Array`.
        array: bool,
        /// The declared variable name(s).
        names: Vec<String>,
        /// The optional `:=` initialiser expression.
        init: Option<Box<Node>>,
    },
    /// A `While`/`Do` loop. `test_after` is the post-test form (`Do … Loop While`); `Until` is
    /// desugared to a `Not`-wrapped condition at parse time, so this node is always "loop while
    /// `cond`". The loop is a statement; it evaluates to `Null`.
    While {
        /// The loop-while condition (already `Not`-desugared if the source used `Until`).
        cond: Box<Node>,
        /// The loop body.
        body: Box<Node>,
        /// `true` for the post-test form (`Do … Loop While`), which runs the body once first.
        test_after: bool,
    },
    /// A `For … To … [Step …]` counting loop (`For i := a To b Step s`). The loop variable counts
    /// from `from` to `to` inclusive; the direction follows the sign of `step` (default `1`). A
    /// statement; it evaluates to `Null`.
    For {
        /// The loop counter variable name.
        var: String,
        /// The start value (`To`'s lower bound / start).
        from: Box<Node>,
        /// The inclusive end value.
        to: Box<Node>,
        /// The optional `Step` increment (defaults to `1`).
        step: Option<Box<Node>>,
        /// The loop body.
        body: Box<Node>,
    },
    /// `Exit For`/`Exit While`/`Exit Do` — breaks out of the innermost enclosing loop. A statement;
    /// it evaluates to `Null` (an `Exit` with no enclosing loop is an evaluation error).
    Exit(ExitKind),
    /// A statement sequence (`;`-separated in Crystal, newline-separated in Basic).
    Seq(Vec<Node>),
    /// A construct the parser recognised but does not yet model (e.g. `Select`, declarations,
    /// Basic statement bodies). Children preserved best-effort.
    Unparsed(Vec<Node>),
    /// A parse error was recovered here.
    Error,
    /// Empty input / empty branch.
    Empty,
}
