//! Parser tests: precedence sanity, calls, If-expressions, and error recovery (never panics).

use crate::ast::Node;
use crate::parse;
use crate::token::{op, Syntax};

#[test]
fn unary_binds_looser_than_power() {
    // -2^2 parses as -(2^2): a Unary wrapping a `^` Binary.
    let (node, diags) = parse("-2^2", Syntax::Crystal);
    assert!(diags.is_empty(), "{diags:?}");
    match node {
        Node::Unary { op: u, expr } => {
            assert_eq!(u, 0x7a);
            assert!(matches!(*expr, Node::Binary { op: o, .. } if o == op::CARET));
        }
        other => panic!("expected unary, got {other:?}"),
    }
}

#[test]
fn concat_binds_tighter_than_comparison() {
    // a & b = c  →  (a & b) = c
    let (node, _) = parse("{t.a} & {t.b} = {t.c}", Syntax::Crystal);
    match node {
        Node::Binary { op: o, left, .. } => {
            assert_eq!(o, op::EQ);
            assert!(matches!(*left, Node::Binary { op: l, .. } if l == op::AMP));
        }
        other => panic!("expected `=` at root, got {other:?}"),
    }
}

#[test]
fn additive_left_associative() {
    // a - b - c  →  (a - b) - c
    let (node, _) = parse("1 - 2 - 3", Syntax::Crystal);
    match node {
        Node::Binary { op: o, left, .. } => {
            assert_eq!(o, op::MINUS);
            assert!(matches!(*left, Node::Binary { op: l, .. } if l == op::MINUS));
        }
        other => panic!("expected `-` at root, got {other:?}"),
    }
}

#[test]
fn mul_tighter_than_add() {
    // 1 + 2 * 3  →  1 + (2 * 3)
    let (node, _) = parse("1 + 2 * 3", Syntax::Crystal);
    match node {
        Node::Binary { op: o, right, .. } => {
            assert_eq!(o, op::PLUS);
            assert!(matches!(*right, Node::Binary { op: r, .. } if r == op::STAR));
        }
        other => panic!("expected `+` at root, got {other:?}"),
    }
}

#[test]
fn word_operators_and() {
    let (node, _) = parse("{t.a} And {t.b}", Syntax::Crystal);
    assert!(matches!(node, Node::Binary { op: 0x3f, .. }));
}

#[test]
fn call_with_args() {
    let (node, diags) = parse("Sum({t.x}, {t.g})", Syntax::Crystal);
    assert!(diags.is_empty(), "{diags:?}");
    match node {
        Node::Call { name, args } => {
            assert_eq!(name, "Sum");
            assert_eq!(args.len(), 2);
        }
        other => panic!("expected call, got {other:?}"),
    }
}

#[test]
fn if_expression() {
    let (node, diags) = parse("If {t.x} > 0 Then Sum({t.y}) Else 0", Syntax::Crystal);
    assert!(diags.is_empty(), "{diags:?}");
    assert!(matches!(node, Node::If { .. }));
}

#[test]
fn if_with_else_if() {
    let (node, _) = parse("If a Then 1 Else If b Then 2 Else 3", Syntax::Crystal);
    match node {
        Node::If { elifs, els, .. } => {
            assert_eq!(elifs.len(), 1);
            assert!(els.is_some());
        }
        other => panic!("expected if, got {other:?}"),
    }
}

#[test]
fn crystal_assignment() {
    let (node, _) = parse("x := {t.f} + 1", Syntax::Crystal);
    assert!(matches!(node, Node::Assign { .. }));
}

#[test]
fn statement_sequence() {
    let (node, _) = parse("a := 1; b := 2; a + b", Syntax::Crystal);
    match node {
        Node::Seq(stmts) => assert_eq!(stmts.len(), 3),
        other => panic!("expected seq, got {other:?}"),
    }
}

#[test]
fn no_whitespace_parses_same_as_spaced() {
    // `a-b` and `a - b` must produce the same AST (a `-` binary of two idents).
    let tight = parse("a-b", Syntax::Crystal).0;
    let spaced = parse("a - b", Syntax::Crystal).0;
    assert_eq!(tight, spaced);
    assert!(matches!(tight, Node::Binary { op: o, .. } if o == op::MINUS));
}

#[test]
fn no_whitespace_reference_arithmetic() {
    let tight = parse("{t.a}-{t.b}", Syntax::Crystal).0;
    let spaced = parse("{t.a} - {t.b}", Syntax::Crystal).0;
    assert_eq!(tight, spaced);
}

#[test]
fn error_recovery_never_panics() {
    for body in [
        "",
        "(((",
        ")))",
        "Sum({a.b},",
        "If If If",
        "{unterminated",
        "1 + + + 2",
        "@#$%^",
        "[1, 2,",
        "Select",
        "Local NumberVar x := 1",
    ] {
        let (_node, _diags) = parse(body, Syntax::Crystal);
    }
}
