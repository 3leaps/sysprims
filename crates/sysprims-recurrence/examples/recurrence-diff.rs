//! Differential driver: reads one JSON rule per stdin line and prints one
//! result line per rule (refusal reason, or canonical digest plus a fixed
//! set of queries). The Go module has a matching driver; CI diffs the
//! outputs. Errors print as stable codes so wording never differs.

use std::io::{self, BufRead, Write};
use std::num::NonZeroU32;

use sysprims_recurrence::{Instant, QueryError, RecurrenceRule};

fn code(e: &QueryError) -> &'static str {
    match e {
        QueryError::ReversedRange => "error:reversed_range",
        QueryError::SearchLimitExceeded => "error:search_limit",
        _ => "error:other",
    }
}

fn main() {
    let stdin = io::stdin();
    let mut out = io::BufWriter::new(io::stdout());
    let windows = [
        ("2026-01-01T00:00:00Z", "2027-01-01T00:00:00Z", 40),
        ("2026-03-01T00:00:00Z", "2026-04-15T00:00:00Z", 25),
        ("2026-10-01T00:00:00Z", "2026-11-15T00:00:00Z", 25),
    ];
    for line in stdin.lock().lines() {
        let line = line.expect("stdin");
        let rule = match RecurrenceRule::from_json(line.as_bytes()) {
            Ok(r) => r,
            Err(e) => {
                writeln!(out, "refused {}", e.reason()).unwrap();
                continue;
            }
        };
        let mut parts = vec![rule.digest(), rule.to_text().join("|")];
        for (from, to, limit) in windows {
            let got = rule.expand(
                Instant::parse(from).unwrap(),
                Instant::parse(to).unwrap(),
                NonZeroU32::new(limit).unwrap(),
            );
            parts.push(match got {
                Ok(v) => v
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
                Err(e) => code(&e).to_owned(),
            });
        }
        let next = rule.next_after(Instant::parse("2026-06-15T12:00:00Z").unwrap());
        parts.push(match next {
            Ok(Some(i)) => i.to_string(),
            Ok(None) => "none".into(),
            Err(e) => code(&e).to_owned(),
        });
        writeln!(out, "{}", parts.join(" ")).unwrap();
    }
}
