# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** This file maintains the latest 10 releases in reverse chronological order.
> Older releases are archived in `docs/releases/`.

## [Unreleased]

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

[Unreleased]: https://github.com/3leaps/sysprims/compare/v0.1.14...HEAD
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
