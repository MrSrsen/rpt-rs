//! Financial builtins — the time-value-of-money and depreciation functions. Argument order, the
//! `type` (0 = end of period, 1 = beginning) and `guess` defaults, and the cash-flow sign
//! convention (money paid out is negative) all follow the Excel/VB6 financial functions Crystal
//! mirrors.

use super::{bad_arg, mismatch, num_arg, opt_num, Builtin};
use crate::eval::{EvalError, Value};

/// Handle a financial [`Builtin`] (routed here by [`super::Builtin::family`]).
pub(super) fn call(b: Builtin, name: &str, args: &[Value]) -> Result<Value, EvalError> {
    use Builtin as B;
    match b {
        B::Pmt => {
            let (rate, nper, pv) = (
                num_arg(name, args, 0)?,
                num_arg(name, args, 1)?,
                num_arg(name, args, 2)?,
            );
            let fv = opt_num(args, 3).unwrap_or(0.0);
            let typ = opt_num(args, 4).unwrap_or(0.0);
            Ok(Value::Number(pmt(rate, nper, pv, fv, typ)))
        }
        B::FV => {
            let (rate, nper, pmt_) = (
                num_arg(name, args, 0)?,
                num_arg(name, args, 1)?,
                num_arg(name, args, 2)?,
            );
            let pv = opt_num(args, 3).unwrap_or(0.0);
            let typ = opt_num(args, 4).unwrap_or(0.0);
            Ok(Value::Number(fv(rate, nper, pmt_, pv, typ)))
        }
        B::PV => {
            let (rate, nper, pmt_) = (
                num_arg(name, args, 0)?,
                num_arg(name, args, 1)?,
                num_arg(name, args, 2)?,
            );
            let fv_ = opt_num(args, 3).unwrap_or(0.0);
            let typ = opt_num(args, 4).unwrap_or(0.0);
            Ok(Value::Number(pv(rate, nper, pmt_, fv_, typ)))
        }
        B::Npv => {
            let rate = num_arg(name, args, 0)?;
            let flows = num_array(name, args.get(1))?;
            // Each cash flow is discounted from period 1 onward (Excel `NPV` convention).
            let n: f64 = flows
                .iter()
                .enumerate()
                .map(|(i, cf)| cf / (1.0 + rate).powi(i as i32 + 1))
                .sum();
            Ok(Value::Number(n))
        }
        B::Irr => {
            let flows = num_array(name, args.first())?;
            let guess = opt_num(args, 1).unwrap_or(0.1);
            // IRR discounts the first flow at period 0 (Excel convention), so the root of
            // `sum(cf[i] / (1+r)^i)` is what we solve for.
            let f = |r: f64| -> f64 {
                flows
                    .iter()
                    .enumerate()
                    .map(|(i, cf)| cf / (1.0 + r).powi(i as i32))
                    .sum()
            };
            solve(name, f, guess).map(Value::Number)
        }
        B::Rate => {
            let (nper, pmt_, pv_) = (
                num_arg(name, args, 0)?,
                num_arg(name, args, 1)?,
                num_arg(name, args, 2)?,
            );
            let fv_ = opt_num(args, 3).unwrap_or(0.0);
            let typ = opt_num(args, 4).unwrap_or(0.0);
            let guess = opt_num(args, 5).unwrap_or(0.1);
            let f = |r: f64| tvm(r, nper, pmt_, pv_, fv_, typ);
            solve(name, f, guess).map(Value::Number)
        }
        B::Ddb => {
            let (cost, salvage, life, period) = (
                num_arg(name, args, 0)?,
                num_arg(name, args, 1)?,
                num_arg(name, args, 2)?,
                num_arg(name, args, 3)?,
            );
            let factor = opt_num(args, 4).unwrap_or(2.0);
            Ok(Value::Number(ddb(cost, salvage, life, period, factor)))
        }
        B::Sln => {
            let (cost, salvage, life) = (
                num_arg(name, args, 0)?,
                num_arg(name, args, 1)?,
                num_arg(name, args, 2)?,
            );
            if life == 0.0 {
                return Err(EvalError::DivideByZero);
            }
            Ok(Value::Number((cost - salvage) / life))
        }
        B::Syd => {
            let (cost, salvage, life, period) = (
                num_arg(name, args, 0)?,
                num_arg(name, args, 1)?,
                num_arg(name, args, 2)?,
                num_arg(name, args, 3)?,
            );
            if life <= 0.0 {
                return Err(bad_arg(name, "life must be positive"));
            }
            Ok(Value::Number(
                (cost - salvage) * (life - period + 1.0) * 2.0 / (life * (life + 1.0)),
            ))
        }
        other => unreachable!("non-financial builtin {other:?} routed to financial"),
    }
}

/// The array-of-numbers argument shared by `NPV`/`IRR`.
fn num_array(name: &str, arg: Option<&Value>) -> Result<Vec<f64>, EvalError> {
    match arg {
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| v.as_number().ok_or_else(|| mismatch(name, v)))
            .collect(),
        Some(v) => Err(mismatch(name, v)),
        None => Err(bad_arg(name, "expects an array of cash flows")),
    }
}

