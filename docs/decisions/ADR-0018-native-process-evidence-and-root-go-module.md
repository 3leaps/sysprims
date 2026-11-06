# ADR-0018: Native Process Evidence Surfaces and Root Pure-Go Module

> **Status**: Accepted
> **Date**: 2026-11-05
> **Authors**: sysprims maintainers
> **Supersedes**: none
> **Superseded by**: none

## Context

sysprims needs two related capabilities for process incarnation and local
session evidence (`contract: identity-evidence/v0`):

1. **Portable wire, report, and observation evaluation**: RFC 8785 canonical
   JSON wire admission (`report-admission:v0`), report envelope validation,
   `local-process-session:v0` fit checks, and `native-process-observation:v0`
   incarnation/context comparison, live freshness admission, and session
   projection. This layer parses and evaluates untrusted bytes and structured
   evidence payloads with no OS process syscalls and no `unsafe` code.
2. **Live native process observation and private continuity handles**:
   per-platform kernel readers (`macos/aarch64` `sysctl` + `kqueue` `EVFILT_PROC`
   `NOTE_EXIT`; `linux/x86_64` descriptor-verified `/proc` + `pidfd_open`) that
   collect `native-process-observation:v0` snapshots and maintain non-serializable,
   read-only continuity handles without signal or kill capability.

Both capabilities are required in Rust and in pure Go (`CGO_ENABLED=0`), with
byte-identical canonicalization, digests, refusal codes, and comparison
outcomes across both languages.

Two existing architectural records constrain the layout:

- **ADR-0002 (Crate Structure)** organizes Rust functionality into focused
  workspace crates so consumers do not pull platform inspection or FFI code when
  they only need portable types and verification logic.
- **ADR-0017 (Recurrence Go Module and Release Tag Set)** placed
  `github.com/3leaps/sysprims/go/recurrence` in a subdirectory with companion tag
  `go/recurrence/vX.Y.Z` as an explicit, single-module exception, while keeping
  the cgo binding at `github.com/3leaps/sysprims/bindings/go/sysprims` (tagged
  `bindings/go/sysprims/vX.Y.Z`). ADR-0017 §Decision.5 states that any later
  Go module layout addition is a new decision rather than an implicit extension
  of recurrence's subdirectory exception.

At the same time, the repository root already declares
`module github.com/3leaps/sysprims` (`go.mod`) and is tagged `vX.Y.Z` on every
release, with zero root Go packages prior to this decision. Adding another
subdirectory module and a fourth signed release tag per cut would compound
release ceremony complexity without keeping pure-Go consumers any cleaner than
importing packages directly under `github.com/3leaps/sysprims/...`.

## Decision

### 1. Rust Workspace Split and Dependency Direction

1. Add the portable library crate `crates/sysprims-evidence` (`sysprims-evidence`,
   `publish = true`, listed in `config/release/publishable-crates.txt`).
   - `sysprims-evidence` sets `#![forbid(unsafe_code)]`, performs no OS process
     or procfs/sysctl syscalls, and depends on `canonprims-core = "=0.1.4"` and
     `canonprims-json = "=0.1.4"` for strict I-JSON / RFC 8785 (`jcs/rfc8785`)
     wire admission and canonical digests.
   - `sysprims-evidence` owns `report-admission:v0` wire admission,
     `identity-evidence/v0` report validation, `local-process-session:v0` fit
     evaluation, and `native-process-observation:v0` comparison, freshness
     admission, context-digest derivation helpers, and fail-closed session
     projection.
2. Host live native process observation and private read-only continuity handles
   in `crates/sysprims-proc::observation`.
   - Dependency direction is strictly **`sysprims-proc -> sysprims-evidence`**
     (never `sysprims-evidence -> sysprims-proc`), so downstream verifiers that
     evaluate imported reports or conformance vectors do not link platform
     process-inspection code or `libc`/`windows-sys` syscalls.
   - In `config/release/publishable-crates.txt`, `sysprims-evidence` precedes
     `sysprims-proc`.
   - Continuity handles (`ProcessContinuityHandle`) wrap
     private file descriptors (`kqueue` on macOS, `pidfd` on Linux), expose no
     raw descriptor getter (`AsRawFd`/`IntoRawFd` are intentionally absent) and
     no signal/terminate methods, and are neither `Serialize` nor `Deserialize`.
   - All PID inputs validate `1..=i32::MAX` (`ADR-0011`) before any kernel call.

### 2. Pure-Go Packages in the Root Module (`github.com/3leaps/sysprims`)

1. Host pure-Go (`CGO_ENABLED=0`) packages directly in the existing root Go
   module `github.com/3leaps/sysprims` (`go.mod` at repository root):
   - `github.com/3leaps/sysprims/evidence`: portable wire admission, report
     validation, `local-process-session:v0` fit checks, and
     `native-process-observation:v0` comparison, freshness admission, context
     derivations, and fail-closed session projection, backed by
     `github.com/3leaps/canonprims v0.1.4` (`canon`).
   - `github.com/3leaps/sysprims/procobserve`: native `CGO_ENABLED=0` collectors
     and private unexported continuity handles on `macos/aarch64` and
     `linux/x86_64` (using `golang.org/x/sys/unix` where needed and explicit
     `unsupported` outcomes on unsupported targets).
