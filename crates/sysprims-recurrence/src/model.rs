//! The rule model: JSON and text decoding, schema checks, normalization,
//! canonical bytes, digest and canonical text.

use sha2::{Digest, Sha256};

use crate::civil::{civil_from_fields, lex_basic, lex_extended, CivilDateTime, Instant};
use crate::json::{self, Out, Value};
use crate::refusal::{self, Refusal};

/// The capability string this crate implements.
pub const CONTRACT: &str = "contract: recurrence/v0";

/// Recurrence frequency.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Freq {
    Minutely,
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Freq {
    const ALL: [(Freq, &'static str); 6] = [
        (Freq::Minutely, "MINUTELY"),
        (Freq::Hourly, "HOURLY"),
        (Freq::Daily, "DAILY"),
        (Freq::Weekly, "WEEKLY"),
        (Freq::Monthly, "MONTHLY"),
        (Freq::Yearly, "YEARLY"),
    ];

    pub fn as_str(&self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(f, _)| f == self)
            .map(|(_, s)| *s)
            .unwrap_or("")
    }

    fn from_str(s: &str) -> Option<Self> {
        Self::ALL.iter().find(|(_, n)| *n == s).map(|(f, _)| *f)
    }
}

/// Day of the week, in the contract's MO..SU order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Weekday {
    Mo,
    Tu,
    We,
    Th,
    Fr,
    Sa,
    Su,
}

impl Weekday {
    const NAMES: [&'static str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];
    const ALL: [Weekday; 7] = [
        Weekday::Mo,
        Weekday::Tu,
        Weekday::We,
        Weekday::Th,
        Weekday::Fr,
        Weekday::Sa,
        Weekday::Su,
    ];

    pub fn as_str(&self) -> &'static str {
        Self::NAMES[self.index() as usize]
    }

    /// Monday = 0 .. Sunday = 6.
    pub fn index(&self) -> u8 {
        *self as u8
    }

    fn from_str(s: &str) -> Option<Self> {
        Self::NAMES
            .iter()
            .position(|n| *n == s)
            .map(|i| Self::ALL[i])
    }
}

/// One `BYDAY` entry: a weekday with an optional nonzero ordinal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ByDay {
    pub day: Weekday,
    pub ordinal: Option<i8>,
}

impl ByDay {
    /// Normal-form order: weekday, then the unqualified entry, then ascending ordinals.
    fn sort_key(&self) -> (u8, bool, i8) {
        (
            self.day.index(),
            self.ordinal.is_some(),
            self.ordinal.unwrap_or(0),
        )
    }
}

/// A validated, normalized recurrence rule. Constructors are the only way to
/// obtain one, so every value is schema-valid, semantically valid and in
/// normal form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrenceRule {
    pub(crate) dtstart: CivilDateTime,
    pub(crate) tzid: String,
    pub(crate) freq: Freq,
    pub(crate) interval: u16,
    pub(crate) count: Option<u32>,
    pub(crate) until: Option<Instant>,
    pub(crate) wkst: Weekday,
    pub(crate) by_day: Vec<ByDay>,
    pub(crate) by_month: Vec<u8>,
    pub(crate) by_month_day: Vec<i8>,
    pub(crate) by_hour: Vec<u8>,
    pub(crate) by_minute: Vec<u8>,
    pub(crate) by_set_pos: Vec<i16>,
    pub(crate) rdate: Vec<CivilDateTime>,
    pub(crate) exdate: Vec<CivilDateTime>,
}

/// Fields as decoded, before calendar validity and normalization.
struct Draft {
    dtstart: [u32; 6],
    tzid: String,
    freq: Freq,
    interval: i64,
    count: Option<i64>,
    until: Option<[u32; 6]>,
    wkst: Weekday,
    by_day: Vec<(Weekday, Option<i64>)>,
    by_month: Vec<i64>,
    by_month_day: Vec<i64>,
    by_hour: Vec<i64>,
    by_minute: Vec<i64>,
    by_set_pos: Vec<i64>,
    rdate: Vec<[u32; 6]>,
    exdate: Vec<[u32; 6]>,
}

const RULE_FIELDS: [&str; 16] = [
    "dtstart",
    "tzid",
    "freq",
    "interval",
    "count",
    "until",
    "wkst",
    "by_day",
    "by_month",
    "by_month_day",
    "by_hour",
    "by_minute",
    "by_set_pos",
    "rdate",
    "exdate",
    "calendar",
];

