//! Date/time builtins, built on the zero-dependency civil calendar in
//! [`rpt_format_value::civil`] (`Date`/`Time` day-number and second arithmetic).

use super::{bad_arg, mismatch, num_arg, str_arg, Builtin};
use crate::eval::{parse_date_literal, Date, EvalError, Time, Value};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Handle a date/time [`Builtin`] (routed here by [`super::Builtin::family`]).
pub(super) fn call(b: Builtin, name: &str, args: &[Value]) -> Result<Value, EvalError> {
    use Builtin as B;
    match b {
        B::DateCtor => match args {
            [Value::DateTime(d, _)] => Ok(Value::Date(*d)),
            [Value::Date(d)] => Ok(Value::Date(*d)),
            [Value::Str(s)] => parse_date_literal(s),
            // A numeric serial is an OLE Automation date (days since 1899-12-30).
            [Value::Number(n)] => Ok(Value::Date(Date::from_ole_days(n.floor() as i64))),
            [y, m, d] => ymd(name, y, m, d).map(Value::Date),
            _ => Err(bad_arg(name, "expects (y,m,d), a DateTime, or a string")),
        },
        B::TimeCtor => match args {
            [Value::DateTime(_, t)] => Ok(Value::Time(*t)),
            [Value::Time(t)] => Ok(Value::Time(*t)),
            [Value::Str(s)] => parse_date_literal(s),
            [h, m, s] => hms(name, h, m, s).map(Value::Time),
            _ => Err(bad_arg(name, "expects (h,m,s), a DateTime, or a string")),
        },
        B::DateTimeCtor => match args {
            [Value::Date(d)] => Ok(Value::DateTime(*d, Time::new(0, 0, 0))),
            [Value::DateTime(d, t)] => Ok(Value::DateTime(*d, *t)),
            [Value::Str(s)] => match parse_date_literal(s)? {
                Value::Date(d) => Ok(Value::DateTime(d, Time::new(0, 0, 0))),
                v => Ok(v),
            },
            [Value::Date(d), Value::Time(t)] => Ok(Value::DateTime(*d, *t)),
            // A numeric serial is an OLE Automation date-time (fractional days since 1899-12-30).
            [Value::Number(n)] => Ok(ole_datetime(*n)),
            [y, m, d] => ymd(name, y, m, d).map(|d| Value::DateTime(d, Time::new(0, 0, 0))),
            [y, m, d, h, mi, s] => Ok(Value::DateTime(ymd(name, y, m, d)?, hms(name, h, mi, s)?)),
            _ => Err(bad_arg(
                name,
                "expects (y,m,d[,h,m,s]), (date,time), or a string",
            )),
        },
        B::DateValue => match args {
            [Value::Date(d)] | [Value::DateTime(d, _)] => Ok(Value::Date(*d)),
            [Value::Str(s)] => match parse_date_literal(s)? {
                Value::DateTime(d, _) => Ok(Value::Date(d)),
                v => Ok(v),
            },
            // A numeric serial is an OLE Automation date (days since 1899-12-30).
            [Value::Number(n)] => Ok(Value::Date(Date::from_ole_days(n.floor() as i64))),
            [y, m, d] => ymd(name, y, m, d).map(Value::Date),
            _ => Err(bad_arg(name, "expects a date/datetime/string or (y,m,d)")),
        },
        B::TimeValue => match args {
            [Value::Time(t)] | [Value::DateTime(_, t)] => Ok(Value::Time(*t)),
            [Value::Str(s)] => match parse_date_literal(s)? {
                Value::DateTime(_, t) => Ok(Value::Time(t)),
                v => Ok(v),
            },
            [h, m, s] => hms(name, h, m, s).map(Value::Time),
            _ => Err(bad_arg(name, "expects a time/datetime/string or (h,m,s)")),
        },
        B::DateSerial => {
            // VB `DateSerial(y, m, d)`: months normalise into years, then day is a 1-based offset,
            // so out-of-range months/days roll over.
            let y = num_arg(name, args, 0)? as i32;
            let m = num_arg(name, args, 1)? as i32;
            let d = num_arg(name, args, 2)? as i32;
            let total = y * 12 + (m - 1);
            let base = Date::new(total.div_euclid(12), (total.rem_euclid(12) + 1) as u8, 1);
            Ok(Value::Date(Date::from_days(
                base.to_days() + i64::from(d - 1),
            )))
        }
        B::TimeSerial => {
            // VB `TimeSerial(h, m, s)`: seconds wrap modulo one day (via `Time::from_seconds`).
            let h = num_arg(name, args, 0)? as i64;
            let m = num_arg(name, args, 1)? as i64;
            let s = num_arg(name, args, 2)? as i64;
            Ok(Value::Time(Time::from_seconds(h * 3600 + m * 60 + s)))
        }
        B::DatePart => date_part(name, args),
        B::Year => Ok(Value::Number(f64::from(date_of(name, &args[0])?.year))),
        B::Month => Ok(Value::Number(f64::from(date_of(name, &args[0])?.month))),
        B::Day => Ok(Value::Number(f64::from(date_of(name, &args[0])?.day))),
        B::Hour => Ok(Value::Number(f64::from(time_of(name, &args[0])?.hour))),
        B::Minute => Ok(Value::Number(f64::from(time_of(name, &args[0])?.minute))),
        B::Second => Ok(Value::Number(f64::from(time_of(name, &args[0])?.second))),
        B::DayOfWeek | B::Weekday => {
            let first = first_day_of_week(name, args.get(1))?;
            Ok(Value::Number(f64::from(weekday_num(
                date_of(name, &args[0])?,
                first,
            ))))
        }
        B::MonthName => {
            let m = num_arg(name, args, 0)? as usize;
            if !(1..=12).contains(&m) {
                return Err(bad_arg(name, "month out of range"));
            }
            Ok(Value::Str(maybe_abbrev(MONTHS[m - 1], args.get(1))))
        }
        B::WeekdayName => {
            // `n` is 1..7 relative to `firstDayOfWeek` (arg 3, default Sunday), not absolute.
            let n = num_arg(name, args, 0)? as i64;
            if !(1..=7).contains(&n) {
                return Err(bad_arg(name, "weekday out of range"));
            }
            let first = first_day_of_week(name, args.get(2))?;
            let abs = ((n - 1 + i64::from(first) - 1).rem_euclid(7)) as usize;
            Ok(Value::Str(maybe_abbrev(DAYS[abs], args.get(1))))
        }
        B::DateAdd => date_add(name, args),
        B::DateDiff => date_diff(name, args),
        B::IsDate => Ok(Value::Bool(match &args[0] {
            Value::Date(_) | Value::DateTime(..) => true,
            Value::Str(s) => matches!(
                parse_date_literal(s),
                Ok(Value::Date(_) | Value::DateTime(..))
            ),
            _ => false,
        })),
        B::IsTime => Ok(Value::Bool(match &args[0] {
            Value::Time(_) | Value::DateTime(..) => true,
            Value::Str(s) => matches!(parse_date_literal(s), Ok(Value::Time(_))),
            _ => false,
        })),
        B::IsDateTime => Ok(Value::Bool(match &args[0] {
            Value::DateTime(..) => true,
            Value::Str(s) => matches!(parse_date_literal(s), Ok(Value::DateTime(..))),
            _ => false,
        })),
        other => unreachable!("non-datetime builtin {other:?} routed to datetime"),
    }
}

