//! Semantic-validation tests: unknown functions, arity, operator type errors, unknown references.

use crate::parser::Severity;
use crate::token::Syntax;
use crate::validate::{validate, validate_str, ValidationContext};
use crate::{parse, Diagnostic};

/// Validate a Crystal formula with an empty context; asserts the parse is clean so the diagnostics
/// are purely semantic.
fn diags(src: &str) -> Vec<Diagnostic> {
    diags_ctx(src, &ValidationContext::default())
}

fn diags_ctx(src: &str, ctx: &ValidationContext) -> Vec<Diagnostic> {
    let (node, pdiags) = parse(src, Syntax::Crystal);
    assert!(
        pdiags.is_empty(),
        "unexpected parse diags for {src:?}: {pdiags:?}"
    );
    validate(&node, ctx)
}

fn messages(ds: &[Diagnostic]) -> String {
    ds.iter()
        .map(|d| d.message.as_str())
        .collect::<Vec<_>>()
        .join(" | ")
}

// ---- valid formulas → zero diagnostics ----------------------------------------------------

#[test]
fn valid_formulas_have_no_diagnostics() {
    for src in [
        "1 + 2",
        "\"a\" & 1",     // `&` coerces
        "\"a\" + \"b\"", // string concat via `+`
        "Length(\"hi\")",
        "IIf(True, 1, 2)",
        "Sum({?x})", // reference type is unknown → not arity/type checked away
        "Abs(-5)",
        "Switch(1 > 0, \"a\", 1 < 0, \"b\")",
        "Not True",
        "2 * 3 - 1",
    ] {
        let ds = diags(src);
        assert!(
            ds.is_empty(),
            "expected no diagnostics for {src:?}, got: {}",
            messages(&ds)
        );
    }
}

// ---- unknown / misspelled functions -------------------------------------------------------

#[test]
fn unknown_function_without_context_is_a_warning() {
    let ds = diags("Frobnicate(1)");
    assert_eq!(ds.len(), 1);
    assert_eq!(ds[0].severity, Severity::Warning);
    assert!(
        ds[0].message.contains("unknown function"),
        "{}",
        ds[0].message
    );
    assert!(
        ds[0].message.contains("custom function"),
        "{}",
        ds[0].message
    );
}

#[test]
fn unknown_function_with_function_context_is_an_error() {
    let ctx = ValidationContext::default().with_functions(["MyFunc"]);
    let ds = diags_ctx("Frobnicate(1)", &ctx);
    assert_eq!(ds.len(), 1);
    assert_eq!(ds[0].severity, Severity::Error);
}

#[test]
fn declared_custom_function_is_accepted() {
    let ctx = ValidationContext::default().with_functions(["Frobnicate"]);
    assert!(diags_ctx("Frobnicate(1)", &ctx).is_empty());
}

#[test]
fn misspelled_builtin_suggests_nearest() {
    let ds = diags("Uppercas(\"x\")");
    assert_eq!(ds.len(), 1);
    assert!(ds[0].message.contains("did you mean"), "{}", ds[0].message);
    assert!(ds[0].message.contains("uppercase"), "{}", ds[0].message);
}

#[test]
fn bare_identifier_is_not_flagged_as_unknown() {
    // A bare identifier is a variable or 0-ary field, not a call — never an "unknown function".
    assert!(diags("myvar").is_empty());
    assert!(diags("myvar + 1").is_empty());
}

// ---- arity --------------------------------------------------------------------------------

#[test]
fn iif_wrong_arity() {
    let ds = diags("IIf(True)");
    assert_eq!(ds.len(), 1);
    assert_eq!(ds[0].severity, Severity::Error);
    assert!(ds[0].message.contains("3 arguments"), "{}", ds[0].message);
}

#[test]
fn aggregate_needs_an_argument() {
    let ds = diags("Sum()");
    assert_eq!(ds.len(), 1);
    assert!(
        ds[0].message.contains("at least 1 argument"),
        "{}",
        ds[0].message
    );
}

#[test]
fn switch_needs_even_arguments() {
    let ds = diags("Switch(1 > 0, \"a\", 1 < 0)");
    assert_eq!(ds.len(), 1);
    assert!(ds[0].message.contains("even number"), "{}", ds[0].message);
}

#[test]
fn choose_needs_two_arguments() {
    let ds = diags("Choose(1)");
    assert_eq!(ds.len(), 1);
    assert!(
        ds[0].message.contains("at least 2 argument"),
        "{}",
        ds[0].message
    );
}

// ---- operator type errors -----------------------------------------------------------------

#[test]
fn arithmetic_on_string_is_an_error() {
    let ds = diags("\"a\" - 1");
    assert_eq!(ds.len(), 1);
    assert_eq!(ds[0].severity, Severity::Error);
    assert!(
        ds[0].message.contains("cannot be applied"),
        "{}",
        ds[0].message
    );
    assert!(ds[0].message.contains('-'), "{}", ds[0].message);
}

#[test]
fn multiply_string_is_an_error() {
    assert_eq!(diags("\"a\" * 2").len(), 1);
}

