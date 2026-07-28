//! Statistical builtins over an array argument: sample (`StdDev`/`Variance`, n−1 divisor) and
//! population (`PopulationStdDev`/`PopulationVariance`, n divisor) dispersion.
//!
//! Crystal's `StdDev`/`Variance` also have record-set forms (`StdDev({field}, {group})`); those need
//! the data pipeline and report `Unsupported`, mirroring `Sum`/`Average` in [`super::math`]. Crystal
//! has no array-literal `Median`/`Mode` (they exist only as summary functions), so they are not
//! implemented here.

use crate::eval::{EvalError, Value};

/// The statistical-family builtins. Wrapped by [`super::Builtin::Statistical`] in the dispatch table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StatisticalFn {
    StdDev,
    Variance,
    PopulationStdDev,
    PopulationVariance,
}

/// Handle a statistical builtin (routed here from [`super::call`]).
pub(super) fn call(b: StatisticalFn, name: &str, args: &[Value]) -> Result<Value, EvalError> {
    use StatisticalFn as B;
    let sample = matches!(b, B::StdDev | B::Variance);
    let variance = variance_of(name, args, sample)?;
    match variance {
        None => Ok(Value::Null),
        Some(v) => Ok(Value::Number(match b {
            B::StdDev | B::PopulationStdDev => v.sqrt(),
            B::Variance | B::PopulationVariance => v,
        })),
    }
}

/// The variance of the array argument: sample (divisor n−1) or population (divisor n). `None` for an
/// empty (all-null) input, matching the aggregate builtins. A single value with the sample divisor
/// is undefined (n−1 = 0) and errors.
fn variance_of(name: &str, args: &[Value], sample: bool) -> Result<Option<f64>, EvalError> {
    let vals = super::numeric_non_null(name, super::array_arg(name, args)?)?;
    let n = vals.len();
    if n == 0 {
        return Ok(None);
    }
    if sample && n < 2 {
        return Err(EvalError::BadArg(format!(
            "{name}: needs at least two values"
        )));
    }
    let mean = vals.iter().sum::<f64>() / n as f64;
    let ss: f64 = vals.iter().map(|x| (x - mean) * (x - mean)).sum();
    let divisor = if sample { (n - 1) as f64 } else { n as f64 };
    Ok(Some(ss / divisor))
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
    fn sample_vs_population_divisor() {
        // Fixture [1,10,15,20,25]: mean 14.2, Σ(dev²) = 342.8.
        assert!((num("Variance([1, 10, 15, 20, 25])") - 85.7).abs() < 1e-9); // 342.8 / 4
        assert!((num("PopulationVariance([1, 10, 15, 20, 25])") - 68.56).abs() < 1e-9); // 342.8 / 5
        assert!((num("StdDev([1, 10, 15, 20, 25])") - 85.7_f64.sqrt()).abs() < 1e-9);
        assert!((num("PopulationStdDev([1, 10, 15, 20, 25])") - 68.56_f64.sqrt()).abs() < 1e-9);
    }

    #[test]
    fn nulls_are_skipped() {
        let ctx = crate::eval::MapContext::default().with_field(
            crate::token::RefKind::Field,
            "t.n",
            Value::Null,
        );
        let (ast, _) = parse("Variance([1, {t.n}, 10, 15, 20, 25])", Syntax::Crystal);
        assert!((matches!(eval(&ast, &ctx), Ok(Value::Number(n)) if (n - 85.7).abs() < 1e-9)));
    }

    #[test]
    fn edge_and_error_cases() {
        // Constant data has zero dispersion.
        assert_eq!(num("PopulationVariance([5, 5, 5])"), 0.0);
        // A single value has no sample variance (n−1 = 0).
        assert!(matches!(run("Variance([5])"), Err(EvalError::BadArg(_))));
        // Population variance of one value is defined (zero).
        assert_eq!(num("PopulationVariance([5])"), 0.0);
        // The record-set form (a scalar field argument) needs a data context.
        let ctx = crate::eval::MapContext::default().with_field(
            crate::token::RefKind::Field,
            "t.x",
            Value::Number(5.0),
        );
        let (ast, _) = parse("StdDev({t.x})", Syntax::Crystal);
        assert!(matches!(eval(&ast, &ctx), Err(EvalError::Unsupported(_))));
    }
}
