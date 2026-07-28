//! The relative-date-range constants (`YearToDate`, `Last7Days`, `LastFullMonth`, `Calendar1stQtr`,
//! …): 0-ary Crystal constants that evaluate to a `To` [`Value::Range`] of dates relative to a
//! reference "today". The reference is the context's `CurrentDate`/`Today` special (the print date);
//! all bounds are inclusive, matching the engine's built-in ranges.

use crate::eval::{Date, EvalContext, EvalError, Value};

/// A Crystal relative-date-range constant. Each resolves to an inclusive `[lo, hi]` date range
/// (see [`DateRange::bounds`]) against a reference `today`.
#[derive(Clone, Copy, Debug)]
pub(super) enum DateRange {
    /// `weektodatefromsun` (funcID 197): the current (Sunday-started) week up to `today`.
    WeekToDateFromSun,
    /// `monthtodate` (funcID 198): the 1st of the current month up to `today`.
    MonthToDate,
    /// `yeartodate` (funcID 199): Jan 1 of the current year up to `today`.
    YearToDate,
    /// `last7days` (funcID 200): the seven days ending at `today` (`today − 6 .. today`).
    Last7Days,
    /// `last4weekstosun` (funcID 201): the four weeks (28 days) ending at the nearest past Sunday.
    Last4WeeksToSun,
    /// `lastfullweek` (funcID 202): the previous complete Sunday–Saturday week.
    LastFullWeek,
    /// `lastfullmonth` (funcID 203): the whole previous calendar month.
    LastFullMonth,
    /// `alldatestotoday` (funcID 204): every date up to and including `today`.
    AllDatesToToday,
    /// `alldatestoyesterday` (funcID 205): every date up to and including `today − 1`.
    AllDatesToYesterday,
    /// `alldatesfromtoday` (funcID 206): `today` and every later date.
    AllDatesFromToday,
    /// `alldatesfromtomorrow` (funcID 207): `today + 1` and every later date.
    AllDatesFromTomorrow,
    /// `aged0to30days` (funcID 208): `today − 30 .. today`.
    Aged0To30Days,
    /// `aged31to60days` (funcID 209): `today − 60 .. today − 31`.
    Aged31To60Days,
    /// `aged61to90days` (funcID 210): `today − 90 .. today − 61`.
    Aged61To90Days,
    /// `over90days` (funcID 211): every date older than 90 days (up to `today − 91`).
    Over90Days,
    /// `next30days` (funcID 212): `today .. today + 30`.
    Next30Days,
    /// `next31to60days` (funcID 213): `today + 31 .. today + 60`.
    Next31To60Days,
    /// `next61to90days` (funcID 214): `today + 61 .. today + 90`.
    Next61To90Days,
    /// `next91to365days` (funcID 215): `today + 91 .. today + 365`.
    Next91To365Days,
    /// `calendar1stqtr` (funcID 216): Jan 1 – Mar 31 of the current year.
    Calendar1stQtr,
    /// `calendar2ndqtr` (funcID 217): Apr 1 – Jun 30 of the current year.
    Calendar2ndQtr,
    /// `calendar3rdqtr` (funcID 218): Jul 1 – Sep 30 of the current year.
    Calendar3rdQtr,
    /// `calendar4thqtr` (funcID 219): Oct 1 – Dec 31 of the current year.
    Calendar4thQtr,
    /// `calendar1sthalf` (funcID 220): Jan 1 – Jun 30 of the current year.
    Calendar1stHalf,
    /// `calendar2ndhalf` (funcID 221): Jul 1 – Dec 31 of the current year.
    Calendar2ndHalf,
    /// `lastyearmtd` (funcID 222): last year's 1st-of-this-month up to `today` a year ago.
    LastYearMtd,
    /// `lastyearytd` (funcID 223): last year's Jan 1 up to `today` a year ago.
    LastYearYtd,
}

/// Sentinel lower bound for the open-ended ranges (`AllDatesToToday`, `Over90Days`, …) — earlier
/// than any date a report field can hold, so range membership is effectively unbounded below.
const MIN_DATE: Date = Date {
    year: 1,
    month: 1,
    day: 1,
};
/// Sentinel upper bound for the open-ended ranges (`AllDatesFromToday`, …).
const MAX_DATE: Date = Date {
    year: 9999,
    month: 12,
    day: 31,
};

