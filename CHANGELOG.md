# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** This file maintains the latest 10 releases in reverse chronological order.
> Older releases are archived in `docs/releases/`.

## [Unreleased]

## [0.2.1] - Pending

### Added

- Safe `spawn_contained(Command)` acquisition for owned Unix children. The
  child enters a new session and process group in its pre-exec hook, and the
  returned guard retains the same-spawn proof needed for group signaling.
- A receipt-bound PTY adapter seam: `prepare_session_acquisition` provides the
  replacement `setsid` hook and pending proof, while the generic
  `contain_acquired_session` constructor is explicitly `unsafe`.
- Fixed-size, async-signal-safe child acknowledgement and one-shot acquisition
  tokens that fail closed on missing, partial, duplicated, or mismatched use.
- Privileged disposable-container coverage for hostile descendants that leave
  their acquired Unix process group.

### Changed

- `TreeKillReliability::Guaranteed` now requires race-free spawn-time
  acquisition plus retained group-signaling eligibility. It does not claim
  that descendants cannot later leave a cooperative Unix process group.
- Receipt-bound termination revalidates the child, session, process group, and
  exclusive unreaped ownership before signaling. The leader remains unreaped
  through final group escalation so its PID and PGID cannot be reused.
- PID-only and legacy group-spawn paths remain `best_effort`; post-spawn owned
  adoption remains `unproven`.
- Guaranteed Windows spawn continues to fail closed until create-suspended Job
  assignment is available.

### Fixed

- A leader-exit race between live identity validation and process-group lookup
  now accepts only the exact receipt-bound exited-but-unreaped transition.
  Lost reap ownership and live identity mismatches still fail closed.
- Linux process start-time conversion preserves clock-tick precision when
  binding receipt identity.

### Upgrade Notes

- The new acquisition and containment APIs are Rust-only in this release.
  FFI, Go, and TypeScript binding surfaces are unchanged.
- A PTY integration must install the sysprims hook instead of performing its
  own `setsid`, and must retain exclusive unreaped ownership for the unsafe
  generic adapter contract.
- No portable-PTY companion crate or consumer integration is included.

## [0.2.0] - Pending

### Added

- Owned `ContainmentGuard` APIs for process-group and Windows Job lifecycle
  management, including normal completion, explicit termination, child recovery,
  and active-guard cleanup.
- `unproven` tree-kill reliability for owned post-spawn containment with an
  observable descendant escape window.
- Structured point-in-time `Empty`, `Survivors`, or `Unknown` completion
  evidence for owned Rust containment, with platform provenance and bounded,
  fail-closed observation.
- Timeout result schema v1.1 with the expanded reliability contract.
- TypeScript descendant detail options for opt-in environment and thread
  enrichment, with default-off behavior and permission-aware results.
- Generated TypeScript public API contracts and documentation derived from
  emitted declarations, the N-API inventory, and the public C header.

### Changed

- Windows post-spawn adoption now retains the Job handle in the owned guard.
- Windows `spawn_contained` and PID-returning `spawn_in_group` fail before spawn
  until create-suspended Job assignment is available.
- Dropping an active containment guard terminates the contained tree and makes a
  bounded child reap attempt on Unix and Windows.
- TypeScript PID, process-group, signal, depth, duration, port, and filter
  inputs are validated before JavaScript coercion or native loading and are
  revalidated at the N-API boundary.
- TypeScript native behavior is exercised under Node.js 18, Node.js 22, and Bun
  1.3.3 on Linux x64, macOS arm64, and Windows x64, with package-only ARM lanes.
- Pre-1.0 minor releases may contain explicitly documented compatibility changes.

### Fixed

- The nohup working-directory/environment test now routes terminal output to
  its temporary directory instead of leaving a crate-local `nohup.out`.

### Upgrade Notes

- Rust consumers must handle `TreeKillReliability::Unproven` when exhaustively
  matching the public enum.
- Windows consumers of `spawn_in_group` must migrate to an owned guard integration.
- `ContainmentOutcome.completion` is a Rust-only observation in this release;
  Go and TypeScript do not expose an owned-containment lifecycle.
- Completion evidence does not upgrade acquisition reliability, and survivor
  PIDs are evidence only rather than safe signaling targets.
- Dependency-major upgrades are intentionally excluded from this release.
- Go and TypeScript native artifacts for all supported platforms must be
  regenerated from the final tag-target commit before publication.

## [0.1.20] - 2026-08-21

v0.1.20 pins TypeScript N-API prebuild CI to Rust 1.88.0 and resumes TypeScript npm
publication. No public API or process-control behavior changes are intended.

### Changed

- **TypeScript N-API prebuilds**: `dtolnay/rust-toolchain` in
  `.github/workflows/typescript-napi-prebuilds.yml` is pinned to **1.88.0**.

### Notes

- **v0.1.19** remains the signed GitHub and Go module release. TypeScript npm
  publication resumes on 0.1.20.
