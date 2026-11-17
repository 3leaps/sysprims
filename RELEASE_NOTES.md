# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

---

## v0.5.0 - 2026-11-16

**Status:** Pure-Go Twins, Four-Language Conformance Vectors, and cgo/FFI Retirement

v0.5.0 makes every sysprims Go package pure Go (`CGO_ENABLED=0`) under the root
module `github.com/3leaps/sysprims`. New `syserror`, `signal`, `session`,
`timeout`, and `proc` packages join the existing `evidence` and `procobserve`
packages, so Go consumers no longer need a C toolchain or prebuilt native
libraries. Rust, Go, Python, and TypeScript now share conformance vectors for
`signal`, `session`, `timeout`, and `proc`. CI checks that Go and Python produce
RFC 8785 (JCS) output identical to Rust for them, and the TypeScript tests
verify the same vectors. The cgo Go bindings module (`bindings/go/sysprims`)
and the C-ABI crate (`sysprims-ffi`) are retired.

### Highlights

- **OS-verified process handles (`ProcessHandle`, `SignalBinding`)** in Rust,
  Go, Python, and TypeScript: `pidfd` on Linux (with a checked fallback for
  kernels older than 5.3), `kqueue` `EVFILT_PROC` with start-time verification
  on macOS, and `OpenProcess`/`GetProcessTimes` on Windows, so a signal reaches
  the process that was opened rather than a reused PID.
- **Pure-Go `signal` and `syserror`**: the signal catalog, PID/PGID validation,
  name matching, and group dispatch, with structured errors shared by all
  pure-Go packages.
- **Pure-Go `session`**: `RunSetsid`, `SpawnDetached`, `RunNohup`,
  `SpawnInGroup`, `SpawnWithSessionReceipt`, and the process-group and session
  primitives.
- **Pure-Go `timeout`**: `RunWithTimeout`, `TerminateTree`, and
  `SpawnContained`/`ContainedProcess` with containment completion receipts
  (process groups on Unix, Job Objects on Windows).
- **Pure-Go `proc`**: snapshots and filtering, `GetProcess`, `WaitPID`,
  ancestor and descendant traversal, `KillDescendants`, guard steps, `ListFds`,
  `ListeningPorts`, and `ProcessByPort` on Linux, macOS, and Windows.
- **Four-language conformance vectors** under `testdata/conformance/`
  (`signal`, `session`, `timeout`, `proc`): Rust, Go, and Python output is
  diffed in CI on Linux, macOS, and Windows, and the TypeScript tests verify
  the same vectors.
- **Registry publication from the immutable release**: PyPI and npm Trusted
  Publishing (OIDC) run from the signed GitHub Release assets without
  rebuilding them.

### Release Coordinates

- Rust crates: `sysprims-core`, `sysprims-evidence`, `sysprims-signal`,
  `sysprims-session`, `sysprims-proc`, `sysprims-timeout`, and
  `sysprims-recurrence` at `0.5.0`.
- Rust workspace and root Go module (`github.com/3leaps/sysprims@v0.5.0`,
  covering `syserror`, `signal`, `session`, `timeout`, `proc`, `evidence`, and
  `procobserve`): repository tag `v0.5.0`.
- Go recurrence module: path-prefixed tag `go/recurrence/v0.5.0` on the same
  commit (`github.com/3leaps/sysprims/go/recurrence@v0.5.0`).
- Python: `sysprims==0.5.0` on PyPI, published via GitHub Actions Trusted
  Publishing with PEP 740 attestations from the verified core tag.
- TypeScript: `@3leaps/sysprims@0.5.0`, published from the verified core tag.

### Upgrade Notes

- **Go bindings module retired.** There is no
  `bindings/go/sysprims/v0.5.0` tag. Consumers of
  `github.com/3leaps/sysprims/bindings/go/sysprims` should move to the root
  module packages (`github.com/3leaps/sysprims/{signal,session,timeout,proc}`).
  The `v0.4.0` and earlier bindings tags remain resolvable for existing pins.
- The root module requires Go `1.26.0` or later. `go/recurrence` still targets
  `go 1.21`.
- **C-ABI retired.** `sysprims-ffi`, its generated `sysprims.h` header, and the
  prebuilt static and shared libraries are no longer built or shipped.
- `sysprims-cli`, `sysprims-ts-napi`, and `sysprims-py` remain
  `publish = false` on crates.io.

---

## v0.4.0 - 2026-11-08

