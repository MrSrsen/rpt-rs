//! Type-system tests: result-type deduction + string-length.

use crate::formula::parse;
use crate::formula::token::{RefKind, Syntax};
use crate::formula::types::{deduce_type, func_id, string_max_bytes, ResultKind as K};

/// Deduce the result kind of a Crystal formula body with no resolvable references.
fn ty(src: &str) -> K {
    let (node, diags) = parse(src, Syntax::Crystal);
    assert!(diags.is_empty(), "parse diags for {src:?}: {diags:?}");
    deduce_type(&node, &|_, _| None)
}

/// Deduce with a reference resolver mapping every reference name to a fixed kind.
fn ty_ref(src: &str, f: impl Fn(RefKind, &str) -> Option<K>) -> K {
    let (node, diags) = parse(src, Syntax::Crystal);
    assert!(diags.is_empty(), "parse diags for {src:?}: {diags:?}");
    deduce_type(&node, &f)
}

// ---- literals -----------------------------------------------------------------------------

#[test]
fn literals() {
    assert_eq!(ty("42"), K::Number);
    assert_eq!(ty("3.14"), K::Number);
    assert_eq!(ty("\"hello\""), K::String);
    assert_eq!(ty("True"), K::Boolean);
    assert_eq!(ty("False"), K::Boolean);
}

// ---- the `&` vs `+` distinction -----------------------------------------------------------

#[test]
fn amp_is_always_string() {
    assert_eq!(ty("\"a\" & \"b\""), K::String);
    assert_eq!(ty("\"x\" & 5"), K::String); // `&` coerces the number operand
    assert_eq!(ty("1 & 2"), K::String); // even all-numeric: `&` is concat
}

#[test]
fn plus_is_arithmetic_or_string_concat() {
    assert_eq!(ty("1 + 2"), K::Number); // numeric add
    assert_eq!(ty("\"a\" + \"b\""), K::String); // string `+` is concat
}

// ---- arithmetic + numeric promotion -------------------------------------------------------

#[test]
fn arithmetic_returns_number() {
    assert_eq!(ty("2 * 3"), K::Number);
    assert_eq!(ty("7 / 2"), K::Number);
    assert_eq!(ty("7 \\ 2"), K::Number);
    assert_eq!(ty("7 Mod 2"), K::Number);
    assert_eq!(ty("2 ^ 10"), K::Number); // `^` is always Number
}

#[test]
fn currency_prefix_and_promotion() {
    assert_eq!(ty("$5"), K::Currency); // `$` prefix
    assert_eq!(ty("$5 + 3"), K::Currency); // Number↔Currency promotion in `+`
    assert_eq!(ty("$5 * 2"), K::Currency); // promotion in `*`
    assert_eq!(ty("$10 - $3"), K::Currency);
}

#[test]
fn unary_copies_operand() {
    assert_eq!(ty("-5"), K::Number);
    assert_eq!(ty("-$5"), K::Currency);
    assert_eq!(ty("+5"), K::Number);
}

// ---- comparisons / boolean ops -> Boolean -------------------------------------------------

#[test]
fn comparisons_and_bool_ops_are_boolean() {
    assert_eq!(ty("1 = 2"), K::Boolean);
    assert_eq!(ty("1 <> 2"), K::Boolean);
    assert_eq!(ty("1 < 2"), K::Boolean);
    assert_eq!(ty("1 >= 2"), K::Boolean);
    assert_eq!(ty("True And False"), K::Boolean);
    assert_eq!(ty("True Or False"), K::Boolean);
    assert_eq!(ty("Not True"), K::Boolean);
    assert_eq!(ty("\"a\" Like \"b*\""), K::Boolean);
    assert_eq!(ty("\"a\" StartsWith \"b\""), K::Boolean);
}

// ---- function return types (fixed) --------------------------------------------------------

#[test]
fn fixed_function_returns() {
    assert_eq!(ty("Length(\"abc\")"), K::Number);
    assert_eq!(ty("Len(\"abc\")"), K::Number);
    assert_eq!(ty("ToText(5)"), K::String);
    assert_eq!(ty("CStr(5)"), K::String);
    assert_eq!(ty("UpperCase(\"x\")"), K::String);
    assert_eq!(ty("Left(\"abc\", 2)"), K::String);
    assert_eq!(ty("IsNull(5)"), K::Boolean);
    assert_eq!(ty("CDate(\"2020-01-01\")"), K::Date);
    assert_eq!(ty("CTime(\"12:00\")"), K::Time);
    assert_eq!(ty("CCur(5)"), K::Currency);
    assert_eq!(ty("CurrentDate"), K::Date); // 0-ary special field as Ident
    assert_eq!(ty("Roman(5)"), K::String);
}