/// Truncate to the 3-letter abbreviation when the optional flag argument is `true`.
fn maybe_abbrev(full: &str, flag: Option<&Value>) -> String {
    match flag {
        Some(Value::Bool(true)) => full[..3].to_string(),
        _ => full.to_string(),
    }
}

fn ymd(name: &str, y: &Value, m: &Value, d: &Value) -> Result<Date, EvalError> {
    Ok(Date::new(
        y.as_number().ok_or_else(|| mismatch(name, y))? as i32,
        m.as_number().ok_or_else(|| mismatch(name, m))? as u8,
        d.as_number().ok_or_else(|| mismatch(name, d))? as u8,
    ))
}

fn hms(name: &str, h: &Value, m: &Value, s: &Value) -> Result<Time, EvalError> {
    Ok(Time::new(
        h.as_number().ok_or_else(|| mismatch(name, h))? as u8,
        m.as_number().ok_or_else(|| mismatch(name, m))? as u8,
        s.as_number().ok_or_else(|| mismatch(name, s))? as u8,
    ))
}

fn date_of(name: &str, v: &Value) -> Result<Date, EvalError> {
    match v {
        Value::Date(d) | Value::DateTime(d, _) => Ok(*d),
        v => Err(mismatch(name, v)),
    }
}

fn time_of(name: &str, v: &Value) -> Result<Time, EvalError> {
    match v {
        Value::Time(t) | Value::DateTime(_, t) => Ok(*t),
        v => Err(mismatch(name, v)),
    }
}