**Status:** Native Process Observation, Python Bindings, and Release Ceremony Hardening

v0.4.0 adds `contract: identity-evidence/v0` process-session and native process
observation derivation in Rust (`sysprims-evidence`) and pure Go (`evidence`),
paired with live OS process observation and kernel continuity handles in Rust
(`sysprims-proc::observation`) and pure Go (`procobserve`). Rust and Go agree
byte-for-byte across the `identity-evidence/v0` conformance fixtures and
`native-process-observation:v0` derivation vectors. This release also ships
`cp310-abi3` Python bindings (`sysprims` on PyPI) with wheel and
source-distribution release packaging and PEP 740 attestation verification,
centralizes CI runner-label and Rust toolchain pins under `config/ci/pins.toml`,
switches Cargo publication checks to the `crates.io` sparse index, and stages
verified npm `.tgz` release tarballs.

### Highlights

- **Identity evidence derivation (`sysprims-evidence` and pure-Go `evidence`)**:
  wire admission, report construction, `local-process-session:v0`, and
  `native-process-observation:v0` evidence derivation with RFC 8785 canonical
  JSON (JCS), SHA-256 digests, and cross-language conformance parity.
- **Native process observation (`sysprims-proc::observation` and pure-Go `procobserve`)**:
  process snapshots, executable hashing, 64-bit Darwin `KERN_PROCARGS2` and
  Linux `/proc/<pid>` `openat2` (`RESOLVE_NO_XDEV | RESOLVE_NO_MAGICLINKS`)
  collectors, and `ProcessContinuityHandle` (`pidfd` on Linux, `kqueue`
  `EVFILT_PROC` on macOS) for race-checked exit waiting and re-verification.
- **Python bindings (`bindings/python`, PyPI `sysprims`)**: PyO3/maturin
  `cp310-abi3` extension covering process inspection, signals, session and
  group spawn, contained spawn (`spawn_contained`), timeout execution, and
  `recurrence/v0` evaluation, with typed `.pyi` stubs (`py.typed`), offline
  source-distribution verification, multi-platform wheels, and PyPI PEP 740
  attestation checks.
- **Centralized CI pins and sparse-index Cargo verification**: GitHub Actions
  runner labels and Rust toolchains (`msrv = "1.88.0"`, `stable = "1.98.1"`)
  validate against `config/ci/pins.toml` alongside `.github/actions/setup-rust`;
  Cargo release dry-run and registry integrity checks query `index.crates.io`
  directly via `scripts/release-index-entry.py` instead of the cached
  `crates.io` REST API.
- **Staged npm tarballs and cross-OS CI validation**: TypeScript release
  packaging stages verified `.tgz` archives with strict tar-header and manifest
  checks, and CI adds Linux, macOS, and Windows validation for `evidence` and
  `python-bindings` (including Windows `NOT_SUPPORTED` test parity).

### Release Coordinates

- Rust crates: `sysprims-core`, `sysprims-evidence`, `sysprims-signal`,
  `sysprims-session`, `sysprims-proc`, `sysprims-timeout`, and
  `sysprims-recurrence` at `0.4.0`.
- Rust workspace and root Go module (`github.com/3leaps/sysprims/evidence@v0.4.0`,
  `github.com/3leaps/sysprims/procobserve@v0.4.0`): repository tag `v0.4.0`.
- Go bindings module: path-prefixed tag `bindings/go/sysprims/v0.4.0` on the
  same commit (`github.com/3leaps/sysprims/bindings/go/sysprims@v0.4.0`).
- Go recurrence module: path-prefixed tag `go/recurrence/v0.4.0` on the same
  commit (`github.com/3leaps/sysprims/go/recurrence@v0.4.0`).
- Python: `sysprims==0.4.0` on PyPI, published via GitHub Actions Trusted
  Publishing with PEP 740 attestations from the verified core tag.
- TypeScript: `@3leaps/sysprims@0.4.0`, published from the verified core tag.

### Upgrade Notes

- `sysprims-evidence` (`0.4.0`) and pure-Go
  `github.com/3leaps/sysprims/evidence@v0.4.0` and
  `github.com/3leaps/sysprims/procobserve@v0.4.0` require no `cgo`.
- Python wheels (`cp310-abi3` for Linux `x86_64`/`aarch64` `manylinux_2_17`,
  macOS `arm64`/`x86_64`, and Windows `amd64`) plus `sysprims-0.4.0.tar.gz`
  are included in the signed GitHub Release inventory and published to PyPI
  with PEP 740 provenance attestations.
