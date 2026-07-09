//! Proleptic-Gregorian calendar types with the civil day-number arithmetic Crystal uses
//! (Date − Date = days, DateTime ± Number = fractional days, Time ± Number = seconds).
//!
//! This is the one home for [`Date`]/[`Time`]; the formula evaluator re-exports them so there is a
//! single calendar implementation across the workspace.

/// A calendar date. Arithmetic runs on civil day numbers (Howard Hinnant's algorithms).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Date {
    /// Proleptic-Gregorian year.
    pub year: i32,
    /// Month of year, 1–12.
    pub month: u8,
    /// Day of month, 1–31.
    pub day: u8,
}

/// A time of day (whole seconds, matching Crystal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Time {
    /// Hour of day, 0–23.
    pub hour: u8,
    /// Minute of hour, 0–59.
    pub minute: u8,
    /// Second of minute, 0–59.
    pub second: u8,
}

impl Date {
    /// A date from its year, month (1–12) and day (1–31) components.
    pub fn new(year: i32, month: u8, day: u8) -> Date {
        Date { year, month, day }
    }

    /// Civil day number (days since 1970-01-01).
    pub fn to_days(self) -> i64 {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let d = i64::from(self.day);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    }

    /// Inverse of [`to_days`](Date::to_days).
    pub fn from_days(z: i64) -> Date {
        let z = z + 719468;
        let era = if z >= 0 { z } else { z - 146096 } / 146097;
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        Date {
            year: (y + i64::from(m <= 2)) as i32,
            month: m as u8,
            day: d as u8,
        }
    }

    /// Day of week, Crystal convention: 1 = Sunday … 7 = Saturday.
    pub fn day_of_week(self) -> u8 {
        (self.to_days().rem_euclid(7) as u8 + 5 - 1) % 7 + 1
    }

    /// From an OLE Automation serial day count (whole days since 1899-12-30, the epoch Crystal's
    /// numeric date serials count from).
    pub fn from_ole_days(n: i64) -> Date {
        Date::from_days(n + OLE_EPOCH_DAYS)
    }

    /// This date as an OLE Automation serial day count (days since 1899-12-30).
    pub fn to_ole_days(self) -> i64 {
        self.to_days() - OLE_EPOCH_DAYS
    }

    /// From a Julian Day Number — the integer date serial Crystal stores in a saved-data batch for a
    /// Date/DateTime field (`2_460_312` is 2024-01-03). Midnight-based JDN, so `serial − 2_440_587`
    /// is the civil day number.
    pub fn from_julian_serial(n: i64) -> Date {
        Date::from_days(n - JDN_EPOCH_DAYS)
    }

    /// This date as a Julian Day Number (the inverse of [`from_julian_serial`](Date::from_julian_serial)).
    pub fn to_julian_serial(self) -> i64 {
        self.to_days() + JDN_EPOCH_DAYS
    }
}

/// Civil day number of the OLE Automation date epoch (1899-12-30): the `to_days` value that a
/// numeric date serial of `0` maps to.
const OLE_EPOCH_DAYS: i64 = -25569;

/// Julian Day Number of the civil epoch (1970-01-01): the JDN that maps to civil day `0`. Crystal's
/// saved-data date serials are JDNs, so `Date::from_days(serial − JDN_EPOCH_DAYS)` recovers the date.
const JDN_EPOCH_DAYS: i64 = 2_440_587;

impl Time {
    /// A time from its hour (0–23), minute (0–59) and second (0–59) components.
    pub fn new(hour: u8, minute: u8, second: u8) -> Time {
        Time {
            hour,
            minute,
            second,
        }
    }

    /// Seconds since midnight.
    pub fn to_seconds(self) -> i64 {
        i64::from(self.hour) * 3600 + i64::from(self.minute) * 60 + i64::from(self.second)
    }

    /// From seconds since midnight (wraps modulo one day).
    pub fn from_seconds(s: i64) -> Time {
        let s = s.rem_euclid(86_400);
        Time {
            hour: (s / 3600) as u8,
            minute: (s % 3600 / 60) as u8,
            second: (s % 60) as u8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn julian_serial_round_trips_saved_data_dates() {
        // Serials observed in a real saved-data batch: 2460312 → 2024-01-03, +2 → 2024-01-05.
        assert_eq!(Date::from_julian_serial(2_460_312), Date::new(2024, 1, 3));
        assert_eq!(Date::from_julian_serial(2_460_314), Date::new(2024, 1, 5));
        assert_eq!(Date::new(2024, 1, 3).to_julian_serial(), 2_460_312);
        for d in [
            Date::new(1970, 1, 1),
            Date::new(1999, 12, 31),
            Date::new(2038, 1, 19),
        ] {
            assert_eq!(Date::from_julian_serial(d.to_julian_serial()), d);
        }
        // The JDN epoch is distinct from the OLE epoch (a common confusion for date serials).
        assert_ne!(
            Date::new(2024, 1, 3).to_julian_serial(),
            Date::new(2024, 1, 3).to_ole_days()
        );
    }
}