fn tzid_lexically_valid(s: &str) -> bool {
    // ^[A-Za-z][A-Za-z0-9_+-]*(/[A-Za-z0-9_+-]+)*$, at most 64 characters.
    let tail = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-');
    let mut segments = s.split('/');
    let first = segments.next().unwrap_or("").as_bytes();
    s.chars().count() <= 64
        && first.first().is_some_and(u8::is_ascii_alphabetic)
        && first.iter().copied().all(tail)
        && segments.all(|seg| !seg.is_empty() && seg.bytes().all(tail))
}

impl Draft {
    fn from_value(v: &Value) -> Result<Self, Refusal> {
        let Value::Object(map) = v else {
            return Err(refusal::rule("rule is not an object"));
        };
        if let Some(_unknown) = map.keys().find(|k| !RULE_FIELDS.contains(&k.as_str())) {
            return Err(refusal::rule("rule has an unknown member"));
        }
        let get = |k: &str| map.get(k);
        let string = |k: &'static str| -> Result<Option<&str>, Refusal> {
            match get(k) {
                None => Ok(None),
                Some(Value::Str(s)) => Ok(Some(s)),
                Some(_) => Err(refusal::rule(k)),
            }
        };
        let integer = |k: &'static str, lo: i64, hi: i64| -> Result<Option<i64>, Refusal> {
            match get(k) {
                None => Ok(None),
                Some(v) => match v.as_integer() {
                    Some(n) if (lo..=hi).contains(&n) => Ok(Some(n)),
                    _ => Err(refusal::rule(k)),
                },
            }
        };
        let int_list =
            |k: &'static str, lo: i64, hi: i64, nonzero: bool| -> Result<Vec<i64>, Refusal> {
                match get(k) {
                    None => Ok(Vec::new()),
                    Some(Value::Array(items)) => items
                        .iter()
                        .map(|v| match v.as_integer() {
                            Some(n) if (lo..=hi).contains(&n) && !(nonzero && n == 0) => Ok(n),
                            _ => Err(refusal::rule(k)),
                        })
                        .collect(),
                    Some(_) => Err(refusal::rule(k)),
                }
            };
        let local_list = |k: &'static str| -> Result<Vec<[u32; 6]>, Refusal> {
            match get(k) {
                None => Ok(Vec::new()),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|v| match v {
                        Value::Str(s) => lex_extended(s, false).ok_or_else(|| refusal::rule(k)),
                        _ => Err(refusal::rule(k)),
                    })
                    .collect(),
                Some(_) => Err(refusal::rule(k)),
            }
        };

        let dtstart = string("dtstart")?.ok_or_else(|| refusal::rule("dtstart is required"))?;
        let dtstart = lex_extended(dtstart, false).ok_or_else(|| refusal::rule("dtstart"))?;
        let tzid = string("tzid")?.ok_or_else(|| refusal::rule("tzid is required"))?;
        if !tzid_lexically_valid(tzid) {
            return Err(refusal::rule("tzid"));
        }
        let freq = string("freq")?.ok_or_else(|| refusal::rule("freq is required"))?;
        let freq = Freq::from_str(freq).ok_or_else(|| refusal::rule("freq"))?;
        let interval = integer("interval", 1, 1000)?.unwrap_or(1);
        let count = integer("count", 1, 100_000)?;
        let until = match string("until")? {
            None => None,
            Some(s) => Some(lex_extended(s, true).ok_or_else(|| refusal::rule("until"))?),
        };
        if get("count").is_some() && get("until").is_some() {
            return Err(refusal::rule("count and until are exclusive"));
        }
        let wkst = match string("wkst")? {
            None => Weekday::Mo,
            Some(s) => Weekday::from_str(s).ok_or_else(|| refusal::rule("wkst"))?,
        };
        if let Some(c) = string("calendar")? {
            if c != "gregorian" {
                return Err(refusal::rule("calendar"));
            }
        }
        let by_day = match get("by_day") {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    let Value::Object(o) = item else {
                        return Err(refusal::rule("by_day"));
                    };
                    if o.keys().any(|k| k != "day" && k != "ordinal") {
                        return Err(refusal::rule("by_day"));
                    }
                    let day = match o.get("day") {
                        Some(Value::Str(s)) => {
                            Weekday::from_str(s).ok_or_else(|| refusal::rule("by_day"))?
                        }
                        _ => return Err(refusal::rule("by_day")),
                    };
                    let ordinal = match o.get("ordinal") {
                        None => None,
                        Some(v) => match v.as_integer() {
                            Some(n) if (-53..=53).contains(&n) && n != 0 => Some(n),
                            _ => return Err(refusal::rule("by_day ordinal")),
                        },
                    };
                    Ok((day, ordinal))
                })
                .collect::<Result<_, _>>()?,
            Some(_) => return Err(refusal::rule("by_day")),
        };
        let draft = Draft {
            dtstart,
            tzid: tzid.to_owned(),
            freq,
            interval,
            count,
            until,
            wkst,
            by_day,
            by_month: int_list("by_month", 1, 12, false)?,
            by_month_day: int_list("by_month_day", -31, 31, true)?,
            by_hour: int_list("by_hour", 0, 23, false)?,
            by_minute: int_list("by_minute", 0, 59, false)?,
            by_set_pos: int_list("by_set_pos", -366, 366, true)?,
            rdate: local_list("rdate")?,
            exdate: local_list("exdate")?,
        };
        draft.check_combinations()?;
        Ok(draft)
    }

    /// Cross-field schema rules (A6).
    fn check_combinations(&self) -> Result<(), Refusal> {
        let companion = !self.by_day.is_empty()
            || !self.by_month_day.is_empty()
            || !self.by_month.is_empty()
            || !self.by_hour.is_empty()
            || !self.by_minute.is_empty();
        if !self.by_set_pos.is_empty() && !companion {
            return Err(refusal::rule("by_set_pos needs another nonempty BY-part"));
        }
        let ordinal_freq = matches!(self.freq, Freq::Monthly | Freq::Yearly);
        if !ordinal_freq && self.by_day.iter().any(|(_, o)| o.is_some()) {
            return Err(refusal::rule("by_day ordinals need MONTHLY or YEARLY"));
        }
        if self.freq == Freq::Weekly && !self.by_month_day.is_empty() {
            return Err(refusal::rule("by_month_day is not allowed with WEEKLY"));
        }
        Ok(())
    }

    /// Calendar validity of every explicit date-time, then normalization.
    fn into_rule(self) -> Result<RecurrenceRule, Refusal> {
        let civil = |f: [u32; 6], what: &'static str| {
            civil_from_fields(f).ok_or_else(|| refusal::date(what))
        };
        let dtstart = civil(self.dtstart, "dtstart")?;
        let mut rdate = self
            .rdate
            .into_iter()
            .map(|f| civil(f, "rdate"))
            .collect::<Result<Vec<_>, _>>()?;
        let mut exdate = self
            .exdate
            .into_iter()
            .map(|f| civil(f, "exdate"))
            .collect::<Result<Vec<_>, _>>()?;
        let until = match self.until {
            None => None,
            Some(f) => Some(Instant::from_raw(civil(f, "until")?.label_seconds())),
        };
        rdate.sort_unstable();
        rdate.dedup();
        exdate.sort_unstable();
        exdate.dedup();

        fn norm<T: Ord + Copy>(v: Vec<i64>, f: impl Fn(i64) -> T) -> Vec<T> {
            let mut out: Vec<T> = v.into_iter().map(f).collect();
            out.sort_unstable();
            out.dedup();
            out
        }
        // Bounds were checked during decoding, so these narrowing casts are exact.
        let mut by_day: Vec<ByDay> = self
            .by_day
            .into_iter()
            .map(|(day, o)| ByDay {
                day,
                ordinal: o.map(|n| n as i8),
            })
            .collect();
        by_day.sort_unstable_by_key(ByDay::sort_key);
        by_day.dedup();

        Ok(RecurrenceRule {
            dtstart,
            tzid: self.tzid,
            freq: self.freq,
            interval: self.interval as u16,
            count: self.count.map(|n| n as u32),
            until,
            wkst: self.wkst,
            by_day,
            by_month: norm(self.by_month, |n| n as u8),
            by_month_day: norm(self.by_month_day, |n| n as i8),
            by_hour: norm(self.by_hour, |n| n as u8),
            by_minute: norm(self.by_minute, |n| n as u8),
            by_set_pos: norm(self.by_set_pos, |n| n as i16),
            rdate,
            exdate,
        })
    }
}

