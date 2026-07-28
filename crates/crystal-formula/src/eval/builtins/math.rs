//! Math and array-aggregate builtins.

use super::{arg, map_numeric, mismatch, num_arg, opt_num};
use crate::eval::{EvalError, Value};

/// The math and array-aggregate builtins. Wrapped by [`super::Builtin::Math`] in the dispatch table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MathFn {
    Abs,
    Sgn,
    Int,
    Fix,
    Truncate,
    Round,
    RoundUp,
    MRound,
    Floor,
    Ceiling,
    Remainder,
    Sqr,
    Exp,
    Log,
    Sin,
    Cos,
    Tan,
    Atn,
    Pi,
    Minimum,
    Maximum,
    Sum,
    Average,
    Count,
    UBound,
}

/// Handle a math/aggregate builtin (routed here from [`super::call`]).
pub(super) fn call(b: MathFn, name: &str, args: &[Value]) -> Result<Value, EvalError> {
    use MathFn as B;
    match b {
        B::Abs => map_numeric(name, args, 0, f64::abs),
        B::Sgn => Ok(Value::Number(match num_arg(name, args, 0)? {
            n if n > 0.0 => 1.0,
            n if n < 0.0 => -1.0,
            _ => 0.0,
        })),
        B::Int => map_numeric(name, args, 0, f64::floor),
        B::Fix => map_numeric(name, args, 0, f64::trunc),
        B::Floor => {
            let m = opt_multiple(name, args)?;
            map_numeric(name, args, 0, |n| (n / m).floor() * m)
        }
        B::Ceiling => {
            let m = opt_multiple(name, args)?;
            map_numeric(name, args, 0, |n| (n / m).ceil() * m)
        }
        B::RoundUp => {
            // Round away from zero to `places` decimals (default 0).
            let places = opt_num(args, 1).unwrap_or(0.0) as i32;
            let scale = 10f64.powi(places);
            map_numeric(name, args, 0, |n| {
                n.signum() * (n.abs() * scale).ceil() / scale
            })
        }
        B::MRound => {
            // Nearest multiple of arg1 (half away from zero); a zero multiple yields zero.
            let m = num_arg(name, args, 1)?;
            map_numeric(name, args, 0, |n| {
                if m == 0.0 {
                    0.0
                } else {
                    (n / m).round() * m
                }
            })
        }
        B::Pi => Ok(Value::Number(std::f64::consts::PI)),
        B::Truncate => {
            let places = opt_num(args, 1).unwrap_or(0.0) as i32;
            let scale = 10f64.powi(places);
            map_numeric(name, args, 0, |n| (n * scale).trunc() / scale)
        }
        B::Round => {
            let places = opt_num(args, 1).unwrap_or(0.0) as i32;
            let scale = 10f64.powi(places);
            // Half away from zero, like the engine (f64::round is half-away too).
            map_numeric(name, args, 0, |n| (n * scale).round() / scale)
        }
        B::Remainder => {
            let (a, b) = (num_arg(name, args, 0)?, num_arg(name, args, 1)?);
            if b == 0.0 {
                return Err(EvalError::DivideByZero);
            }
            Ok(Value::Number(a % b))
        }
        B::Sqr => Ok(Value::Number(num_arg(name, args, 0)?.sqrt())),
        B::Exp => Ok(Value::Number(num_arg(name, args, 0)?.exp())),
        B::Log => Ok(Value::Number(num_arg(name, args, 0)?.ln())),
        B::Sin => Ok(Value::Number(num_arg(name, args, 0)?.sin())),
        B::Cos => Ok(Value::Number(num_arg(name, args, 0)?.cos())),
        B::Tan => Ok(Value::Number(num_arg(name, args, 0)?.tan())),
        B::Atn => Ok(Value::Number(num_arg(name, args, 0)?.atan())),
        // Array forms only; record-set aggregation needs the data pipeline.
        B::Minimum | B::Maximum | B::Sum | B::Average | B::Count => aggregate(b, name, args),
        B::UBound => match arg(name, args, 0)? {
            Value::Array(a) => Ok(Value::Number(a.len() as f64)),
            v => Err(mismatch(name, v)),
        },
    }
}

/// The optional `multiple` argument of `Floor`/`Ceiling` (default 1); a zero multiple is a
/// divide-by-zero.
fn opt_multiple(name: &str, args: &[Value]) -> Result<f64, EvalError> {
    match args.get(1) {
        None => Ok(1.0),
        Some(v) => {
            let m = v.as_number().ok_or_else(|| mismatch(name, v))?;
            if m == 0.0 {
                Err(EvalError::DivideByZero)
            } else {
                Ok(m)
            }
        }
    }
}

