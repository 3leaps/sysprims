# Recurrence rules

`sysprims-recurrence` (Rust) and `go/recurrence` (pure Go) implement
[`contract: recurrence/v0`](https://github.com/3leaps/crucible/blob/v0.2.0/docs/standards/recurrence-rule-contract.md):
a frozen subset of RFC 5545 RRULE with one canonical JSON form, a digest, and
expansion semantics, as published in crucible v0.2.0. The contract is
normative; this page summarizes it.

## Supported subset

`FREQ` = `MINUTELY` | `HOURLY` | `DAILY` | `WEEKLY` | `MONTHLY` | `YEARLY`;
`INTERVAL`, `COUNT` or `UNTIL`, `BYDAY` (ordinals with `MONTHLY`/`YEARLY` only),
`BYMONTHDAY`, `BYMONTH`, `BYHOUR`, `BYMINUTE`, `BYSETPOS`, `WKST`, `DTSTART`
with an IANA `TZID`, `RDATE`, `EXDATE`. Gregorian calendar only.

`SECONDLY`, `BYSECOND`, `BYWEEKNO`, `BYYEARDAY`, `EXRULE` and multiple `RRULE`
lines are refused, never ignored.

## Time

- `DTSTART`, `RDATE` and `EXDATE` are local civil date-times (whole seconds, no
  offset) in the rule's `TZID`. `UNTIL` is an inclusive UTC instant.
- Results are UTC instants with zero fractional seconds.
- **Generated times in a DST gap are skipped** and consume no `COUNT`. A skipped
  time is a non-slot: it has no slot identity, and a scheduler must not record
  it as missed, late, or any other outcome. Expansion never returns it.
- **Explicit times in a gap** (`DTSTART`, `RDATE`, `EXDATE`) use the offset before
  the gap. **Overlaps** always resolve to the earlier instant.
- There is no configurable shift policy. A shift-forward behaviour, if one is
  ever added, would be a separately named policy that is part of the rule's
  identity and digest; it would never change the meaning of an existing rule.
- Time-zone names are exact. `Europe/Oslo` is accepted and stays distinct from
  `Europe/Berlin`; Link-only aliases such as `US/Eastern` and `UTC` are refused
  (use `Etc/UTC`).

Both implementations embed the same tzdb bundle (`tzif-bundle-2026e.bin`, built
by `scripts/recurrence-tzdata-bundle.sh` from the pinned IANA release and
compiled with `zic -b fat`). Neither reads `TZ`, `ZONEINFO` or system zoneinfo.

## Queries

`expand(from, to, limit)` returns occurrences in the half-open UTC range
`[from, to)`, earliest first, at most `limit`. Equal bounds give an empty
result; reversed bounds are an error. `next_after(t)` is strictly after `t`.

A query that exhausts its search budget fails with a search-limit error. That
error is never a claim that no later occurrence exists; an empty answer is only
returned when `COUNT`, `UNTIL` or the end of year 9999 proves it.

## Refusals

| Reason | Meaning |
| --- | --- |
| `invalid_rule` | The JSON model fails the schema, or a text integer is outside its bounds |
| `invalid_text` | RFC 5545 text is malformed, or `DTSTART` is floating or UTC |
| `unsupported_part` | Text uses a part or property outside the subset |
| `invalid_tzid` | The name is not in the pinned roster |
| `invalid_date` | An explicit date-time does not exist in the calendar |
| `dtstart_not_synchronized` | `DTSTART` is not selected by the rule's first period |

## Conformance

Both implementations pass every accept and reject vector of the recurrence/v0
corpus for the pair:

| Corpus digest | tzdb |
| --- | --- |
| `sha256:69387023c21ca147f2dd59c4e47f48447f5eb3ad77121975aab0e4fe0680b35b` | `2026e` |

The corpus is vendored under `crates/sysprims-recurrence/tests/recurrence-v0`
by `scripts/sync-recurrence-corpus.sh v0.2.0`, which verifies every file
against the manifest; `PIN` records the crucible tag and commit. CI also compares the
two implementations' canonical bytes, digests, text and every query result
byte for byte.
