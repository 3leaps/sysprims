//! Expansion (contract A3): civil periods anchored at `dtstart`, BY-part
//! candidate generation, gap removal, set-position selection, bounds, and the
//! union with explicit inclusion and exclusion dates.

use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::num::NonZeroU32;

use jiff::tz::TimeZone;

use crate::civil::{
    civil_from_days, days_from_civil, days_in_month, weekday_of_days, CivilDateTime, Instant,
    MAX_YEAR, SECONDS_PER_DAY,
};
use crate::model::{ByDay, Freq, RecurrenceRule};
use crate::tz;

/// Larger than any UTC offset in tzdb history; bounds a period's instants.
const OFFSET_MARGIN: i64 = 30 * 3600;

/// Consecutive evaluated periods without an eligible occurrence before a
/// query gives up with [`QueryError::SearchLimitExceeded`]. Reaching this
/// limit is a resource refusal, never proof that no occurrence remains.
pub const MAX_EMPTY_PERIODS: u32 = 2_000_000;

/// A query that could not be answered.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum QueryError {
    /// `from` is later than `to`.
    ReversedRange,
    /// The search stopped at [`MAX_EMPTY_PERIODS`] empty periods. Later
    /// occurrences may still exist.
    SearchLimitExceeded,
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReversedRange => f.write_str("query range is reversed"),
            Self::SearchLimitExceeded => write!(
                f,
                "search limit of {MAX_EMPTY_PERIODS} empty periods exceeded; later occurrences are not ruled out"
            ),
        }
    }
}

impl std::error::Error for QueryError {}

/// Precomputed view of a rule for expansion.
pub(crate) struct Engine<'r> {
    rule: &'r RecurrenceRule,
    tz: TimeZone,
    interval: i64,
    start_label: i64,
    start_instant: i64,
    start_day: i64,
    /// Unit index of period 0 (minute, hour, day, week-start day, month index, year).
    unit0: i64,
    until: Option<i64>,
}

/// Label seconds of a day plus a time of day.
fn label(day: i64, hour: u8, minute: u8, second: u8) -> i64 {
    day * SECONDS_PER_DAY + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second)
}

fn to_civil(label: i64) -> CivilDateTime {
    let (y, mo, d, h, mi, s) = CivilDateTime::from_label_seconds(label);
    CivilDateTime {
        year: y,
        month: mo,
        day: d,
        hour: h,
        minute: mi,
        second: s,
    }
}

fn year_of(day: i64) -> i32 {
    civil_from_days(day).0
}

/// The day a month-day selector names, if that day exists.
fn month_day(year: i32, month: u8, md: i8) -> Option<i64> {
    let dim = i32::from(days_in_month(year, month));
    let d = if md > 0 {
        i32::from(md)
    } else {
        dim + i32::from(md) + 1
    };
    (1..=dim)
        .contains(&d)
        .then(|| days_from_civil(year, month, d as u8))
}

/// Days in `[first, last]` that a BYDAY entry selects within that scope.
fn by_day_in(scope: (i64, i64), entry: &ByDay, out: &mut Vec<i64>) {
    let (first, last) = scope;
    let wd = i64::from(entry.day.index());
    let first_match = first + (wd - i64::from(weekday_of_days(first))).rem_euclid(7);
    let last_match = last - (i64::from(weekday_of_days(last)) - wd).rem_euclid(7);
    match entry.ordinal {
        None => out.extend((first_match..=last).step_by(7)),
        Some(o) if o > 0 => {
            let d = first_match + 7 * (i64::from(o) - 1);
            if d <= last {
                out.push(d);
            }
        }
        Some(o) => {
            let d = last_match - 7 * (-i64::from(o) - 1);
            if d >= first {
                out.push(d);
            }
        }
    }
}

fn month_scope(year: i32, month: u8) -> (i64, i64) {
    let first = days_from_civil(year, month, 1);
    (first, first + i64::from(days_in_month(year, month)) - 1)
}

fn year_scope(year: i32) -> (i64, i64) {
    (days_from_civil(year, 1, 1), days_from_civil(year, 12, 31))
}

fn sorted_unique(mut v: Vec<i64>) -> Vec<i64> {
    v.sort_unstable();
    v.dedup();
    v
}

