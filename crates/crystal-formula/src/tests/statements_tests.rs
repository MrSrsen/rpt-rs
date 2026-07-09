//! Tests for statement bodies and grammar corners: Basic-syntax loops / `If`-blocks /
//! `Dim` / `Select Case`, Crystal-syntax `Select`/loops, and `#..#` date-literal internals.
//!
//! Both parse structure (the AST the parser produces) and evaluation (the VM result) are checked.

use crate::ast::Node;
use crate::eval::{eval, Date, EmptyContext, EvalError, Time, Value};
use crate::{parse, Syntax};

/// Parse under `syntax` (asserting no diagnostics) and evaluate against an empty context.
fn run(src: &str, syntax: Syntax) -> Result<Value, EvalError> {
    let (ast, diags) = parse(src, syntax);
    assert!(diags.is_empty(), "parse diagnostics for `{src}`: {diags:?}");
    eval(&ast, &EmptyContext)
}
fn basic_num(src: &str) -> f64 {
    match run(src, Syntax::Basic) {
        Ok(Value::Number(n)) => n,
        other => panic!("`{src}` → {other:?}"),
    }
}
fn basic_text(src: &str) -> String {
    match run(src, Syntax::Basic) {
        Ok(Value::Str(s)) => s,
        other => panic!("`{src}` → {other:?}"),
    }
}

// ---- Basic-syntax result variable ----

#[test]
fn basic_formula_result_variable() {
    // The Basic return value is whatever is assigned to `formula`, not the last statement.
    assert_eq!(basic_num("formula = 40 + 2\n99"), 42.0);
    assert_eq!(basic_text("Formula = \"hi\""), "hi");
}

// ---- For … Next ----

#[test]
fn basic_for_next_sums() {
    let src = "Dim total As Number\n\
               total = 0\n\
               For i = 1 To 5\n\
                 total = total + i\n\
               Next i\n\
               formula = total";
    assert_eq!(basic_num(src), 15.0);
}

#[test]
fn basic_for_with_step_and_countdown() {
    let up = "Dim t As Number\nt = 0\nFor i = 0 To 10 Step 2\nt = t + i\nNext\nformula = t";
    assert_eq!(basic_num(up), 30.0); // 0+2+4+6+8+10
    let down = "Dim t As Number\nt = 0\nFor i = 3 To 1 Step -1\nt = t + i\nNext\nformula = t";
    assert_eq!(basic_num(down), 6.0); // 3+2+1
                                      // A backwards range with a positive step runs zero times.
    let none = "Dim t As Number\nt = 7\nFor i = 5 To 1\nt = 0\nNext\nformula = t";
    assert_eq!(basic_num(none), 7.0);
}

#[test]
fn for_parses_to_for_node() {
    let (ast, diags) = parse("For i = 1 To 3\nx = i\nNext", Syntax::Basic);
    assert!(diags.is_empty(), "{diags:?}");
    assert!(matches!(ast, Node::For { .. }));
}

// ---- While … Wend / Do … Loop ----

#[test]
fn basic_while_wend() {
    let src = "Dim n As Number\nn = 0\nWhile n < 5\nn = n + 1\nWend\nformula = n";
    assert_eq!(basic_num(src), 5.0);
}

#[test]
fn basic_do_loop_until_is_post_test() {
    // Post-test: the body runs at least once even though the condition is already true.
    let src = "Dim n As Number\nn = 9\nDo\nn = n + 1\nLoop Until n >= 3\nformula = n";
    assert_eq!(basic_num(src), 10.0);
}

#[test]
fn basic_do_while_is_pre_test() {
    let src = "Dim n As Number\nn = 10\nDo While n > 0\nn = n - 3\nLoop\nformula = n";
    assert_eq!(basic_num(src), -2.0);
}

#[test]
fn basic_bare_do_loop_is_infinite_exited_by_exit_do() {
    // A bare `Do … Loop` (no While/Until) parses to an infinite pre-test loop that the
    // `Exit Do` breaks out of — it must not be rejected by the parser.
    let (ast, diags) = parse("Do\nn = n + 1\nLoop", Syntax::Basic);
    assert!(
        diags.is_empty(),
        "bare Do…Loop should parse cleanly: {diags:?}"
    );
    assert!(matches!(
        ast,
        Node::While {
            test_after: false,
            ..
        }
    ));
    let src = "Dim n As Number\nn = 0\nDo\nn = n + 1\nIf n >= 4 Then Exit Do\nLoop\nformula = n";
    assert_eq!(basic_num(src), 4.0);
}

#[test]
fn basic_equals_in_condition_is_comparison_not_assignment() {
    // In an `If` condition a Basic `=` is equality, not assignment.
    let taken = "Dim r As Number\nr = 0\nIf 1 = 1 Then\nr = 5\nEnd If\nformula = r";
    assert_eq!(basic_num(taken), 5.0);
    let skipped = "Dim r As Number\nr = 0\nIf 1 = 2 Then\nr = 5\nEnd If\nformula = r";
    assert_eq!(basic_num(skipped), 0.0);
    // Single-line form and a `=` against a loop variable (the case that surfaced the bug).
    let loopcase = "Dim r As Number\nr = 0\nFor i = 1 To 5\nIf i = 3 Then r = i\nNext\nformula = r";
    assert_eq!(basic_num(loopcase), 3.0);
    // Statement-position `=` is still an assignment.
    assert_eq!(basic_num("Dim x As Number\nx = 7\nformula = x"), 7.0);
}