- Go prebuilt libraries are produced by the Go Bindings Prep workflow after this
  version commit is merged to `main`; do not tag v0.1.20 before that artifact PR
  merges.

## [0.1.19] - 2026-08-21

v0.1.19 is a small maintenance release. It refreshes the compatible Rust lockfile and
TypeScript `@types/node` 22.20.1. No public API or process-control behavior changes are
intended.

### Changed

- **Compatible Rust lockfile refresh**: updated crate versions within existing constraints.
  `thiserror` remains 1.0.69. napi remains 2.16.x.
- **TypeScript types**: `@types/node` 22.20.0 → 22.20.1. The `package.json` range stays
  `^22.10.0`. Node runtime policy is unchanged.

### Fixed

- **CLI signal table print**: dropped a redundant borrow that newer Clippy rejected
  (`useless_borrows_in_formatting`).

### Notes

- Signed GitHub release and Go module tags were published. TypeScript npm
  publication resumes in v0.1.20.

## [0.1.18] - 2026-07-07

v0.1.18 is a small maintenance release. It refreshes compatible dependency locks,
aligns the documented Rust baseline with the workspace's practical requirement,
updates Windows FFI dependencies, and hardens the TypeScript npm publish path. No
public API or process-control behavior changes are intended.

### Changed

- **Compatible dependency refresh**: refreshed Rust and TypeScript lockfiles,
  including `@types/node` 22.20.0, without changing Node runtime support policy.
- **Rust baseline**: set the workspace `rust-version` and public build guidance to
  Rust 1.88.0, matching the resolved dependency baseline.
- **Windows FFI dependencies**: updated `rsfulmen` to `=0.1.5` and workspace
  `windows-sys` to `0.61`; the lockfile now resolves a single `windows-sys`
  0.61.x graph, with Windows-only handle checks adjusted for pointer-typed
  `HANDLE`s.
- **TypeScript npm publishing**: the npm publish workflow now uses Node 24 and
  validates the trusted-publishing runtime floor before publishing.

### Notes

- Go prebuilt libraries are still produced by the Go Bindings Prep workflow after
  this release-prep commit is merged to `main`; do not tag v0.1.18 before that
  artifact PR merges.

## [0.1.17] - 2026-07-06

Session-spawn primitives cross the FFI boundary in v0.1.17: detached, parent-outliving
`setsid` and `nohup` spawns are now reachable from TypeScript/Bun as well as Rust. The
release also adds portable process-liveness predicates for the common "did the process I
just signalled actually stop?" workflow. The through-line is safety: structurally derived
`sid`/`pgid` for `setsid`, honest caller-context identifiers for `nohup`, `O_NOFOLLOW`
on explicit nohup output targets, and wider ADR-0011 PID validation. All changes are
additive; existing signal, PID, timeout, and process-tree behavior is unchanged, so
v0.1.16 consumers can upgrade freely.

### Added

- **`sysprims_proc::is_live(pid)` and `sysprims_proc::is_fully_gone(pid)`** — single-shot
  liveness predicates that give a portable answer to "did the process I just signalled
  actually stop?". They normalize a cross-platform divergence: an exited-but-unreaped child
  is a zombie still present in the process table on Linux but is typically already unreadable
  on macOS. `is_live` returns `false` for a zombie on every platform; `is_fully_gone`
  distinguishes an unreaped zombie (neither live nor fully gone) from a fully-reaped PID.
  Windows has no zombie state, so a PID is simply live or gone. To stay portable, `is_live`
  treats a present-but-unreadable PID (the macOS killed-but-unreaped case) as not live — a
  deliberate liveness bias for the kill-then-check pattern, not a replacement for `get_process`
  diagnostics. Both predicates reject PID 0 and PIDs above `i32::MAX` (ADR-0011).
- **Session-spawn FFI and TypeScript bindings**: added JSON C-ABI exports
  `sysprims_run_setsid` / `sysprims_run_nohup`, plus TypeScript `runSetsid` /
  `runNohup`. `runSetsid` returns structurally derived child `pid`/`sid`/`pgid`
  identifiers; `runNohup` returns the inherited caller session context and is
  supervised by child `pid`.
- **Session-spawn schemas**: added v1.0.0 config/result schemas for `runSetsid`,
  `runNohup`, and their shared session-spawn result envelope.
- **Documentation**: added ADR-0016 for the session-spawn FFI contract and promoted the
  session-safety incident writeup for easier discovery.

### Changed

- **`sysprims-session` configs**: `SetsidConfig` and `NohupConfig` now honor
  per-spawn `cwd` and inherited-environment overrides. `NohupConfig` opens an
  explicit output target with append/create semantics while rejecting a final
  symlink.
- **`get_process` documentation**: added a cross-platform note that an exited-but-unreaped
  child returns `Ok(_)` with `state == Zombie` on Linux but `Err(NotFound)` on macOS, so
  `Ok(_)` is not a portable liveness signal; points callers to the new predicates and
  `wait_pid`.