// --- RFC 5545 text ---------------------------------------------------------------

/// Integer lexeme grammar (A5): `0`, or an optional sign, a nonzero digit and at
/// most eight further digits. Checked before conversion.
fn text_int(s: &str) -> Result<i64, Refusal> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    let ok = if digits.len() == s.len() && s == "0" {
        true
    } else {
        (1..=9).contains(&digits.len())
            && digits.as_bytes()[0] != b'0'
            && digits.bytes().all(|b| b.is_ascii_digit())
    };
    if !ok {
        return Err(refusal::text("integer lexeme"));
    }
    s.parse::<i64>()
        .map_err(|_| refusal::text("integer lexeme"))
}

const UNSUPPORTED_RRULE_PARTS: [&str; 3] = ["BYSECOND", "BYWEEKNO", "BYYEARDAY"];

fn parse_rrule(value: &str, d: &mut TextDraft) -> Result<(), Refusal> {
    let mut seen: Vec<&str> = Vec::new();
    for part in value.split(';') {
        let (name, v) = part
            .split_once('=')
            .ok_or_else(|| refusal::text("RRULE part"))?;
        if v.is_empty() {
            return Err(refusal::text("RRULE part value"));
        }
        if seen.contains(&name) {
            return Err(refusal::text("repeated RRULE part"));
        }
        seen.push(name);
        let list = |v: &str| v.split(',').map(text_int).collect::<Result<Vec<i64>, _>>();
        match name {
            "FREQ" => {
                d.freq = Some(match Freq::from_str(v) {
                    Some(f) => f,
                    None if v == "SECONDLY" => return Err(refusal::unsupported("FREQ=SECONDLY")),
                    None => return Err(refusal::text("FREQ")),
                })
            }
            "INTERVAL" => d.interval = Some(text_int(v)?),
            "COUNT" => d.count = Some(text_int(v)?),
            "UNTIL" => d.until = Some(lex_basic(v, true).ok_or_else(|| refusal::text("UNTIL"))?),
            "WKST" => d.wkst = Some(Weekday::from_str(v).ok_or_else(|| refusal::text("WKST"))?),
            "BYDAY" => {
                d.by_day = v
                    .split(',')
                    .map(|item| {
                        let split = item
                            .len()
                            .checked_sub(2)
                            .ok_or_else(|| refusal::text("BYDAY"))?;
                        if !item.is_char_boundary(split) {
                            return Err(refusal::text("BYDAY"));
                        }
                        let (ord, day) = item.split_at(split);
                        let day = Weekday::from_str(day).ok_or_else(|| refusal::text("BYDAY"))?;
                        let ord = if ord.is_empty() {
                            None
                        } else {
                            Some(text_int(ord)?)
                        };
                        Ok((day, ord))
                    })
                    .collect::<Result<_, _>>()?
            }
            "BYMONTH" => d.by_month = list(v)?,
            "BYMONTHDAY" => d.by_month_day = list(v)?,
            "BYHOUR" => d.by_hour = list(v)?,
            "BYMINUTE" => d.by_minute = list(v)?,
            "BYSETPOS" => d.by_set_pos = list(v)?,
            n if UNSUPPORTED_RRULE_PARTS.contains(&n) => {
                return Err(refusal::unsupported("RRULE part"))
            }
            _ => return Err(refusal::unsupported("RRULE part")),
        }
    }
    if d.freq.is_none() {
        return Err(refusal::text("FREQ is required"));
    }
    Ok(())
}

