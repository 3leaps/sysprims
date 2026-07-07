# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

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

## v0.1.17 - 2026-07-06

**Status:** Session-Spawn Bindings + Portable Liveness

This release makes sysprims' detached session-spawn primitives available beyond Rust. TypeScript
and Bun consumers can now launch parent-outliving processes through `runSetsid()` and `runNohup()`
and supervise them using the identifiers sysprims returns, with the same safety posture as the
Rust implementation. v0.1.17 also adds portable process-liveness predicates so callers can answer
"did the process I just signalled actually stop?" without depending on platform-specific zombie
behavior.

### Highlights

- **Session-spawn FFI + TypeScript bindings**: JSON C-ABI exports `sysprims_run_setsid` and
  `sysprims_run_nohup` now back TypeScript `runSetsid()` and `runNohup()`.
- **Structural supervision model**: `runSetsid()` returns child `pid`/`sid`/`pgid` derived from
  the spawned child PID, avoiding post-spawn child session/group lookups and PID-reuse races.
- **Honest nohup identifiers**: `runNohup()` returns the caller's inherited session and process
  group context, making the parent-outliving semantics explicit instead of pretending the child
  owns a new group.
- **Portable liveness checks**: `is_live(pid)` and `is_fully_gone(pid)` normalize zombie handling
  across Linux, macOS, and Windows for kill-then-check workflows.
- **Bun support**: Bun >=1.3 is now documented as supported for the Node-API TypeScript binding
  surface, alongside Node.js >=18.
- **Security hardening**: explicit nohup output files use append/create semantics with final-path
  symlink rejection, ADR-0011 PID validation now covers more process APIs, and ADR-0016 documents
  the session-spawn FFI contract.

### Upgrade Notes

- **Additive release**: existing signal, PID, timeout, and process-tree behavior is unchanged.
- `runNohup()` returns the caller's own `pgid`; supervise the spawned child by `pid`, and never
  `kill(-pgid, ...)` from that result because it would signal the caller and its siblings.
- TypeScript consumers on Bun should use Bun >=1.3.
- Go prebuilt libraries for v0.1.17 are produced by the Go-bindings prep workflow after this
  release-doc/version package is merged to `main`; do not tag before that artifacts PR merges.

---

## v0.1.16 - 2026-04-18

**Status:** Windows ARM64 Go Bindings Release

This release closes the long-standing limitation that kept Go consumers off Windows arm64.
sysprims now ships a prebuilt `libsysprims_ffi.a` for `aarch64-pc-windows-gnullvm` via
[llvm-mingw](https://github.com/mstorsjo/llvm-mingw), alongside the existing windows-amd64
binding built with msys2/MinGW-w64. Consumers installing llvm-mingw locally can `go get`
sysprims and link cgo code on Windows arm64 the same way they already do on every other
platform.

### Highlights

- **Windows arm64 Go bindings**: Prebuilt `libsysprims_ffi.a` for `aarch64-pc-windows-gnullvm`
  via llvm-mingw. `go build` on Windows arm64 now links successfully against sysprims cgo.
- **Shared-mode parity**: `cgo_windows_arm64_shared.go` + `cgo_windows_arm64_shared_local.go`
  close the gap between shipped shared libraries and Go linker directives on arm64.
- **CI regression coverage**: `test-go` matrix gains a native `windows-latest-arm64-s` leg so
  arm64-specific cgo/linker regressions are caught on every PR.
- **Release-pipeline consistency**: windows-arm64 FFI release artifacts now use the GNU ABI
  path that matches Go cgo's expectations (previously shipped MSVC `.lib`, which Go can't
  consume).
- **Repository workflow**: Retires the guardian-hook commit gate in favor of a feature-branch
  / PR workflow with squash-merge default. Adds `make pr-final` as the merge-readiness gate.

### Windows ARM64 Go: Why Now

The previous documentation said "Go cgo on Windows requires MinGW, and MinGW does not support
arm64" — technically correct about msys2/MinGW-w64, but the ecosystem moved on. Two changes
made this release possible:

1. **`aarch64-pc-windows-gnullvm`** graduated to Rust Tier 2 with host tools. This is the
   llvm-mingw-flavored Windows GNU-ABI target that produces `.a` and `.dll.a` artifacts
   consumable by Go cgo.
2. **GitHub Actions `windows-latest-arm64-s` runners** are already in active use across our
   release pipelines for CLI and TypeScript. No new runner plumbing needed.

Combining the two gives a clean path: install llvm-mingw on the arm64 runner, build
sysprims-ffi for gnullvm, ship the resulting `.a` to consumers.

### Consumer Requirement

Building Go code against sysprims on Windows arm64 requires a local GNU-ABI C toolchain with
`aarch64-w64-mingw32-gcc` on `PATH`. Install:

```powershell
# Download llvm-mingw latest release (pick the *-ucrt-aarch64.zip)
# Extract and add <install>/bin to PATH
$env:PATH = "C:\tools\llvm-mingw\bin;$env:PATH"
$env:CC = "aarch64-w64-mingw32-gcc"
go build ./...
```

Linux and macOS consumers need no extra toolchain — the platform default GCC/clang works as
before.

### What's Shipped

| Artifact | Before v0.1.16 | In v0.1.16 |
|---|---|---|
| `bindings/go/sysprims/lib/windows-arm64/libsysprims_ffi.a` | Not shipped | GNU-ABI via llvm-mingw |
| Release bundle `static/windows-arm64/libsysprims_ffi.a` | MSVC `.lib` (unusable for Go) | GNU-ABI `.a` |
| Release bundle `shared/windows-arm64/*` | MSVC `.dll` + `.dll.lib` | GNU-ABI `.dll` + `.dll.a` |
| CI coverage | No arm64 Go matrix leg | `windows-latest-arm64-s` runs `go test` per PR |

CLI binaries on Windows arm64 continue to ship MSVC-built (`aarch64-pc-windows-msvc`) since
CLI does not involve cgo.

### Release Workflow Hardening

Secondary but maintainer-facing: v0.1.16 also finalizes the shift to PR-based change control.

- Repository now requires PRs to merge into `main` (squash default, rebase allowed, merge
  commits disabled).
- Guardian-hook browser-approval gate is retired — PR review plus protected-branch controls
  provide equivalent change control without the browser-approval friction.
- `make pr-final` wraps `prepush` as the merge-readiness gate and is referenced from
  `RELEASE_CHECKLIST.md` as a prerequisite before starting any release.

### Upgrade Notes

- **Additive across the board** — no breaking changes. Existing binding consumers on other
  platforms see no behavioral change.
- Windows arm64 Go consumers need llvm-mingw installed locally (see the Consumer Requirement
  section above). Without it, `go build` will fail at link time with missing GCC driver — an
  expected failure mode, not a bug.
- Windows amd64 Go consumers continue to use msys2/MinGW-w64 exactly as before.
- Python bindings on Windows arm64 remain unsupported.

### Follow-ups

- **Pin llvm-mingw version**: All three workflows currently resolve llvm-mingw via GitHub's
  `releases/latest` at CI time, which means two runs against the same sysprims commit can link
  against different toolchains if mstorsjo publishes a new release between runs. Pinning to a
  specific tag is tracked as reproducibility debt and will land as a follow-up PR.

---

_Older releases are archived in `docs/releases/`._
