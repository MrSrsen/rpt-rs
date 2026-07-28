//! Shared value semantics — the operator, comparison, date-literal, and default-value logic that
//! **both** the bytecode [`vm`](crate::eval::vm) and the tree-walking `Evaluator` call, so the two
//! evaluators produce byte-identical results. Nothing here is evaluator-specific: it operates on
//! already-evaluated [`Value`]s.

use crate::ast::{Node, VarKind};
use crate::eval::{Date, EvalError, Time, Value};
use crate::token::op;

/// Apply a unary operator to an already-evaluated value. Shared by the tree-walker
/// (`Evaluator::eval_unary`) and the bytecode VM so both have identical semantics.
pub(super) fn apply_unary(code: u8, v: Value) -> Result<Value, EvalError> {
    if v.is_null() {
        return Ok(Value::Null);
    }
    match code {
        op::UNARY_MINUS => match v {
            Value::Number(n) => Ok(Value::Number(-n)),
            Value::Currency(n) => Ok(Value::Currency(-n)),
            v => Err(type_mismatch("unary `-`", &v)),
        },
        op::UNARY_PLUS => Ok(v),
        op::NOT => match v {
            Value::Bool(b) => Ok(Value::Bool(!b)),
            v => Err(type_mismatch("Not", &v)),
        },
        op::DOLLAR => match v.as_number() {
            Some(n) => Ok(Value::Currency(n)),
            None => Err(type_mismatch("`$`", &v)),
        },
        c => Err(EvalError::Unsupported(format!("unary operator 0x{c:02x}"))),
    }
}

/// Apply a binary operator to two already-evaluated values (both operands are eager — even `And`/
/// `Or`, matching the engine). Shared by the tree-walker and the bytecode VM.
pub(super) fn apply_binary(code: u8, l: Value, r: Value) -> Result<Value, EvalError> {
    // Ranges are built even from null bounds; everything else propagates Null.
    if !op::is_range(code) && (l.is_null() || r.is_null()) {
        // Comparisons against Null are false (the engine's null comparisons never hold).
        if matches!(
            code,
            op::EQ
                | op::NE
                | op::LT
                | op::GT
                | op::GE
                | op::LE
                | op::IN
                | op::LIKE
                | op::STARTS_WITH
        ) {
            return Ok(Value::Bool(false));
        }
        return Ok(Value::Null);
    }
    match code {
        op::PLUS => add(l, r),
        op::MINUS => sub(l, r),
        op::STAR => numeric(l, r, "`*`", |a, b| Ok(a * b)),
        op::SLASH => numeric(l, r, "`/`", |a, b| {
            if b == 0.0 {
                Err(EvalError::DivideByZero)
            } else {
                Ok(a / b)
            }
        }),
        op::BACKSLASH => numeric(l, r, "`\\`", |a, b| {
            if b == 0.0 {
                Err(EvalError::DivideByZero)
            } else {
                Ok((a / b).trunc())
            }
        }),
        op::MOD => numeric(l, r, "Mod", |a, b| {
            if b == 0.0 {
                Err(EvalError::DivideByZero)
            } else {
                Ok(a % b)
            }
        }),
        op::CARET => numeric(l, r, "`^`", |a, b| Ok(a.powf(b))),
        // Binary `%`: x percent of y — `x % y` = 100 * x / y.
        op::PERCENT => numeric(l, r, "`%`", |a, b| {
            if b == 0.0 {
                Err(EvalError::DivideByZero)
            } else {
                Ok(a * 100.0 / b)
            }
        }),
        op::AMP => {
            let (a, b) = (coerce_text(&l)?, coerce_text(&r)?);
            Ok(Value::Str(a + &b))
        }
        op::EQ => Ok(Value::Bool(values_eq(&l, &r)?)),
        op::NE => Ok(Value::Bool(!values_eq(&l, &r)?)),
        op::LT | op::GT | op::GE | op::LE => {
            let ord = compare(&l, &r)?;
            Ok(Value::Bool(match code {
                op::LT => ord.is_lt(),
                op::GT => ord.is_gt(),
                op::GE => ord.is_ge(),
                _ => ord.is_le(),
            }))
        }
        // `To` ranges: the `_`-marked side is exclusive.
        c if op::is_range(c) => Ok(Value::Range {
            lo: Box::new(l),
            hi: Box::new(r),
            lo_incl: code == op::RANGE_TO || code == op::RANGE_HI_EXCL,
            hi_incl: code == op::RANGE_TO || code == op::RANGE_LO_EXCL,
        }),
        op::IN => value_in(&l, &r),
        op::LIKE => match (&l, &r) {
            (Value::Str(s), Value::Str(pat)) => Ok(Value::Bool(like_match(s, pat))),
            _ => Err(type_mismatch("Like", &l)),
        },
        op::STARTS_WITH => match (&l, &r) {
            (Value::Str(s), Value::Str(p)) => Ok(Value::Bool(s.starts_with(p.as_str()))),
            _ => Err(type_mismatch("StartsWith", &l)),
        },
        c if op::is_bool_op(c) => {
            let (Value::Bool(a), Value::Bool(b)) = (&l, &r) else {
                return Err(type_mismatch("boolean operator", &l));
            };
            let (a, b) = (*a, *b);
            Ok(Value::Bool(match code {
                op::AND => a && b,
                op::OR => a || b,
                op::XOR => a ^ b,
                op::EQV => a == b,
                _ => !a || b, // Imp
            }))
        }
        c => Err(EvalError::Unsupported(format!("binary operator 0x{c:02x}"))),
    }
}

