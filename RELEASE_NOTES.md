# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

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

## v0.1.18 - 2026-07-07

**Status:** Maintenance Release

v0.1.18 is a small maintenance release. It refreshes compatible dependency locks, aligns the
documented Rust baseline with the workspace's practical requirement, updates Windows FFI
dependencies, and hardens the TypeScript npm publish path. No public API or process-control
behavior changes are intended.

### Highlights

- **Compatible dependency refresh**: refreshed Rust and TypeScript lockfiles, including
  `@types/node` 22.20.0, without changing Node runtime support policy.
- **Rust baseline**: set the workspace `rust-version` and public build guidance to Rust 1.88.0,
  matching the resolved dependency baseline.
- **Windows FFI dependencies**: updated `rsfulmen` to `=0.1.5` and workspace `windows-sys` to
  `0.61`; the lockfile now resolves a single `windows-sys` 0.61.x graph, with Windows-only
  handle checks adjusted for pointer-typed `HANDLE`s.
- **TypeScript npm publishing**: the npm publish workflow now uses Node 24 and validates the
  trusted-publishing runtime floor before publishing.

### Upgrade Notes

- No public API changes are intended.
- Rust builders should use Rust 1.88.0 or newer.
- Go prebuilt libraries are produced by the Go Bindings Prep workflow after this release-prep
  commit is merged to `main`; do not tag v0.1.18 before that artifact PR merges.
- TypeScript release publishing should continue to run N-API prebuilds from the tag before npm
  publish.

---

_Older releases are archived in `docs/releases/`._
