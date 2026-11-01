# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

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

---

## v0.2.4 - 2026-09-19

**Status:** Managed Containment Bindings

v0.2.4 adds managed contained-process handles for Go and TypeScript. Each
handle owns the native child lifecycle and exposes identity, polling, bounded
waiting, termination, and deterministic close operations.

### Highlights

- **Managed Go lifecycle**: `SpawnContained` returns a `ContainedProcess` whose
  `Identity`, `Poll`, `Wait`, `Terminate`, and `Close` methods operate through
  an opaque native-owned capability.
- **Managed TypeScript lifecycle**: `spawnContained` returns a disposable
  `ContainedProcess` with async `identity`, `poll`, `wait`, `terminate`, and
  `close` methods.
- **Native execution deadlines**: optional deadlines are enforced by the native
  owner without language-side polling. Cleanup retries preserve the deadline
  until a terminal transition is confirmed.
- **Explicit completion evidence**: natural completion remains distinct from
  deadline enforcement, acquisition reliability, and boundary strength.
- **Fail-closed lifecycle cleanup**: active handles retain their capability
  until cleanup confirms that the child has been reaped, allowing failed close
  attempts to be retried safely.
- **Release integrity checks**: machine-readable surface plans and guards bind
  release provenance, tag coordinates, generated bindings, downloaded assets,
  checksums, signatures, remote draft contents, and publication state.

### Release Coordinates

- Rust crates: `sysprims-core`, `sysprims-signal`, `sysprims-session`,
  `sysprims-proc`, and `sysprims-timeout` at `0.2.4`.
- Rust workspace: repository tag `v0.2.4`.
- Go module: path-prefixed tag `bindings/go/sysprims/v0.2.4` on the same
  commit.
- TypeScript: `@3leaps/sysprims@0.2.4`, published from the verified core tag.

### Upgrade Notes

- Go callers should always arrange `Close`, including on earlier errors. A
  failed close keeps the handle active and may be retried.
- TypeScript callers should use `await using` or call `close()` in a `finally`
  block so native ownership is released deterministically.
- A bounded `Wait`/`wait` timeout reports the current active state and does not
  terminate the child. The execution deadline, when configured at spawn, is a
  separate native policy.
- Go `Wait` rejects negative durations. Positive sub-millisecond durations are
  bounded waits rather than infinite waits.

---

## v0.2.3 - 2026-09-01

**Status:** Cargo Publication Enablement

v0.2.3 enables crates.io publication for the Rust library surface while keeping
the command-line, C FFI, and TypeScript native implementation crates private to
the repository release workflow.

### Highlights

- **Rust library publication**: `sysprims-core`, `sysprims-signal`,
  `sysprims-session`, `sysprims-proc`, and `sysprims-timeout` opt in to Cargo
  publication with standalone crate README files and repository metadata.
- **Private implementation crates**: `sysprims-cli`, `sysprims-ffi`, and
  `sysprims-ts-napi` remain unpublished as Rust crates.
- **Versioned workspace dependencies**: internal Cargo path dependencies carry
  the same release version so published crates resolve through crates.io.
- **Release dry-run target**: `make release-check` validates the version pack
  and constructs Cargo package tarballs without publishing.
- **Release checklist update**: crates.io publication order and negative
  controls are documented for the first upload.
- **TypeScript CI install mode**: pull-request validation omits optional
  same-version platform packages until the npm release workflow publishes them.

### Release Coordinates

- Rust crates: `sysprims-core`, `sysprims-signal`, `sysprims-session`,
  `sysprims-proc`, and `sysprims-timeout` at `0.2.3`.
- Rust workspace: repository tag `v0.2.3`.
- Go module: path-prefixed tag `bindings/go/sysprims/v0.2.3` on the same
  commit.
- TypeScript: `@3leaps/sysprims@0.2.3`, published from the verified core tag.

### Upgrade Notes

- Rust consumers can switch the five public library crates from git-tag
  dependencies to crates.io versions after publication.
- The first crates.io upload must publish in dependency order:
  `sysprims-core`, then `sysprims-signal` and `sysprims-session`, then
  `sysprims-proc`, then `sysprims-timeout`.
- TypeScript platform packages are still produced by the tag-based npm
  workflows; they are not installed during pull-request validation before
  publication.

_Older releases are archived in `docs/releases/`._
