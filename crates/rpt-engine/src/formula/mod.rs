//! Crystal/Basic formula language: tokenizer, recursive-descent parser, and reference extraction.
//!
//! Layers:
//! - [`token`] — token kinds, the 5 reference-token classes, operator codes.
//! - [`lexer`] — error-tolerant tokenizer ([`tokenize`]).
//! - [`ast`] — the AST node model.
//! - [`parser`] — error-recovering recursive descent with the operator-precedence ladder
//!   ([`parse`]); the foundation for evaluation, type deduction, and an LSP.
//! - [`refs`] — token-stream reference extraction ([`references`]) used by `UseCount` /
//!   parameter-usage counting; cannot fail on an unparseable construct.
//!
//! The counting path ([`crate`] `lib.rs`) routes through [`refs`], not [`parser`], so partial
//! parses never affect `Field.UseCount`.

pub mod ast;
pub mod lexer;
pub mod parser;
pub mod refs;
pub mod token;
pub mod types;

pub use ast::Node;
pub use parser::Diagnostic;
pub use refs::Ref;
pub use token::{op, RefKind, Syntax, Token, TokenKind};
pub use types::{deduce_type, func_id, string_max_bytes, ResultKind};

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

// Unit tests live in their own subfolder (`formula/tests/`), split by area, so they can grow
// without bloating the module sources.
#[cfg(test)]
mod tests;
