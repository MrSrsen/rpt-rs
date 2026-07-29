//! Type-conversion builtins (`Val`/`ToNumber`/`ToText`/`CCur`/`CBool`) and the `IsNumeric` predicate.

use super::{arg, bad_arg, mismatch, opt_num};
use crate::eval::{EvalError, Value};
use rpt_format_value::{
    format_currency, format_number, format_picture, parse_number_picture, CurrencyFormat,
    NumberFormat,
};

/// The conversion-family builtins. Wrapped by [`super::Builtin::Conversion`] in the dispatch table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConversionFn {
    Val,
    IsNumeric,
    ToNumber,
    CCur,
    CBool,
    ToText,
}

/// Handle a conversion builtin (routed here from [`super::call`]).
pub(super) fn call(b: ConversionFn, name: &str, args: &[Value]) -> Result<Value, EvalError> {
    use ConversionFn as B;
    match b {
        B::Val => {
            // Leading-prefix numeric parse (VB `Val`): longest numeric prefix, 0 if none.
            let s = super::str_arg(name, args, 0)?;
            let t = s.trim_start();
            let mut end = 0;
            for (i, c) in t.char_indices() {
                if c.is_ascii_digit()
                    || (c == '.' && !t[..i].contains('.'))
                    || ((c == '-' || c == '+') && i == 0)
                {
                    end = i + c.len_utf8();
                } else {
                    break;
                }
            }
            Ok(Value::Number(t[..end].parse().unwrap_or(0.0)))
        }
        B::IsNumeric => Ok(Value::Bool(match arg(name, args, 0)? {
            Value::Number(_) | Value::Currency(_) => true,
            Value::Str(s) => s.trim().parse::<f64>().is_ok(),
            _ => false,
        })),
        B::ToNumber => match arg(name, args, 0)? {
            Value::Number(n) | Value::Currency(n) => Ok(Value::Number(*n)),
            Value::Bool(b) => Ok(Value::Number(f64::from(*b))),
            Value::Str(s) => match s.trim().parse::<f64>() {
                Ok(n) => Ok(Value::Number(n)),
                Err(_) => Err(bad_arg(name, "string is not numeric")),
            },
            v => Err(mismatch(name, v)),
        },
        B::CCur => {
            let v = arg(name, args, 0)?;
            match v.as_number() {
                Some(n) => Ok(Value::Currency(n)),
                None => match v {
                    Value::Str(s) => {
                        match s.trim().trim_start_matches('$').replace(',', "").parse() {
                            Ok(n) => Ok(Value::Currency(n)),
                            Err(_) => Err(bad_arg(name, "string is not numeric")),
                        }
                    }
                    v => Err(mismatch(name, v)),
                },
            }
        }
        B::CBool => match arg(name, args, 0)? {
            Value::Bool(b) => Ok(Value::Bool(*b)),
            v => match v.as_number() {
                Some(n) => Ok(Value::Bool(n != 0.0)),
                None => Err(mismatch(name, v)),
            },
        },
        B::ToText => to_text(name, args),
    }
}

/// The first character of a separator-string argument, or `default` when absent/empty.
fn sep_arg(arg: Option<&Value>, default: char) -> char {
    match arg {
        Some(Value::Str(s)) => s.chars().next().unwrap_or(default),
        _ => default,
    }
}

/// `ToText`/`CStr`. Forms: bare scalars; Number/Currency with either a **picture string**
/// (`ToText(x, "#,##0.00")`, via [`parse_number_picture`]) or a numeric `decimals` arg (+ optional
/// thousands/decimal separator strings); date/time with a **picture string**
/// (`ToText(d, "dd-MMM-yyyy")`, via [`format_picture`]) or, bare, the default pattern.
fn to_text(name: &str, args: &[Value]) -> Result<Value, EvalError> {
    let v = args
        .first()
        .ok_or_else(|| EvalError::BadArg(format!("{name}: missing argument")))?;
    match v {
        Value::Number(_) | Value::Currency(_) => {
            let n = v.as_number().unwrap();
            let currency = matches!(v, Value::Currency(_));
            // Picture-string form: `ToText(x, "#,##0.00")`.
            if let Some(Value::Str(picture)) = args.get(1) {
                let spec = parse_number_picture(picture).ok_or_else(|| {
                    EvalError::Unsupported(format!("ToText picture string `{picture}`"))
                })?;
                let base = format_number(n, &spec);
                return Ok(Value::Str(if currency { format!("${base}") } else { base }));
            }
            // Numeric-decimals form: `ToText(x[, decimals[, thouSep[, decSep]]])`.
            let decimals = opt_num(args, 1).unwrap_or(2.0).max(0.0) as u32;
            let thousands_sep = sep_arg(args.get(2), ',');
            let decimal_sep = sep_arg(args.get(3), '.');
            let number = NumberFormat {
                decimals,
                thousands_sep,
                decimal_sep,
                ..NumberFormat::default()
            };
            Ok(Value::Str(if currency {
                format_currency(
                    n,
                    &CurrencyFormat {
                        number,
                        ..CurrencyFormat::default()
                    },
                )
            } else {
                format_number(n, &number)
            }))
        }
        Value::Date(_) | Value::Time(_) | Value::DateTime(..) => {
            // Picture-string form: `ToText(<date/time>, "dd-MMM-yyyy")`. One pattern resolves date
            // tokens (`d`/`M`/`y`) and time tokens (`H`/`h`/`m`/`s`/`t`); a bare date/time carries
            // only its own tokens.
            if let Some(Value::Str(picture)) = args.get(1) {
                let (date, time) = match v {
                    Value::Date(d) => (Some(*d), None),
                    Value::Time(t) => (None, Some(*t)),
                    Value::DateTime(d, t) => (Some(*d), Some(*t)),
                    // The enclosing arm already narrowed `v` to exactly these three.
                    other => {
                        debug_assert!(false, "temporal arm reached with {other:?}");
                        return Err(EvalError::Internal("ToText temporal narrowing"));
                    }
                };
                return Ok(Value::Str(format_picture(date, time, picture)));
            }
            // A date/time always has a default text form; if that ever stops holding, report it
            // rather than unwrapping into a panic.
            match v.to_text_default() {
                Some(s) => Ok(Value::Str(s)),
                None => Err(mismatch(name, v)),
            }
        }
        Value::Str(_) | Value::Bool(_) | Value::Null => Ok(Value::Str(
            v.to_text_default().ok_or_else(|| mismatch(name, v))?,
        )),
        v => Err(mismatch(name, v)),
    }
}