impl<'r> Engine<'r> {
    pub(crate) fn new(rule: &'r RecurrenceRule) -> Self {
        let tz = tz::zone(&rule.tzid).expect("validated rules name a roster zone");
        let start_label = rule.dtstart.label_seconds();
        let start_instant =
            tz::resolve(&tz, rule.dtstart, true).expect("explicit times always resolve");
        let start_day = rule.dtstart.days();
        let unit0 = match rule.freq {
            Freq::Minutely => start_label.div_euclid(60),
            Freq::Hourly => start_label.div_euclid(3600),
            Freq::Daily => start_day,
            Freq::Weekly => {
                let back = (i64::from(weekday_of_days(start_day)) - i64::from(rule.wkst.index()))
                    .rem_euclid(7);
                start_day - back
            }
            Freq::Monthly => i64::from(rule.dtstart.year) * 12 + i64::from(rule.dtstart.month) - 1,
            Freq::Yearly => i64::from(rule.dtstart.year),
        };
        Self {
            rule,
            tz,
            interval: i64::from(rule.interval),
            start_label,
            start_instant,
            start_day,
            unit0,
            until: rule.until.map(|u| u.unix_seconds()),
        }
    }

    fn step(&self) -> i64 {
        match self.rule.freq {
            Freq::Weekly => 7 * self.interval,
            _ => self.interval,
        }
    }

    fn unit(&self, n: i64) -> i64 {
        self.unit0 + n * self.step()
    }

    /// The first label of period `n`, or `None` once the period starts past year 9999.
    fn period_start(&self, n: i64) -> Option<i64> {
        let u = self.unit(n);
        let start = match self.rule.freq {
            Freq::Minutely => u * 60,
            Freq::Hourly => u * 3600,
            Freq::Daily | Freq::Weekly => u * SECONDS_PER_DAY,
            Freq::Monthly => {
                let (y, m) = (u.div_euclid(12), u.rem_euclid(12) + 1);
                if y > i64::from(MAX_YEAR) {
                    return None;
                }
                days_from_civil(y as i32, m as u8, 1) * SECONDS_PER_DAY
            }
            Freq::Yearly => {
                if u > i64::from(MAX_YEAR) {
                    return None;
                }
                days_from_civil(u as i32, 1, 1) * SECONDS_PER_DAY
            }
        };
        (year_of(start.div_euclid(SECONDS_PER_DAY)) <= MAX_YEAR).then_some(start)
    }

    /// Day-level limits used by DAILY, HOURLY and MINUTELY rules.
    fn day_passes(&self, day: i64) -> bool {
        let r = self.rule;
        let (y, m, d) = civil_from_days(day);
        (r.by_month.is_empty() || r.by_month.contains(&m))
            && (r.by_month_day.is_empty() || r.by_month_day.iter().any(|&md| month_day(y, m, md) == Some(day)))
            && (r.by_day.is_empty() || r.by_day.iter().any(|b| b.day.index() == weekday_of_days(day)))
        // DAILY and finer rules carry no ordinals (schema), so weekday equality is the whole test.
        && d >= 1
    }

    /// BYDAY as a limit over month-day candidates, with ordinals in `scope`.
    fn by_day_limits(&self, day: i64, scope: (i64, i64)) -> bool {
        let wd = weekday_of_days(day);
        self.rule.by_day.iter().any(|b| {
            b.day.index() == wd && {
                let mut hit = Vec::new();
                by_day_in(scope, b, &mut hit);
                hit.contains(&day)
            }
        })
    }

    fn hours(&self) -> Vec<u8> {
        if self.rule.by_hour.is_empty() {
            vec![self.rule.dtstart.hour]
        } else {
            self.rule.by_hour.clone()
        }
    }

    fn minutes(&self) -> Vec<u8> {
        if self.rule.by_minute.is_empty() {
            vec![self.rule.dtstart.minute]
        } else {
            self.rule.by_minute.clone()
        }
    }

    /// Candidate days of a monthly or yearly scope, before times are applied.
    fn days_of_month(&self, year: i32, month: u8, ordinal_scope: (i64, i64)) -> Vec<i64> {
        let r = self.rule;
        let mut days = Vec::new();
        if !r.by_month_day.is_empty() {
            for &md in &r.by_month_day {
                if let Some(d) = month_day(year, month, md) {
                    if r.by_day.is_empty() || self.by_day_limits(d, ordinal_scope) {
                        days.push(d);
                    }
                }
            }
        } else if !r.by_day.is_empty() {
            for b in &r.by_day {
                by_day_in(month_scope(year, month), b, &mut days);
            }
        } else if r.dtstart.day <= days_in_month(year, month) {
            days.push(days_from_civil(year, month, r.dtstart.day));
        }
        days
    }

