//! Evaluator unit tests: operators, control flow, variables, and the tier-1 builtins.

use crate::eval::{eval, Date, EmptyContext, EvalError, MapContext, Time, Value};
use crate::token::RefKind;
use crate::{parse, Syntax};

/// Parse (Crystal syntax) + evaluate against an empty context.
fn run(src: &str) -> Result<Value, EvalError> {
    let (ast, diags) = parse(src, Syntax::Crystal);
    assert!(diags.is_empty(), "parse diagnostics for `{src}`: {diags:?}");
    eval(&ast, &EmptyContext)
}

fn run_ctx(src: &str, ctx: &MapContext) -> Result<Value, EvalError> {
    let (ast, diags) = parse(src, Syntax::Crystal);
    assert!(diags.is_empty(), "parse diagnostics for `{src}`: {diags:?}");
    eval(&ast, ctx)
}

fn num(src: &str) -> f64 {
    match run(src) {
        Ok(Value::Number(n)) => n,
        other => panic!("`{src}` → {other:?}, expected Number"),
    }
}

fn text(src: &str) -> String {
    match run(src) {
        Ok(Value::Str(s)) => s,
        other => panic!("`{src}` → {other:?}, expected Str"),
    }
}

fn boolean(src: &str) -> bool {
    match run(src) {
        Ok(Value::Bool(b)) => b,
        other => panic!("`{src}` → {other:?}, expected Bool"),
    }
}

// ---- operators ----

#[test]
fn arithmetic() {
    assert_eq!(num("1 + 2 * 3"), 7.0);
    assert_eq!(num("(1 + 2) * 3"), 9.0);
    assert_eq!(num("10 / 4"), 2.5);
    assert_eq!(num("10 \\ 4"), 2.0);
    assert_eq!(num("10 Mod 3"), 1.0);
    assert_eq!(num("2 ^ 10"), 1024.0);
    assert_eq!(num("-3 + 1"), -2.0);
    assert_eq!(num("50 % 200"), 25.0); // x % y = 100*x/y
}

#[test]
fn division_by_zero_errors() {
    assert_eq!(run("1 / 0"), Err(EvalError::DivideByZero));
    assert_eq!(run("1 Mod 0"), Err(EvalError::DivideByZero));
}

#[test]
fn currency_promotion() {
    assert_eq!(run("$2 + 3"), Ok(Value::Currency(5.0)));
    assert_eq!(run("2 * $3"), Ok(Value::Currency(6.0)));
}