2. Reconcile with ADR-0012 and ADR-0017 without breaking existing consumers or
   expanding the release tag set:
   - The root module `github.com/3leaps/sysprims` resolves directly from the
     canonical release tag `vX.Y.Z` that every sysprims release already signs
     and pushes. Go module boundaries automatically exclude subdirectories that
     contain their own `go.mod` (`bindings/go/sysprims` and `go/recurrence`), so
     importing `github.com/3leaps/sysprims/evidence` or
     `github.com/3leaps/sysprims/procobserve` never pulls in cgo or prebuilt FFI
     archives.
   - `github.com/3leaps/sysprims/bindings/go/sysprims` keeps its module path and
     `bindings/go/sysprims/vX.Y.Z` tag (ADR-0012).
   - `github.com/3leaps/sysprims/go/recurrence` keeps its module path and
     `go/recurrence/vX.Y.Z` tag (ADR-0017).
   - `config/release/release-tags.txt` and `scripts/release-tag-set.mjs` remain
     unchanged at the three existing tag prefixes (`v`,
     `bindings/go/sysprims/v`, `go/recurrence/v`).
3. Root pure-Go packages are gated in CI and local checks with
   `GOWORK=off CGO_ENABLED=0 go vet ./...`,
   `GOWORK=off CGO_ENABLED=0 go test -count=1 ./...`, `-race` where supported,
   an out-of-tree module copy check, and a cross-language parity harness
   (`scripts/evidence-crosscheck.sh`) requiring byte-identical canonical JSON,
   digests, refusal codes, and evaluation results between `sysprims-evidence`
   and `github.com/3leaps/sysprims/evidence`.

## Consequences

### Positive

- Verifiers in Rust and Go can evaluate `identity-evidence/v0` wire inputs and
  `native-process-observation:v0` payloads with zero `unsafe` Rust, zero cgo,
  and zero OS process-inspection dependencies.
- Pure-Go consumers import `github.com/3leaps/sysprims/evidence@vX.Y.Z` and
  `github.com/3leaps/sysprims/procobserve@vX.Y.Z` using the canonical `vX.Y.Z`
  release tag, adding zero new companion tags to the release ceremony.
- Pinning `canonprims` (`0.1.4` / `v0.1.4`) in both ecosystems guarantees
  identical RFC 8785 / I-JSON refusal behavior rather than maintaining bespoke
  canonicalizers.

### Negative

- The repository now carries three Go `go.mod` files (root pure-Go module,
  `go/recurrence` pure-Go module, and `bindings/go/sysprims` cgo module); local
  `go.work` convenience must include all three while CI/release gates continue
  to enforce `GOWORK=off` standalone module resolution.
- `sysprims-evidence` adds a seventh crate to the ordered crates.io publication
  sequence in `config/release/publishable-crates.txt`.

### Neutral

- Existing imports of `github.com/3leaps/sysprims/bindings/go/sysprims` and
  `github.com/3leaps/sysprims/go/recurrence` are unaffected.
- `sysprims-ffi`, TypeScript N-API bindings, and Python bindings do not wrap
  native continuity handles in this slice; downstream Rust and Go consumers use
  the native library surfaces directly.

## Alternatives Considered

### Alternative 1: Put Pure-Go Evidence and Observation Under Subdirectory Modules (`go/evidence`, `go/procobserve`)

Rejected. Each subdirectory `go.mod` requires its own `go/<name>/vX.Y.Z` signed
annotated tag on every release, multiplying tag-ceremony and verification surface
when the root `github.com/3leaps/sysprims` module is already clean, pure-Go, and
addressed by `vX.Y.Z`.

### Alternative 2: Put Portable Evidence Evaluation Inside `sysprims-proc`

Rejected. `sysprims-proc` compiles platform `libc`/`windows-sys` process
collectors; keeping `sysprims-evidence` separate with `#![forbid(unsafe_code)]`
lets remote/offline report evaluators depend only on portable canonicalization
and schema/contract logic.

### Alternative 3: Expose Raw `pidfd` / `kqueue` Descriptors Across FFI

Rejected. Continuity handles exist solely to corroborate same-incarnation
liveness/exit across bracketed observations without granting signal delivery or
arbitrary descriptor operations. Keeping handles private to native Rust and Go
types preserves that authority boundary by construction.

## References

- [ADR-0002: Crate Structure](ADR-0002-crate-structure.md)
- [ADR-0006: Dependency Governance](ADR-0006-dependency-governance.md)
- [ADR-0011: PID Validation Safety](ADR-0011-pid-validation-safety.md)
- [ADR-0012: Language Bindings Distribution](ADR-0012-language-bindings-distribution.md)
- [ADR-0017: Recurrence Go Module and Release Tag Set](ADR-0017-recurrence-go-module-and-release-tags.md)