// ---- overloaded function returns ----------------------------------------------------------

#[test]
fn aggregation_returns_arg_numeric_type() {
    // Sum returns the numeric type of its value arg.
    assert_eq!(
        ty_ref("Sum({t.f})", |_, n| if n == "t.f" {
            Some(K::Currency)
        } else {
            None
        }),
        K::Currency
    );
    assert_eq!(
        ty_ref("Sum({t.f})", |_, n| if n == "t.f" {
            Some(K::Number)
        } else {
            None
        }),
        K::Number
    );
    // Average of a currency field is still Currency.
    assert_eq!(
        ty_ref("Average({t.f})", |_, _| Some(K::Currency)),
        K::Currency
    );
}

#[test]
fn max_min_copy_arg_type() {
    assert_eq!(ty_ref("Maximum({t.d})", |_, _| Some(K::Date)), K::Date);
    assert_eq!(ty_ref("Minimum({t.s})", |_, _| Some(K::String)), K::String);
    // A range arg de-arrays to its element scalar.
    assert_eq!(ty_ref("Maximum({t.r})", |_, _| Some(K::DateRange)), K::Date);
}

#[test]
fn round_copies_numeric_arg() {
    assert_eq!(
        ty_ref("Round({t.c})", |_, _| Some(K::Currency)),
        K::Currency
    );
    assert_eq!(ty_ref("Round({t.n})", |_, _| Some(K::Number)), K::Number);
}

#[test]
fn iif_choose_switch_unify_branches() {
    assert_eq!(ty("IIf(1 > 0, \"a\", \"b\")"), K::String);
    assert_eq!(ty("IIf(1 > 0, 1, 2)"), K::Number);
    // numeric promotion across branches
    assert_eq!(ty("IIf(1 > 0, $1, 2)"), K::Currency);
    assert_eq!(ty("Choose(1, \"a\", \"b\", \"c\")"), K::String);
    assert_eq!(ty("Switch(1 > 0, 10, 1 < 0, 20)"), K::Number);
}

// ---- If-expression branch unification -----------------------------------------------------

#[test]
fn if_expr_unifies_branches() {
    assert_eq!(ty("If 1 > 0 Then \"a\" Else \"b\""), K::String);
    assert_eq!(ty("If 1 > 0 Then 1 Else 2"), K::Number);
    assert_eq!(ty("If 1 > 0 Then $1 Else 2"), K::Currency); // promotion
}

// ---- references via the lookup hook -------------------------------------------------------

#[test]
fn references_resolve_via_lookup() {
    let f = |k: RefKind, _: &str| match k {
        RefKind::Field => Some(K::Currency),
        RefKind::Parameter => Some(K::Date),
        RefKind::Formula => Some(K::String),
        _ => None,
    };
    assert_eq!(ty_ref("{t.amount}", f), K::Currency);
    assert_eq!(ty_ref("{?when}", f), K::Date);
    assert_eq!(ty_ref("{@other}", f), K::String);
    // unresolved reference -> Unknown
    assert_eq!(ty_ref("{t.x}", |_, _| None), K::Unknown);
    // expression mixing a currency field with a number stays Currency
    assert_eq!(ty_ref("{t.amount} + 1", f), K::Currency);
    assert_eq!(ty_ref("{t.amount} & \" each\"", f), K::String);
}

// ---- arrays / subscript -------------------------------------------------------------------

#[test]
fn array_and_subscript() {
    assert_eq!(ty("[1, 2, 3]"), K::NumberArray);
    assert_eq!(ty("[\"a\", \"b\"]"), K::StringArray);
    // subscript de-arrays back to the element scalar
    assert_eq!(ty("[1, 2, 3][1]"), K::Number);
    assert_eq!(ty("Split(\"a,b\", \",\")"), K::StringArray);
}

// ---- ResultKind classification helpers ----------------------------------------------------