- **Uniform PID-safety validation** (ADR-0011): `get_process` / `get_process_with_options`,
  `wait_pid`, and `cpu_total_time_ns` now reject a PID above `i32::MAX` with `InvalidArgument`,
  matching `list_fds` / `ancestors` / `descendants` / `guard`. Previously they gated only on
  PID 0, so an out-of-range PID could reach a `pid as pid_t` cast (a negative/broadcast PID
  under `kill(pid, 0)`). Such PIDs are invalid regardless; well-formed callers are unaffected.

- **Pin llvm-mingw to `20260407`** across CI, release, and Go bindings prep workflows
  (`.github/workflows/ci.yml`, `.github/workflows/release.yml`,
  `.github/workflows/go-bindings.yml`). A top-level `LLVM_MINGW_VERSION` env variable replaces
  the previous `/releases/latest` API query; same sysprims commit now links against the same
  llvm-mingw toolchain on every build. Bumps require one-line edits in the three workflows.
- **Platform support docs** (`docs/standards/platform-support.md`): reconcile the shipped-
  artifact lists with release reality. `darwin-amd64` CLI tarball and `libsysprims_ffi.a` are
  now documented as legacy artifacts retained for backward compatibility, scheduled for
  removal. The "Explicitly Unsupported" section continues to document the v0.1.7 deprecation
  decision.
- **TypeScript runtime support docs**: Bun >=1.3 is documented as supported for
  the Node-API binding surface, alongside Node.js >=18.

## [0.1.16] - 2026-04-18

Windows arm64 Go binding support lands. Closes a long-standing limitation where Go consumers on
Windows arm64 had no path to link against sysprims. Also finalizes the feature-branch / PR workflow
and retires the guardian-hook commit gate.

### Added

- **Windows arm64 Go bindings** (`bindings/go`, `bindings/go/sysprims/lib/windows-arm64`):
  Prebuilt `libsysprims_ffi.a` for `aarch64-pc-windows-gnullvm` via llvm-mingw. Go cgo on Windows
  arm64 now links successfully when consumers have llvm-mingw (`aarch64-w64-mingw32-gcc` on PATH).
  Mirrors the existing windows-amd64 msys2/MinGW-w64 story with a different toolchain distribution.
- **Windows arm64 shared-mode cgo directives** (`bindings/go`): New
  `cgo_windows_arm64_shared.go` and `cgo_windows_arm64_shared_local.go` close the gap between
  shipped shared artifacts and Go linker directives, enabling `go build -tags=sysprims_shared` on
  windows/arm64.
- **Windows arm64 Go CI coverage** (`.github/workflows/ci.yml`): `test-go` now includes a native
  `windows-latest-arm64-s` matrix leg with llvm-mingw so arm64-specific cgo/linker regressions are
  caught automatically on every PR.

### Changed

- **Release pipeline** (`.github/workflows/release.yml`): windows-arm64 FFI artifacts in the
  tagged release bundle now use the GNU ABI path (`aarch64-pc-windows-gnullvm` via llvm-mingw)
  matching what Go cgo consumes. CLI continues to ship MSVC-built.
- **Release process** (`.github/workflows/`, `RELEASE_CHECKLIST.md`): Repository moves to a
  feature-branch / PR workflow with squash-merge default. The guardian-hook commit gate is
  retired in favor of PR review and protected-branch controls. Adds `make pr-final` as the
  merge-readiness gate (wraps `prepush`).

### Notes