    /// Sorted unique candidate labels of period `n`; `None` past year 9999.
    fn period_labels(&self, n: i64) -> Option<Vec<i64>> {
        let r = self.rule;
        let start = self.period_start(n)?;
        let second = r.dtstart.second;
        let with_times = |days: Vec<i64>| -> Vec<i64> {
            let (hours, minutes) = (self.hours(), self.minutes());
            let mut out = Vec::with_capacity(days.len() * hours.len() * minutes.len());
            for day in sorted_unique(days) {
                if !(1..=MAX_YEAR).contains(&year_of(day)) {
                    continue;
                }
                for &h in &hours {
                    for &m in &minutes {
                        out.push(label(day, h, m, second));
                    }
                }
            }
            out
        };
        let labels = match r.freq {
            Freq::Yearly => {
                let year = self.unit(n) as i32;
                let mut days = Vec::new();
                if r.by_month_day.is_empty() && !r.by_day.is_empty() && r.by_month.is_empty() {
                    for b in &r.by_day {
                        by_day_in(year_scope(year), b, &mut days);
                    }
                } else {
                    let months: Vec<u8> = if !r.by_month.is_empty() {
                        r.by_month.clone()
                    } else if !r.by_month_day.is_empty() {
                        (1..=12).collect()
                    } else {
                        vec![r.dtstart.month]
                    };
                    let year_ordinals = r.by_month.is_empty();
                    for m in months {
                        let scope = if year_ordinals {
                            year_scope(year)
                        } else {
                            month_scope(year, m)
                        };
                        days.extend(self.days_of_month(year, m, scope));
                    }
                }
                with_times(days)
            }
            Freq::Monthly => {
                let u = self.unit(n);
                let (year, month) = (u.div_euclid(12) as i32, (u.rem_euclid(12) + 1) as u8);
                if !r.by_month.is_empty() && !r.by_month.contains(&month) {
                    Vec::new()
                } else {
                    with_times(self.days_of_month(year, month, month_scope(year, month)))
                }
            }
            Freq::Weekly => {
                let first = self.unit(n);
                let weekdays: Vec<u8> = if r.by_day.is_empty() {
                    vec![weekday_of_days(self.start_day)]
                } else {
                    r.by_day.iter().map(|b| b.day.index()).collect()
                };
                let days = (first..first + 7)
                    .filter(|&d| weekdays.contains(&weekday_of_days(d)))
                    .filter(|&d| {
                        r.by_month.is_empty() || r.by_month.contains(&civil_from_days(d).1)
                    })
                    .collect();
                with_times(days)
            }
            Freq::Daily => {
                let day = self.unit(n);
                if self.day_passes(day) {
                    with_times(vec![day])
                } else {
                    Vec::new()
                }
            }
            Freq::Hourly => {
                let (day, hour) = (
                    start.div_euclid(SECONDS_PER_DAY),
                    (start.rem_euclid(SECONDS_PER_DAY) / 3600) as u8,
                );
                if self.day_passes(day) && (r.by_hour.is_empty() || r.by_hour.contains(&hour)) {
                    self.minutes()
                        .into_iter()
                        .map(|m| label(day, hour, m, second))
                        .collect()
                } else {
                    Vec::new()
                }
            }
            Freq::Minutely => {
                let day = start.div_euclid(SECONDS_PER_DAY);
                let tod = start.rem_euclid(SECONDS_PER_DAY);
                let (hour, minute) = ((tod / 3600) as u8, (tod % 3600 / 60) as u8);
                let ok = self.day_passes(day)
                    && (r.by_hour.is_empty() || r.by_hour.contains(&hour))
                    && (r.by_minute.is_empty() || r.by_minute.contains(&minute));
                if ok {
                    vec![label(day, hour, minute, second)]
                } else {
                    Vec::new()
                }
            }
        };
        Some(labels)
    }

