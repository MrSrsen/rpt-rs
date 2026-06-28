//! Formula AST.
//!
//! Node kind = enum variant; operator nodes carry the operator's token code in `op`. Result-type
//! deduction is a separate pass ([`super::types`]); the AST itself carries no result type.

use super::token::RefKind;

/// A parsed formula node. `Error`/`Empty` keep the tree total so the parser never panics and an
/// LSP can still walk a partial parse.
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
    Reference { kind: RefKind, name: String },
    /// A bare identifier / variable / 0-ary built-in.
    Ident(String),
    /// A function/built-in call `name(args...)`.
    Call { name: String, args: Vec<Node> },
    /// Postfix subscript `base[index]`.
    Index { base: Box<Node>, index: Box<Node> },
    /// Unary prefix operator; `op` is the operator token code.
    Unary { op: u8, expr: Box<Node> },
    /// Binary operator; `op` is the operator token code.
    Binary {
        op: u8,
        left: Box<Node>,
        right: Box<Node>,
    },
    /// Array literal `[a, b, ...]`.
    Array(Vec<Node>),
    /// Crystal `If cond Then a [Else If...] [Else b]` — an expression.
    If {
        cond: Box<Node>,
        then: Box<Node>,
        elifs: Vec<(Node, Node)>,
        els: Option<Box<Node>>,
    },
    /// Assignment `name := value` (Crystal) / `name = value` (Basic).
    Assign { name: String, value: Box<Node> },
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
