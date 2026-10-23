# sysprims-recurrence

RFC 5545 recurrence rules under `contract: recurrence/v0`: a typed rule model,
canonical JSON (RFC 8785) and digest, canonical RFC 5545 text, and expansion
into UTC instants.

Time zones come only from a tzdb bundle compiled into the crate
(`tzdb_version()`, currently `2026e`). Host time-zone data is never consulted.

```rust
use std::num::NonZeroU32;
use sysprims_recurrence::{Instant, RecurrenceRule};

let rule = RecurrenceRule::from_text(&[
    "DTSTART;TZID=America/New_York:20260305T093000",
    "RRULE:FREQ=MONTHLY;BYDAY=1TH;COUNT=3",
])?;
let from = Instant::parse("2026-01-01T00:00:00Z").unwrap();
let to = Instant::parse("2027-01-01T00:00:00Z").unwrap();
for at in rule.expand(from, to, NonZeroU32::new(10).unwrap())? {
    println!("{at}");
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

See [docs/recurrence.md](https://github.com/3leaps/sysprims/blob/main/docs/recurrence.md)
for semantics, refusal reasons, and the conformance pair.
