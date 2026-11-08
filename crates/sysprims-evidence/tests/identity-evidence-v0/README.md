# Identity evidence v0 — proposed

This draft family contains a strict report schema, synthetic examples and logical
controls. Read the [contract](../../../docs/standards/identity-evidence-contract.md)
and [classifier standard](../../../docs/standards/identity-assurance-classification.md).

The canonical classifier definitions live in
`config/classifiers/dimensions/{identity-binding,key-custody,execution-isolation,platform-evidence}.dimension.json`.
Schema `$defs` expose each vocabulary for reuse; parity is checked against those
definitions. Vendor definitions and schemas together at an immutable pin.

Run from the repository root:

```sh
python3 scripts/test-identity-evidence-controls.py
```

The script requires `goneat`. It checks classifier meta-schema validity,
vocabulary parity, positive report shapes, malformed/missing values, evidence
reference scope, necessary evidence categories and assessment windows. It also
checks that all optional capability outcomes remain representable and that stale
history is retained without presenting it as current.

All example identities, digests, times and evidence are synthetic. A record with
`attested-key` is a shape/logic example, not a verified hardware attestation.
These controls are not the future Rust/Go classification oracle. That corpus
must add provider profiles, evidence verification outcomes, fresh challenge and
replay cases, policy predicates and normalized expected results as explicit data.
Native adapter qualification is an additional, independent requirement.

## Proposed adoption checklist

- Approve vocabulary and subject/reference rules.
- Freeze derivation and evidence-verification profiles, including exact key
  representation, trust installation and freshness/revocation behavior.
- Author shared evaluation vectors independently of either implementation.
- Pin the accepted contract and corpus into both language implementations.
- Qualify claimed OS/architecture/provider combinations with native receipts.

No schema URL in this draft implies that the asset has been published.

## Field pass after direction review

Observation-only reports omit the evaluator and have no assessments. Evaluated
reports name evaluator software, a claimed principal and a pinned verification
profile. They are not authenticated by those fields alone.

Keys are separate from enrollment identifiers. Isolation assessments carry a
directed boundary and threats; platform assessments select exact property claims.
Input-report digests and per-record origins preserve collection provenance.

Start with these paired examples:

- `observation-only-installation.json` → `evaluated-installation.json`: new report,
  original evidence/capability times and descriptors retained.
- `software-key-rotation.json`: one enrollment identifier, two exact keys, historical
  old-key assessments and current new-key assessments.
- `attested-key-origin-property.json`: the key-origin claim is covered; no OS-version
  or runtime-integrity claim follows from it.
- `shared-guest-hardware-issuer.json`: explicit process comparison and external guest
  boundary; issuer custody does not transfer to the session.

The fixture hashing helper accepts only the ASCII, nonnumeric JSON subset used
in these examples. It is not a reusable general-purpose JCS implementation.
Profiles and public keys in all examples remain synthetic, with no verified proof.

## Evidence formats and process/session fit

Evidence records now require `format_profile` even without an assessment. Accepted
configuration and authenticated bindings select decoders/verifiers; a source string
never selects a verifier or starts a fetch. `not-applicable` custody forbids a key
reference; an existing key with unestablished custody uses `unknown`.

The [native process observation profile](profiles/native-process-observation/README.md)
and `examples/native-process-observation.json` define an observation-only provider
payload for OS process incarnation, seven-state fact visibility, Linux PID
numbering-namespace verification, start-identity sub-profiles, strict incarnation
comparison and fail-closed projection into `local-process-session:v0`.

The [local process/session profile](profiles/local-process-session/README.md) and
`examples/local-process-session-launch.json` demonstrate complete launch binding
with explicit synthetic verifier inputs and thirty-one context/report fit cases. The profile's exact
specification and payload schema are pinned by digest. A two-producer import
collision fixture lives under `fixtures/`. Human/account subjects remain deferred.

These profiles are contract-fit and provider-shape drafts; algorithms, accepted
roots, native adapters, authenticated exchange, budget/replay enforcement and full
twin vectors are not qualified by this corpus.

## Experimental wire pass

The [report admission profile](profiles/report-admission/README.md) defines the
working 256 KiB received/canonical cap, JSON depth 16 and per-event optional
`time_attribution`. `time-attribution-mixed.json` is a synthetic report whose
trusted-time receipt is deliberately unverified. Seventeen hand-authored wire
vectors and mutation checks exercise the parsing boundary. Passing them is not
provider, provenance, cryptographic or cross-language implementation qualification.
