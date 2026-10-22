//! Proleptic Gregorian civil arithmetic on whole seconds.
//!
//! Days are counted from 1970-01-01. Every value in this module stays inside
//! years 0001..=9999, the contract's calendar domain.

use std::fmt;

pub(crate) const MIN_YEAR: i32 = 1;
pub(crate) const MAX_YEAR: i32 = 9999;
pub(crate) const SECONDS_PER_DAY: i64 = 86_400;

/// Days from 1970-01-01 to the given civil date (proleptic Gregorian).
pub(crate) fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`].
pub(crate) fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
    (year, month, day)
}

pub(crate) fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub(crate) fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(year) => 29,
        _ => 28,
    }
}

/// Weekday index of a day count, Monday = 0 .. Sunday = 6.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn weekday_of_days(days: i64) -> u8 {
    // 1970-01-01 was a Thursday (index 3).
    (days + 3).rem_euclid(7) as u8
}

/// A local civil date-time with whole seconds, no offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilDateTime {
    pub(crate) year: i32,
    pub(crate) month: u8,
    pub(crate) day: u8,
    pub(crate) hour: u8,
    pub(crate) minute: u8,
    pub(crate) second: u8,
}

impl CivilDateTime {
    /// Builds a value only when it names a real Gregorian date-time in years 0001..=9999.
    pub fn new(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Option<Self> {
        let valid = (MIN_YEAR..=MAX_YEAR).contains(&year)
            && (1..=12).contains(&month)
            && day >= 1
            && day <= days_in_month(year, month)
            && hour <= 23
            && minute <= 59
            && second <= 59;
        valid.then_some(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }

    pub fn year(&self) -> i32 {
        self.year
    }
    pub fn month(&self) -> u8 {
        self.month
    }
    pub fn day(&self) -> u8 {
        self.day
    }
    pub fn hour(&self) -> u8 {
        self.hour
    }
    pub fn minute(&self) -> u8 {
        self.minute
    }
    pub fn second(&self) -> u8 {
        self.second
    }

    pub(crate) fn days(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    /// Seconds of this label read as if it were UTC; orders labels.
    pub(crate) fn label_seconds(&self) -> i64 {
        self.days() * SECONDS_PER_DAY
            + i64::from(self.hour) * 3600
            + i64::from(self.minute) * 60
            + i64::from(self.second)
    }

    pub(crate) fn from_label_seconds(seconds: i64) -> (i32, u8, u8, u8, u8, u8) {
        let days = seconds.div_euclid(SECONDS_PER_DAY);
        let rem = seconds.rem_euclid(SECONDS_PER_DAY);
        let (y, mo, d) = civil_from_days(days);
        (
            y,
            mo,
            d,
            (rem / 3600) as u8,
            (rem % 3600 / 60) as u8,
            (rem % 60) as u8,
        )
    }

    /// `YYYY-MM-DDTHH:MM:SS`
    pub(crate) fn to_extended(self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// `YYYYMMDDTHHMMSS`
    pub(crate) fn to_basic(self) -> String {
        format!(
            "{:04}{:02}{:02}T{:02}{:02}{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

impl fmt::Display for CivilDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_extended())
    }
}

/// Splits a fixed-width all-digit field list; `None` unless every byte is an ASCII digit.
pub(crate) fn digits(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Lexical form check for `YYYY-MM-DDTHH:MM:SS` (optionally with a trailing `Z`).
/// Returns the six numeric fields without judging calendar validity.
pub(crate) fn lex_extended(s: &str, utc: bool) -> Option<[u32; 6]> {
    let body = if utc { s.strip_suffix('Z')? } else { s };
    let b = body.as_bytes();
    if b.len() != 19
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    Some([
        digits(&body[0..4])?,
        digits(&body[5..7])?,
        digits(&body[8..10])?,
        digits(&body[11..13])?,
        digits(&body[14..16])?,
        digits(&body[17..19])?,
    ])
}

/// Lexical form check for `YYYYMMDDTHHMMSS` (optionally with a trailing `Z`).
pub(crate) fn lex_basic(s: &str, utc: bool) -> Option<[u32; 6]> {
    let body = if utc { s.strip_suffix('Z')? } else { s };
    let b = body.as_bytes();
    if b.len() != 15 || b[8] != b'T' {
        return None;
    }
    Some([
        digits(&body[0..4])?,
        digits(&body[4..6])?,
        digits(&body[6..8])?,
        digits(&body[9..11])?,
        digits(&body[11..13])?,
        digits(&body[13..15])?,
    ])
}

pub(crate) fn civil_from_fields(f: [u32; 6]) -> Option<CivilDateTime> {
    let narrow = |v: u32| u8::try_from(v).ok();
    CivilDateTime::new(
        i32::try_from(f[0]).ok()?,
        narrow(f[1])?,
        narrow(f[2])?,
        narrow(f[3])?,
        narrow(f[4])?,
        narrow(f[5])?,
    )
}

/// A UTC instant in whole seconds since 1970-01-01T00:00:00Z, within years 0001..=9999.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(i64);

impl Instant {
    const MIN: i64 = -62_135_596_800; // 0001-01-01T00:00:00Z
    const MAX: i64 = 253_402_300_799; // 9999-12-31T23:59:59Z

    /// Builds an instant from Unix seconds, if it lies in years 0001..=9999.
    pub fn from_unix_seconds(seconds: i64) -> Option<Self> {
        (Self::MIN..=Self::MAX)
            .contains(&seconds)
            .then_some(Self(seconds))
    }

    pub fn unix_seconds(&self) -> i64 {
        self.0
    }

    /// Parses `YYYY-MM-DDTHH:MM:SSZ`.
    pub fn parse(s: &str) -> Option<Self> {
        let c = civil_from_fields(lex_extended(s, true)?)?;
        Some(Self(c.label_seconds()))
    }

    pub(crate) fn from_raw(seconds: i64) -> Self {
        Self(seconds)
    }
}

impl fmt::Display for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, mo, d, h, mi, s) = CivilDateTime::from_label_seconds(self.0);
        write!(f, "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_counts_round_trip_across_the_domain() {
        for days in [
            days_from_civil(1, 1, 1),
            -1,
            0,
            11_016,
            days_from_civil(2000, 2, 29),
            days_from_civil(9999, 12, 31),
        ] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(weekday_of_days(days_from_civil(2026, 1, 5)), 0); // Monday
    }

    #[test]
    fn instant_bounds_and_format() {
        assert_eq!(
            Instant::parse("0001-01-01T00:00:00Z").unwrap().0,
            Instant::MIN
        );
        assert_eq!(
            Instant::parse("9999-12-31T23:59:59Z").unwrap().0,
            Instant::MAX
        );
        assert!(Instant::parse("0000-12-31T23:59:59Z").is_none());
        assert!(Instant::parse("2026-02-29T00:00:00Z").is_none());
        assert!(Instant::parse("2026-01-01T00:00:00").is_none());
        let i = Instant::parse("2026-03-08T07:30:00Z").unwrap();
        assert_eq!(i.to_string(), "2026-03-08T07:30:00Z");
    }
}