#[derive(Default)]
struct TextDraft {
    dtstart: Option<([u32; 6], String)>,
    rrule_seen: bool,
    freq: Option<Freq>,
    interval: Option<i64>,
    count: Option<i64>,
    until: Option<[u32; 6]>,
    wkst: Option<Weekday>,
    by_day: Vec<(Weekday, Option<i64>)>,
    by_month: Vec<i64>,
    by_month_day: Vec<i64>,
    by_hour: Vec<i64>,
    by_minute: Vec<i64>,
    by_set_pos: Vec<i64>,
    rdate: Vec<(String, Vec<[u32; 6]>)>,
    exdate: Vec<(String, Vec<[u32; 6]>)>,
}

/// `NAME;TZID=zone:value` → (zone, value); the only parameter form v0 accepts.
fn tzid_param<'a>(params: &[&'a str]) -> Result<&'a str, Refusal> {
    match params {
        [p] => p
            .strip_prefix("TZID=")
            .filter(|z| !z.is_empty())
            .ok_or_else(|| refusal::text("parameter")),
        _ => Err(refusal::text("a single TZID parameter is required")),
    }
}

fn local_values(value: &str) -> Result<Vec<[u32; 6]>, Refusal> {
    value
        .split(',')
        .map(|v| lex_basic(v, false).ok_or_else(|| refusal::text("local date-time")))
        .collect()
}