#[test]
fn string_plus_number_is_an_error() {
    // `+` is not `&`: mixing a string and a number is a type error.
    assert_eq!(diags("\"a\" + 1").len(), 1);
}

#[test]
fn unary_not_on_string_is_an_error() {
    let ds = diags("Not \"x\"");
    assert_eq!(ds.len(), 1);
    assert!(ds[0].message.contains("Not"), "{}", ds[0].message);
}

#[test]
fn unary_minus_on_string_is_an_error() {
    assert_eq!(diags("-\"x\"").len(), 1);
}

#[test]
fn comparing_string_to_number_is_an_error() {
    assert_eq!(diags("\"a\" < 1").len(), 1);
    assert_eq!(diags("\"a\" = 1").len(), 1);
}

#[test]
fn boolean_operator_on_string_is_an_error() {
    assert_eq!(diags("\"a\" And True").len(), 1);
}

#[test]
fn concat_ampersand_never_errors() {
    assert!(diags("\"a\" & 1 & True & 2.5").is_empty());
}

// ---- unknown references (context-gated) ---------------------------------------------------

#[test]
fn unknown_parameter_flagged_only_with_context() {
    // Without a parameter set, cross-reference checks are skipped entirely.
    assert!(diags("{?Missing} + 1").is_empty());

    let ctx = ValidationContext::default().with_parameters(["Known"]);
    let ds = diags_ctx("{?Missing} + 1", &ctx);
    assert_eq!(ds.len(), 1);
    assert!(ds[0].message.contains("parameter"), "{}", ds[0].message);
    assert!(ds[0].message.contains("Missing"), "{}", ds[0].message);
}

#[test]
fn known_parameter_passes() {
    let ctx = ValidationContext::default().with_parameters(["Known"]);
    assert!(diags_ctx("{?Known} + 1", &ctx).is_empty());
}

#[test]
fn reference_membership_is_case_insensitive() {
    let ctx = ValidationContext::default().with_fields(["Customer.Country"]);
    assert!(diags_ctx("{customer.country}", &ctx).is_empty());
}

#[test]
fn known_formula_reference_passes() {
    let ctx = ValidationContext::default().with_formulas(["other"]);
    assert!(diags_ctx("{@other} + 1", &ctx).is_empty());

    let ds = diags_ctx("{@nope} + 1", &ctx);
    assert_eq!(ds.len(), 1);
    assert!(ds[0].message.contains("formula"), "{}", ds[0].message);
}

#[test]
fn only_the_supplied_reference_kinds_are_checked() {
    // A parameter set is supplied but a field set is not: the unknown field is not flagged, the
    // unknown parameter is.
    let ctx = ValidationContext::default().with_parameters(["p"]);
    let ds = diags_ctx("{Table.Field} + {?q}", &ctx);
    assert_eq!(ds.len(), 1, "{}", messages(&ds));
    assert!(ds[0].message.contains("parameter"), "{}", ds[0].message);
}

// ---- spans (validate_str) -----------------------------------------------------------------

#[test]
fn validate_str_locates_unknown_function_token() {
    let ds = validate_str(
        "Frobnicate(1)",
        Syntax::Crystal,
        &ValidationContext::default(),
    );
    assert_eq!(ds.len(), 1);
    // Points at the `Frobnicate` identifier, not the whole formula.
    assert_eq!((ds[0].start, ds[0].end), (0, "Frobnicate".len()));
}

#[test]
fn validate_str_locates_unknown_reference_token() {
    let ctx = ValidationContext::default().with_parameters(["known"]);
    let ds = validate_str("1 + {?Missing}", Syntax::Crystal, &ctx);
    assert_eq!(ds.len(), 1);
    let src = "1 + {?Missing}";
    assert_eq!(&src[ds[0].start..ds[0].end], "{?Missing}");
}

#[test]
fn validate_str_includes_parse_diagnostics() {
    // A syntactic error surfaces from the parser, still with the unknown-function warning.
    let ds = validate_str(
        "Frobnicate(",
        Syntax::Crystal,
        &ValidationContext::default(),
    );
    assert!(ds.iter().any(|d| d.severity == Severity::Warning));
}

// ---- Basic syntax -------------------------------------------------------------------------

#[test]
fn basic_syntax_unknown_function() {
    let (node, _) = parse("Frobnicate(1)", Syntax::Basic);
    let ds = validate(&node, &ValidationContext::default());
    assert_eq!(ds.len(), 1);
    assert!(
        ds[0].message.contains("unknown function"),
        "{}",
        ds[0].message
    );
}

#[test]
fn basic_syntax_operator_type_error() {
    let (node, pdiags) = parse("formula = \"a\" * 2", Syntax::Basic);
    assert!(pdiags.is_empty(), "parse diags: {pdiags:?}");
    let ds = validate(&node, &ValidationContext::default());
    assert_eq!(ds.len(), 1, "{}", messages(&ds));
    assert!(
        ds[0].message.contains("cannot be applied"),
        "{}",
        ds[0].message
    );
}