#[cfg(test)]
mod tests {
    use crate::eval::{eval, EmptyContext, EvalError, Value};
    use crate::{parse, Syntax};

    fn run(src: &str) -> Result<Value, EvalError> {
        let (ast, diags) = parse(src, Syntax::Crystal);
        assert!(diags.is_empty(), "parse diagnostics for `{src}`: {diags:?}");
        eval(&ast, &EmptyContext)
    }
    fn num(src: &str) -> f64 {
        match run(src) {
            Ok(Value::Number(n)) => n,
            other => panic!("`{src}` → {other:?}"),
        }
    }
    fn text(src: &str) -> String {
        match run(src) {
            Ok(Value::Str(s)) => s,
            other => panic!("`{src}` → {other:?}"),
        }
    }
    fn boolean(src: &str) -> bool {
        match run(src) {
            Ok(Value::Bool(b)) => b,
            other => panic!("`{src}` → {other:?}"),
        }
    }

    #[test]
    fn val_and_tonumber() {
        assert_eq!(num(r#"Val("12.5abc")"#), 12.5);
        assert_eq!(num(r#"Val("abc")"#), 0.0);
        assert_eq!(num(r#"Val("-3.2")"#), -3.2);
        assert_eq!(num(r#"ToNumber("42")"#), 42.0);
        assert_eq!(num("ToNumber(true)"), 1.0);
    }

    #[test]
    fn is_numeric_and_cbool_ccur() {
        assert!(boolean(r#"IsNumeric("3.14")"#));
        assert!(!boolean(r#"IsNumeric("pi")"#));
        assert!(boolean("IsNumeric(5)"));
        assert!(!boolean("CBool(0)"));
        assert!(boolean("CBool(5)"));
        assert!(boolean("CBool(-1)"));
        assert_eq!(run("CCur(5)"), Ok(Value::Currency(5.0)));
        assert_eq!(run(r#"CCur("$1,234.50")"#), Ok(Value::Currency(1234.5)));
        // Currency keeps full precision — the engine snaps no value, only the display format is 2dp.
        assert_eq!(run("CCur(1.23456)"), Ok(Value::Currency(1.23456)));
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
        assert_eq!(text("ToText(1234.5, \"#,##0.00\")"), "1,234.50");
        assert_eq!(text("ToText(12, \"###\")"), "12");
    }

    #[test]
    fn totext_date_picture() {
        assert_eq!(
            text(r#"ToText(Date(2023,1,31), "dd-MMM-yyyy")"#),
            "31-Jan-2023"
        );
        assert_eq!(
            text(r#"ToText(Date(2023,1,31), "yyyy-MM-dd")"#),
            "2023-01-31"
        );
        assert_eq!(text(r#"ToText(Date(2023,1,31), "M/d/yyyy")"#), "1/31/2023");
        assert_eq!(text(r#"ToText(Date(2023,1,31), "dddd")"#), "Tuesday");
        // DateTime picture mixing date + time tokens (24h and 12h/AM-PM).
        assert_eq!(
            text(r#"ToText(DateTime(2023,1,31,14,5,9), "yyyy-MM-dd HH:mm:ss")"#),
            "2023-01-31 14:05:09"
        );
        assert_eq!(
            text(r#"ToText(DateTime(2023,1,31,14,5,9), "hh:mm tt")"#),
            "02:05 PM"
        );
        // Bare Time value with a time-only picture.
        assert_eq!(text(r#"ToText(Time(9,7,0), "H:mm")"#), "9:07");
        // Regression: an Unsupported sub-call used to blank the whole enclosing concatenation.
        assert_eq!(
            text(r#""As of " & ToText(Date(2023,1,31), "yyyy-MM-dd")"#),
            "As of 2023-01-31"
        );
    }

    #[test]
    fn error_cases() {
        assert!(matches!(run(r#"ToNumber("x")"#), Err(EvalError::BadArg(_))));
        assert!(matches!(
            run("CBool(\"x\")"),
            Err(EvalError::TypeMismatch { .. })
        ));
        assert!(matches!(
            run(r#"ToText(12, "0.0%")"#),
            Err(EvalError::Unsupported(_))
        ));
    }
}
