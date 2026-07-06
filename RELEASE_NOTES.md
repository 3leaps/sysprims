# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

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

## v0.1.15 - 2026-03-27

**Status:** Guard Automation & Provenance Release

This release turns the recurring VSCodium runaway-plugin incident into a first-class sysprims
workflow. The release adds a reusable one-shot guard primitive, subtree-aware remediation, a
managed guard loop for long-lived watchdogs, a new ancestors surface for provenance, and
operational guard controls for background execution and discovery. The result is a cleaner path
from "find the hot offender" to "kill the whole offending subtree when needed" to "explain it" to
"run a watchdog that can keep the host healthy."

### Highlights

- **One-shot guard primitive**: `GuardStep` gives Rust, FFI, Go, and TypeScript consumers a shared
  per-tick remediation kernel instead of forcing each ecosystem to reimplement detection and safety
  logic.
- **Cascade remediation**: `kill-descendants --cascade` and guard actions can expand each matched
  offender to its subtree so cleanup does not leave child work running behind the hot process.
- **Managed watchdog loop**: `GuardRunner` extracts long-running guard behavior into a reusable
  library surface and now powers the CLI.
- **Provenance support**: New `ancestors` APIs make it easier to answer "what spawned this?" when
  diagnosing runaway helpers and plugin children.
- **Background guard operations**: `sysprims guard` now supports `--daemon`, `--pidfile`,
  `--status`, and `--stop` for production-style watchdog management on Unix hosts.
- **Self-discovery**: Running guards are discoverable through sysprims itself instead of requiring
  `ps | grep` workflows.
- **Shared runtime primitives**: `Tick`, `now_rfc3339()`, and `GuardSignals` standardize timing,
  timestamps, and signal-aware shutdown for long-running loops.

### GuardStep and `sysprims guard`

The original dogfood need was straightforward but painful: reliably find actively hot descendants,
preview the impact, then kill the offenders without taking down the parent editor process. This
release turns that workflow into a reusable contract.

`GuardStep` is the new one-shot primitive behind that workflow. It evaluates a guard rule, can
expand a matched descendant to its subtree with cascade targeting, applies an optional action only
when explicitly enabled, and emits a structured event suitable for logs and metrics.

On top of that, `sysprims guard` now acts as a thin orchestrator rather than owning bespoke loop
logic. It benefits from the shared guard surface while keeping the CLI behavior operators expect.

### `GuardRunner`: Managed Guard Loop

For long-running watchdog use cases, `GuardRunner` in `sysprims-proc` now provides the managed
loop:

- drift-free scheduling using `Tick`
- clean shutdown via `GuardSignals`
- cloneable programmatic stop handles
- max-iteration stop support
- stop-reason summaries (`Signal`, `Requested`, `MaxIterations`)
- presets for `interactive`, `background`, and `watchdog` intervals/sample windows

This lets Rust consumers reuse the same loop semantics as the CLI instead of rebuilding timers,
stop conditions, and signal handling themselves.

### FFI and Go Watchdog Support

For bindings, the release adds a polling-style runner lifecycle rather than cross-language
callbacks:

```c
sysprims_proc_guard_runner_create(...)
sysprims_proc_guard_runner_tick(...)
sysprims_proc_guard_runner_stop(...)
sysprims_proc_guard_runner_free(...)
```

Go now layers typed support on top with `GuardPreset`, `GuardRunnerConfig`, `NewGuardRunner`,
`Tick()`, `Stop()`, and `Close()`. TypeScript gets the one-shot `guardStep()` surface in this
release; the managed runner remains Rust/FFI/Go for now. The recommended path for most non-Rust
consumers remains a native runtime loop around `GuardStep`, but the polling runner is available for
teams that want the managed Rust-side contract.

Devrev hardening for this surface included:

- synchronized runner state in Rust so concurrent polling and stop requests do not race
- serialized handle lifecycle in Go to avoid `Tick`/`Stop`/`Close` misuse
- create-time validation of static guard config so bad inputs fail before a long-running handle is
  returned

### Provenance: `ancestors`

This release also adds a new provenance surface across Rust, CLI, FFI, Go, and TypeScript:

```bash
sysprims ancestors <pid> --max-depth 10 --json
```

That fills the "what spawned this?" gap that shows up immediately once sysprims can identify a hot
plugin helper or runaway descendant. Operators can now go from hot process discovery to a parent
chain without leaving sysprims.

### Daemon Mode + Pidfile Management

For long-running hosts and edge agents, `sysprims guard` now supports background execution and
pidfile management on Unix:

```bash
sysprims guard 27776 --daemon --preset watchdog --yes
sysprims guard 27776 --status
sysprims guard 27776 --stop
```

Key behaviors:

- `--daemon` detaches with `setsid()` and redirects stdio to null
- `--pidfile <PATH>` overrides the default `/tmp/sysprims-guard-<root-pid>.pid`
- pidfiles are removed on clean shutdown via drop-based lifecycle cleanup
- stale or invalid pidfiles are cleaned up instead of being trusted blindly
- `--status` and `--stop` verify that the pidfile target is actually a live sysprims guard process
- daemon startup now waits for initialization to complete before reporting success

Windows intentionally remains out of scope for daemon mode in v0.1.15; the CLI returns a clear
not-supported error and directs operators to a service manager.

### Self-Discovery: Find Running Guards with sysprims

This release also closes the observability gap around long-running guards. A running guard now sets
a best-effort platform title where supported, and sysprims process inspection rewrites matching
guard cmdlines to `sysprims-guard:<root_pid>` for ergonomic lookup. On Linux, the kernel-visible
thread name remains truncated by `PR_SET_NAME`, so the full identity primarily comes from
cmdline-backed discovery inside sysprims itself.

That enables workflows like:

```bash
sysprims descendants 1 --name sysprims-guard --max-levels all
sysprims pstat --name sysprims-guard:27776 --table
```

The implementation was hardened to account for real CLI invocation shape, including global flags
appearing before the `guard` subcommand.

### Shared Runtime Primitives

Long-running loops also gained a shared runtime foundation in `sysprims-core`:

- `now_rfc3339()` for consistent timestamp rendering
- `Tick` for drift-resistant periodic scheduling
- `GuardSignals` for signal-aware shutdown with consistent stop semantics

These changes matter beyond `guard` itself because they give future long-running sysprims workflows
a shared timing and shutdown contract instead of ad hoc per-command logic.

### Release Hardening

Release preparation for v0.1.15 also tightened the delivery path:

- stronger release preflight guidance
- explicit TypeScript binding validation in the release path
- clean prepush validation restored before release cut
- clearer repository policy that prebuilt native binding artifacts come from CI, not local builds

### Upgrade Notes

- `sysprims guard` gains additive new flags only; existing foreground guard invocations continue to
  work.
- `kill-descendants --cascade` is additive; existing non-cascade behavior stays the default.
- Unix daemon mode is new; Windows remains intentionally unsupported in this release.
- FFI consumers must rebuild shared/static libraries to pick up the new guard runner exports.
- `ancestors` is additive across all supported surfaces.
- Go verification for the new runner surface currently uses a freshly built local `sysprims-ffi`
  artifact during development until release workflows refresh checked-in prebuilt libraries.

_Older releases are archived in `docs/releases/`._
