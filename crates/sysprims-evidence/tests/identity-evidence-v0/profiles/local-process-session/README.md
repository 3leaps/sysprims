# Local process/session binding — contract-fit profile

**Status:** proposed software-only contract-fit profile, not a qualified provider
or interoperable cryptographic protocol. `profile.json` pins this specification
and `evidence.schema.json`; evidence records pin the exact profile artifact.
The algorithm/proof encoding choice remains open and blocks wire freeze.

## Purpose and authority boundary

Establish which observed process incarnation and execution context participated
in a particular launch/session/key binding. This profile does not establish that
an agent was appointed, that a launch was admitted, that an action is permitted,
or that hostile processes sharing an account cannot use the key. The consuming
service owns those decisions. An accepted local launch authority records launch
intent; its receipt is evidence of that event, not a general authorization token.

Read `evidence.example.json` with the complete report at
`../../examples/local-process-session-launch.json`. The report retains ordinary
software-key binding/custody and adds a process-specific `launch-bound-session`
property, assessed as locally observed. It does not promote isolation or platform
integrity. The session evidence and process observation refer to the same bundle.

## Exact bindings

| Payload             | Required meaning                                                                                                                                                                            |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `process`           | Report subject; scoped host/installation context; boot identity; PID namespace; PID; provider-qualified start identity at native precision; observed execution context                      |
| `execution_context` | Scoped native security principal and digests of namespaces, executable, arguments and security context under the accepted adapter profile                                                   |
| `session`           | Session subject, enrollment identifier reference and digest, exact public-key digest and representation-profile pin; report-local references alone cannot substitute for the pinned content |
| `launch`            | Collision-resistant receipt ID, accepted issuer, launch-intent digest, exact process/session digests and issue/expiry window                                                                |
| `challenge`         | Fresh verifier nonce, expected audience, receipt digest and issue/expiry window                                                                                                             |
| `proof`             | Exact session key reference, full transcript digest and digest of detached possession-proof bytes                                                                                           |

The process start identity is an opaque value under an accepted native profile,
not a report timestamp. The synthetic decimal start counter deliberately retains
more precision than whole seconds. PID equality without boot, namespace and start
identity is insufficient. A retained native handle may corroborate incarnation,
but it is owned by a live adapter and is never serialized as portable proof.

The launch-intent digest must cover the exact intended request and execution
configuration under the consuming service's accepted intent profile. Context
fingerprints use deterministic, length-delimited, domain-separated encodings;
provider profiles must specify fields/normalization. A pathname alone is not an
executable identity, and a file digest alone is not proof of running-image integrity.
Unsupported native observations remain unavailable/unknown rather than invented.

## Transcript and verification

Use RFC 8785 canonical JSON for payload-object digests and the transcript. The
possession-proof transcript is exactly:

```
{
  domain: profile.transcript_domain,
  profile: {id, revision},
  process_digest: SHA256(canonical process),
  session_digest: SHA256(canonical session),
  receipt_digest: SHA256(canonical launch),
  challenge: complete challenge object
}
```

This is field notation, not an alternative wire encoding. The profile's domain
and exact revision participate in the transcript. Receipt authentication covers
the complete launch object. The verifier must bind an accepted launch principal
to that exact receipt digest, then verify possession using the exact session key
and complete transcript. It checks the supplied proof bytes against their digest.
The cryptographic suite, key representation, proof container and accepted roots
must be pinned before production verification. No caller-provided `verified`
boolean can discharge these checks.

An accepted local evaluator receives observations through an authenticated local
adapter/launcher boundary and retains the relevant process identity/handle state.
A remote consumer needs an authenticated evaluator/report exchange covering the
complete report and evidence descriptors, plus its own accepted evidence policy.
Software names, source strings and local profile files do not authenticate that
exchange. The current examples demonstrate the binding fields, not either trust
boundary's implementation.

## Report descriptor and assessment bindings

Evaluators resolve each assessment's actual `evidence_refs` and `claim_refs`, then
select the accepted `format_profile`; fixture record IDs have no verifier meaning.
For this single-bundle fit slice, both the possession and process assessments must
cite matching bundle descriptors. A correct but uncited claim cannot satisfy a
selected claim, and a verified bundle cannot authenticate a relabeled descriptor.

- A possession descriptor uses `possession-proof` and exactly the bundle's session
  subject, identifier and key references. Its consuming assessment is `enrolled-key`
  identity binding for those same three references. The enrollment evidence remains
  separately verified; this bundle does not assert hardware attestation or custody.
- A process descriptor uses `os-observation` and exactly the process subject, with
  no identifier/key/boundary reference. Its consuming assessment uses this rule
  profile and `locally-observed` platform evidence for that process. Every selected
  claim must be covered by the cited descriptor and match this profile's
  `launch-bound-session` property, process subject and session value; no key property
  is implied. Additional claims need their own applicable profile and verification.