/// Decompose a temporal argument into date + time halves (a bare Date gets midnight).
fn temporal(name: &str, v: &Value) -> Result<(Date, Time), EvalError> {
    match v {
        Value::Date(d) => Ok((*d, Time::new(0, 0, 0))),
        Value::DateTime(d, t) => Ok((*d, *t)),
        v => Err(mismatch(name, v)),
    }
}

/// The optional `firstDayOfWeek` argument (Crystal `crSunday`=1 … `crSaturday`=7; `crUseSystem`=0
/// and an omitted arg both default to Sunday).
fn first_day_of_week(name: &str, arg: Option<&Value>) -> Result<u8, EvalError> {
    match arg {
        None => Ok(1),
        Some(v) => {
            let n = v.as_number().ok_or_else(|| mismatch(name, v))? as i64;
            match n {
                0 => Ok(1), // crUseSystem → Sunday
                1..=7 => Ok(n as u8),
                _ => Err(bad_arg(name, &format!("firstDayOfWeek {n} out of range"))),
            }
        }
    }
}

/// An OLE Automation fractional-day serial as a `DateTime` (whole part = date, fraction = time).
fn ole_datetime(serial: f64) -> Value {
    let days = serial.floor();
    let mut secs = ((serial - days) * 86_400.0).round() as i64;
    let mut days = days as i64;
    if secs >= 86_400 {
        days += 1;
        secs = 0;
    }
    Value::DateTime(Date::from_ole_days(days), Time::from_seconds(secs))
}

/// Weekday number of `d` in a week that starts on `first` (`first`=1 → the Crystal default,
/// Sunday=1 … Saturday=7).
fn weekday_num(d: Date, first: u8) -> u8 {
    (d.day_of_week() + 7 - first) % 7 + 1
}

/// Week of the year under the default `crFirstJan1` rule: week 1 is the week containing January 1,
/// with weeks starting on `first`. Late-December dates never roll into the next year (VBA semantics).
fn week_of_year(d: Date, first: u8) -> i64 {
    let jan1 = Date::new(d.year, 1, 1);
    let doy = d.to_days() - jan1.to_days(); // 0-based day of year
    let jan1_idx = i64::from(weekday_num(jan1, first)) - 1;
    (doy + jan1_idx) / 7 + 1
}

/// The day-number of the start of week 1 of `year` for a `threshold`-days rule: week 1 is the first
/// week (starting on `first`) that has at least `threshold` days in the new year. `threshold` = 1 is
/// `crFirstJan1`, 4 is `crFirstFourDays` (ISO-8601-like), 7 is `crFirstFullWeek`.
fn week1_start_days(year: i32, first: u8, threshold: i64) -> i64 {
    let jan1 = Date::new(year, 1, 1);
    let jan1_off = i64::from(weekday_num(jan1, first)) - 1; // 0..6: days from `first` to Jan 1
    let containing_start = jan1.to_days() - jan1_off; // start of the week containing Jan 1
    let days_in_first = 7 - jan1_off; // days of the new year in that week
    if days_in_first >= threshold {
        containing_start
    } else {
        containing_start + 7
    }
}