    /// Selected `(label, instant)` pairs of period `n` in label order: gap
    /// candidates removed (the explicit gap `dtstart` stays as the seed of
    /// period 0), overlaps at their first instant, then BYSETPOS.
    pub(crate) fn selected(&self, n: i64) -> Option<Vec<(i64, i64)>> {
        let labels = self.period_labels(n)?;
        let mapped: Vec<(i64, i64)> = labels
            .into_iter()
            .filter_map(|l| {
                if n == 0 && l == self.start_label {
                    return Some((l, self.start_instant));
                }
                tz::resolve(&self.tz, to_civil(l), false).map(|i| (l, i))
            })
            .collect();
        let set_pos = &self.rule.by_set_pos;
        if set_pos.is_empty() {
            return Some(mapped);
        }
        let len = mapped.len() as i64;
        let picks: BTreeSet<usize> = set_pos
            .iter()
            .filter_map(|&p| {
                let idx = if p > 0 {
                    i64::from(p) - 1
                } else {
                    len + i64::from(p)
                };
                (0..len).contains(&idx).then_some(idx as usize)
            })
            .collect();
        Some(picks.into_iter().map(|i| mapped[i]).collect())
    }

    /// Whether `dtstart` is one of the selected labels of the first period.
    pub(crate) fn synchronized(&self) -> bool {
        self.selected(0)
            .is_some_and(|sel| sel.iter().any(|&(l, _)| l == self.start_label))
    }

    /// Smallest period index whose unit is at or after `unit`.
    fn index_at_unit(&self, unit: i64) -> i64 {
        let step = self.step();
        (unit - self.unit0 + step - 1).div_euclid(step).max(0)
    }

    /// The next period worth evaluating after an empty period `n`. Sub-daily
    /// rules jump over days (and, for MINUTELY, hours) their limits exclude.
    fn next_period(&self, n: i64, was_empty: bool) -> i64 {
        if !was_empty || !matches!(self.rule.freq, Freq::Hourly | Freq::Minutely) {
            return n + 1;
        }
        let Some(start) = self.period_start(n) else {
            return n + 1;
        };
        let day = start.div_euclid(SECONDS_PER_DAY);
        let per_day = if self.rule.freq == Freq::Hourly {
            24
        } else {
            1440
        };
        if !self.day_passes(day) {
            return self.index_at_unit((day + 1) * per_day).max(n + 1);
        }
        if self.rule.freq == Freq::Minutely {
            let hour = start.rem_euclid(SECONDS_PER_DAY) / 3600;
            if !self.rule.by_hour.is_empty() && !self.rule.by_hour.contains(&(hour as u8)) {
                return self.index_at_unit(day * 1440 + (hour + 1) * 60).max(n + 1);
            }
        }
        n + 1
    }

    /// First period that can hold an instant at or after `from` (preserves
    /// the dtstart anchor; only valid when `count` does not need the prefix).
    fn first_period_for(&self, from: i64) -> i64 {
        let low = from - OFFSET_MARGIN;
        let day = low.div_euclid(SECONDS_PER_DAY);
        let unit = match self.rule.freq {
            Freq::Minutely => low.div_euclid(60),
            Freq::Hourly => low.div_euclid(3600),
            Freq::Daily => day,
            Freq::Weekly => day,
            Freq::Monthly => {
                let (y, m, _) = civil_from_days(day);
                i64::from(y) * 12 + i64::from(m) - 1
            }
            Freq::Yearly => i64::from(year_of(day)),
        };
        ((unit - self.unit0).div_euclid(self.step()) - 1).max(0)
    }
}

enum Next {
    Item(i64),
    /// No RRULE occurrence remains (count, until, or the end of year 9999).
    Exhausted,
    /// Every remaining period lies at or after the horizon.
    Beyond,
}

/// Lazily yields the bounded RRULE occurrences in increasing instant order.
struct RuleIter<'e, 'r> {
    eng: &'e Engine<'r>,
    n: i64,
    buf: VecDeque<i64>,
    last: Option<i64>,
    emitted: u32,
    done: bool,
    max_empty: u32,
}

impl<'e, 'r> RuleIter<'e, 'r> {
    fn new(eng: &'e Engine<'r>, from: i64, max_empty: u32) -> Self {
        let n = if eng.rule.count.is_none() {
            eng.first_period_for(from)
        } else {
            0
        };
        Self {
            eng,
            n,
            buf: VecDeque::new(),
            last: None,
            emitted: 0,
            done: false,
            max_empty,
        }
    }