/// `Minimum`/`Maximum`/`Sum`/`Average`/`Count` over an array argument. The record-set forms
/// (`Sum({field}, {group})`) need the data pipeline and are reported as such.
fn aggregate(builtin: MathFn, name: &str, args: &[Value]) -> Result<Value, EvalError> {
    // Min/Max accept a range directly — its bounds are the extremes.
    if let [Value::Range { lo, hi, .. }] = args {
        if matches!(builtin, MathFn::Minimum | MathFn::Maximum) {
            return Ok(if builtin == MathFn::Minimum {
                (**lo).clone()
            } else {
                (**hi).clone()
            });
        }
    }
    let items = super::array_arg(name, args)?;
    if builtin == MathFn::Count {
        return Ok(Value::Number(
            items.iter().filter(|v| !v.is_null()).count() as f64
        ));
    }
    let non_null: Vec<&Value> = items.iter().filter(|v| !v.is_null()).collect();
    if non_null.is_empty() {
        return Ok(Value::Null);
    }
    match builtin {
        MathFn::Minimum | MathFn::Maximum => {
            let mut best = non_null[0];
            for v in &non_null[1..] {
                let ord = crate::eval::compare(v, best)?;
                if (builtin == MathFn::Minimum && ord.is_lt())
                    || (builtin == MathFn::Maximum && ord.is_gt())
                {
                    best = v;
                }
            }
            Ok(best.clone())
        }
        MathFn::Sum | MathFn::Average => {
            let mut total = 0.0;
            let mut currency = false;
            for v in &non_null {
                total += v.as_number().ok_or_else(|| mismatch(name, v))?;
                currency |= matches!(v, Value::Currency(_));
            }
            let n = if builtin == MathFn::Average {
                total / non_null.len() as f64
            } else {
                total
            };
            Ok(if currency {
                Value::Currency(n)
            } else {
                Value::Number(n)
            })
        }
        // Every `MathFn` the dispatcher routes here has an arm above; a new variant without one lands
        // here, which is a gap in this crate rather than a bad formula.
        other => {
            debug_assert!(false, "unhandled MathFn: {other:?}");
            Err(EvalError::Internal("unhandled math builtin"))
        }
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

    #[test]
    fn rounding_family() {
        assert_eq!(num("Abs(-3)"), 3.0);
        assert_eq!(num("Int(2.7)"), 2.0);
        assert_eq!(num("Int(-2.7)"), -3.0);
        assert_eq!(num("Fix(2.7)"), 2.0);
        assert_eq!(num("Fix(-2.7)"), -2.0);
        assert_eq!(num("Truncate(2.789, 2)"), 2.78);
        assert_eq!(num("Round(2.5)"), 3.0);
        assert_eq!(num("Round(2.345, 2)"), 2.35);
        assert_eq!(num("Floor(-2.3)"), -3.0);
        assert_eq!(num("Floor(7, 5)"), 5.0);
        assert_eq!(num("Ceiling(2.1)"), 3.0);
        assert_eq!(num("Ceiling(7, 5)"), 10.0);
        assert_eq!(num("MRound(10, 3)"), 9.0);
        assert_eq!(num("MRound(11, 3)"), 12.0);
        assert_eq!(num("RoundUp(2.341, 2)"), 2.35);
        assert_eq!(num("RoundUp(-2.341, 2)"), -2.35);
    }

    #[test]
    fn currency_preserved_and_signs() {
        assert_eq!(run("Fix($2.7)"), Ok(Value::Currency(2.0)));
        assert_eq!(run("Abs($-3)"), Ok(Value::Currency(3.0)));
        assert_eq!(num("Sgn(-9)"), -1.0);
        assert_eq!(num("Sgn(9)"), 1.0);
        assert_eq!(num("Sgn(0)"), 0.0);
    }

    #[test]
    fn transcendental() {
        assert_eq!(num("Sqr(9)"), 3.0);
        assert!((num("crPi") - std::f64::consts::PI).abs() < 1e-12);
        assert!((num("Exp(0)") - 1.0).abs() < 1e-12);
        assert!((num("Log(Exp(1))") - 1.0).abs() < 1e-12);
        assert_eq!(num("Remainder(10, 3)"), 1.0);
    }

    #[test]
    fn error_cases() {
        assert_eq!(run("Remainder(1, 0)"), Err(EvalError::DivideByZero));
        assert_eq!(run("Ceiling(7, 0)"), Err(EvalError::DivideByZero));
        assert!(matches!(
            run("Abs(\"x\")"),
            Err(EvalError::TypeMismatch { .. })
        ));
        assert!(matches!(run("Sqr()"), Err(EvalError::Arity { .. })));
    }

    #[test]
    fn aggregates_over_arrays() {
        assert_eq!(num("Sum([1, 2, 3])"), 6.0);
        assert_eq!(num("Average([1, 2, 3])"), 2.0);
        assert_eq!(num("Maximum([1, 9, 3])"), 9.0);
        assert_eq!(num("Minimum([4, 2, 3])"), 2.0);
        assert_eq!(num("Count([1, 2, 3])"), 3.0);
        assert_eq!(num("UBound([1, 2, 3])"), 3.0);
        // A record-set aggregate with no data context is a clean Unsupported.
        assert!(run("Sum({t.x})").is_err());
    }
}