/// Week of the year under the `crFirstFourDays`/`crFirstFullWeek` rules (`threshold` 4 / 7). Unlike
/// `crFirstJan1`, a date near a year boundary takes its number from the year whose week 1 it belongs
/// to — so late-December dates can be week 1 of the next year and early-January dates the last week
/// of the previous year (ISO-8601-style week-year assignment).
fn week_number_thresholded(d: Date, first: u8, threshold: i64) -> i64 {
    let dd = d.to_days();
    for cy in [d.year + 1, d.year, d.year - 1] {
        let w1s = week1_start_days(cy, first, threshold);
        if dd >= w1s {
            return (dd - w1s) / 7 + 1;
        }
    }
    1 // Unreachable: the previous year's week 1 always starts on or before `d`.
}

/// `DatePart(interval, date[, firstDayOfWeek[, firstWeekOfYear]])` — the numeric component named by
/// the interval code. Only the default `crFirstJan1` week rule is supported for `"ww"`.
fn date_part(name: &str, args: &[Value]) -> Result<Value, EvalError> {
    let code = str_arg(name, args, 0)?.to_lowercase();
    let (d, t) = match args.get(1) {
        Some(Value::Date(d)) => (*d, Time::new(0, 0, 0)),
        Some(Value::DateTime(d, t)) => (*d, *t),
        Some(Value::Time(t)) => (Date::from_days(0), *t),
        Some(Value::Str(s)) => match parse_date_literal(s)? {
            Value::Date(d) => (d, Time::new(0, 0, 0)),
            Value::DateTime(d, t) => (d, t),
            Value::Time(t) => (Date::from_days(0), t),
            _ => return Err(bad_arg(name, "bad date string")),
        },
        Some(v) => return Err(mismatch(name, v)),
        None => return Err(bad_arg(name, "missing argument 2")),
    };
    let n: i64 = match code.as_str() {
        "yyyy" => i64::from(d.year),
        "q" => i64::from((d.month - 1) / 3 + 1),
        "m" => i64::from(d.month),
        "d" => i64::from(d.day),
        // `"y"` is day-of-year (distinct from `"d"`, day-of-month).
        "y" => d.to_days() - Date::new(d.year, 1, 1).to_days() + 1,
        "w" => i64::from(weekday_num(d, first_day_of_week(name, args.get(2))?)),
        "ww" => {
            let first = first_day_of_week(name, args.get(2))?;
            // firstWeekOfYear: 0 (crUseSystem) / 1 (crFirstJan1), 2 (crFirstFourDays),
            // 3 (crFirstFullWeek). An omitted argument defaults to crFirstJan1.
            let mode = match args.get(3) {
                Some(v) => v.as_number().ok_or_else(|| mismatch(name, v))? as i64,
                None => 1,
            };
            match mode {
                0 | 1 => week_of_year(d, first),
                2 => week_number_thresholded(d, first, 4),
                3 => week_number_thresholded(d, first, 7),
                _ => {
                    return Err(bad_arg(
                        name,
                        &format!("firstWeekOfYear {mode} out of range"),
                    ))
                }
            }
        }
        "h" => i64::from(t.hour),
        "n" => i64::from(t.minute),
        "s" => i64::from(t.second),
        _ => return Err(bad_arg(name, &format!("interval `{code}`"))),
    };
    Ok(Value::Number(n as f64))
}

/// The `DateAdd`/`DateDiff` interval codes (VB semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interval {
    Year,
    Quarter,
    Month,
    Day,
    CalendarWeek,
    Week,
    Hour,
    Minute,
    Second,
}

impl Interval {
    fn from_code(code: &str) -> Option<Interval> {
        Some(match code {
            "yyyy" => Interval::Year,
            "q" => Interval::Quarter,
            "m" => Interval::Month,
            // `y` (day-of-year) and `d` are equivalent for add/diff purposes.
            "d" | "y" => Interval::Day,
            "ww" => Interval::CalendarWeek,
            "w" => Interval::Week,
            "h" => Interval::Hour,
            "n" => Interval::Minute,
            "s" => Interval::Second,
            _ => return None,
        })
    }
}

