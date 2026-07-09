//! The Crystal/Basic report **formula language**: tokenizer, recursive-descent parser, AST, static
//! type system, and a bytecode evaluator.
//!
//! This is a standalone crate rather than a module of the report reader because the formula language
//! is independent of the `.rpt` binary container: it depends only on `rpt-format-value` (a
//! dependency-free leaf, for the `Date`/`Time` a [`eval::Value`] carries) and **not** on the `rpt`
//! decoder. That keeps it reusable on its own — by a Crystal LSP server, a WASM formula sandbox, or a
//! standalone validator/playground — without pulling in the whole binary reader. A consumer that
//! also needs the `rpt` model owns any [`types::ResultKind`]-to-model-type mapping itself, so no
//! such bridge couples this crate to that model.
//!
//! Layers:
//! - [`token`] — token kinds, the 5 reference-token classes, operator codes.
//! - [`lexer`] — error-tolerant tokenizer ([`tokenize`]).
//! - [`ast`] — the AST node model.
//! - [`parser`] — error-recovering recursive descent with the operator-precedence ladder
//!   ([`parse`]); the foundation for evaluation, type deduction, and an LSP.
//! - [`refs`] — token-stream reference extraction ([`references`]) used by `UseCount` /
//!   parameter-usage counting; cannot fail on an unparseable construct.
//! - [`validate`](mod@validate) — a semantic diagnostics pass over the AST (`validate` /
//!   `validate_str`): unknown functions, arity, operator type errors, and (with an injected symbol
//!   context) unknown references — the source for the Crystal LSP.
//!
//! The `UseCount` counting path routes through [`refs`], not [`parser`], so partial parses never
//! affect `Field.UseCount`.
//!
//! # Example
//!
//! Parse a formula, then evaluate it with [`eval::eval`] (the bytecode VM — the production path).
//! References the formula pulls from its surroundings (`{table.field}`, parameters, …) are resolved
//! through an [`EvalContext`]; [`eval::MapContext`] is the map-backed workhorse.
//!
//! ```
//! use crystal_formula::eval::{eval, MapContext};
//! use crystal_formula::{parse, RefKind, Syntax, Value};
//!
//! // Parse a formula that multiplies two database fields.
//! let (ast, diagnostics) = parse("{Orders.Quantity} * {Orders.Price}", Syntax::Crystal);
//! assert!(diagnostics.is_empty());
//!
//! // Supply each field's current value through an evaluation context, then evaluate.
//! let ctx = MapContext::default()
//!     .with_field(RefKind::Field, "Orders.Quantity", Value::Number(3.0))
//!     .with_field(RefKind::Field, "Orders.Price", Value::Number(25.0));
//! assert_eq!(eval(&ast, &ctx).unwrap(), Value::Number(75.0));
//! ```

pub mod ast;
pub mod eval;
pub mod lexer;
pub mod parser;
pub mod refs;
pub mod token;
pub mod types;
pub mod validate;

pub use ast::{Node, VarScope};
#[cfg(any(test, feature = "differential"))]
pub use eval::Evaluator;
pub use eval::{is_print_state_special, is_record_nav, EvalContext, EvalError, Value};
pub use parser::{Diagnostic, Severity};
pub use refs::Ref;
pub use token::{
    brace_groups, last_segment, op, short_name, split_reference, strip_braces, RefKind, Syntax,
    Token, TokenKind,
};
pub use types::{deduce_type, func_id, string_max_bytes, ResultKind};
pub use validate::{validate, validate_str, ValidationContext};

/// Tokenize a formula body under the given [`Syntax`] (see [`lexer::tokenize`]).
pub fn tokenize(src: &str, syntax: Syntax) -> Vec<Token> {
    lexer::tokenize(src, syntax)
}

/// Parse a formula body into a (possibly partial) AST plus diagnostics (see [`parser::parse`]).
pub fn parse(src: &str, syntax: Syntax) -> (Node, Vec<Diagnostic>) {
    parser::parse(src, syntax)
}

/// Extract every reference (Crystal syntax) with its enclosing-call context (see
/// [`refs::references`]). This is the entry point engine counting uses.
pub fn references(body: &str) -> impl Iterator<Item = Ref> {
    refs::references(body).into_iter()
}

/// Parse a Crystal `#…#` date/time literal into its [`Value`]. Recognizes the numeric `#m/d/yyyy#`
/// / `#yyyy-m-d#` forms, the textual `#Month d, yyyy#` form, an optional `hh:mm[:ss] [AM|PM]` time
/// tail, and a bare time — yielding [`Value::Date`], [`Value::Time`], or [`Value::DateTime`]. Errors
/// on any other spelling. Exposed so consumers (e.g. SQL push-down) can reuse the one literal parser
/// instead of re-implementing it.
pub fn parse_date_literal(src: &str) -> Result<Value, EvalError> {
    eval::parse_date_literal(src)
}

// Unit tests live in their own subfolder (`formula/tests/`), split by area, so they can grow
// without bloating the module sources.
#[cfg(test)]
mod tests;