/// Evaluate a date-range constant against the context's reference date, yielding an inclusive
/// [`Value::Range`] of dates. Errors only when the context supplies no reference date at all.
pub(super) fn eval(range: DateRange, ctx: &dyn EvalContext) -> Result<Value, EvalError> {
    let today = reference_date(ctx)?;
    let (lo, hi) = range.bounds(today);
    Ok(Value::Range {
        lo: Box::new(Value::Date(lo)),
        hi: Box::new(Value::Date(hi)),
        lo_incl: true,
        hi_incl: true,
    })
}

/// The reference "today" for a relative date range: the context's `CurrentDate`/`Today` special (the
/// print date), falling back to `DataDate`. A `DateTime` special contributes its date part.
fn reference_date(ctx: &dyn EvalContext) -> Result<Date, EvalError> {
    for name in ["currentdate", "today", "datadate"] {
        match ctx.special(name) {
            Some(Value::Date(d)) => return Ok(d),
            Some(Value::DateTime(d, _)) => return Ok(d),
            _ => {}
        }
    }
    Err(EvalError::BadArg(
        "date range needs a reference date (CurrentDate/Today)".to_string(),
    ))
}

impl DateRange {
    /// The inclusive `[lo, hi]` date bounds of this range relative to `today`.
    pub(super) fn bounds(self, today: Date) -> (Date, Date) {
        use DateRange as R;
        let y = today.year;
        match self {
            R::WeekToDateFromSun => (week_start_sun(today), today),
            R::MonthToDate => (first_of_month(today), today),
            R::YearToDate => (ym_first(y, 1), today),
            R::Last7Days => (add_days(today, -6), today),
            R::Last4WeeksToSun => {
                let sun = week_start_sun(today);
                (add_days(sun, -27), sun)
            }
            R::LastFullWeek => {
                let last_sun = add_days(week_start_sun(today), -7);
                (last_sun, add_days(last_sun, 6))
            }
            R::LastFullMonth => (
                ym_first(y, i32::from(today.month) - 1),
                add_days(first_of_month(today), -1),
            ),
            R::AllDatesToToday => (MIN_DATE, today),
            R::AllDatesToYesterday => (MIN_DATE, add_days(today, -1)),
            R::AllDatesFromToday => (today, MAX_DATE),
            R::AllDatesFromTomorrow => (add_days(today, 1), MAX_DATE),
            R::Aged0To30Days => (add_days(today, -30), today),
            R::Aged31To60Days => (add_days(today, -60), add_days(today, -31)),
            R::Aged61To90Days => (add_days(today, -90), add_days(today, -61)),
            R::Over90Days => (MIN_DATE, add_days(today, -91)),
            R::Next30Days => (today, add_days(today, 30)),
            R::Next31To60Days => (add_days(today, 31), add_days(today, 60)),
            R::Next61To90Days => (add_days(today, 61), add_days(today, 90)),
            R::Next91To365Days => (add_days(today, 91), add_days(today, 365)),
            R::Calendar1stQtr => (ym_first(y, 1), end_of_month(y, 3)),
            R::Calendar2ndQtr => (ym_first(y, 4), end_of_month(y, 6)),
            R::Calendar3rdQtr => (ym_first(y, 7), end_of_month(y, 9)),
            R::Calendar4thQtr => (ym_first(y, 10), end_of_month(y, 12)),
            R::Calendar1stHalf => (ym_first(y, 1), end_of_month(y, 6)),
            R::Calendar2ndHalf => (ym_first(y, 7), end_of_month(y, 12)),
            R::LastYearMtd => (
                ym_first(y - 1, i32::from(today.month)),
                shift_years(today, -1),
            ),
            R::LastYearYtd => (ym_first(y - 1, 1), shift_years(today, -1)),
        }
    }
}

/// `date + n` days (negative to go back), via civil day-number arithmetic.
fn add_days(d: Date, n: i64) -> Date {
    Date::from_days(d.to_days() + n)
}

/// The Sunday that starts `date`'s week (`date` itself when it is a Sunday). Crystal's
/// [`Date::day_of_week`] is `1 = Sunday … 7 = Saturday`.
fn week_start_sun(date: Date) -> Date {
    add_days(date, -(i64::from(date.day_of_week()) - 1))
}

/// The first day of `date`'s calendar month.
fn first_of_month(date: Date) -> Date {
    Date {
        year: date.year,
        month: date.month,
        day: 1,
    }
}

/// The first of a month given as a possibly out-of-range 1-based month index, normalizing into a
/// valid `(year, month)` (`month = 0` → previous December, `13` → next January).
fn ym_first(year: i32, month: i32) -> Date {
    let idx = month - 1;
    Date {
        year: year + idx.div_euclid(12),
        month: (idx.rem_euclid(12) + 1) as u8,
        day: 1,
    }
}

