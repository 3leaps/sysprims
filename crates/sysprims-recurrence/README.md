# sysprims-recurrence

RFC 5545 recurrence rules under `contract: recurrence/v0`: a typed rule model,
canonical JSON (RFC 8785) and digest, and canonical RFC 5545 text.

Time zones come only from a tzdb bundle compiled into the crate
(`tzdb_version()`, currently `2026e`). Host time-zone data is never consulted.

```rust
use sysprims_recurrence::RecurrenceRule;

let rule = RecurrenceRule::from_text(&[
    "DTSTART;TZID=America/New_York:20260305T093000",
    "RRULE:FREQ=MONTHLY;BYDAY=1TH;COUNT=3",
])?;
println!("{}", String::from_utf8(rule.to_canonical_json())?);
println!("{}", rule.digest());
# Ok::<(), Box<dyn std::error::Error>>(())
```

See [docs/recurrence.md](https://github.com/3leaps/sysprims/blob/main/docs/recurrence.md)
for semantics, refusal reasons, and the conformance pair.