impl TextDraft {
    fn parse(lines: &[&str]) -> Result<Self, Refusal> {
        let mut d = TextDraft::default();
        for line in lines {
            let (head, value) = line
                .split_once(':')
                .ok_or_else(|| refusal::text("content line"))?;
            if value.is_empty() {
                return Err(refusal::text("empty value"));
            }
            let mut head_parts = head.split(';');
            let name = head_parts.next().unwrap_or("");
            let params: Vec<&str> = head_parts.collect();
            match name {
                "RRULE" => {
                    if d.rrule_seen {
                        return Err(refusal::unsupported("more than one RRULE"));
                    }
                    if !params.is_empty() {
                        return Err(refusal::text("RRULE parameter"));
                    }
                    d.rrule_seen = true;
                    parse_rrule(value, &mut d)?;
                }
                "DTSTART" => {
                    if d.dtstart.is_some() {
                        return Err(refusal::text("repeated DTSTART"));
                    }
                    let zone = tzid_param(&params)?;
                    let v = lex_basic(value, false).ok_or_else(|| refusal::text("DTSTART"))?;
                    d.dtstart = Some((v, zone.to_owned()));
                }
                "RDATE" | "EXDATE" => {
                    let zone = tzid_param(&params)?.to_owned();
                    let values = local_values(value)?;
                    if name == "RDATE" {
                        d.rdate.push((zone, values));
                    } else {
                        d.exdate.push((zone, values));
                    }
                }
                _ => return Err(refusal::unsupported("property")),
            }
        }
        if d.dtstart.is_none() || !d.rrule_seen {
            return Err(refusal::text("DTSTART and RRULE are required"));
        }
        Ok(d)
    }

    /// Text decoding ends here; value bounds are judged by the shared schema
    /// checks, so an in-grammar out-of-range integer is `invalid_rule`.
    fn into_draft(self) -> Result<Draft, Refusal> {
        let (dtstart, tzid) = self.dtstart.expect("checked in parse");
        let flatten = |groups: Vec<(String, Vec<[u32; 6]>)>| -> Result<Vec<[u32; 6]>, Refusal> {
            let mut out = Vec::new();
            for (zone, values) in groups {
                if zone != tzid {
                    return Err(refusal::text("RDATE/EXDATE TZID differs from DTSTART"));
                }
                out.extend(values);
            }
            Ok(out)
        };
        let rdate = flatten(self.rdate)?;
        let exdate = flatten(self.exdate)?;

        let in_range = |v: i64, lo: i64, hi: i64, nonzero: bool, what: &'static str| {
            if (lo..=hi).contains(&v) && !(nonzero && v == 0) {
                Ok(v)
            } else {
                Err(refusal::rule(what))
            }
        };
        let list = |v: Vec<i64>, lo, hi, nonzero, what| -> Result<Vec<i64>, Refusal> {
            v.into_iter()
                .map(|n| in_range(n, lo, hi, nonzero, what))
                .collect()
        };
        if !tzid_lexically_valid(&tzid) {
            return Err(refusal::rule("tzid"));
        }
        if self.count.is_some() && self.until.is_some() {
            return Err(refusal::rule("count and until are exclusive"));
        }
        let draft = Draft {
            dtstart,
            tzid,
            freq: self.freq.expect("checked in parse_rrule"),
            interval: match self.interval {
                None => 1,
                Some(v) => in_range(v, 1, 1000, false, "INTERVAL")?,
            },
            count: self
                .count
                .map(|v| in_range(v, 1, 100_000, false, "COUNT"))
                .transpose()?,
            until: self.until,
            wkst: self.wkst.unwrap_or(Weekday::Mo),
            by_day: self
                .by_day
                .into_iter()
                .map(|(d, o)| {
                    Ok((
                        d,
                        o.map(|v| in_range(v, -53, 53, true, "BYDAY ordinal"))
                            .transpose()?,
                    ))
                })
                .collect::<Result<_, Refusal>>()?,
            by_month: list(self.by_month, 1, 12, false, "BYMONTH")?,
            by_month_day: list(self.by_month_day, -31, 31, true, "BYMONTHDAY")?,
            by_hour: list(self.by_hour, 0, 23, false, "BYHOUR")?,
            by_minute: list(self.by_minute, 0, 59, false, "BYMINUTE")?,
            by_set_pos: list(self.by_set_pos, -366, 366, true, "BYSETPOS")?,
            rdate,
            exdate,
        };
        draft.check_combinations()?;
        Ok(draft)
    }
}