/// Apply a subscript `base[index]` to already-evaluated values (Crystal arrays are 1-based). Shared
/// by the tree-walker and the bytecode VM.
pub(super) fn apply_index(b: Value, i: Value) -> Result<Value, EvalError> {
    if b.is_null() || i.is_null() {
        return Ok(Value::Null);
    }
    let idx = i
        .as_number()
        .ok_or_else(|| type_mismatch("subscript", &i))?
        .trunc() as i64;
    match b {
        Value::Array(items) => items
            .get((idx - 1).max(0) as usize)
            .filter(|_| idx >= 1)
            .cloned()
            .ok_or_else(|| EvalError::BadArg(format!("subscript {idx} out of bounds"))),
        v => Err(type_mismatch("subscript base", &v)),
    }
}

/// The maximum iterations a single loop runs before evaluation aborts — a guard against a
/// pathological or non-terminating formula hanging the evaluator. Shared by both evaluators (the
/// tree-walker and the [`vm`](crate::eval::vm)) so a formula aborts at the same point in each.
pub(crate) const LOOP_LIMIT: usize = 5_000_000;

pub(crate) fn loop_limit() -> EvalError {
    EvalError::Unsupported("loop iteration limit exceeded".into())
}

/// The error an `Exit` outside any loop raises (both evaluators emit it identically).
pub(super) fn exit_outside_loop() -> EvalError {
    EvalError::BadArg("`Exit` outside a loop".into())
}

/// The default value of a freshly declared variable.
pub(super) fn var_default(kind: VarKind) -> Value {
    match kind {
        VarKind::Number => Value::Number(0.0),
        VarKind::Currency => Value::Currency(0.0),
        VarKind::Boolean => Value::Bool(false),
        VarKind::String => Value::Str(String::new()),
        // Date/time variables start out null-ish; the engine errors on use-before-set.
        VarKind::Date | VarKind::Time | VarKind::DateTime => Value::Null,
    }
}

/// The default an `If` without `Else` yields, from the then-branch's statically deduced type.
pub(super) fn branch_default(then: &Node) -> Value {
    use crate::types::ResultKind as K;
    match crate::types::deduce_type(then, &|_, _| None) {
        K::String => Value::Str(String::new()),
        K::Number => Value::Number(0.0),
        K::Currency => Value::Currency(0.0),
        K::Boolean => Value::Bool(false),
        _ => Value::Null,
    }
}