- Repository checkouts now include a root multi-module `go.work` (`go 1.26.0`).
  Standalone Go commands run inside sub-modules (`bindings/go/sysprims` or
  `go/recurrence`, both targeting `go 1.21`) under an older Go toolchain should
  set `GOWORK=off`, and containerized Go invocations mounted across mismatched
  host/container UIDs should set `GOFLAGS=-buildvcs=false`. Module consumers
  resolving tagged coordinates are unaffected.
- On Linux, `sysprims-proc::observation` and `procobserve` enforce
  `openat2(RESOLVE_NO_XDEV | RESOLVE_NO_MAGICLINKS)` across `/proc` and
  `/proc/sys/kernel`. In unprivileged OCI containers whose runtime bind-mounts
  `/proc/sys` read-only over `/proc`, observation calls fail closed with
  `procfs-confinement-violation`.

---

## v0.3.0 - 2026-11-02

**Status:** Recurrence and Release Integrity Hardening

v0.3.0 adds `contract: recurrence/v0` in Rust (`sysprims-recurrence`) and in
pure Go (`go/recurrence`). The two implementations agree on the pinned
conformance corpus and on generated rules. This release also hardens the
release ceremony with committed public signing-key pins and fingerprint
anchors, read-only CI artifact builds, immutable GitHub Release promotion,
Sigstore SLSA provenance verification for npm packages, and a Cargo
credential-provider publication gate.

### Highlights

- **Rust recurrence crate (`sysprims-recurrence`)**: RFC 5545 recurrence rules
  under `contract: recurrence/v0` with canonical JSON (RFC 8785), SHA-256
  digest, canonical RFC 5545 text, and expansion into UTC instants over an
  embedded `tzdb` `2026e` bundle.
- **Pure-Go recurrence module (`go/recurrence`)**: standard-library-only Go
  implementation (`github.com/3leaps/sysprims/go/recurrence`, no `cgo`) backed
  by the same embedded `tzdb` `2026e` bundle and verified byte-for-byte against
  the Rust implementation.
- **Committed release identity and signed tag ceremony**: public OpenPGP and
  minisign signing keys and `decernor` fingerprint anchors are committed in-tree
  and verified across the three coordinated release tags (`v0.3.0`,
  `bindings/go/sysprims/v0.3.0`, and `go/recurrence/v0.3.0`).
- **Read-only release workflows and immutable GitHub Releases**: CI release
  workflows run with read-only permissions and upload unsigned workflow
  artifacts; draft creation, dual signing, immutable release promotion, and
  post-publish verification run through maintainer-gated targets.
- **Cargo credential-provider publication gate**: `make release-crates-publish`
  exports the tagged commit to an isolated directory, verifies tag and release
  immutability, checks publishable crate sources, and requires per-crate
  interactive confirmation before releasing a `crates.io` credential.
- **Timeout CLI documentation and positioning**: clarifies GNU `timeout` 9.11
  and uutils group-by-default behavior versus `sysprims` differentiators
  (Windows Job Objects, typed reliability receipts, and MIT/Apache-2.0 library
  embedding) and adds `docs/guides/timeout-cli.md`.

### Release Coordinates

- Rust crates: `sysprims-core`, `sysprims-signal`, `sysprims-session`,
  `sysprims-proc`, `sysprims-timeout`, and `sysprims-recurrence` at `0.3.0`.
- Rust workspace: repository tag `v0.3.0`.
- Go bindings module: path-prefixed tag `bindings/go/sysprims/v0.3.0` on the
  same commit (`github.com/3leaps/sysprims/bindings/go/sysprims@v0.3.0`).
- Go recurrence module: path-prefixed tag `go/recurrence/v0.3.0` on the same
  commit (`github.com/3leaps/sysprims/go/recurrence@v0.3.0`).
- TypeScript: `@3leaps/sysprims@0.3.0`, published from the verified core tag.

### Upgrade Notes

- `sysprims-recurrence` (`0.3.0`) and
  `github.com/3leaps/sysprims/go/recurrence@v0.3.0` are available as standalone
  recurrence libraries with no host tzdata dependency.
- Release verification assets now include `expected-fingerprints.txt` and
  `expected-fingerprints.ndjson` alongside `SHA256SUMS`, `SHA512SUMS`, and the
  public signing keys.
