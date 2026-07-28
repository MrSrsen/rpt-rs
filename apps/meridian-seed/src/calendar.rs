//! Proleptic-Gregorian date arithmetic on a day count.
//!
//! Dates are carried as a signed day offset from the civil epoch 1970-01-01
//! (day 0). Conversion uses Howard Hinnant's well-known `days_from_civil` /
//! `civil_from_days` algorithms — pure integer math, no calendar library, fully
//! deterministic. Master data closes on [`last_day`] (2023-12-31); the market/KPI
//! series span the wider transactional window (see [`market_window`]).

/// Days from 1970-01-01 to the given civil date (Hinnant).
pub(crate) const fn days_from_civil(y: i32, m: u32, d: u32) -> i32 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = (y - era * 400) as i64; // [0, 399]
    let mshift = if m > 2 { m as i64 - 3 } else { m as i64 + 9 };
    let doy = (153 * mshift + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    (era as i64 * 146097 + doe - 719468) as i32
}

/// The civil `(year, month, day)` for a day offset from 1970-01-01 (Hinnant).
pub(crate) fn civil_from_days(z: i32) -> (i32, u32, u32) {
    let z = z as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (y as i32 + if m <= 2 { 1 } else { 0 }, m, d)
}

/// A calendar date as a day offset from 1970-01-01.
pub(crate) fn ymd(y: i32, m: u32, d: u32) -> i32 {
    days_from_civil(y, m, d)
}

/// The latest master-data day (inclusive); the window opens on 2021-01-01.
pub(crate) fn last_day() -> i32 {
    ymd(2023, 12, 31)
}

/// The first calendar year of the market/KPI coverage window.
pub(crate) const MARKET_START_YEAR: i32 = 2021;

/// Months in the market/KPI coverage window (48: 2021-01 .. 2024-12).
pub(crate) const MARKET_MONTHS: i32 = 48;

/// The market/KPI coverage window `(first_day, last_day)` — the full
/// transactional span (2021-01-01 .. 2024-12-31), so every order and shipment
/// date has a matching fuel price, FX rate and carrier scorecard row.
pub(crate) fn market_window() -> (i32, i32) {
    (ymd(2021, 1, 1), ymd(2024, 12, 31))
}

/// `'YYYY-MM-DD'` for a day offset (no quotes).
pub(crate) fn fmt_date(day: i32) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `'YYYY-MM-DD HH:MM:SS'` for a day offset plus a within-day second count.
pub(crate) fn fmt_timestamp(day: i32, secs_in_day: u32) -> String {
    let (y, m, d) = civil_from_days(day);
    let s = secs_in_day % 86_400;
    let (hh, mm, ss) = (s / 3600, (s % 3600) / 60, s % 60);
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
}
