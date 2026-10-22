# Recurrence rule schema family v0

**Proposed.** Capability: `contract: recurrence/v0`. The normative
[Recurrence Rule Contract](../../../docs/standards/recurrence-rule-contract.md)
defines the supported subset, expansion semantics, normalization and
conformance claims.

| Artifact | Purpose |
| --- | --- |
| `rule-document.schema.json` | Entry: standalone envelope `{capabilities, rule}` for exchanging one rule |
| `recurrence-rule.schema.json` | Canonical JSON form of one rule with `rdate` and `exdate`; embedding contracts reference this |
| `examples/` | Synthetic rules in normal form |
| `documents/` | One envelope example and one envelope reject (wrong capability) |
| `corpus/` | Conformance corpus: accept vectors, reject vectors, tzdb evidence and manifest |

## Conformance corpus

| Field | Value |
| --- | --- |
| Corpus digest | `sha256:69387023c21ca147f2dd59c4e47f48447f5eb3ad77121975aab0e4fe0680b35b` |
| tzdb version | `2026e` |
| tzdb artifact | `tzdata2026e.tar.gz`, `sha256:b26882805f26aac59d5b222978e6580484b834ccdc98be89df2f05a6dc53a652` |

The corpus digest is the SHA-256 of `corpus/MANIFEST.json`, which pins every
other corpus file. Conformance is claimed per (corpus digest, tzdb version).

- `corpus/vectors/` holds accept vectors: an input rule (JSON `rule`, or RFC 5545
  lines in `input_text`), the expected canonical JSON, digest and text form,
  half-open `expand` queries with a limit, and `next_after` results.
- `corpus/rejects/` holds reject vectors named `<reason>--<name>.json`; the reason
  is one row of the standard's refusal-reason table.
- `corpus/tzdb-evidence.json` lists the accepted `tzid` names and, for each zone
  the vectors use, the offset transitions in the evidence window with the
  source lines in the 2026e release.

Coverage includes DST gaps that consume no `COUNT` (`America/New_York`,
`Australia/Lord_Howe`), overlaps that resolve to the first instant, an explicit
time in a gap (including the UTC lower bound it sets), `BYSETPOS` after gap
removal, a gap `DTSTART` as a `BYSETPOS` seed, an explicit gap `EXDATE` matched
by resolved instant, `UNTIL` before `DTSTART`, an `RDATE` before `DTSTART`,
normalization before `DTSTART` synchronization, empty lists, an unreachable
ordinal beside a plain weekday, month-end and leap-day skipping, negative `BYMONTHDAY` and
`BYSETPOS`, `EXDATE` over `RDATE`, inclusive `UNTIL` against a half-open
query, `WKST`, sub-daily frequencies, the 2026e change for `America/Winnipeg`,
a zone without DST (`Asia/Tokyo`), and normalization equivalence.

## Example digests

Each example is in normal form. Its digest is the SHA-256 of its JCS bytes.

| Example | Canonical digest |
| --- | --- |
| `business-days.json` | `sha256:06e46da0d21cc3f44b175428f121c382529f8a4c1c68471633b870d874ea0d64` |
| `first-thursday-monthly.json` | `sha256:6404aa91a0ad7b1a9839b4e972ec6c7a03e60edfd68198567c015037e100b024` |
| `five-minute-cadence.json` | `sha256:310cef1a77a612f02ddccbf30dbf43d8242dc2539f02e17f16778cb88c3ca187` |
| `last-weekday-quarterly.json` | `sha256:88dfb15cd27aa3cac83dfa59d87539dcd75705b014b6bbab62cfc52f3cb554b9` |

## Controls

`python3 scripts/test-recurrence-controls.py` validates the examples and the
corpus. The script is an independent check, not a generator. It re-derives each
expected value from the standard and the tzdb evidence and fails on any
disagreement. It also checks the manifest pins, keeps the reject reasons in step
with the standard's table, and runs fixed mutations that must make named
witnesses disagree. It does not read host time-zone data and qualifies no
implementation.
