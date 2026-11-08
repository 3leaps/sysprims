# Report admission — experimental profile

This profile defines a working wire boundary for `identity-evidence/v0`. It is
subject to field iteration; it does not qualify an evidence provider or freeze v0.
`profile.json` records the limits and the pinned canonicalization dependency.

## Size and parser domain

Received report bytes, including whitespace, are limited to **262,144 bytes**.
Enforce that limit before parsing or unbounded allocation. A streaming reader may
read at most one additional detecting byte; an input beyond the limit refuses with
`report-too-large`. Canonical output is independently limited to 262,144 bytes;
canonical expansion beyond it refuses with `canonical-report-too-large`.

Array/object nesting is limited to 16, with the outermost container at depth 1;
a scalar is depth 0. Brackets inside strings do not count. This is distinct from
the maximum provenance dependency depth of 4. The selected root report is at
provenance depth 0; following one `input_reports` edge increases that depth by 1.
Cycles refuse independently of the depth budget. Arrays still have their schema
limits. Those maxima do not promise that every maximal combination fits a report.

Use the `jcs/rfc8785` input domain and canonical bytes defined by the pinned
[canonical JSON specification](https://github.com/3leaps/canonprims/blob/v0.1.1/docs/spec/jcs-rfc8785.md).
The profile pins its exact specification bytes and corpus identity. This is a
format dependency, not a requirement to call a particular implementation.
Duplicate decoded member names, invalid UTF-8, lone surrogates, noncharacters,
invalid JSON and numbers outside that domain refuse. No Unicode normalization or
silent repairs occur. Number spelling/equivalence follows the pinned domain;
booleans remain distinct from numbers. Native counters and exact quantities that
do not fit the accepted numeric domain use profile-defined strings.

The shared `vectors/wire-admission.json` cases have hand-authored expected outcomes.
They cover input/canonical byte caps, depth, Unicode, number and duplicate-member
boundaries. The independent Python witness checks this limited corpus; it is not
a general implementation of every numeric rule of the pinned dependency.

## Other work budgets

The working profile also records 256 KiB per evidence item, 2 MiB aggregate
retrieved evidence, 32 retrieval attempts and a 2,000 ms verification deadline.
The aggregate/count/deadline are shared across recursive work, not reset for each
report or retry. Each resolver invocation consumes a retrieval attempt, including
failed attempts; every returned byte counts toward its applicable byte budgets.
Cached results still count toward the logical evidence-byte budget once per unique
content digest in an evaluation. Retried transfer bytes additionally count against
an implementation's transport budget. No unconfigured network fetch is allowed.

These resolver/dependency/deadline choices still require dedicated enforcement and
cancellation vectors. The initial wire corpus does not exercise a recursive
resolver, provider, clock service or verification deadline. An implementation must
report which layer it exercised; successful parsing cannot imply those budgets
were enforced. Blocking I/O needs a caller-owned deadline/cancellation boundary.

## Time attachment

A report, evidence record, assessment or capability record may carry one optional
`time_attribution` object. It reuses the timestamp pattern of durable-record v1:
`instant`, `source: producer_clock | trusted_time`, and `evidence` required for
`trusted_time`. With `producer_clock`, evidence is forbidden. Missing attribution
means no additional source assertion; it does not imply trusted time.

| Owner      | Exact event to which `time_attribution.instant` must be equal |
| ---------- | ------------------------------------------------------------- |
| Report     | `created_at`                                                  |
| Evidence   | `collected_at`                                                |
| Assessment | `assessed_at`                                                 |
| Capability | `observed_at`                                                 |

The two strings must match exactly. Every instant remains whole-second UTC; no
implicit conversion from durable-record's microsecond type occurs. Attribution
never attaches to `valid_until` or `expires_at`, which remain policy deadlines.
Imported observations preserve the entire attribution with their original record.

Trusted-time evidence requires `{format_profile: {id, revision}, digest}`, with an
optional opaque `ref`. It is an assertion until an accepted verifier authenticates
the evidence, authority and exact event/byte target. A reference is not a URL to
fetch automatically. Historical evidence does not establish current verifier time
or prove the described event occurred at the asserted time. A source assertion
never upgrades other timestamps, identity dimensions or permissions.

A time provider profile must declare whether it establishes not-before, existed-by,
or both bounds, its target/preimage and authority rules. A timestamp over the whole
report lives detached to avoid a digest cycle. Pre-incorporation beacons or receipts
over separate evidence may be inline only with an acyclic, explicit target binding.

Preserve the original receipt bytes and precision. A provider projecting to whole
seconds uses floor-to-second for the displayed instant, and must include the
resulting subsecond remainder (less than one second) in the verified uncertainty
interval. It cannot narrow that interval by discarding precision. For exclusive
expiry require the entire verified current-time interval to precede expiry:
`now_max < expiry`. Future issue-skew uses `issued_max - now_min <= max_future_skew`;
maximum age uses `now_max - observed_min <= max_age`. A boundary-straddling interval
refuses the required predicate. Provider authentication, target binding and interval
construction remain unimplemented by the wire controls.

External time is optional. Ordinary producer-clock operation remains available.
An unavailable optional provider leaves unrelated observations useful; a consumer
requiring verified time refuses that requirement instead of substituting local time.
The synthetic mixed-source example demonstrates only the attachment representation.