#[test]
fn string_ops() {
    assert_eq!(text(r#""foo" + "bar""#), "foobar");
    assert_eq!(text(r#""a" & "b""#), "ab");
    // `&` coerces numbers with the default 2-decimal grouped format.
    assert_eq!(text(r#""n=" & 1234.5"#), "n=1,234.50");
    assert!(boolean(r#""abc" startswith "ab""#));
    assert!(boolean(r#""b" in "abc""#));
    assert!(!boolean(r#""z" in "abc""#));
    assert!(boolean(r#""hello" like "h*o""#));
    assert!(boolean(r#""hat" like "?at""#));
    assert!(!boolean(r#""chat" like "?at""#));
}

#[test]
fn comparisons_and_logic() {
    assert!(boolean("1 < 2 And 2 <= 2"));
    assert!(boolean("3 > 2 Or false"));
    assert!(boolean("Not (1 = 2)"));
    assert!(boolean("1 <> 2"));
    assert!(boolean(r#""a" < "b""#));
    assert!(boolean("true Xor false"));
    assert!(boolean("#1/2/2003# < #1/3/2003#"));
}

#[test]
fn ranges_and_in() {
    assert!(boolean("5 in 1 to 10"));
    assert!(boolean("1 in 1 to 10"));
    assert!(!boolean("1 in 1 _to 10"));
    assert!(!boolean("10 in 1 to_ 10"));
    assert!(boolean("3 in [1, 3, 5]"));
    assert!(!boolean("2 in [1, 3, 5]"));
}

#[test]
fn array_subscript_is_one_based() {
    assert_eq!(num("[10, 20, 30][2]"), 20.0);
    assert!(matches!(run("[10][0]"), Err(EvalError::BadArg(_))));
}

// ---- control flow, variables ----

#[test]
fn if_expression() {
    assert_eq!(num("If true Then 1 Else 2"), 1.0);
    assert_eq!(num("If false Then 1 Else 2"), 2.0);
    assert_eq!(num("If false Then 1 Else If true Then 3 Else 2"), 3.0);
    // No Else: the then-branch type's default.
    assert_eq!(text(r#"If false Then "x""#), "");
    assert_eq!(num("If false Then 42"), 0.0);
}

#[test]
fn variables_and_declarations() {
    assert_eq!(num("Local NumberVar x := 5; x * 2"), 10.0);
    assert_eq!(text(r#"StringVar s; s := "hi"; s & "!""#), "hi!");
    // Uninitialised numeric variable defaults to 0.
    assert_eq!(num("NumberVar n; n + 1"), 1.0);
    // Re-declaration does not reset.
    assert_eq!(num("NumberVar n; n := 7; NumberVar n; n"), 7.0);
}

#[test]
fn worrall_cctld_formula_shape() {
    // The phase-1 gate formula (worrall_AlphaISOsByCountry `@CCTLD_formatted`).
    let src = "Local StringVar DotCode := {countries_all_iso.internet_cctld};\n\
               If Length(DotCode) = 0 Then \"-\" Else DotCode;";
    let ctx = MapContext::default().with_field(
        RefKind::Field,
        "countries_all_iso.internet_cctld",
        Value::Str(".af".into()),
    );
    assert_eq!(run_ctx(src, &ctx), Ok(Value::Str(".af".into())));
    let empty = MapContext::default().with_field(
        RefKind::Field,
        "countries_all_iso.internet_cctld",
        Value::Str(String::new()),
    );
    assert_eq!(run_ctx(src, &empty), Ok(Value::Str("-".into())));
}

// ---- lazy forms ----

#[test]
fn iif_switch_choose_are_lazy() {
    // The unselected branch must not evaluate (division by zero).
    assert_eq!(num("IIf(true, 1, 1/0)"), 1.0);
    assert_eq!(num("Switch(false, 1/0, true, 42)"), 42.0);
    assert_eq!(num("Choose(2, 1/0, 7)"), 7.0);
}

// ---- null semantics ----

#[test]
fn null_propagation() {
    let ctx = MapContext::default().with_field(RefKind::Field, "t.x", Value::Null);
    assert_eq!(run_ctx("{t.x} + 1", &ctx), Ok(Value::Null));
    assert_eq!(run_ctx("{t.x} = 1", &ctx), Ok(Value::Bool(false)));
    assert_eq!(run_ctx("IsNull({t.x})", &ctx), Ok(Value::Bool(true)));
    assert_eq!(run_ctx("Length({t.x})", &ctx), Ok(Value::Null));
    // Strict null propagation (the engine's default null mode): `&` with Null is Null.
    assert_eq!(run_ctx(r#""a" & {t.x}"#, &ctx), Ok(Value::Null));
}

// ---- failure modes ----

#[test]
fn unknown_vs_unsupported() {
    assert!(matches!(run("NoSuchFn(1)"), Err(EvalError::UnknownName(_))));
    // `Previous` is a known funcID but needs record context → Unsupported.
    assert!(matches!(run("Previous(1)"), Err(EvalError::Unsupported(_))));
}

// ---- builtins ----

#[test]
fn string_builtins() {
    assert_eq!(num(r#"Length("abcd")"#), 4.0);
    assert_eq!(text(r#"UpperCase("aBc")"#), "ABC");
    assert_eq!(text(r#"LowerCase("aBc")"#), "abc");
    assert_eq!(
        text(r#"ProperCase("john SMITH-jones")"#),
        "John Smith-Jones"
    );
    assert_eq!(text(r#"Trim("  x  ")"#), "x");
    assert_eq!(text(r#"Left("crystal", 3)"#), "cry");
    assert_eq!(text(r#"Right("crystal", 3)"#), "tal");
    assert_eq!(text(r#"Mid("crystal", 2, 3)"#), "rys");
    assert_eq!(num(r#"InStr("hello", "ll")"#), 3.0);
    assert_eq!(num(r#"InStr("hello", "z")"#), 0.0);
    assert_eq!(num(r#"InStr(3, "hello hello", "he")"#), 7.0);
    assert_eq!(text(r#"Replace("a-b-c", "-", "+")"#), "a+b+c");
    assert_eq!(text(r#"ReplicateString("ab", 3)"#), "ababab");
    assert_eq!(text("Space(3)"), "   ");
    assert_eq!(text(r#"StrReverse("abc")"#), "cba");
    assert_eq!(text("Chr(65)"), "A");
    assert_eq!(num(r#"Asc("A")"#), 65.0);
    assert_eq!(num(r#"Val("12.5abc")"#), 12.5);
    assert!(boolean(r#"IsNumeric("3.14")"#));
    assert!(!boolean(r#"IsNumeric("pi")"#));
    assert_eq!(num(r#"ToNumber("42")"#), 42.0);
}

#[test]
fn totext_forms() {
    assert_eq!(text("ToText(1234.5)"), "1,234.50");
    assert_eq!(text("ToText(1234.5, 0)"), "1,235");
    assert_eq!(text(r#"ToText(1234.5, 1, ".", ",")"#), "1.234,5");
    assert_eq!(text("ToText(true)"), "True");
    assert_eq!(text(r#"ToText("x")"#), "x");
    assert_eq!(text("ToText($1)"), "$1.00");
    assert_eq!(text("ToText(#1/3/2004#)"), "1/3/2004");
    // Picture-string form: `#,##0.00`-style masks format.
    assert_eq!(text("ToText(1234.5, \"#,##0.00\")"), "1,234.50");
    assert_eq!(text("ToText(12, \"###\")"), "12");
    // Percent/section pictures remain unsupported (parser declines rather than guess).
    assert!(matches!(
        run("ToText(12, \"0.0%\")"),
        Err(EvalError::Unsupported(_))
    ));
}

#[test]
fn math_builtins() {
    assert_eq!(num("Abs(-3)"), 3.0);
    assert_eq!(num("Int(2.7)"), 2.0);
    assert_eq!(num("Int(-2.7)"), -3.0); // floor
    assert_eq!(num("Truncate(-2.7)"), -2.0); // toward zero
    assert_eq!(num("Truncate(2.789, 2)"), 2.78);
    assert_eq!(num("Round(2.5)"), 3.0);
    assert_eq!(num("Round(2.345, 2)"), 2.35);
    assert_eq!(num("Remainder(10, 3)"), 1.0);
    assert_eq!(num("Sqr(9)"), 3.0);
    assert_eq!(num("Sgn(-9)"), -1.0);
    assert_eq!(num("Sum([1, 2, 3])"), 6.0);
    assert_eq!(num("Average([1, 2, 3])"), 2.0);
    assert_eq!(num("Maximum([1, 9, 3])"), 9.0);
    assert_eq!(num("Minimum([4, 2, 3])"), 2.0);
    assert_eq!(num("Count([1, 2, 3])"), 3.0);
    assert_eq!(num("UBound([1, 2, 3])"), 3.0);
}

#[test]
fn date_builtins() {
    assert_eq!(
        run("Date(2004, 1, 3)"),
        Ok(Value::Date(Date::new(2004, 1, 3)))
    );
    assert_eq!(num("Year(#1/3/2004#)"), 2004.0);
    assert_eq!(num("Month(#1/3/2004#)"), 1.0);
    assert_eq!(num("Day(#1/3/2004#)"), 3.0);
    assert_eq!(num("Hour(#1/3/2004 14:05:06#)"), 14.0);
    assert_eq!(num("Minute(#14:05:06#)"), 5.0);
    // 2004-01-03 was a Saturday (Crystal: Sunday=1 … Saturday=7).
    assert_eq!(num("DayOfWeek(#1/3/2004#)"), 7.0);
    assert_eq!(text("MonthName(2)"), "February");
    assert_eq!(text("WeekdayName(1)"), "Sunday");
    assert_eq!(num("#1/10/2004# - #1/3/2004#"), 7.0);
    assert_eq!(
        run("#1/3/2004# + 4"),
        Ok(Value::Date(Date::new(2004, 1, 7)))
    );
    assert_eq!(num(r#"DateDiff("d", #1/3/2004#, #2/3/2004#)"#), 31.0);
    assert_eq!(num(r#"DateDiff("m", #1/3/2004#, #3/1/2004#)"#), 2.0);
    assert_eq!(num(r#"DateDiff("yyyy", #6/1/2003#, #1/1/2004#)"#), 1.0);
    assert_eq!(
        run(r#"DateAdd("m", 1, #1/31/2004#)"#),
        Ok(Value::DateTime(Date::new(2004, 2, 29), Time::new(0, 0, 0)))
    );
    assert!(boolean(r#"IsDate("1/3/2004")"#));
    assert!(!boolean(r#"IsDate("nope")"#));
    // Time arithmetic runs in seconds.
    assert_eq!(run("#04:05:06# + 60"), Ok(Value::Time(Time::new(4, 6, 6))));
}

#[test]
fn date_literals() {
    assert_eq!(run("#2004-1-3#"), Ok(Value::Date(Date::new(2004, 1, 3))));
    assert_eq!(
        run("#1/3/2004 4:05:06 pm#"),
        Ok(Value::DateTime(Date::new(2004, 1, 3), Time::new(16, 5, 6)))
    );
    assert_eq!(run("#12:00:00 am#"), Ok(Value::Time(Time::new(0, 0, 0))));
}

#[test]
fn specials_route_through_context() {
    let ctx = MapContext::default().with_special("pagenumber", Value::Number(3.0));
    assert_eq!(run_ctx("PageNumber", &ctx), Ok(Value::Number(3.0)));
    assert!(matches!(run("PageNumber"), Err(EvalError::Unsupported(_))));
    // Evaluation-time markers are no-op statements.
    assert_eq!(num("WhilePrintingRecords; 1 + 1"), 2.0);
}

#[test]
fn switch_heavy_shape() {
    // The corpus' dominant pattern (3,740 Switch calls).
    let src = r#"Switch({t.code} = 1, "one", {t.code} = 2, "two", true, "other")"#;
    let ctx = MapContext::default().with_field(RefKind::Field, "t.code", Value::Number(2.0));
    assert_eq!(run_ctx(src, &ctx), Ok(Value::Str("two".into())));
}

#[test]
fn civil_date_roundtrip() {
    // Exhaustive-ish roundtrip across leap boundaries.
    for &(y, m, d) in &[
        (1899, 12, 30),
        (1970, 1, 1),
        (2000, 2, 29),
        (2004, 2, 29),
        (2100, 2, 28),
        (2026, 7, 4),
    ] {
        let date = Date::new(y, m, d);
        assert_eq!(Date::from_days(date.to_days()), date, "{y}-{m}-{d}");
    }
}

#[test]
fn vm_matches_tree_walker() {
    use crate::eval::vm;
    use crate::{parse, RefKind, Syntax};

    let ctx = MapContext::default()
        .with_field(RefKind::Field, "t.x", Value::Number(10.0))
        .with_field(RefKind::Field, "t.y", Value::Number(3.0))
        .with_field(RefKind::Field, "t.name", Value::Str("Bob".into()))
        .with_field(RefKind::Field, "t.z", Value::Null)
        .with_field(RefKind::Parameter, "p", Value::Number(2.0))
        .with_special("pagenumber", Value::Number(5.0));

    let cases = [
        "1 + 2 * 3",
        "{t.x} - {t.y}",
        "{t.x} / {t.y}",
        "{t.x} / 0",
        "{t.x} > {t.y} And {t.y} > 0",
        "{t.x} > {t.y} Or {t.z} > 0",
        "\"a\" & \"b\" & {t.name}",
        "{t.x} = 10",
        "{t.z} + 1",
        "{t.z} = 10",
        "Not ({t.x} > 5)",
        "If {t.x} > 5 Then \"big\" Else \"small\"",
        "If {t.z} > 5 Then 1",
        "If {t.x} > 100 Then 1",
        "If {t.x} > 100 Then 1 Else If {t.y} > 1 Then 2 Else 3",
        "IIf({t.x} > 5, 100, 200)",
        "IIf({t.z} > 5, 100, 200)",
        "Choose(2, 10, 20, 30)",
        "Choose(5, 10, 20)",
        "Choose({t.z}, 10, 20)",
        "[1, 2, 3][2]",
        "{?p} * 3",
        "PageNumber + 1",
        "Local NumberVar v := {t.x} * 2; v + 1",
        "-{t.x}",
        // Switch null-fallthrough: the first (null) condition is skipped, the second matches.
        "Switch({t.z} = 1, \"a\", {t.x} = 10, \"b\")",
        "Switch({t.x} = 1, \"a\", {t.x} = 2, \"b\")",
        // Membership with an array of ranges, and a plain range.
        "5 In [1 To 3, 4 To 10]",
        "100 In [1 To 3, 4 To 10]",
        "{t.y} In 1 To 5",
        // String predicates.
        "{t.name} Like \"B?b\"",
        "{t.name} StartsWith \"Bo\"",
        // Date / time arithmetic.
        "Date(2024, 1, 3) + 5",
        "Date(2024, 1, 3) - Date(2024, 1, 1)",
        "Time(9, 30, 0)",
        // Nested If with a null guard in the outer condition.
        "If {t.z} > 5 Then 1 Else If {t.x} > 5 Then 2 Else 3",
    ];
    for src in cases {
        let (ast, _) = parse(src, Syntax::Crystal);
        // Reference is the tree-walker directly (the free `eval` fn now runs the VM).
        let tw = format!("{:?}", crate::eval::Evaluator::new(&ctx).eval(&ast));
        let chunk = vm::compile(&ast);
        let vmr = format!("{:?}", vm::run(&chunk, &ctx));
        assert_eq!(tw, vmr, "VM != tree-walker for {src:?}");
    }
}

#[test]
fn vm_matches_tree_walker_loops() {
    use crate::eval::vm;
    use crate::{parse, Syntax};

    // Multi-statement loop bodies with `Exit` (Basic syntax), including nesting, conditional and
    // trailing breaks, and an `Exit` with no enclosing loop (a clean error in both evaluators).
    let cases = [
        "Dim t As Number\nt = 0\nFor i = 1 To 10\nt = t + i\nIf i >= 3 Then Exit For\nNext\nformula = t",
        "Dim n As Number\nn = 0\nWhile n < 100\nn = n + 1\nIf n >= 5 Then Exit While\nWend\nformula = n",
        "Dim n As Number\nn = 0\nDo While n < 100\nn = n + 1\nIf n >= 7 Then Exit Do\nLoop\nformula = n",
        "Dim c As Number\nc = 0\nFor i = 1 To 3\nFor j = 1 To 10\nIf j >= 3 Then Exit For\nc = c + 1\nNext j\nNext i\nformula = c",
        "Dim n As Number\nn = 0\nFor i = 1 To 5\nn = n + 1\nExit For\nNext\nformula = n",
        "Exit For",
        "Dim t As Number\nt = 0\nFor i = 1 To 3\nt = t + i\nNext\nformula = t",
        // A bare `Do … Loop` broken by `Exit Do`.
        "Dim n As Number\nn = 0\nDo\nn = n + 1\nIf n >= 4 Then Exit Do\nLoop\nformula = n",
    ];
    for src in cases {
        let (ast, _) = parse(src, Syntax::Basic);
        let tw = format!(
            "{:?}",
            crate::eval::Evaluator::new(&EmptyContext).eval(&ast)
        );
        let chunk = vm::compile(&ast);
        let vmr = format!("{:?}", vm::run(&chunk, &EmptyContext));
        assert_eq!(tw, vmr, "VM != tree-walker for {src:?}");
    }
}

#[test]
fn vm_aborts_a_runaway_loop_at_its_per_loop_cap() {
    use crate::eval::vm;
    use crate::{parse, Syntax};

    // A non-terminating loop trips the per-loop iteration guard (the VM formerly used an unrelated
    // per-instruction step cap). Uses a small injected cap so the test is fast; production uses the
    // shared LOOP_LIMIT that also bounds the tree-walker.
    let src = "Dim n As Number\nn = 0\nDo\nn = n + 1\nLoop\nformula = n";
    let (ast, _) = parse(src, Syntax::Basic);
    let chunk = vm::compile(&ast);
    let r = vm::run_with_loop_limit(&chunk, &EmptyContext, 1000);
    assert!(
        r.is_err() && format!("{r:?}").contains("loop iteration limit"),
        "expected a loop-limit error, got {r:?}"
    );
}

#[test]
fn vm_counts_loop_iterations_per_loop_not_globally() {
    use crate::eval::vm;
    use crate::{parse, Syntax};

    // Many sequential loops whose combined iteration count dwarfs any single loop: each loop resets
    // its own budget (the fix), so this completes rather than tripping a global counter. A small
    // injected cap (5000, above any single loop's 2000 but below the 100000 total) proves the reset,
    // and both evaluators agree on the result.
    let src = "Dim t As Number\nt = 0\nFor k = 1 To 50\nFor i = 1 To 2000\nt = t + 1\nNext i\nNext k\nformula = t";
    let (ast, _) = parse(src, Syntax::Basic);
    let tw = crate::eval::Evaluator::new(&EmptyContext).eval(&ast);
    let chunk = vm::compile(&ast);
    let vmr = vm::run_with_loop_limit(&chunk, &EmptyContext, 5000);
    assert_eq!(format!("{tw:?}"), format!("{vmr:?}"));
    assert_eq!(
        format!("{vmr:?}"),
        format!("{:?}", Ok::<_, crate::EvalError>(Value::Number(100_000.0)))
    );
}

#[test]
fn vm_matches_tree_walker_non_bool_condition_errors() {
    use crate::eval::vm;
    use crate::{parse, Syntax};

    // A non-Bool condition is a clean error in both evaluators, and both report the same
    // per-construct `what` (If / IIf / Switch condition).
    let cases = [
        "If 5 Then 1 Else 2",
        "IIf(5, 1, 2)",
        "Switch(5, 1, True, 2)",
    ];
    for src in cases {
        let (ast, _) = parse(src, Syntax::Crystal);
        let tw = format!(
            "{:?}",
            crate::eval::Evaluator::new(&EmptyContext).eval(&ast)
        );
        let chunk = vm::compile(&ast);
        let vmr = format!("{:?}", vm::run(&chunk, &EmptyContext));
        assert_eq!(tw, vmr, "VM != tree-walker for {src:?}");
        assert!(
            tw.contains("condition"),
            "expected a condition error for {src:?}: {tw}"
        );
    }
}