#[test]
fn while_parses_to_while_node() {
    let (ast, _) = parse("While n < 3\nn = n + 1\nWend", Syntax::Basic);
    assert!(matches!(
        ast,
        Node::While {
            test_after: false,
            ..
        }
    ));
    let (ast2, _) = parse("Do\nn = n + 1\nLoop While n < 3", Syntax::Basic);
    assert!(matches!(
        ast2,
        Node::While {
            test_after: true,
            ..
        }
    ));
}

// ---- If … Then … End If ----

#[test]
fn basic_if_block_with_elseif_else() {
    let src = "Dim r As String\n\
               If 1 > 2 Then\n\
                 r = \"a\"\n\
               ElseIf 3 > 2 Then\n\
                 r = \"b\"\n\
               Else\n\
                 r = \"c\"\n\
               End If\n\
               formula = r";
    assert_eq!(basic_text(src), "b");
}

#[test]
fn basic_if_single_line() {
    let src = "Dim r As Number\nIf 5 > 3 Then r = 1 Else r = 2\nformula = r";
    assert_eq!(basic_num(src), 1.0);
}

#[test]
fn basic_if_block_parses_to_if_node() {
    let (ast, diags) = parse(
        "If a Then\nx = 1\nElseIf b Then\nx = 2\nEnd If",
        Syntax::Basic,
    );
    assert!(diags.is_empty(), "{diags:?}");
    match ast {
        Node::If { elifs, .. } => assert_eq!(elifs.len(), 1),
        other => panic!("expected If, got {other:?}"),
    }
}

// ---- Select Case ----

#[test]
fn basic_select_case_values() {
    let src = "Dim r As String\n\
               Select Case 2\n\
                 Case 1\n\
                   r = \"one\"\n\
                 Case 2, 3\n\
                   r = \"two-or-three\"\n\
                 Case Else\n\
                   r = \"other\"\n\
               End Select\n\
               formula = r";
    assert_eq!(basic_text(src), "two-or-three");
}

#[test]
fn basic_select_case_is_and_range() {
    let mid = "Dim r As String\n\
               Select Case 15\n\
                 Case Is < 10\n\
                   r = \"low\"\n\
                 Case 10 To 20\n\
                   r = \"mid\"\n\
                 Case Else\n\
                   r = \"high\"\n\
               End Select\n\
               formula = r";
    assert_eq!(basic_text(mid), "mid");
    let low = mid.replace("Select Case 15", "Select Case 4");
    assert_eq!(basic_text(&low), "low");
    let high = mid.replace("Select Case 15", "Select Case 99");
    assert_eq!(basic_text(&high), "high");
}

#[test]
fn crystal_select_expression() {
    // Crystal `Select` is an expression yielding the matching case's value.
    let src = "Select 2 Case 1 : \"one\" Case 2 : \"two\" Default : \"other\"";
    assert_eq!(run(src, Syntax::Crystal), Ok(Value::Str("two".into())));
    let other = "Select 9 Case 1 : \"one\" Default : \"other\"";
    assert_eq!(run(other, Syntax::Crystal), Ok(Value::Str("other".into())));
}

#[test]
fn select_lowers_to_if_chain() {
    let (ast, _) = parse("Select 1 Case 1 : \"a\" Default : \"b\"", Syntax::Crystal);
    assert!(matches!(ast, Node::If { .. }));
}

// ---- Crystal-syntax loops ----

#[test]
fn crystal_while_do_statement() {
    let src = "Local NumberVar n := 0;\nWhile n < 3 Do n := n + 1;\nn";
    assert_eq!(run(src, Syntax::Crystal), Ok(Value::Number(3.0)));
}

#[test]
fn crystal_for_do_statement() {
    let src = "Local NumberVar t := 0;\nFor i := 1 To 4 Do t := t + i;\nt";
    assert_eq!(run(src, Syntax::Crystal), Ok(Value::Number(10.0)));
}

// ---- Dim ----

#[test]
fn basic_dim_declares_typed_default() {
    // An uninitialised typed Dim gets its type's default (String → "").
    let src = "Dim s As String\nformula = s & \"!\"";
    assert_eq!(basic_text(src), "!");
    let src2 = "Dim n As Number\nformula = n + 1";
    assert_eq!(basic_num(src2), 1.0);
}

// ---- #..# date-literal internals ----