- Windows arm64 Go consumers must install [llvm-mingw](https://github.com/mstorsjo/llvm-mingw)
  locally (pick the `*-ucrt-aarch64.zip` release) and ensure `aarch64-w64-mingw32-gcc` is on
  `PATH` before `go build`. Linux and macOS consumers need no extra toolchain.
- `aarch64-pc-windows-gnullvm` is Rust Tier 2 with host tools; the gnullvm target itself is stable.
- llvm-mingw is downloaded via GitHub's `releases/latest` in all three workflows today. Pinning to
  a specific llvm-mingw release tag is tracked as follow-up reproducibility debt.

## [0.1.15] - 2026-03-27

Guard automation and provenance release work. This cycle turns the VSCodium runaway-plugin dogfood
incident into a reusable workflow: detect hot descendants reliably, expand a matched offender to
its subtree when needed, explain where it came from, and run sysprims as a long-lived watchdog
instead of a foreground-only CLI loop.

### Added

- **GuardStep one-shot remediation primitive** (`sysprims-proc`, `sysprims-ffi`, `bindings/go`,
  `bindings/typescript`): New structured guard evaluation API for one-shot monitoring and optional
  remediation, with explicit action enablement and per-tick event output.
- **Cascade descendant remediation** (`sysprims-proc`, `sysprims-cli`, `sysprims-ffi`,
  `bindings/go`, `bindings/typescript`): `kill-descendants --cascade` and guard actions can expand
  each matched offender to its subtree so cleanup does not leave child work behind.
- **Ancestors provenance API** (`sysprims-proc`, `sysprims-cli`, `sysprims-ffi`, `bindings/go`,
  `bindings/typescript`): New ancestor-walk surface for answering "what spawned this?" across Rust,
  CLI, and bindings.
- **Managed guard loop** (`sysprims-proc`, `sysprims-cli`, `sysprims-ffi`, `bindings/go`):
  `GuardRunner` extracts the long-running loop into reusable Rust and polling-style FFI surfaces,
  with typed Go wrappers for watchdog consumers.
- **Guard daemon management** (`sysprims-cli`): `sysprims guard` gains `--daemon`,
  `--pidfile <PATH>`, `--status`, and `--stop` for background operation and pidfile-based process
  management on Unix.
- **Guard self-discovery convention** (`sysprims-proc`, `sysprims-cli`): Running guards expose a
  best-effort guard title, and sysprims process inspection normalizes matching guard cmdlines to
  `sysprims-guard:<root_pid>` for ergonomic lookup.
- **Shared runtime primitives** (`sysprims-core`): Added `now_rfc3339()`, drift-resistant `Tick`,
  and `GuardSignals` to standardize timestamps, scheduling, and signal-aware shutdown for
  long-running loops.

### Changed

- **CLI: `sysprims guard`** (`sysprims-cli`): Now runs as a thin consumer of `GuardRunner`, keeps
  observation mode as the default, and adds `--preset` guidance for `interactive`, `background`,
  and `watchdog` guard profiles.
- **Release process hardening**: v0.1.15 prep adds stronger release preflight guidance, TypeScript
  binding validation, restored clean prepush checks, and explicit CI-only policy for prebuilt
  native binding artifacts.

### Fixed

- **Guard preset override semantics** (`sysprims-cli`): Explicit `--cpu-mode lifetime` now wins
  over preset-provided sampling instead of being silently promoted to monitor mode.
- **GuardRunner FFI safety** (`sysprims-ffi`, `bindings/go`): Polling runner state is now
  synchronized in Rust and serialized in the Go wrapper, avoiding concurrent lifecycle races.
- **GuardRunner create-time validation** (`sysprims-ffi`): Runner creation now rejects static guard
  misconfiguration up front instead of creating handles that would fail forever at tick time.
- **Daemon startup acknowledgment** (`sysprims-cli`): `--daemon` now waits until the detached child
  finishes initialization before reporting success, and fails cleanly on early startup errors.

### Notes

- Unix daemon mode is implemented for v0.1.15; Windows currently returns a clear not-supported
  error directing operators to a service manager.
- Go local verification for the new FFI runner surface currently links against a freshly built
  local `sysprims-ffi` from `target/debug` until prebuilt binding artifacts are refreshed by the
  release workflow.

## [0.1.14] - 2026-02-24

Process intelligence and Go team depth. Surfaces process environment variables and thread count
through `proc_ext`, extends CPU measurement parity to all tree commands, fixes a schema compliance
bug in `pstat --pid --json`, and ships a documentation sprint targeting Go platform team adoption.

### Added

- **`proc_ext`: `ProcessOptions` with `IncludeEnv` and `IncludeThreads`** (`sysprims-proc`,
  `sysprims-ffi`, `bindings/go`, `bindings/typescript`): New opt-in fields on `ProcessInfo`
  (`env: Option<BTreeMap<String, String>>`, `thread_count: Option<u32>`). Zero cost when not
  requested. Linux: `/proc/[pid]/environ` + `/proc/[pid]/status`; macOS: `sysctl(KERN_PROCARGS2)`
  env block + `proc_taskinfo` thread count; Windows: thread count only (env deferred).
  EPERM on env read → `env: null`, no error propagation.

- **CPU mode on `descendants` and `kill-descendants`** (`sysprims-proc`, `sysprims-cli`,
  `sysprims-ffi`, `bindings/go`, `bindings/typescript`): `DescendantsConfig` gains
  `cpu_mode: CpuMode` and `sample_duration: Option<Duration>`. CLI: `--cpu-mode monitor
--sample 3s`. Found during dogfooding — 4 spinning zombie VSCodium plugin processes, only 2
  visible to lifetime mode, all 4 visible with monitor sampling. Sampled output uses new
  `descendants-result-sampled.schema.json` v1.1.0.

- **CLI: `sysprims help <topic>` subcommand** (`sysprims-cli`): Concept-level reference for
  `cpu-mode`, `signals`, and `safety` topics. Output suppressed when `SYSPRIMS_NO_HINTS=1`.

- **CLI: `after_help` examples** (`sysprims-cli`): Workflow examples added to `pstat`,
  `descendants`, and `kill-descendants` subcommands.

- **Contextual hint: `--cpu-above` without monitor mode** (`sysprims-cli`): One-line stderr hint
  suggesting `--cpu-mode monitor --sample 3s` when lifetime mode is active. Suppressed with
  `--json`, `SYSPRIMS_NO_HINTS=1`, or explicit `--cpu-mode`.

- **Rustdoc examples** (`sysprims-proc`, `sysprims-signal`, `sysprims-timeout`): `# Examples`
  block on every public function; doc tests run in CI.

- **Documentation** (`docs/guides/`): `replace-shell-outs-go.md` (DM-1),
  `process-intelligence-without-shell-outs.md` (DM-2),
  `docs/one-pagers/go-team-adoption-v0.1.14.md` (DM-3).

### Fixed

- **`pstat --pid --json` schema compliance** (`sysprims-cli`): Output now wraps in
  `SnapshotResult` envelope with `schema_id` as required by ADR-0005. Previously returned a flat
  `ProcessInfo` object — a contract violation. Process-not-found path returns empty `processes: []`
  array instead of a JSON parse error.

### Changed

- **Schema**: `process-info.schema.json` and `process-info-sampled.schema.json` bumped to v1.1.0
  (add optional `env` and `thread_count` fields — minor bump per ADR-0005).
- **Schema**: New `descendants-result-sampled.schema.json` v1.1.0 for sampled tree output (CPU
  values may exceed 100 for multi-core consumers).
- **Go pkg docs**: Updated for v0.1.14 API surface including `ProcessGetWithOptions`,
  `ProcessListWithOptions`, `DescendantsWithOptions`, and `KillDescendantsWithOptions` with CPU
  mode options.
- **README**: "As a Go Library" section updated to show `ProcessGetWithOptions` (env + threads)
  and `KillDescendantsWithOptions` (monitor CPU mode) — the primary v0.1.14 surface.
- **Developer experience**: `make fmt`, `make fmt-check`, and `make lint` now run multi-language
  goneat checks alongside strict Rust checks
  (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`) so local quality gates
  match CI expectations.
- **Dependencies**: `rsfulmen` pinned version updated from `0.1.2` to `0.1.4` with lockfile refresh.
- **Security policy noise**: Removed stale `cargo-deny` source allowlist entries from `deny.toml`
  (`allow-git` / `allow-org`) to eliminate false-medium unmatched-source findings in security scans.
- **Repository formatting**: Non-markdown formatting normalized across workflows, schemas, role
  configs, and tooling config files using goneat v0.5.4 behavior.

---

## [0.1.13] - 2026-02-13

macOS command-line fidelity fix and binding coverage expansion.

### Fixed

- **macOS cmdline truncation** (`sysprims-proc`): `processList()` and `pstat` now return the full argument vector on macOS instead of just the process name. Uses `sysctl(CTL_KERN, KERN_PROCARGS2)` to read the actual argv from the kernel. Previously returned `["bun"]` instead of `["bun", "run", "scripts/dev.ts", "--root", "/path"]`, breaking downstream consumers that filter by command-line arguments. Includes PID safety guard, argc cap (4096), and empty-entry filtering.

### Added

- **FFI: `sysprims_proc_descendants()`** and **`sysprims_proc_kill_descendants()`** (`sysprims-ffi`): Exports v0.1.12 process tree capabilities through the C-ABI FFI layer with JSON config/result pattern
- **Go binding: `Descendants()`** and **`KillDescendants()`** (`bindings/go`): Process tree traversal and targeted subtree termination with option pattern
- **TypeScript binding: `descendants()`** and **`killDescendants()`** (`bindings/typescript`): N-API native addon for process tree operations
- **Role: `deliverylead`** (`config/agentic/roles/`): Delivery coordination role for readiness assessments and release gating

### Changed

- **Co-Authored-By email policy**: All AI model trailers now use `noreply@3leaps.net` to prevent third-party email squatting on GitHub contributor attribution

## [0.1.12] - 2026-02-06

Process tree operations & enhanced discovery release. Adds process tree traversal with ASCII visualization, surgical subtree termination, age-based filtering, and parent PID filtering.

### Added

- **CLI: `sysprims descendants`** (`sysprims-cli`)
  - List child processes of a root PID with depth control
  - ASCII tree visualization with `--tree` flag
  - Filter by name, user, CPU, memory, age, and parent PID
  - Depth control via `--max-levels N` (1 = direct children, "all" = full subtree)
- **CLI: `sysprims kill-descendants`** (`sysprims-cli`)
  - Send signals to descendants of a process without affecting parent
  - Same filter options as `descendants`
  - Safety defaults: preview mode unless `--yes`, excludes parent/self/PID1/root
  - Force override with `--force` for protected targets

- **CLI: Enhanced `sysprims pstat`** (`sysprims-cli`)
  - `--ppid <PID>` filter option for parent-based filtering
  - `--running-for <DURATION>` filter option for age-based filtering
  - Filter support extended to all existing filter options

- **CLI: Enhanced `sysprims kill`** (`sysprims-cli`)
  - All filter options (`--ppid`, `--name`, `--user`, `--cpu-above`, `--memory-above`, `--running-for`) now supported

### Fixed

- **Process Filter**: Age-based filtering now works correctly on all platforms
- **CLI Safety**: `descendants --max-levels` and `kill-descendants` now exclude parent/root/self by default
- **CLI Feature**: `descendants` accepts "all" keyword for full subtree traversal
- **Bug**: Parent process included in `kill-descendants` dry-run output (fixed with explicit exclusion)
- **Security**: Updated `time` crate from 0.3.45 to 0.3.47 (fixes RUSTSEC-2026-0009 DoS via stack exhaustion)

### Changed

- **CLI**: `descendants` output format includes level grouping and matched filter counts
- **Schema**: Added `ppid` and `running_for_at_least_secs` fields to `process-filter.schema.json`
- **Schema**: Added `descendants-result.schema.json` for `descendants` command output

---

## [0.1.11] - 2026-02-04

macOS port discovery and Bun runtime support release. Fixes `listeningPorts()` returning empty results on macOS and adds a new `ports` CLI command.

### Added

- **CLI: `sysprims ports`** (`sysprims-cli`)
  - List listening port bindings with optional filtering
  - Filter by protocol: `--protocol tcp|udp`
  - Filter by port: `--local-port 8080`
  - Output formats: `--json` (default) or `--table`
  - Includes full process info (name, PID, exe_path, cmdline)

### Fixed

- **macOS: `listeningPorts()` Reliability** (`sysprims-proc`)
  - Fixed socket fdinfo parsing that was failing due to SDK struct layout mismatch
  - Now correctly discovers TCP listeners on macOS (was returning empty results)
  - Added UID filtering to scan current-user processes only (reduces SIP/TCC noise)
  - Heuristic vinfo_stat size detection (136/144 bytes) for SDK compatibility
  - Offset-based parsing instead of fixed struct layout (future-proof)
  - Strict TCP listener filtering (`TSI_S_LISTEN` state only)

### Changed

- **TypeScript Bindings: Bun Runtime Support** (`bindings/typescript/sysprims/`)
  - Removed explicit Bun runtime block that threw an error on load
  - Bun's N-API compatibility is now leveraged directly
  - Core functionality validated: `procGet()`, `terminate()`, `listeningPorts()`

### Notes

- macOS port discovery now works for current-user processes; other users' processes are filtered with warnings
- Bun support validated by kitfly team before release

## [0.1.10] - 2026-02-03

Fast-follow polish release improving Go shared-library mode developer experience and clarifying multi-Rust FFI collision guidance.

### Added

- **Go Bindings: Developer-Local Shared Library Override** (`bindings/go/sysprims/`)
  - New build tag: `sysprims_shared_local` for local development workflows
  - Allows linking against locally-built shared libraries in `lib-shared/local/<platform>/`
  - Separates shipped prebuilt libs from developer-local overrides to eliminate linker confusion
  - Usage: `-tags="sysprims_shared,sysprims_shared_local" ./...`

### Changed

- **Go Bindings: Cleaner Default Shared Mode** (`bindings/go/sysprims/`)
  - `sysprims_shared` tag no longer searches `lib-shared/local/...` paths by default
  - Eliminates confusing linker warnings when local override directory doesn't exist
  - Prebuilt libraries remain available via `sysprims_shared` tag only

### Documentation

- **README.md**: Added explicit guidance for multi-Rust FFI collision scenarios
  - Documents duplicate symbol `_rust_eh_personality` failure mode
  - Clear tag selection guide:
    - `-tags=sysprims_shared` (glibc/macOS/Windows)
    - `-tags="musl,sysprims_shared"` (Alpine/musl)
    - `-tags="sysprims_shared,sysprims_shared_local"` (local dev override)

### Upgrade Notes

- If relying on `bindings/go/sysprims/lib-shared/local/...` implicitly with `sysprims_shared`, add the `sysprims_shared_local` tag explicitly.
- No breaking changes to existing `sysprims_shared` workflows using prebuilt libraries.

## [0.1.9] - 2026-02-01

Process visibility and batch operations release. Adds `sysprims fds` for inspecting open file descriptors and multi-PID kill for batch signal operations, completing the diagnostic and remediation toolkit.

### Added

- **CLI: `sysprims fds`** (`sysprims-cli`, `sysprims-proc`)
  - Inspect open file descriptors for any process (the `lsof` use-case, GPL-free)
  - Platform support: Linux (full paths), macOS (best-effort), Windows (not supported)
  - Filter by resource type: `--kind file|socket|pipe|unknown`
  - JSON schema-backed output (`process/v1.0.0/fd-snapshot`)
  - Library: `list_fds(pid, filter) -> FdSnapshot`
  - FFI: `sysprims_proc_list_fds(pid, filter_json, result_json_out)`
  - Bindings: Go `ListFds`, TypeScript `listFds`

- **Library: Batch Signal Operations** (`sysprims-signal`)
  - `kill_many(pids, signal) -> BatchKillResult` - Send signal to multiple processes
  - `terminate_many(pids)` - Convenience wrapper for SIGTERM batch
  - `force_kill_many(pids)` - Convenience wrapper for SIGKILL batch
  - Per-PID result tracking (succeeded/failed split)
  - All PIDs validated before any signals sent
  - FFI: `sysprims_signal_send_many(pids_json, signal, result_json_out)`
  - Bindings: Go `KillMany`, TypeScript `killMany`

- **CLI: Multi-PID Kill** (`sysprims-cli`)
  - `sysprims kill <PID> <PID> ... -s <SIGNAL>` - Batch signal delivery
  - JSON output with per-PID results (`signal/v1.0.0/batch-kill-result` schema)
  - Exit codes: 0 (all success), 1 (partial), 2 (all failed)
  - Individual failures don't abort the batch

- **Go Bindings: Shared Library Mode** (`bindings/go/sysprims/`)
  - New build tag: `sysprims_shared` for dlopen/dlsym loading patterns
  - Supported platforms: macOS, Linux (glibc), Linux musl, Windows (not Windows ARM64)
  - Musl support: `-tags="musl,sysprims_shared"` for Alpine containers
  - Rpath-based runtime resolution avoids symbol collisions when linking multiple Rust staticlibs
  - CI validates musl shared mode via Alpine container job

- **Documentation**
  - New app note: `docs/appnotes/fds-validation/` - Synthetic test cases for FD inspection
  - Updated guide: `docs/guides/runaway-process-diagnosis.md` - Now includes `fds` workflow
  - New schemas: `fd-snapshot.schema.json`, `fd-filter.schema.json`, `batch-kill-result.schema.json`

### Notes

- `sysprims fds` fills the diagnostic gap noted in v0.1.8's runaway process guide (previously required external `lsof`)
- Multi-PID kill enables surgical strikes on multiple runaway processes without loops or scripts
- Together with `pstat` and `terminate-tree`, completes the "diagnose → remediate" workflow
- Go shared library mode enables Alpine/musl consumers to avoid symbol collisions when linking sysprims alongside other Rust staticlibs

## [0.1.8] - 2026-01-29

CLI tree termination release. Adds `terminate-tree` subcommand for safe, structured termination of existing process trees, plus `pstat` sampling enhancements for runaway process diagnosis.

### Added

- **CLI: `sysprims terminate-tree`** (`sysprims-cli`)
  - Terminate an existing process tree by PID with graceful-then-kill escalation
  - Identity guards: `--require-start-time-ms`, `--require-exe-path` for PID reuse protection
  - Timing control: `--grace`, `--kill-after`, `--signal`, `--kill-signal`
  - Safety: refuses PID 1, self, or parent without `--force`
  - JSON output with `tree_kill_reliability` and `warnings`

- **CLI: `pstat` Sampling Mode** (`sysprims-cli`)
  - `--sample <DURATION>`: compute CPU rate over sampling interval (e.g., `--sample 250ms`)
  - `--top <N>`: limit output to top N processes by CPU after filtering
  - Enables "what's burning CPU right now?" investigation workflow

- **Documentation**
  - New guide: `docs/guides/runaway-process-diagnosis.md`
  - Real-world walkthrough: diagnosing and terminating runaway Electron/VSCodium plugin helpers
  - Documents surgical (single PID) vs tree termination approaches
  - Notes that SIGTERM may be ignored by runaway processes; escalate to SIGKILL

### Notes

- `terminate-tree` CLI wraps the `sysprims_timeout::terminate_tree` library function (added in v0.1.6)
- Library-level footgun protections (PID 0, MAX_SAFE_PID bounds) apply; CLI adds interactive safety guards
- Future releases will add process visibility enhancements (`fds` command) for deeper investigation

## [0.1.7] - 2026-01-26

TypeScript bindings infrastructure release. Migrates from koffi FFI to Node-API (N-API) native addon, enabling Alpine/musl support.

### Changed

- **TypeScript Bindings Architecture** (`bindings/typescript/sysprims/`)
  - Migrated from koffi + vendored C-ABI shared libraries to Node-API (N-API) native addon via napi-rs
  - Prebuilt `.node` binaries loaded from `native/` directory instead of `_lib/<platform>/libsysprims_ffi.*`
  - FFI returns `{ code, json?, message? }` internally; JS layer throws `SysprimsError` with same numeric error codes

### Added

- **Linux musl/Alpine Support** (TypeScript)
  - TypeScript bindings now work in Alpine-based containers
  - Removes the "glibc-only" limitation from v0.1.4-v0.1.6

### Notes

- **No API Changes**: Existing TypeScript imports and function calls remain unchanged
- **Build from Source**: Installing from git checkout requires Rust toolchain and C/C++ build tools
- **npm Prebuilds**: Deferred to future release pending consumer validation

## [0.1.6] - 2026-01-25

Supervisor and job manager primitives release. Teams building long-running supervisors can now spawn kill-tree-safe jobs, detect PID reuse, and cleanly terminate process trees.

### Added

- **Process Identity Fields** (`sysprims-proc`)
  - `start_time_unix_ms` and `exe_path` fields in `ProcessInfo` for PID reuse detection
  - Best-effort on all platforms: Linux (`/proc`), macOS (`libproc`), Windows (`Win32`)
  - Enables supervisors to verify a PID still refers to the expected process

- **Spawn In Group** (`sysprims-timeout`)
  - `spawn_in_group(config: SpawnInGroupConfig) -> SpawnInGroupResult`
  - Creates child in new process group (Unix) or Job Object (Windows)
  - Returns `pid`, `pgid` (Unix only; null on Windows), and `tree_kill_reliability`
  - FFI: `sysprims_spawn_in_group(config_json, *result_json_out)`
  - Bindings: Go `SpawnInGroup`, TypeScript `spawnInGroup`

- **Wait PID With Timeout** (`sysprims-proc`)
  - `wait_pid(pid, timeout) -> WaitPidResult`
  - Best-effort polling for arbitrary PIDs (not just children)
  - Returns `exited`, `timed_out`, `exit_code`, `warnings`
  - FFI: `sysprims_proc_wait_pid(pid, timeout_ms, *json_out)`
  - Bindings: Go `WaitPID`, TypeScript `waitPID`

- **Terminate Tree** (`sysprims-timeout`)
  - `terminate_tree(pid, config) -> TerminateTreeResult`
  - Graceful signal, wait, escalate to kill—as a standalone primitive
  - Independent of `run_with_timeout` for use with externally-spawned processes
  - FFI: `sysprims_terminate_tree(pid, json_config, *json_out)`
  - Bindings: Go `TerminateTree`, TypeScript `terminateTree`

- **Documentation**
  - Job Object registry documentation for Windows platform behavior

### Changed

- `ProcessInfo` schema updated to include optional `start_time_unix_ms` and `exe_path` fields
- Go and TypeScript bindings updated for new primitives

## [0.1.5] - 2026-01-24

TypeScript bindings parity release for proc/ports/signals. Node.js developers now have access to process inspection, port mapping, and signal APIs.

### Added

- **TypeScript Bindings Parity** (`bindings/typescript/sysprims/`)
  - `processList(filter?)` - list processes with optional filtering
  - `listeningPorts(filter?)` - port-to-PID mapping
  - `signalSend(pid, signal)` - send signal to process
  - `signalSendGroup(pgid, signal)` - send signal to process group (Unix)
  - `terminate(pid)` - graceful termination (SIGTERM / TerminateProcess)
  - `forceKill(pid)` - immediate kill (SIGKILL / TerminateProcess)
  - Full TypeScript type definitions for all schemas

- **CI Improvements**
  - Separated binding validation from release validation workflow
  - Clarified Go module tagging requirements in validate-release

### Changed

- **Go Prebuilt Libraries**
  - Updated all 7 platform libraries for v0.1.5

### Fixed

- **Windows Signal Tests**
  - Signal tests now use deterministic patterns: reject pid=0, spawn-and-kill for terminate/forceKill
  - Eliminates flakiness from arbitrary PIDs that may exist on CI runners

[Unreleased]: https://github.com/3leaps/sysprims/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/3leaps/sysprims/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/3leaps/sysprims/compare/v0.1.20...v0.2.0
[0.1.20]: https://github.com/3leaps/sysprims/compare/v0.1.19...v0.1.20
[0.1.19]: https://github.com/3leaps/sysprims/compare/v0.1.18...v0.1.19
[0.1.18]: https://github.com/3leaps/sysprims/compare/v0.1.17...v0.1.18
[0.1.17]: https://github.com/3leaps/sysprims/compare/v0.1.16...v0.1.17
[0.1.16]: https://github.com/3leaps/sysprims/compare/v0.1.15...v0.1.16
[0.1.15]: https://github.com/3leaps/sysprims/compare/v0.1.14...v0.1.15
[0.1.14]: https://github.com/3leaps/sysprims/compare/v0.1.13...v0.1.14
[0.1.13]: https://github.com/3leaps/sysprims/compare/v0.1.12...v0.1.13
[0.1.12]: https://github.com/3leaps/sysprims/compare/v0.1.11...v0.1.12
[0.1.11]: https://github.com/3leaps/sysprims/compare/v0.1.10...v0.1.11
[0.1.10]: https://github.com/3leaps/sysprims/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/3leaps/sysprims/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/3leaps/sysprims/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/3leaps/sysprims/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/3leaps/sysprims/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/3leaps/sysprims/compare/v0.1.4...v0.1.5