// --- public surface -----------------------------------------------------------------

impl RecurrenceRule {
    /// Decodes and validates a rule from its JSON form (`recurrence-rule.schema.json`).
    pub fn from_json(bytes: &[u8]) -> Result<Self, Refusal> {
        let value = json::parse(bytes)
            .ok_or_else(|| refusal::rule("malformed JSON or duplicate member"))?;
        Self::from_value(&value)
    }

    /// Decodes and validates a rule-document envelope (`rule-document.schema.json`).
    pub fn from_document_json(bytes: &[u8]) -> Result<Self, Refusal> {
        let value = json::parse(bytes)
            .ok_or_else(|| refusal::rule("malformed JSON or duplicate member"))?;
        let Value::Object(doc) = &value else {
            return Err(refusal::rule("document is not an object"));
        };
        if doc.len() != 2 {
            return Err(refusal::rule("document members"));
        }
        let (Some(Value::Array(caps)), Some(rule)) = (doc.get("capabilities"), doc.get("rule"))
        else {
            return Err(refusal::rule("document members"));
        };
        let mut names = Vec::with_capacity(caps.len());
        for cap in caps {
            match cap {
                Value::Str(s) if !s.is_empty() && !names.contains(&s) => names.push(s),
                _ => return Err(refusal::rule("capabilities")),
            }
        }
        if !names.iter().any(|s| s.as_str() == CONTRACT) {
            return Err(refusal::rule("capabilities"));
        }
        Self::from_value(rule)
    }

    /// Decodes and validates a rule from RFC 5545 content lines.
    pub fn from_text<S: AsRef<str>>(lines: &[S]) -> Result<Self, Refusal> {
        let lines: Vec<&str> = lines.iter().map(AsRef::as_ref).collect();
        let draft = TextDraft::parse(&lines)?.into_draft()?;
        Self::finish(draft)
    }

    pub(crate) fn from_value(value: &Value) -> Result<Self, Refusal> {
        Self::finish(Draft::from_value(value)?)
    }

    fn finish(draft: Draft) -> Result<Self, Refusal> {
        crate::validate::check_tzid(&draft.tzid)?;
        let rule = draft.into_rule()?;
        crate::validate::check_synchronized(&rule)?;
        Ok(rule)
    }

    pub fn dtstart(&self) -> CivilDateTime {
        self.dtstart
    }
    pub fn tzid(&self) -> &str {
        &self.tzid
    }
    pub fn freq(&self) -> Freq {
        self.freq
    }