pub(super) fn type_mismatch(what: &str, got: &Value) -> EvalError {
    EvalError::TypeMismatch {
        what: what.to_string(),
        got: got.type_name().to_string(),
    }
}

/// Coerce an operand of `&` to text (Null → empty string).
fn coerce_text(v: &Value) -> Result<String, EvalError> {
    v.to_text_default()
        .ok_or_else(|| type_mismatch("text coercion", v))
}

/// Numeric binary operator with Currency promotion (Currency if either side is).
fn numeric(
    l: Value,
    r: Value,
    what: &str,
    f: impl Fn(f64, f64) -> Result<f64, EvalError>,
) -> Result<Value, EvalError> {
    let (a, b) = match (l.as_number(), r.as_number()) {
        (Some(a), Some(b)) => (a, b),
        _ => {
            return Err(type_mismatch(
                what,
                if l.as_number().is_none() { &l } else { &r },
            ))
        }
    };
    let n = f(a, b)?;
    if matches!(l, Value::Currency(_)) || matches!(r, Value::Currency(_)) {
        Ok(Value::Currency(n))
    } else {
        Ok(Value::Number(n))
    }
}

/// A DateTime as fractional civil days.
fn dt_to_f(d: Date, t: Time) -> f64 {
    d.to_days() as f64 + t.to_seconds() as f64 / 86_400.0
}

fn f_to_dt(f: f64) -> (Date, Time) {
    let days = f.floor() as i64;
    let secs = ((f - f.floor()) * 86_400.0).round() as i64;
    // A rounded-up full day carries into the date.
    let (days, secs) = if secs >= 86_400 {
        (days + 1, 0)
    } else {
        (days, secs)
    };
    (Date::from_days(days), Time::from_seconds(secs))
}