- Both descriptors pin this profile and the exact canonical bundle digest. Their
  methods cannot be relabeled to promote the result. This fit helper handles one
  bundle, not arbitrary multi-bundle aggregation.

The claim's `value` is a **report-local session subject reference**. Portable session
identity resides in the identifier descriptor and bundle `identifier_digest`; a
matching local name alone does not establish continuity across reports.

## Freshness, invalidation and replay

Candidate limits are explicit in `profile.json`: receipt lifetime at most 300 s,
challenge lifetime at most 60 s, future issue-clock skew at most 5 s, and a native
observation no more than 5 s old. These are reviewable initial bounds, not measured
service defaults. The evaluator's trusted clock and clock-failure behavior must
be established by the accepted verification profile.

Both issue windows must have positive duration. Expiry is exclusive: `now` must
be strictly before receipt and challenge expiry. Future-skew tolerance does not
extend expiry. An assessment cannot outlive either window or the accepted key/
enrollment authority; shorter consumer limits prevail. At use, `now` must also be
strictly before each accepted assessment's own `valid_until`, its state must be
`current`, and its `assessed_at` cannot be in the future. A fresh process observation
or an unexpired challenge does not refresh an expired assessment. Fresh evaluation
produces a new immutable report and assessments with traceable input provenance;
it preserves original observations and collection times.

Reobserve the incarnation and relevant context at admission and each required use
boundary, or use a qualified live mechanism that establishes those predicates.
Process exit, PID reuse, changed boot/namespace/start identity, changed execution
context, replacement/revocation of the session key or enrollment, withdrawn launch
intent, expiry and loss of required visibility invalidate use. A cached timestamp
or retained PID does not prove continuity. Monitoring failures yield an unavailable
result. Native adapter qualification must state what changes it can detect and
where a recheck/use race remains; this fit profile does not erase that race.

The admission verifier atomically claims each `(accepted issuer, receipt ID)` and
challenge for a single admission. Replay persistence is outside the report and
must survive the relevant restart/restore boundary. A used receipt is not reused
for later admissions; later operation checks use the admitted session and fresh
required evidence. Replay-store uncertainty refuses admission. Snapshot replay
checks in the fit controls do not implement atomicity or persistence.

Historical reports can remain in storage after invalidation. Restoring a checkpoint
never restores expired authority or clears replay state. Re-evaluation creates new
evidence/assessments and does not freshen the old collection time.

## Budgets and outcome distinctions

The working profile uses a 256 KiB received and canonical report cap, 256 KiB per
evidence item, 2 MiB total evidence, JSON nesting depth 16, provenance dependency
depth 4, 32 retrieval attempts and a 2 s verification deadline. See the
[report admission profile](../report-admission/README.md) for exact depth and byte
semantics. The wire corpus exercises report parsing limits; recursive evidence,
retrieval and deadline enforcement remain separate qualification work. Profile names
never start implicit fetches.

Keep unsupported profile/provider, unavailable evidence/visibility, invalid
schema/proof, changed incarnation/context, expired evidence, replay and budget
exhaustion distinct. None is silently replaced with a stronger or fresh label.
Partial useful observations may still be returned separately from refusal of the
specific process/session claim.

## Fit vectors and their limits

`evaluation-context.example.json` is **trusted test input** supplied outside the
untrusted evidence bundle. It substitutes for independently observed process state,
accepted launch configuration, cryptographic verifier results, clock and replay
store. Never accept this fixture context from the subject being assessed.

`cases.json` contains fifteen context cases, including issuer-scoped receipt replay,
independent challenge replay, and a positive identical-receipt-ID/different-issuer
case. `report-cases.json` contains sixteen report-mutation cases. Every mutated
report must pass schema and necessary-condition checks before its fit outcome is
checked. These cover unchanged-bundle key/process/enrollment misattribution, method/
format/digest substitution, actual selected claims, missing bundle evidence,
renamed local records, before/exact/after assessment expiry, stale state, and a new
assessment/report evaluating preserved observations from an expired prior report.

The controls also check profile/schema/specification pins and bind the report back
to the bundle. Snapshot replay checks use `(accepted issuer, receipt ID)` and the
verifier's issued nonce namespace; they do not implement atomic consumption.

The fixture hashing helper deliberately covers only ASCII, nonnumeric JSON.
Unicode, numeric-domain/canonicalization, duplicate-member and additional time/skew
vectors remain required for full twin conformance. Passing these cases does not
qualify a native adapter, cryptographic suite, authenticated transport, concurrent
replay store or process isolation boundary.
