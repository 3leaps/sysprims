# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

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

## v0.1.19 - 2026-08-21

**Status:** Maintenance Release

v0.1.19 is a small maintenance release. It refreshes the compatible Rust lockfile and
TypeScript `@types/node` 22.20.1. No public API or process-control behavior changes are
intended.

### Highlights

- **Compatible Rust lockfile refresh**: crate versions updated within existing constraints.
  `thiserror` remains 1.0.69. napi remains 2.16.x.
- **TypeScript types**: `@types/node` 22.20.1. The `package.json` range stays `^22.10.0`.
- **CLI Clippy**: dropped a redundant borrow in the signal table printer.

### Upgrade Notes

- No public API changes are intended.
- Signed GitHub release and Go module tags were published.
- TypeScript npm publication resumes in v0.1.20.

---

_Older releases are archived in `docs/releases/`._
