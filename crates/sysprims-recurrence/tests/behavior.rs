//! Behaviour the corpus does not pin directly: query errors, limits, text
//! grammar edges, and ordering properties.

use std::num::NonZeroU32;

use sysprims_recurrence::{Instant, QueryError, RecurrenceRule, RefusalReason};

fn rule(json: &str) -> RecurrenceRule {
    RecurrenceRule::from_json(json.as_bytes()).unwrap()
}

fn at(s: &str) -> Instant {
    Instant::parse(s).unwrap()
}

fn lim(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}

#[test]
fn reversed_range_is_an_error_and_equal_bounds_are_empty() {
    let r = rule(r#"{"dtstart":"2026-01-05T09:00:00","tzid":"Etc/UTC","freq":"DAILY"}"#);
    let t = at("2026-01-06T00:00:00Z");
    assert_eq!(
        r.expand(t, at("2026-01-05T00:00:00Z"), lim(5)),
        Err(QueryError::ReversedRange)
    );
    assert_eq!(r.expand(t, t, lim(5)), Ok(vec![]));
}

#[test]
fn unbounded_rules_answer_far_queries_without_walking_from_the_start() {
    let r = rule(r#"{"dtstart":"2026-01-05T09:00:00","tzid":"Etc/UTC","freq":"MINUTELY"}"#);
    let got = r
        .expand(
            at("9000-01-01T00:00:00Z"),
            at("9000-01-01T00:03:00Z"),
            lim(10),
        )
        .unwrap();
    let got: Vec<String> = got.iter().map(ToString::to_string).collect();
    assert_eq!(
        got,
        [
            "9000-01-01T00:00:00Z",
            "9000-01-01T00:01:00Z",
            "9000-01-01T00:02:00Z"
        ]
    );
}

#[test]
fn the_calendar_end_is_proof_of_exhaustion_not_a_resource_error() {
    let r = rule(r#"{"dtstart":"9999-12-31T09:00:00","tzid":"Etc/UTC","freq":"YEARLY"}"#);
    assert_eq!(r.next_after(at("9999-12-31T09:00:00Z")), Ok(None));
}

#[test]
fn sparse_rules_stay_cheap_and_never_claim_false_exhaustion() {
    // A MINUTELY rule limited to one minute of February 29 has about two
    // million empty minutes between hits; day skipping keeps that cheap.
    let r = rule(
        r#"{"dtstart":"2024-02-29T09:00:00","tzid":"Etc/UTC","freq":"MINUTELY","by_month":[2],"by_month_day":[29],"by_hour":[9],"by_minute":[0]}"#,
    );
    assert_eq!(
        r.next_after(at("2024-02-29T09:00:00Z"))
            .unwrap()
            .unwrap()
            .to_string(),
        "2028-02-29T09:00:00Z"
    );
    // February 29 that is also a Monday recurs every 28 years at most, but a
    // DAILY rule walks one period per day: well inside the limit.
    let sparse = rule(
        r#"{"dtstart":"2016-02-29T09:00:00","tzid":"Etc/UTC","freq":"DAILY","by_month":[2],"by_month_day":[29],"by_day":[{"day":"MO"}]}"#,
    );
    assert_eq!(
        sparse
            .next_after(at("2016-02-29T09:00:00Z"))
            .unwrap()
            .unwrap()
            .to_string(),
        "2044-02-29T09:00:00Z"
    );
    // The MINUTELY form skips excluded days and hours, so it stays cheap too.
    let minutely = rule(
        r#"{"dtstart":"2016-02-29T09:00:00","tzid":"Etc/UTC","freq":"MINUTELY","interval":7,"by_month":[2],"by_month_day":[29],"by_day":[{"day":"MO"}],"by_hour":[9],"by_minute":[0]}"#,
    );
    assert_eq!(
        minutely
            .next_after(at("2016-02-29T09:00:00Z"))
            .unwrap()
            .unwrap()
            .to_string(),
        "2044-02-29T09:00:00Z"
    );
}

#[test]
fn text_integer_grammar() {
    let parse =
        |rrule: &str| RecurrenceRule::from_text(&["DTSTART;TZID=Etc/UTC:20260105T090000", rrule]);
    let reason = |rrule: &str| parse(rrule).unwrap_err().reason();
    assert!(parse("RRULE:FREQ=DAILY;COUNT=+5").is_ok());
    assert!(parse("RRULE:FREQ=DAILY;BYMINUTE=0").is_ok());
    assert_eq!(
        reason("RRULE:FREQ=DAILY;BYMINUTE=-0"),
        RefusalReason::InvalidText
    );
    assert_eq!(
        reason("RRULE:FREQ=DAILY;COUNT=05"),
        RefusalReason::InvalidText
    );
    assert_eq!(
        reason("RRULE:FREQ=DAILY;COUNT=1 "),
        RefusalReason::InvalidText
    );
    assert_eq!(
        reason("RRULE:FREQ=DAILY;COUNT=2;COUNT=2"),
        RefusalReason::InvalidText
    );
    assert_eq!(
        reason("RRULE:FREQ=DAILY;BYDAY=MOX"),
        RefusalReason::InvalidText
    );
    assert_eq!(
        reason("RRULE:FREQ=MONTHLY;BYDAY=é"),
        RefusalReason::InvalidText
    );
    assert_eq!(
        reason("RRULE:FREQ=DAILY;BYYEARDAY=1"),
        RefusalReason::UnsupportedPart
    );
    assert_eq!(
        reason("RRULE:FREQ=SECONDLY"),
        RefusalReason::UnsupportedPart
    );
}

#[test]
fn json_strictness() {
    let reason = |s: &str| {
        RecurrenceRule::from_json(s.as_bytes())
            .unwrap_err()
            .reason()
    };
    let base = r#""dtstart":"2026-01-05T09:00:00","tzid":"Etc/UTC","freq":"DAILY""#;
    assert_eq!(
        reason(&format!("{{{base},\"freq\":\"DAILY\"}}")),
        RefusalReason::InvalidRule
    );
    assert_eq!(
        reason(&format!("{{{base},\"count\":2.5}}")),
        RefusalReason::InvalidRule
    );
    assert_eq!(
        reason(&format!(
            "{{{base},\"by_day\":[{{\"day\":\"MO\",\"x\":1}}]}}"
        )),
        RefusalReason::InvalidRule
    );
    assert_eq!(
        reason(&format!("{{{base},\"count\":null}}")),
        RefusalReason::InvalidRule
    );
    // JSON Schema integers include 2.0.
    assert!(RecurrenceRule::from_json(format!("{{{base},\"count\":2.0}}").as_bytes()).is_ok());
}

#[test]
fn expansion_is_strictly_increasing_and_consistent_with_next_after() {
    let rules = [
        r#"{"dtstart":"2026-03-07T20:00:00","tzid":"America/New_York","freq":"HOURLY","interval":5,"by_minute":[0,30]}"#,
        r#"{"dtstart":"2026-01-02T17:00:00","tzid":"Europe/Berlin","freq":"MONTHLY","by_day":[{"day":"MO"},{"day":"FR"}],"by_set_pos":[-1,1]}"#,
        r#"{"dtstart":"2026-04-03T01:45:00","tzid":"Australia/Lord_Howe","freq":"DAILY","by_hour":[1,2],"by_minute":[15,45]}"#,
        r#"{"dtstart":"2026-02-09T09:00:00","tzid":"Etc/UTC","freq":"YEARLY","by_day":[{"day":"MO","ordinal":6},{"day":"SU","ordinal":-1}],"by_set_pos":[1]}"#,
    ];
    for json in rules {
        let r = rule(json);
        let (from, to) = (at("2026-01-01T00:00:00Z"), at("2028-01-01T00:00:00Z"));
        let all = r.expand(from, to, lim(500)).unwrap();
        assert!(all.windows(2).all(|w| w[0] < w[1]), "{json}");
        let mut cursor = Instant::from_unix_seconds(from.unix_seconds() - 1).unwrap();
        for want in &all {
            let got = r.next_after(cursor).unwrap().unwrap();
            assert_eq!(&got, want, "{json}");
            cursor = got;
        }
        // Narrowing the query never changes which instants are selected.
        if all.len() > 4 {
            let mid = r.expand(all[2], all[4], lim(500)).unwrap();
            assert_eq!(mid, &all[2..4], "{json}");
        }
    }
}