fn add(l: Value, r: Value) -> Result<Value, EvalError> {
    match (&l, &r) {
        (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
        (Value::Date(d), _) | (_, Value::Date(d)) if other(&l, &r).as_number().is_some() => {
            let n = other(&l, &r).as_number().unwrap();
            Ok(Value::Date(Date::from_days(d.to_days() + n.trunc() as i64)))
        }
        (Value::DateTime(d, t), _) | (_, Value::DateTime(d, t))
            if other(&l, &r).as_number().is_some() =>
        {
            let n = other(&l, &r).as_number().unwrap();
            let (nd, nt) = f_to_dt(dt_to_f(*d, *t) + n);
            Ok(Value::DateTime(nd, nt))
        }
        (Value::Time(t), _) | (_, Value::Time(t)) if other(&l, &r).as_number().is_some() => {
            let n = other(&l, &r).as_number().unwrap();
            Ok(Value::Time(Time::from_seconds(
                t.to_seconds() + n.trunc() as i64,
            )))
        }
        _ => numeric(l, r, "`+`", |a, b| Ok(a + b)),
    }
}

/// The non-temporal operand of a mixed temporal/number pair.
fn other<'v>(l: &'v Value, r: &'v Value) -> &'v Value {
    if matches!(l, Value::Date(_) | Value::DateTime(..) | Value::Time(_)) {
        r
    } else {
        l
    }
}

fn sub(l: Value, r: Value) -> Result<Value, EvalError> {
    match (&l, &r) {
        (Value::Date(a), Value::Date(b)) => Ok(Value::Number((a.to_days() - b.to_days()) as f64)),
        (Value::DateTime(ad, at), Value::DateTime(bd, bt)) => {
            Ok(Value::Number(dt_to_f(*ad, *at) - dt_to_f(*bd, *bt)))
        }
        (Value::Time(a), Value::Time(b)) => {
            Ok(Value::Number((a.to_seconds() - b.to_seconds()) as f64))
        }
        (Value::Date(d), _) if r.as_number().is_some() => Ok(Value::Date(Date::from_days(
            d.to_days() - r.as_number().unwrap().trunc() as i64,
        ))),
        (Value::DateTime(d, t), _) if r.as_number().is_some() => {
            let (nd, nt) = f_to_dt(dt_to_f(*d, *t) - r.as_number().unwrap());
            Ok(Value::DateTime(nd, nt))
        }
        (Value::Time(t), _) if r.as_number().is_some() => Ok(Value::Time(Time::from_seconds(
            t.to_seconds() - r.as_number().unwrap().trunc() as i64,
        ))),
        _ => numeric(l, r, "`-`", |a, b| Ok(a - b)),
    }
}

/// Equality across the scalar types (Number/Currency compare numerically; strings are
/// case-sensitive, matching the formula language).
fn values_eq(l: &Value, r: &Value) -> Result<bool, EvalError> {
    match (l, r) {
        (Value::Bool(a), Value::Bool(b)) => Ok(a == b),
        _ => Ok(compare(l, r)?.is_eq()),
    }
}

pub(crate) fn compare(l: &Value, r: &Value) -> Result<std::cmp::Ordering, EvalError> {
    use std::cmp::Ordering;
    match (l, r) {
        (Value::Str(a), Value::Str(b)) => Ok(a.as_str().cmp(b.as_str())),
        (Value::Date(a), Value::Date(b)) => Ok(a.cmp(b)),
        (Value::Time(a), Value::Time(b)) => Ok(a.cmp(b)),
        (Value::DateTime(ad, at), Value::DateTime(bd, bt)) => Ok((ad, at).cmp(&(bd, bt))),
        (Value::Bool(a), Value::Bool(b)) => Ok(a.cmp(b)),
        _ => match (l.as_number(), r.as_number()) {
            (Some(a), Some(b)) => a
                .partial_cmp(&b)
                .ok_or_else(|| EvalError::BadArg("NaN comparison".into())),
            _ => Err(EvalError::TypeMismatch {
                what: "comparison".into(),
                got: format!("{} vs {}", l.type_name(), r.type_name()),
            }),
        },
    }
    .map(|o: Ordering| o)
}

/// `x In y`: substring for strings, membership for arrays, bounds for ranges.
fn value_in(l: &Value, r: &Value) -> Result<Value, EvalError> {
    match r {
        Value::Str(hay) => match l {
            Value::Str(needle) => Ok(Value::Bool(hay.contains(needle.as_str()))),
            v => Err(type_mismatch("In (string)", v)),
        },
        Value::Array(items) => {
            for item in items {
                // An array of ranges tests range membership per element.
                if let Value::Range { .. } = item {
                    if let Value::Bool(true) = value_in(l, item)? {
                        return Ok(Value::Bool(true));
                    }
                } else if values_eq(l, item)? {
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        Value::Range {
            lo,
            hi,
            lo_incl,
            hi_incl,
        } => {
            let lo_ok = match compare(l, lo)? {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Equal => *lo_incl,
                std::cmp::Ordering::Less => false,
            };
            let hi_ok = match compare(l, hi)? {
                std::cmp::Ordering::Less => true,
                std::cmp::Ordering::Equal => *hi_incl,
                std::cmp::Ordering::Greater => false,
            };
            Ok(Value::Bool(lo_ok && hi_ok))
        }
        v => Err(type_mismatch("In", v)),
    }
}

/// VB-style `Like`: `*` = any run, `?` = any one character (case-sensitive).
fn like_match(s: &str, pat: &str) -> bool {
    fn rec(s: &[char], p: &[char]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some(('*', rest)) => (0..=s.len()).any(|i| rec(&s[i..], rest)),
            Some(('?', rest)) => s.split_first().is_some_and(|(_, srest)| rec(srest, rest)),
            Some((c, rest)) => s
                .split_first()
                .is_some_and(|(sc, srest)| sc == c && rec(srest, rest)),
        }
    }
    let sc: Vec<char> = s.chars().collect();
    let pc: Vec<char> = pat.chars().collect();
    rec(&sc, &pc)
}

/// Parse a `#...#` date/time literal. Forms: numeric `#m/d/yyyy#` / `#yyyy-m-d#`, the textual
/// `#Month d, yyyy#` (full or abbreviated English month name), an optional `hh:mm[:ss] [AM|PM]`
/// time tail, or a bare time.
pub(crate) fn parse_date_literal(src: &str) -> Result<Value, EvalError> {
    let inner = src.trim().trim_matches('#').trim();
    let bad = || EvalError::BadArg(format!("date literal `{src}`"));
    // Split off a trailing AM/PM designator (attached or spaced).
    let lower = inner.to_ascii_lowercase();
    let (body, pm) = if let Some(stripped) = lower.strip_suffix("pm") {
        (stripped.trim_end(), Some(true))
    } else if let Some(stripped) = lower.strip_suffix("am") {
        (stripped.trim_end(), Some(false))
    } else {
        (lower.as_str(), None)
    };
    let mut date: Option<Date> = None;
    let mut time: Option<Time> = None;
    // Textual `Month d, yyyy` accumulators: a month name plus the bare integers (day then year)
    // that follow it. Populated only when a month name appears.
    let mut month_name: Option<u8> = None;
    let mut bare_nums: Vec<i32> = Vec::new();
    for part in body.split_whitespace() {
        if part.contains(':') {
            let nums: Vec<&str> = part.split(':').collect();
            if nums.len() < 2 || nums.len() > 3 {
                return Err(bad());
            }
            let mut hour: u8 = nums[0].parse().map_err(|_| bad())?;
            let minute: u8 = nums[1].parse().map_err(|_| bad())?;
            let second: u8 = nums
                .get(2)
                .map_or(Ok(0), |s| s.parse())
                .map_err(|_| bad())?;
            match pm {
                Some(true) if hour < 12 => hour += 12,
                Some(false) if hour == 12 => hour = 0,
                _ => {}
            }
            time = Some(Time::new(hour, minute, second));
        } else if part.contains('/') || part.contains('-') {
            let sep = if part.contains('/') { '/' } else { '-' };
            let nums: Vec<&str> = part.split(sep).collect();
            if nums.len() != 3 {
                return Err(bad());
            }
            // `yyyy-m-d` when the first component is 4 digits, else US `m/d/y`.
            let (y, m, d) = if nums[0].len() == 4 {
                (nums[0], nums[1], nums[2])
            } else {
                (nums[2], nums[0], nums[1])
            };
            date = Some(Date::new(
                y.parse().map_err(|_| bad())?,
                m.parse().map_err(|_| bad())?,
                d.parse().map_err(|_| bad())?,
            ));
        } else if let Some(m) = month_from_name(part) {
            if month_name.is_some() {
                return Err(bad());
            }
            month_name = Some(m);
        } else if let Ok(n) = part.trim_end_matches(',').parse::<i32>() {
            // A bare integer (day / year) of the textual form; the trailing comma after the day
            // (`March 1, 2024`) is optional.
            bare_nums.push(n);
        } else {
            return Err(bad());
        }
    }
    // Assemble the textual `Month d, yyyy` date, if a month name was seen.
    if let Some(m) = month_name {
        if date.is_some() || bare_nums.len() != 2 {
            return Err(bad());
        }
        let day = u8::try_from(bare_nums[0]).map_err(|_| bad())?;
        date = Some(Date::new(bare_nums[1], m, day));
    } else if !bare_nums.is_empty() {
        // Bare integers with no month name are not a valid date form.
        return Err(bad());
    }
    match (date, time) {
        (Some(d), Some(t)) => Ok(Value::DateTime(d, t)),
        (Some(d), None) => Ok(Value::Date(d)),
        (None, Some(t)) => Ok(Value::Time(t)),
        (None, None) => Err(bad()),
    }
}

/// Map a full or three-letter-abbreviated English month name (already lowercased) to its 1-based
/// number, for the textual `#Month d, yyyy#` literal form.
fn month_from_name(s: &str) -> Option<u8> {
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    MONTHS
        .iter()
        .position(|full| *full == s || (s.len() == 3 && full.starts_with(s)))
        .map(|i| i as u8 + 1)
}
