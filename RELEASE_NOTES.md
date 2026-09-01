# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

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

---

## v0.2.2 - 2026-08-30

**Status:** Prepared PTY Containment

v0.2.2 adds prepared Windows Job acquisition and independent boundary-strength
evidence to the core containment contract. The separately versioned
[`sysprims-pty`](https://github.com/3leaps/sysprims-pty) companion composes the
core Unix and Windows acquisition seams with PTY-owned process creation.

### Highlights

- **Prepared Windows Job**: create a non-breakaway Job before spawn, consume it
  in one exact suspended-process assignment, and seal the verified membership
  into an opaque receipt.
- **Receipt-bound guard**: verify the exact process handle, PID, and Job
  membership again before transferring sole Job and wait/reap authority into
  `ContainmentGuard`.
- **Independent boundary evidence**: report `kernel_enforced_job`,
  `cooperative_group`, or `unknown` without upgrading acquisition reliability,
  completion, or leader-reap evidence.
- **Unix identity hardening**: preserve only the exact exited-but-unreaped
  leader transition during validation and group signaling; fail closed on live
  identity changes or lost reap ownership.
- **Deterministic receipt handoff**: wait within a fixed parent-side bound for
  the complete child pre-exec acknowledgement while continuing to reject
  absent, partial, duplicate, or invalid packets.
- **Coherent version pack**: Rust, TypeScript root/lockfile, and authored npm
  platform manifests move together at `0.2.2`, with pre-tag guards for stale
  coordinates.

### Release Coordinates

- Rust workspace: repository tag `v0.2.2`.
- Go module: path-prefixed tag `bindings/go/sysprims/v0.2.2` on the same
  commit.
- TypeScript: `@3leaps/sysprims@0.2.2`, published from the verified core tag.
- PTY adapter: its own `sysprims-pty` release tag after the core source pin is
  retargeted; it is not included in `v0.2.2`.

### Upgrade Notes

- `TreeKillReliability::Guaranteed` continues to mean proven acquisition and
  retained group-signaling eligibility. Unix groups remain cooperative.
- `kernel_enforced_job` means the exact Windows child entered the immediate Job
  before first execution with neither breakaway mode enabled. It is not a
  sandbox or trust claim.
- Managed owned-containment APIs remain Rust-only. The C FFI, Go, and
  TypeScript bindings do not add that lifecycle in v0.2.2.
- The standard-library Windows `spawn_contained(Command)` path still fails
  closed because it does not own the create-suspended primary-thread seam.

---

## v0.2.1 - 2026-08-27

**Status:** Spawn-Time Containment Acquisition

v0.2.1 adds receipt-bound spawn-time session and process-group acquisition for
owned Rust commands and PTY adapters. The retained proof closes the
spawn-to-containment race while preserving an honest boundary: `guaranteed`
means acquisition and group-signaling eligibility, not OS-enforced descendant
non-escape.

### Highlights

- **Safe owned spawn**: `spawn_contained(Command)` installs the acquisition
  hook, spawns once, validates the child acknowledgement, and returns an owned
  guard.
- **PTY adapter seam**: `prepare_session_acquisition` supplies the replacement
  `setsid` hook and pending proof without taking PTY descriptors or terminal
  ownership.
- **Explicit unsafe boundary**: generic external-child receipt consumption is
  `unsafe` and requires the same child plus exclusive unreaped ownership.
- **One-shot proof**: fixed-size child acknowledgement and non-cloneable tokens
  reject missing, partial, duplicated, and mismatched acquisition.
- **Signal identity**: receipt-bound guards verify child identity, session,
  process group, and unreaped ownership before group signals.
- **Leader-exit handling**: final group escalation remains valid when the exact
  leader has exited but is still unreaped, preventing PID and PGID reuse while
  surviving descendants are signaled.
- **Hostile boundary tests**: privileged container tests exercise descendants
  that intentionally leave the cooperative process group.

### Upgrade Notes

- These new APIs are Rust-only. The FFI, Go, and TypeScript public surfaces are
  unchanged in v0.2.1.
- PTY integrations must install the sysprims acquisition hook instead of their
  own `setsid`, then retain exclusive unreaped ownership through guard
  finalization.
- `TreeKillReliability::Guaranteed` does not promise that a Unix descendant
  cannot create or join a different session or process group.
- PID-only termination remains `best_effort`; post-spawn owned adoption remains
  `unproven`.
- Guaranteed Windows spawn remains unsupported until create-suspended Job
  assignment is available.
- No portable-PTY companion crate or downstream consumer integration ships in
  this release.

_Older releases are archived in `docs/releases/`._
