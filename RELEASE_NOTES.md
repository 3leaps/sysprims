# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

---

## v0.2.1 - Pending

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

---

## v0.2.0 - Pending

**Status:** Process Containment Release

v0.2.0 adds an owned process-containment lifecycle for Rust PTY and supervisor
integrations, structured point-in-time cleanup evidence, and additive
TypeScript parity and numeric-safety enforcement. It also removes PID-only
Windows Job ownership.

### Highlights

- **Owned containment**: retain the child and process-group or Job capability
  through normal completion, forced termination, and active-guard cleanup.
- **Reliability reporting**: distinguish `guaranteed`, `unproven`, and
  `best_effort` tree cleanup.
- **Identity safety**: bind cleanup to captured PID, start time, executable,
  process group, and session evidence.
- **Completion evidence**: report point-in-time `Empty`, `Survivors`, or
  `Unknown` membership with platform provenance after owned-containment cleanup.
- **TypeScript parity**: opt into descendant environment and thread details,
  with default-off behavior and permission-aware visibility.
- **TypeScript safety**: reject invalid PID, process-group, signal, depth,
  duration, port, and filter values before coercion or native loading, then
  revalidate them at the N-API boundary.
- **Projection drift enforcement**: generate and check the public TypeScript
  declaration shape, N-API inventory, C comparison surface, and API reference.
- **Runtime matrix**: exercise native behavior under Node.js 18, Node.js 22,
  and Bun 1.3.3 on Linux x64, macOS arm64, and Windows x64.
- **Schema update**: timeout result schema v1.1 includes `unproven` reliability.

### Upgrade Notes

- Rust consumers exhaustively matching `TreeKillReliability` must add the
  `Unproven` variant.
- Windows `spawn_in_group` now returns `NotSupported`; use an owned guard
  integration. Guaranteed Windows spawn remains unavailable until
  create-suspended Job assignment is implemented.
- Dropping an active `ContainmentGuard` terminates its contained processes.
- Completion evidence does not upgrade acquisition reliability, and returned
  survivor PIDs are evidence rather than safe signaling targets.
- Completion evidence and owned containment remain Rust-only in this release;
  no JavaScript owned-containment or timeout lifecycle is added.
- Dependency-major upgrades are intentionally excluded from this release.
- Go and TypeScript native artifacts for all supported platforms must be
  regenerated from the final tag-target commit before publication.

---

## v0.1.20 - 2026-08-21

**Status:** Maintenance Release

v0.1.20 pins TypeScript N-API prebuild CI to Rust 1.88.0 and resumes TypeScript npm
publication. No public API or process-control behavior changes are intended.

### Highlights

- **TypeScript N-API prebuilds**: pin `dtolnay/rust-toolchain` to **1.88.0**.
- **TypeScript npm**: publication resumes on this cut.

### Upgrade Notes

- No public API changes are intended.
- **v0.1.19** remains the signed GitHub and Go module release for that cut.
- Go prebuilt libraries are produced by the Go Bindings Prep workflow after this
  version commit is merged to `main`; do not tag v0.1.20 before that artifact PR
  merges.

---

_Older releases are archived in `docs/releases/`._