/// Month arithmetic with end-of-month day clamping (VB `DateAdd("m", …)` semantics).
fn add_months(d: Date, n: i32) -> Date {
    let total = d.year * 12 + i32::from(d.month) - 1 + n;
    let (year, month) = (total.div_euclid(12), (total.rem_euclid(12) + 1) as u8);
    let dim = days_in_month(year, month);
    Date::new(year, month, d.day.min(dim))
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

fn interval_arg(name: &str, args: &[Value]) -> Result<Interval, EvalError> {
    let code = str_arg(name, args, 0)?.to_lowercase();
    Interval::from_code(&code).ok_or_else(|| bad_arg(name, &format!("interval `{code}`")))
}

fn date_add(name: &str, args: &[Value]) -> Result<Value, EvalError> {
    let interval = interval_arg(name, args)?;
    let n = num_arg(name, args, 1)?.trunc() as i64;
    let (d, t) = temporal(name, &args[2])?;
    // DateAdd always returns a DateTime (VB semantics).
    let (nd, nt) = match interval {
        Interval::Year => (add_months(d, (n * 12) as i32), t),
        Interval::Quarter => (add_months(d, (n * 3) as i32), t),
        Interval::Month => (add_months(d, n as i32), t),
        Interval::Day => (Date::from_days(d.to_days() + n), t),
        Interval::CalendarWeek | Interval::Week => (Date::from_days(d.to_days() + n * 7), t),
        Interval::Hour | Interval::Minute | Interval::Second => {
            let seconds = match interval {
                Interval::Hour => n * 3600,
                Interval::Minute => n * 60,
                _ => n,
            };
            let total = d.to_days() * 86_400 + t.to_seconds() + seconds;
            (
                Date::from_days(total.div_euclid(86_400)),
                Time::from_seconds(total.rem_euclid(86_400)),
            )
        }
    };
    Ok(Value::DateTime(nd, nt))
}

fn date_diff(name: &str, args: &[Value]) -> Result<Value, EvalError> {
    let interval = interval_arg(name, args)?;
    let (d1, t1) = temporal(name, &args[1])?;
    let (d2, t2) = temporal(name, &args[2])?;
    let n = match interval {
        Interval::Year => i64::from(d2.year - d1.year),
        Interval::Quarter => i64::from(
            (d2.year * 4 + i32::from(d2.month - 1) / 3)
                - (d1.year * 4 + i32::from(d1.month - 1) / 3),
        ),
        Interval::Month => {
            i64::from((d2.year * 12 + i32::from(d2.month)) - (d1.year * 12 + i32::from(d1.month)))
        }
        Interval::Day => d2.to_days() - d1.to_days(),
        // Calendar weeks: `firstDayOfWeek`-boundary crossings between the two dates (arg 4).
        Interval::CalendarWeek => {
            let first = first_day_of_week(name, args.get(3))?;
            let week_start = |d: Date| d.to_days() - i64::from(weekday_num(d, first) - 1);
            (week_start(d2) - week_start(d1)) / 7
        }
        // Whole 7-day spans.
        Interval::Week => (d2.to_days() - d1.to_days()) / 7,
        Interval::Hour | Interval::Minute | Interval::Second => {
            let secs = (d2.to_days() - d1.to_days()) * 86_400 + t2.to_seconds() - t1.to_seconds();
            match interval {
                Interval::Hour => secs / 3600,
                Interval::Minute => secs / 60,
                _ => secs,
            }
        }
    };
    Ok(Value::Number(n as f64))
}

#[cfg(test)]
mod tests {
    use crate::eval::{eval, Date, EmptyContext, EvalError, Time, Value};
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
    fn constructors_and_components() {
        assert_eq!(
            run("Date(2004, 1, 3)"),
            Ok(Value::Date(Date::new(2004, 1, 3)))
        );
        assert_eq!(num("Year(#1/3/2004#)"), 2004.0);
        assert_eq!(num("Month(#1/3/2004#)"), 1.0);
        assert_eq!(num("Day(#1/3/2004#)"), 3.0);
        assert_eq!(num("Hour(#1/3/2004 14:05:06#)"), 14.0);
        assert_eq!(num("Minute(#14:05:06#)"), 5.0);
        assert_eq!(num("Second(#14:05:06#)"), 6.0);
    }

    #[test]
    fn serials_roll_over() {
        assert_eq!(
            run("DateSerial(2004, 13, 1)"),
            Ok(Value::Date(Date::new(2005, 1, 1)))
        );
        assert_eq!(
            run("DateSerial(2004, 1, 0)"),
            Ok(Value::Date(Date::new(2003, 12, 31)))
        );
        assert_eq!(
            run("DateSerial(2004, 2, 29)"),
            Ok(Value::Date(Date::new(2004, 2, 29)))
        );
        assert_eq!(
            run("TimeSerial(25, 0, 0)"),
            Ok(Value::Time(Time::new(1, 0, 0)))
        );
        assert_eq!(
            run("TimeSerial(1, 90, 0)"),
            Ok(Value::Time(Time::new(2, 30, 0)))
        );
    }

    #[test]
    fn datepart_components() {
        assert_eq!(num(r#"DatePart("yyyy", #1/3/2004#)"#), 2004.0);
        assert_eq!(num(r#"DatePart("q", #4/1/2004#)"#), 2.0);
        assert_eq!(num(r#"DatePart("m", #1/3/2004#)"#), 1.0);
        assert_eq!(num(r#"DatePart("d", #1/3/2004#)"#), 3.0);
        assert_eq!(num(r#"DatePart("y", #1/1/2004#)"#), 1.0);
        assert_eq!(num(r#"DatePart("y", #2/1/2004#)"#), 32.0);
        assert_eq!(num(r#"DatePart("ww", #1/1/2004#)"#), 1.0);
        assert_eq!(num(r#"DatePart("h", #1/3/2004 14:05:06#)"#), 14.0);
        assert_eq!(num(r#"DatePart("n", #1/3/2004 14:05:06#)"#), 5.0);
        assert!(matches!(
            run(r#"DatePart("zz", #1/1/2004#)"#),
            Err(EvalError::BadArg(_))
        ));
        assert!(matches!(
            run(r#"DatePart("ww", #1/1/2004#, 1, 5)"#),
            Err(EvalError::BadArg(_))
        ));
    }

    #[test]
    fn datepart_first_four_days_is_iso8601() {
        // With a Monday start (crMonday=2), crFirstFourDays (mode 2) is exactly ISO-8601; these are
        // canonical ISO week numbers across year boundaries.
        let iso = |d: &str| num(&format!(r#"DatePart("ww", {d}, 2, 2)"#));
        assert_eq!(iso("#1/1/2004#"), 1.0); // 2004-W01
        assert_eq!(iso("#1/1/2005#"), 53.0); // 2004 had 53 ISO weeks
        assert_eq!(iso("#1/1/2007#"), 1.0); // 2007-W01
        assert_eq!(iso("#12/31/2007#"), 1.0); // rolls into 2008-W01
        assert_eq!(iso("#12/31/2005#"), 52.0); // 2005-W52
        assert_eq!(iso("#1/1/2000#"), 52.0); // rolls back to 1999-W52
        assert_eq!(iso("#12/31/2012#"), 1.0); // rolls into 2013-W01
        assert_eq!(iso("#1/1/2010#"), 53.0); // rolls back to 2009-W53
    }

    #[test]
    fn datepart_first_full_week() {
        // crFirstFullWeek (mode 3), Sunday start: week 1 is the first full Sun–Sat week. 2004-01-01
        // is Thursday, so the partial first week belongs to the previous year.
        let full = |d: &str| num(&format!(r#"DatePart("ww", {d}, 1, 3)"#));
        assert_eq!(full("#1/1/2004#"), 52.0); // partial week → prior year
        assert_eq!(full("#1/4/2004#"), 1.0); // first full week starts Sun 1/4
        assert_eq!(full("#1/10/2004#"), 1.0); // still week 1 (Sat)
        assert_eq!(full("#1/11/2004#"), 2.0); // next week
    }

    #[test]
    fn datepart_first_jan1_never_rolls_forward() {
        // crFirstJan1 (mode 1) keeps late-December in the current year (VBA semantics), unlike the
        // ISO-style modes.
        assert_eq!(num(r#"DatePart("ww", #1/1/2004#, 1, 1)"#), 1.0);
        assert_eq!(num(r#"DatePart("ww", #12/31/2004#, 1, 1)"#), 53.0);
        assert_eq!(num(r#"DatePart("ww", #12/31/2000#, 1, 1)"#), 54.0);
    }

    #[test]
    fn weekday_and_names() {
        // 2004-01-03 was a Saturday (Crystal: Sunday=1 … Saturday=7).
        assert_eq!(num("DayOfWeek(#1/3/2004#)"), 7.0);
        assert_eq!(num("Weekday(#1/3/2004#)"), 7.0);
        assert_eq!(num("Weekday(#1/3/2004#, 2)"), 6.0); // Monday-start
        assert_eq!(text("MonthName(2)"), "February");
        assert_eq!(text("MonthName(2, true)"), "Feb");
        assert_eq!(text("WeekdayName(1)"), "Sunday");
        assert!(matches!(run("MonthName(13)"), Err(EvalError::BadArg(_))));
        assert!(matches!(run("WeekdayName(0)"), Err(EvalError::BadArg(_))));
    }

    #[test]
    fn add_diff_and_predicates() {
        assert_eq!(num(r#"DateDiff("d", #1/3/2004#, #2/3/2004#)"#), 31.0);
        assert_eq!(num(r#"DateDiff("m", #1/3/2004#, #3/1/2004#)"#), 2.0);
        assert_eq!(num(r#"DateDiff("yyyy", #6/1/2003#, #1/1/2004#)"#), 1.0);
        assert_eq!(
            run(r#"DateAdd("m", 1, #1/31/2004#)"#),
            Ok(Value::DateTime(Date::new(2004, 2, 29), Time::new(0, 0, 0)))
        );
        assert!(boolean(r#"IsDate("1/3/2004")"#));
        assert!(!boolean(r#"IsDate("nope")"#));
        assert!(boolean("IsDateTime(#1/3/2004 14:05:06#)"));
        assert!(!boolean("IsDateTime(#1/3/2004#)"));
        assert!(matches!(
            run(r#"DateAdd("zz", 1, #1/1/2004#)"#),
            Err(EvalError::BadArg(_))
        ));
    }

    #[test]
    fn ole_serial_epoch() {
        // Numeric serials are OLE Automation dates (days since 1899-12-30).
        assert_eq!(
            run("DateValue(35000)"),
            Ok(Value::Date(Date::new(1995, 10, 28)))
        );
        assert_eq!(run("Date(35000)"), Ok(Value::Date(Date::new(1995, 10, 28))));
        assert_eq!(
            run("DateValue(0)"),
            Ok(Value::Date(Date::new(1899, 12, 30)))
        );
        // Fractional part is the time of day (0.5 = noon).
        assert_eq!(
            run("DateTime(35000.5)"),
            Ok(Value::DateTime(
                Date::new(1995, 10, 28),
                Time::new(12, 0, 0)
            ))
        );
    }

    #[test]
    fn firstdayofweek_threading() {
        // DateDiff("ww", …, crMonday=2) counts Monday-boundary crossings.
        assert_eq!(num(r#"DateDiff("ww", #5/1/2003#, #6/1/2003#, 2)"#), 4.0);
        // The default (Sunday) boundary differs from a Monday boundary here.
        assert_eq!(num(r#"DateDiff("ww", #5/1/2003#, #6/1/2003#)"#), 5.0);
        // WeekdayName's n is relative to firstDayOfWeek: (3, abbrev, crMonday) → Wed.
        assert_eq!(text("WeekdayName(3, true, 2)"), "Wed");
        assert_eq!(text("WeekdayName(1, false, 2)"), "Monday");
        // crUseSystem (0) behaves as Sunday.
        assert_eq!(num("Weekday(#1/3/2004#, 0)"), 7.0);
    }
}