    fn next(&mut self, horizon: i64) -> Result<Next, QueryError> {
        let mut empty_run: u32 = 0;
        loop {
            if self.done || self.eng.rule.count.is_some_and(|c| self.emitted >= c) {
                self.done = true;
                return Ok(Next::Exhausted);
            }
            if let Some(x) = self.buf.pop_front() {
                if self.eng.until.is_some_and(|u| x > u) {
                    self.done = true;
                    return Ok(Next::Exhausted);
                }
                self.emitted += 1;
                return Ok(Next::Item(x));
            }
            let Some(start) = self.eng.period_start(self.n) else {
                self.done = true;
                return Ok(Next::Exhausted);
            };
            if start - OFFSET_MARGIN >= horizon {
                return Ok(Next::Beyond);
            }
            let Some(sel) = self.eng.selected(self.n) else {
                self.done = true;
                return Ok(Next::Exhausted);
            };
            let floor = self
                .last
                .map_or(self.eng.start_instant, |l| l + 1)
                .max(self.eng.start_instant);
            let mut eligible: Vec<i64> = sel
                .into_iter()
                .map(|(_, i)| i)
                .filter(|&i| i >= floor && Instant::from_unix_seconds(i).is_some())
                .collect();
            eligible.sort_unstable();
            eligible.dedup();
            self.n = self.eng.next_period(self.n, eligible.is_empty());
            if let Some(&max) = eligible.last() {
                self.last = Some(max);
                self.buf.extend(eligible);
                empty_run = 0;
            } else {
                empty_run += 1;
                if empty_run >= self.max_empty {
                    return Err(QueryError::SearchLimitExceeded);
                }
            }
        }
    }
}

impl RecurrenceRule {
    fn explicit(&self, eng: &Engine<'_>, values: &[CivilDateTime]) -> Vec<i64> {
        sorted_unique(
            values
                .iter()
                .filter_map(|&c| tz::resolve(&eng.tz, c, true))
                .filter(|&i| Instant::from_unix_seconds(i).is_some())
                .collect(),
        )
    }

    /// Occurrences in the half-open UTC range `[from, to)`, earliest first, at most `limit`.
    pub fn expand(
        &self,
        from: Instant,
        to: Instant,
        limit: NonZeroU32,
    ) -> Result<Vec<Instant>, QueryError> {
        let (from, to) = (from.unix_seconds(), to.unix_seconds());
        if from > to {
            return Err(QueryError::ReversedRange);
        }
        self.collect(from, to, limit.get() as usize, MAX_EMPTY_PERIODS)
    }

    /// The first occurrence strictly after `after`; `Ok(None)` only when no
    /// later occurrence exists.
    pub fn next_after(&self, after: Instant) -> Result<Option<Instant>, QueryError> {
        let from = after.unix_seconds() + 1;
        // One past the last representable instant, so the horizon never cuts the search short.
        let to = Instant::from_unix_seconds(i64::MAX).map_or(253_402_300_800, |i| i.unix_seconds());
        Ok(self
            .collect(from, to, 1, MAX_EMPTY_PERIODS)?
            .into_iter()
            .next())
    }