/// The last day of the given month (via the day before the following month's first).
fn end_of_month(year: i32, month: i32) -> Date {
    add_days(ym_first(year, month + 1), -1)
}

/// `date` shifted by whole years, clamping the day of month so Feb 29 lands on Feb 28 in a
/// non-leap year.
fn shift_years(date: Date, delta: i32) -> Date {
    let year = date.year + delta;
    let last = end_of_month(year, i32::from(date.month)).day;
    Date {
        year,
        month: date.month,
        day: date.day.min(last),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u8, day: u8) -> Date {
        Date::new(y, m, day)
    }

    fn range(r: DateRange, today: Date) -> (Date, Date) {
        r.bounds(today)
    }

    /// 2024-03-15 is a Friday; anchor the calendar-relative ranges on it.
    #[test]
    fn month_and_year_to_date() {
        let t = d(2024, 3, 15);
        assert_eq!(range(DateRange::MonthToDate, t), (d(2024, 3, 1), t));
        assert_eq!(range(DateRange::YearToDate, t), (d(2024, 1, 1), t));
        assert_eq!(range(DateRange::Last7Days, t), (d(2024, 3, 9), t));
    }

    #[test]
    fn week_ranges_are_sunday_based() {
        // 2024-03-15 is a Friday; the current week's Sunday is 2024-03-10.
        let t = d(2024, 3, 15);
        assert_eq!(range(DateRange::WeekToDateFromSun, t), (d(2024, 3, 10), t));
        // The previous full week is Sun 2024-03-03 .. Sat 2024-03-09.
        assert_eq!(
            range(DateRange::LastFullWeek, t),
            (d(2024, 3, 3), d(2024, 3, 9))
        );
        // A Sunday reference starts its own week.
        let sun = d(2024, 3, 10);
        assert_eq!(range(DateRange::WeekToDateFromSun, sun), (sun, sun));
    }

    #[test]
    fn last_full_month_spans_previous_month() {
        assert_eq!(
            range(DateRange::LastFullMonth, d(2024, 3, 15)),
            (d(2024, 2, 1), d(2024, 2, 29)) // 2024 is a leap year
        );
        // January rolls back to the previous December.
        assert_eq!(
            range(DateRange::LastFullMonth, d(2024, 1, 10)),
            (d(2023, 12, 1), d(2023, 12, 31))
        );
    }

    #[test]
    fn aging_and_next_buckets() {
        let t = d(2024, 3, 15);
        assert_eq!(range(DateRange::Aged0To30Days, t), (d(2024, 2, 14), t));
        assert_eq!(
            range(DateRange::Aged31To60Days, t),
            (d(2024, 1, 15), d(2024, 2, 13))
        );
        assert_eq!(range(DateRange::Next30Days, t), (t, d(2024, 4, 14)));
    }

    #[test]
    fn open_ended_ranges_use_sentinels() {
        let t = d(2024, 3, 15);
        assert_eq!(range(DateRange::AllDatesToToday, t), (MIN_DATE, t));
        assert_eq!(
            range(DateRange::AllDatesFromTomorrow, t),
            (d(2024, 3, 16), MAX_DATE)
        );
        assert_eq!(range(DateRange::Over90Days, t), (MIN_DATE, d(2023, 12, 15)));
    }

    #[test]
    fn calendar_quarters_and_halves() {
        let t = d(2024, 8, 20);
        assert_eq!(
            range(DateRange::Calendar3rdQtr, t),
            (d(2024, 7, 1), d(2024, 9, 30))
        );
        assert_eq!(
            range(DateRange::Calendar1stHalf, t),
            (d(2024, 1, 1), d(2024, 6, 30))
        );
    }

    #[test]
    fn last_year_ranges_shift_back_a_year() {
        let t = d(2024, 3, 15);
        assert_eq!(
            range(DateRange::LastYearMtd, t),
            (d(2023, 3, 1), d(2023, 3, 15))
        );
        assert_eq!(
            range(DateRange::LastYearYtd, t),
            (d(2023, 1, 1), d(2023, 3, 15))
        );
        // Leap-day reference clamps to Feb 28 in the (non-leap) prior year.
        assert_eq!(
            range(DateRange::LastYearYtd, d(2024, 2, 29)),
            (d(2023, 1, 1), d(2023, 2, 28))
        );
    }
}