#[test]
fn date_literal_internals_parse_the_value() {
    assert_eq!(
        run("#2004-02-29#", Syntax::Crystal),
        Ok(Value::Date(Date::new(2004, 2, 29)))
    );
    assert_eq!(
        run("#2004-02-29 23:59:59#", Syntax::Crystal),
        Ok(Value::DateTime(
            Date::new(2004, 2, 29),
            Time::new(23, 59, 59)
        ))
    );
    assert_eq!(
        run("#10:30:00 pm#", Syntax::Crystal),
        Ok(Value::Time(Time::new(22, 30, 0)))
    );
    assert_eq!(
        run("Year(#12/31/1999#)", Syntax::Crystal),
        Ok(Value::Number(1999.0))
    );
}

#[test]
fn textual_month_name_date_literals_parse() {
    // Full month name with the optional day comma.
    assert_eq!(
        run("#March 1, 2024#", Syntax::Crystal),
        Ok(Value::Date(Date::new(2024, 3, 1)))
    );
    // Full month name + time tail with a spaced AM/PM designator.
    assert_eq!(
        run("#March 1, 2024 10:30 am#", Syntax::Crystal),
        Ok(Value::DateTime(Date::new(2024, 3, 1), Time::new(10, 30, 0)))
    );
    // Three-letter abbreviation, no day comma.
    assert_eq!(
        run("#Dec 25 2000#", Syntax::Crystal),
        Ok(Value::Date(Date::new(2000, 12, 25)))
    );
    // PM designator on a textual date.
    assert_eq!(
        run("#October 23, 1999 2:05:30 pm#", Syntax::Crystal),
        Ok(Value::DateTime(
            Date::new(1999, 10, 23),
            Time::new(14, 5, 30)
        ))
    );
    // A month name with a missing year is malformed (diagnostic, not a panic).
    let (node, diags) = parse("#March 1#", Syntax::Crystal);
    assert!(!diags.is_empty(), "expected a diagnostic for a bad literal");
    assert!(matches!(node, Node::DateLit(_)));
}

#[test]
fn malformed_date_literal_is_a_diagnostic_not_a_panic() {
    let (node, diags) = parse("#not-a-date#", Syntax::Crystal);
    assert!(
        !diags.is_empty(),
        "expected a diagnostic for a bad date literal"
    );
    // The tree stays total (the node is still a DateLit carrying the raw text).
    assert!(matches!(node, Node::DateLit(_)));
}

// ---- Exit (loop break) ----

#[test]
fn exit_for_breaks_the_loop() {
    // Sums 1+2+3 then breaks before 4; i reaches 3, so the running total is 6.
    let src = "Dim t As Number\nt = 0\nFor i = 1 To 10\nt = t + i\nIf i >= 3 Then Exit For\nNext\nformula = t";
    assert_eq!(basic_num(src), 6.0);
}

#[test]
fn exit_while_breaks_the_loop() {
    let src =
        "Dim n As Number\nn = 0\nWhile n < 100\nn = n + 1\nIf n >= 5 Then Exit While\nWend\nformula = n";
    assert_eq!(basic_num(src), 5.0);
}

#[test]
fn exit_do_breaks_the_loop() {
    let src = "Dim n As Number\nn = 0\nDo While n < 100\nn = n + 1\nIf n >= 7 Then Exit Do\nLoop\nformula = n";
    assert_eq!(basic_num(src), 7.0);
}

#[test]
fn exit_breaks_only_the_innermost_loop() {
    // The inner loop breaks at j=2 each pass; the outer loop still runs all 3 iterations.
    // count accumulates 2 inner iterations × 3 outer = 6.
    let src = "Dim c As Number\nc = 0\nFor i = 1 To 3\nFor j = 1 To 10\nIf j >= 3 Then Exit For\nc = c + 1\nNext j\nNext i\nformula = c";
    assert_eq!(basic_num(src), 6.0);
}

#[test]
fn exit_as_last_statement() {
    let src = "Dim n As Number\nn = 0\nFor i = 1 To 5\nn = n + 1\nExit For\nNext\nformula = n";
    assert_eq!(basic_num(src), 1.0);
}

#[test]
fn exit_outside_loop_is_a_clean_error() {
    // Not a panic: an `Exit` with no enclosing loop is an evaluation error.
    assert!(matches!(
        run("Exit For", Syntax::Basic),
        Err(EvalError::BadArg(_))
    ));
}

#[test]
fn exit_parses_to_an_exit_node() {
    let (ast, diags) = parse("Exit While", Syntax::Basic);
    assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    assert!(matches!(ast, Node::Exit(crate::ast::ExitKind::While)));
}

// ---- robustness ----

#[test]
fn statement_bodies_never_panic_on_garbage() {
    for (src, syntax) in [
        ("For", Syntax::Basic),
        ("While", Syntax::Basic),
        ("Do\nLoop", Syntax::Basic),
        ("If x Then", Syntax::Basic),
        ("Select Case", Syntax::Basic),
        ("Dim", Syntax::Basic),
        ("Select 1 Case", Syntax::Crystal),
        ("For i := Do", Syntax::Crystal),
        ("End If End Select Wend Loop Next", Syntax::Basic),
    ] {
        let _ = parse(src, syntax);
    }
}