    fn collect(
        &self,
        from: i64,
        to: i64,
        limit: usize,
        max_empty: u32,
    ) -> Result<Vec<Instant>, QueryError> {
        let mut out = Vec::new();
        if from >= to {
            return Ok(out);
        }
        let eng = Engine::new(self);
        let rdates = self.explicit(&eng, &self.rdate);
        let exdates: BTreeSet<i64> = self.explicit(&eng, &self.exdate).into_iter().collect();
        let mut rd = rdates.into_iter().filter(|&i| i >= from).peekable();
        // `from` here is an instant bound, so earlier RRULE items are skipped
        // by the merge below rather than by the iterator.
        let mut it = RuleIter::new(&eng, from, max_empty);
        let pull = |it: &mut RuleIter<'_, '_>| -> Result<Option<i64>, QueryError> {
            match it.next(to)? {
                Next::Item(x) => Ok(Some(x)),
                Next::Exhausted | Next::Beyond => Ok(None),
            }
        };
        // RRULE items are pulled only when the merge needs the next one, so a
        // satisfied query never searches past its answer.
        let mut rr = pull(&mut it)?;
        loop {
            let next = match (rr, rd.peek().copied()) {
                (None, None) => break,
                (Some(a), None) => a,
                (None, Some(b)) => b,
                (Some(a), Some(b)) => a.min(b),
            };
            if next >= to {
                break;
            }
            let from_rule = rr == Some(next);
            if rd.peek() == Some(&next) {
                rd.next();
            }
            if next >= from && !exdates.contains(&next) {
                out.push(Instant::from_raw(next));
                if out.len() == limit {
                    break;
                }
            }
            if from_rule {
                rr = pull(&mut it)?;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_limit_is_a_refusal_never_an_empty_answer() {
        let r = RecurrenceRule::from_json(
            br#"{"dtstart":"2026-01-31T09:00:00","tzid":"Etc/UTC","freq":"MONTHLY","by_month_day":[31],"by_month":[1,3]}"#,
        )
        .unwrap();
        let after = Instant::parse("2026-01-31T09:00:00Z")
            .unwrap()
            .unix_seconds()
            + 1;
        let end = Instant::parse("9999-12-31T23:59:59Z")
            .unwrap()
            .unix_seconds()
            + 1;
        // March 31 is one empty month (February) away.
        assert_eq!(
            r.collect(after, end, 1, 2).unwrap()[0].to_string(),
            "2026-03-31T09:00:00Z"
        );
        assert_eq!(
            r.collect(after, end, 1, 1),
            Err(QueryError::SearchLimitExceeded)
        );
    }

    fn expand_all(json: &str, from: &str, to: &str, limit: u32) -> Vec<String> {
        let r = RecurrenceRule::from_json(json.as_bytes()).unwrap();
        r.expand(
            Instant::parse(from).unwrap(),
            Instant::parse(to).unwrap(),
            NonZeroU32::new(limit).unwrap(),
        )
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect()
    }

    // Expected values are derived by hand from contract A3, not from an engine.

    /// Hourly through the 2026-11-01 New York fall-back: 01:30 is ambiguous
    /// and takes its first (EDT) instant; 02:30 exists once, in EST.
    #[test]
    fn hourly_through_a_fold_is_strictly_increasing_without_repeats() {
        let got = expand_all(
            r#"{"dtstart":"2026-11-01T00:30:00","tzid":"America/New_York","freq":"HOURLY","count":6}"#,
            "2026-10-31T00:00:00Z",
            "2026-11-02T00:00:00Z",
            10,
        );
        assert_eq!(
            got,
            [
                "2026-11-01T04:30:00Z",
                "2026-11-01T05:30:00Z",
                "2026-11-01T07:30:00Z",
                "2026-11-01T08:30:00Z",
                "2026-11-01T09:30:00Z",
                "2026-11-01T10:30:00Z",
            ]
        );
    }

    /// The explicit gap DTSTART 02:30 resolves at the pre-gap offset to
    /// 07:30Z, the same instant as the generated 03:30 EDT candidate in the
    /// same period; it is counted once.
    #[test]
    fn a_gap_seed_and_a_generated_candidate_at_one_instant_count_once() {
        let got = expand_all(
            r#"{"dtstart":"2026-03-08T02:30:00","tzid":"America/New_York","freq":"DAILY","by_hour":[2,3],"by_minute":[30],"count":3}"#,
            "2026-03-01T00:00:00Z",
            "2026-04-01T00:00:00Z",
            10,
        );
        assert_eq!(
            got,
            [
                "2026-03-08T07:30:00Z",
                "2026-03-09T06:30:00Z",
                "2026-03-09T07:30:00Z",
            ]
        );
    }

    /// Without BYMONTH, a yearly BYDAY ordinal counts within the year even
    /// when BYMONTHDAY drives the candidates: the 5th that is the year's
    /// first Monday is January 5 in years starting on a Thursday.
    #[test]
    fn yearly_ordinal_with_month_day_and_no_month_counts_in_the_year() {
        let got = expand_all(
            r#"{"dtstart":"2026-01-05T09:00:00","tzid":"Etc/UTC","freq":"YEARLY","by_day":[{"day":"MO","ordinal":1}],"by_month_day":[5],"count":3}"#,
            "2026-01-01T00:00:00Z",
            "2040-01-01T00:00:00Z",
            10,
        );
        assert_eq!(
            got,
            [
                "2026-01-05T09:00:00Z",
                "2032-01-05T09:00:00Z",
                "2037-01-05T09:00:00Z",
            ]
        );
    }
}