/// The time-value-of-money identity, zero at the solved rate:
/// `pv·(1+r)ⁿ + pmt·(1+r·type)·((1+r)ⁿ−1)/r + fv`.
fn tvm(r: f64, nper: f64, pmt: f64, pv: f64, fv: f64, typ: f64) -> f64 {
    if r == 0.0 {
        pv + pmt * nper + fv
    } else {
        let p = (1.0 + r).powf(nper);
        pv * p + pmt * (1.0 + r * typ) * (p - 1.0) / r + fv
    }
}

fn pmt(rate: f64, nper: f64, pv: f64, fv: f64, typ: f64) -> f64 {
    if rate == 0.0 {
        -(pv + fv) / nper
    } else {
        let p = (1.0 + rate).powf(nper);
        -(pv * p + fv) * rate / ((1.0 + rate * typ) * (p - 1.0))
    }
}

fn fv(rate: f64, nper: f64, pmt: f64, pv: f64, typ: f64) -> f64 {
    if rate == 0.0 {
        -(pv + pmt * nper)
    } else {
        let p = (1.0 + rate).powf(nper);
        -(pv * p + pmt * (1.0 + rate * typ) * (p - 1.0) / rate)
    }
}

fn pv(rate: f64, nper: f64, pmt: f64, fv: f64, typ: f64) -> f64 {
    if rate == 0.0 {
        -(fv + pmt * nper)
    } else {
        let p = (1.0 + rate).powf(nper);
        -(fv + pmt * (1.0 + rate * typ) * (p - 1.0) / rate) / p
    }
}

/// Double-declining-balance depreciation for a single `period`: each period takes the lesser of the
/// declining-balance amount and the remaining depreciable value, so the balance never dips below
/// salvage.
fn ddb(cost: f64, salvage: f64, life: f64, period: f64, factor: f64) -> f64 {
    if life <= 0.0 {
        return 0.0;
    }
    let rate = factor / life;
    let mut book = cost;
    let mut dep = 0.0;
    let periods = period.max(0.0) as u64;
    for _ in 0..periods {
        dep = (book * rate).min((book - salvage).max(0.0));
        book -= dep;
    }
    dep
}

/// Newton's method (with a numeric derivative) for the rate at which `f` is zero — the engine used
/// by `IRR` and `Rate`. Fails cleanly if it does not converge.
fn solve(name: &str, f: impl Fn(f64) -> f64, guess: f64) -> Result<f64, EvalError> {
    const MAX_ITERS: usize = 128;
    const TOL: f64 = 1e-9;
    let mut r = guess;
    for _ in 0..MAX_ITERS {
        let y = f(r);
        if y.abs() < TOL {
            return Ok(r);
        }
        let h = 1e-6;
        let dy = (f(r + h) - f(r - h)) / (2.0 * h);
        if dy == 0.0 || !dy.is_finite() {
            break;
        }
        let next = r - y / dy;
        if !next.is_finite() {
            break;
        }
        if (next - r).abs() < TOL {
            return Ok(next);
        }
        r = next;
    }
    Err(bad_arg(name, "did not converge"))
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
    fn approx(src: &str, expected: f64) {
        let got = num(src);
        assert!(
            (got - expected).abs() < 0.01,
            "`{src}` → {got}, expected ≈ {expected}"
        );
    }

    #[test]
    fn payment_and_annuity() {
        // A positive present value yields a negative payment (cash-out convention).
        approx("Pmt(0.05/12, 36, 10000)", -299.71);
        approx("FV(0.04/12, 60, -500)", 33149.49);
        // Zero-rate degenerate cases reduce to simple division.
        approx("Pmt(0, 10, 1000)", -100.0);
        approx("FV(0, 12, -100)", 1200.0);
        // PV is the inverse of FV under the same annuity.
        approx("PV(0, 10, -100)", 1000.0);
    }

    #[test]
    fn npv_and_irr() {
        approx("NPV(0.05, [1000, 2000, 1500, 1750])", 5501.93);
        // IRR of a cash-flow series with a sign change (default guess 0.1).
        approx("IRR([-1000, -500, 2000])", 0.1861);
    }

    #[test]
    fn rate_solves_for_the_periodic_rate() {
        // Inverse of the confirmed Pmt fixture: Pmt(0.05/12, 36, 10000) = -299.71, so the periodic
        // rate that produces that payment is 0.05/12.
        approx("Rate(36, -299.7085, 10000)", 0.05 / 12.0);
        // Paying back less (36 × 750 = 27000) than the 35000 borrowed implies a negative rate.
        approx("Rate(36, -750, 35000)", -0.0134);
    }

    #[test]
    fn depreciation() {
        approx("DDB(35000, 5000, 7, 5)", 2603.08);
        approx("SLN(30000, 7500, 10)", 2250.0);
        approx("SYD(30000, 7500, 10, 1)", 4090.91);
        approx("SYD(30000, 7500, 10, 10)", 409.09);
    }

    #[test]
    fn error_and_null_cases() {
        assert_eq!(run("SLN(1000, 100, 0)"), Err(EvalError::DivideByZero));
        // NPV/IRR need an array of cash flows, not a scalar.
        assert!(matches!(
            run("NPV(0.1, 5)"),
            Err(EvalError::TypeMismatch { .. })
        ));
    }
}