#[test]
fn result_kind_helpers() {
    assert!(K::Number.is_scalar());
    assert!(K::NumberRange.is_range());
    assert!(K::StringArray.is_array());
    assert!(K::DateRangeArray.is_range_array());
    assert_eq!(K::Number.to_array(), K::NumberArray);
    assert_eq!(K::String.to_array(), K::StringArray);
    assert_eq!(K::StringArray.to_scalar(), K::String);
    assert_eq!(K::DateRange.to_scalar(), K::Date);
    assert_eq!(K::Date.to_range(), K::DateRange);
    // round-trip raw codes
    for raw in 0u8..=0x1b {
        if let Some(k) = K::from_raw(raw) {
            assert_eq!(k.raw(), raw);
        }
    }
}

#[test]
fn to_field_value_type_maps_scalars() {
    use rpt::model::FieldValueType as F;
    assert_eq!(K::Number.to_field_value_type(), F::Number);
    assert_eq!(K::Currency.to_field_value_type(), F::Currency);
    assert_eq!(K::Boolean.to_field_value_type(), F::Boolean);
    assert_eq!(K::Date.to_field_value_type(), F::Date);
    assert_eq!(K::Time.to_field_value_type(), F::Time);
    assert_eq!(K::DateTime.to_field_value_type(), F::DateTime);
    assert_eq!(K::String.to_field_value_type(), F::String);
    assert_eq!(K::NumberArray.to_field_value_type(), F::Unknown);
    assert_eq!(K::Unknown.to_field_value_type(), F::Unknown);
}

#[test]
fn func_id_lookup_is_case_insensitive() {
    assert_eq!(func_id("ToText"), func_id("totext"));
    assert!(func_id("Sum").is_some());
    assert!(func_id("definitely_not_a_builtin_xyz").is_none());
}

// ---- string_max_bytes ---------------------------------------------------------------------

fn sbytes(src: &str) -> i32 {
    let (node, diags) = parse(src, Syntax::Crystal);
    assert!(diags.is_empty(), "parse diags for {src:?}: {diags:?}");
    string_max_bytes(&node, &|_, _| None, &|_, _| None)
}

#[test]
fn string_literal_byte_width() {
    // (chars + 1) * 2
    assert_eq!(sbytes("\"hello\""), (5 + 1) * 2);
    assert_eq!(sbytes("\"\""), 2);
}

#[test]
fn concat_byte_width() {
    // `&`: strByteLen(L) + strByteLen(R) − 2; two literals = (a+1)*2 + (b+1)*2 − 2.
    assert_eq!(sbytes("\"ab\" & \"cde\""), (2 + 1) * 2 + (3 + 1) * 2 - 2);
    // `+` string concat: same arithmetic.
    assert_eq!(sbytes("\"ab\" + \"cde\""), (2 + 1) * 2 + (3 + 1) * 2 - 2);
}

#[test]
fn totext_of_date_is_conservative_244() {
    // ToText(<date>) bare → (60 + 1) * 4 = 244.
    assert_eq!(sbytes("ToText(CurrentDate)"), 244);
    // ToText(<boolean>) → 15 * 4 = 60.
    assert_eq!(sbytes("ToText(1 > 0)"), 60);
}

#[test]
fn totext_number_is_format_aware() {
    // ToText number char count depends on the format args.
    assert_eq!(sbytes("ToText(123, '#')"), (41 + 1) * 4); // 168
    assert_eq!(sbytes("ToText(123, 0, \"\")"), (40 + 1) * 4); // 164
    assert_eq!(sbytes("ToText(123)"), (75 + 1) * 4); // 304 (bare default)
                                                     // PageXofY shape: ToText(n,'#') + ' ' + ToText(n,'#') with the two 168s and a 4-byte literal.
    assert_eq!(
        sbytes("ToText(123,'#') + ' ' + ToText(456,'#')"),
        168 + 4 + 168 - 2 * 2
    );
}

#[test]
fn const_width_and_copy_rules() {
    assert_eq!(sbytes("Roman(5)"), 32); // const 32
    assert_eq!(sbytes("Chr(65)"), 6); // const 6
                                      // UpperCase copies arg1 width.
    assert_eq!(sbytes("UpperCase(\"hello\")"), (5 + 1) * 2);
    // Left(s, n): n const wide chars × 4.
    assert_eq!(sbytes("Left(\"hello\", 3)"), 3 * 4);
    // Space(n): n*2 + 2.
    assert_eq!(sbytes("Space(4)"), 4 * 2 + 2);
}

#[test]
fn unbounded_caps_at_max() {
    // Join is unbounded → the 65534 = 0xfffe sentinel (32767 chars × 2).
    assert_eq!(sbytes("Join([\"a\", \"b\"], \",\")"), 0xfffe);
}