    fn out(&self) -> Out {
        let mut m: Vec<(&'static str, Out)> = vec![
            ("dtstart", Out::Str(self.dtstart.to_extended())),
            ("tzid", Out::Str(self.tzid.clone())),
            ("freq", Out::Str(self.freq.as_str().to_owned())),
        ];
        if self.interval != 1 {
            m.push(("interval", Out::Int(i64::from(self.interval))));
        }
        if let Some(c) = self.count {
            m.push(("count", Out::Int(i64::from(c))));
        }
        if let Some(u) = self.until {
            m.push(("until", Out::Str(u.to_string())));
        }
        if self.wkst != Weekday::Mo {
            m.push(("wkst", Out::Str(self.wkst.as_str().to_owned())));
        }
        if !self.by_day.is_empty() {
            let items = self
                .by_day
                .iter()
                .map(|b| {
                    let mut o = vec![("day", Out::Str(b.day.as_str().to_owned()))];
                    if let Some(n) = b.ordinal {
                        o.push(("ordinal", Out::Int(i64::from(n))));
                    }
                    Out::Object(o)
                })
                .collect();
            m.push(("by_day", Out::Array(items)));
        }
        let ints = |v: &[i64]| Out::Array(v.iter().map(|&n| Out::Int(n)).collect());
        let lists: [(&'static str, Vec<i64>); 5] = [
            (
                "by_month",
                self.by_month.iter().map(|&n| i64::from(n)).collect(),
            ),
            (
                "by_month_day",
                self.by_month_day.iter().map(|&n| i64::from(n)).collect(),
            ),
            (
                "by_hour",
                self.by_hour.iter().map(|&n| i64::from(n)).collect(),
            ),
            (
                "by_minute",
                self.by_minute.iter().map(|&n| i64::from(n)).collect(),
            ),
            (
                "by_set_pos",
                self.by_set_pos.iter().map(|&n| i64::from(n)).collect(),
            ),
        ];
        for (k, v) in lists {
            if !v.is_empty() {
                m.push((k, ints(&v)));
            }
        }
        for (k, v) in [("rdate", &self.rdate), ("exdate", &self.exdate)] {
            if !v.is_empty() {
                m.push((
                    k,
                    Out::Array(v.iter().map(|c| Out::Str(c.to_extended())).collect()),
                ));
            }
        }
        Out::Object(m)
    }

    /// RFC 8785 (JCS) bytes of the normalized model, UTF-8, no trailing newline.
    pub fn to_canonical_json(&self) -> Vec<u8> {
        let mut s = String::new();
        json::write_jcs(&self.out(), &mut s);
        s.into_bytes()
    }

    /// `sha256:` + lowercase hex SHA-256 of the canonical bytes.
    pub fn digest(&self) -> String {
        let hash = Sha256::digest(self.to_canonical_json());
        let mut out = String::from("sha256:");
        for b in hash {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }

    /// Canonical RFC 5545 content lines (A5 order).
    pub fn to_text(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "DTSTART;TZID={}:{}",
            self.tzid,
            self.dtstart.to_basic()
        )];
        let mut parts = vec![format!("FREQ={}", self.freq.as_str())];
        if self.interval != 1 {
            parts.push(format!("INTERVAL={}", self.interval));
        }
        if let Some(c) = self.count {
            parts.push(format!("COUNT={c}"));
        }
        if let Some(u) = self.until {
            parts.push(format!("UNTIL={}", u.to_string().replace(['-', ':'], "")));
        }
        let join = |v: Vec<String>| v.join(",");
        if !self.by_month.is_empty() {
            parts.push(format!(
                "BYMONTH={}",
                join(self.by_month.iter().map(u8::to_string).collect())
            ));
        }
        if !self.by_month_day.is_empty() {
            parts.push(format!(
                "BYMONTHDAY={}",
                join(self.by_month_day.iter().map(i8::to_string).collect())
            ));
        }
        if !self.by_day.is_empty() {
            let days = self
                .by_day
                .iter()
                .map(|b| {
                    format!(
                        "{}{}",
                        b.ordinal.map(|n| n.to_string()).unwrap_or_default(),
                        b.day.as_str()
                    )
                })
                .collect();
            parts.push(format!("BYDAY={}", join(days)));
        }
        if !self.by_hour.is_empty() {
            parts.push(format!(
                "BYHOUR={}",
                join(self.by_hour.iter().map(u8::to_string).collect())
            ));
        }
        if !self.by_minute.is_empty() {
            parts.push(format!(
                "BYMINUTE={}",
                join(self.by_minute.iter().map(u8::to_string).collect())
            ));
        }
        if !self.by_set_pos.is_empty() {
            parts.push(format!(
                "BYSETPOS={}",
                join(self.by_set_pos.iter().map(i16::to_string).collect())
            ));
        }
        if self.wkst != Weekday::Mo {
            parts.push(format!("WKST={}", self.wkst.as_str()));
        }
        lines.push(format!("RRULE:{}", parts.join(";")));
        for (name, values) in [("RDATE", &self.rdate), ("EXDATE", &self.exdate)] {
            if !values.is_empty() {
                let v: Vec<String> = values.iter().map(|c| c.to_basic()).collect();
                lines.push(format!("{name};TZID={}:{}", self.tzid, v.join(",")));
            }
        }
        lines
    }
}
