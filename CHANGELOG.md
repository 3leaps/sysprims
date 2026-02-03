# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** This file maintains the latest 10 releases in reverse chronological order.
> Older releases are archived in `docs/releases/`.

## [Unreleased]

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

## [0.1.4] - 2026-01-22

TypeScript language bindings release. Node.js developers can now integrate sysprims directly.

### Added

- **TypeScript Bindings** (`bindings/typescript/sysprims/`)
  - koffi-based FFI for Node.js 18+
  - Cross-platform: linux-amd64, linux-arm64, darwin-arm64, windows-amd64
  - ABI version verification on library load
  - CI validates TypeScript bindings on all supported platforms

- **Linux ARM64 CI Coverage**
  - Added linux-arm64 runner to CI matrix for native ARM64 testing

### Changed

- **CI Platform Matrix**
  - Replaced darwin-amd64 with linux-arm64 (Intel Mac runners deprecated)
  - TypeScript CI: linux-amd64, linux-arm64, darwin-arm64, windows-amd64

### Fixed

- **Windows TypeScript Tests**
  - Cross-platform build scripts replace Unix-only shell commands
  - `npm run test:ci` now works on Windows runners

- **Parallel Test Flakiness**
  - Added atomic counter to `unique_marker()` in tree_escape tests
  - Prevents collisions when tests run in parallel with same PID/timestamp

## [0.1.3] - 2026-01-19

Go bindings infrastructure release. Prebuilt static libraries now included in release tags.

### Added

- **Go Bindings Prep Workflow** (`go-bindings.yml`)
  - Manual `workflow_dispatch` trigger to build FFI libs before tagging
  - Creates PR with prebuilt libs for all 7 platforms
  - Must be merged before release tag to ensure `go get` works

- **Release Verification Gate** (`verify-go-bindings-assets`)
  - Fails release early if Go prebuilt libs missing from tagged commit
  - Prevents publishing versions that can't link

- **Go Submodule Tagging** (ADR-0012 section 6.1)
  - Dual-tag policy: `vX.Y.Z` + `bindings/go/sysprims/vX.Y.Z`
  - Both tags point to same commit
  - Enables proper `go get` semver resolution for subdirectory modules

### Changed

- Release workflow no longer auto-generates Go bindings PR on tag push
- Go bindings prep is now a manual pre-release step
- Updated release checklist with dual-tag instructions

### Fixed

- Go bindings now actually usable via `go get` (prebuilt libs included in tags)
- v0.1.1/v0.1.2 had empty lib directories; v0.1.3 is first working Go release

### Deferred

- Python bindings (moved to v0.1.4+)
- TypeScript bindings (moved to v0.1.4+)

## [0.1.2] - 2026-01-19

Security and CI/CD maintenance release.

### Security

- **GHSA-cxww-7g56-2vh6** (High): Updated `actions/download-artifact` from `@v4` to `@v4.1.3`
  - Path traversal vulnerability in GitHub Actions artifact downloads
  - Impact: CI/CD pipeline only; no impact on library code or released binaries

### Changed

- Renamed `RELEASE_TAG` to `SYSPRIMS_RELEASE_TAG` in Makefile and release workflow
  - Prevents cross-repo confusion when working with multiple repositories
- Updated `GONEAT_VERSION` to v0.5.1

### Added

- Vulnerability scanning via goneat/grype integration
  - New `.goneat/dependencies.yaml` with license policy aligned to ADR-0001
  - Added grype to `.goneat/tools.yaml` sbom scope
  - Enables `goneat dependencies --vuln` for SBOM-based vulnerability detection

## [0.1.1] - 2026-01-17

First language bindings release. Completes the FFI surface and ships Go bindings.

### Added

- **FFI Surface** (14 functions)
  - Error handling: `sysprims_last_error_code()`, `sysprims_last_error()`, `sysprims_clear_error()`
  - Version/ABI: `sysprims_version()`, `sysprims_abi_version()`
  - Platform: `sysprims_get_platform()`
  - Memory: `sysprims_free_string()`
  - Signal: `sysprims_signal_send()`, `sysprims_signal_send_group()`, `sysprims_terminate()`, `sysprims_force_kill()`
  - Process: `sysprims_proc_list()`, `sysprims_proc_get()`, `sysprims_proc_listening_ports()`
  - Timeout: `sysprims_timeout_run()`

- **Go Bindings**: First language bindings with cross-platform support
  - Core: `Version()`, `ABIVersion()`, `Platform()`, `ClearError()`
  - Signal: `Kill()`, `Terminate()`, `ForceKill()`, `KillGroup()`
  - Process: `ProcessList()`, `ProcessGet()`, `ListeningPorts()`
  - Timeout: `RunWithTimeout()` with `TimeoutConfig`
  - Prebuilt static libraries for 7 platform targets
  - Cross-platform CI testing (Linux, macOS, Windows)

- **Listening Ports API**: Port-to-PID lookup via `sysprims-proc`
  - Rust: `listening_ports(filter: Option<&PortFilter>) -> PortBindingsSnapshot`
  - FFI: `sysprims_proc_listening_ports(filter_json, result_json_out)`
  - Go: `ListeningPorts(filter *PortFilter) (*PortBindingsSnapshot, error)`
  - Best-effort semantics with platform-appropriate warnings
  - Schema-backed JSON output (`process/v1.0.0/port-bindings`)

- **CLI Enhancements**
  - `sysprims kill -l` lists all available signals
  - `sysprims kill -l SIGNAL` prints signal number for a specific signal
  - `sysprims kill --group PID` sends signal to process group (Unix only)

- **Container Test Fixture**: Docker-based environment for privileged/dangerous tests

### Changed

- CLI version string now shows `sysprims X.Y.Z` (was `sysprims-cli X.Y.Z`)
- Release checksum manifests now include headers and sidecar assets

### Fixed

- FFI surface now complete (v0.1.0 had only scaffolding)
- CLI `kill -l` now implemented (was missing in v0.1.0)

## [0.1.0] - 2026-01-14

Initial release validating CI/CD pipeline and release signing workflow.

### Added
- **CLI Commands**
  - `sysprims timeout` - Run commands with timeout and group-by-default tree-kill
  - `sysprims kill` - Send signals to processes
  - `sysprims pstat` - Process inspection and listing

- **Libraries**
  - `sysprims-core` - Core types and platform detection
  - `sysprims-signal` - Cross-platform signal dispatch
  - `sysprims-timeout` - Timeout execution with process group management
  - `sysprims-proc` - Process enumeration
  - `sysprims-session` - Session/setsid helpers

- **FFI**
  - C header (`sysprims.h`) - Scaffolding only
  - Static libraries for all platforms

- **Platform Support**
  - Linux x64 (glibc + musl)
  - Linux arm64 (glibc + musl)
  - macOS x64
  - macOS arm64
  - Windows x64

- **Documentation**
  - Architecture Decision Records (ADRs)
  - Safety protocols for process control
  - Release checklist and signing workflow

### Known Limitations
- FFI surface is minimal (only `get_platform()` + `free_string()`)
- No language bindings (Go, Python, TypeScript)
- CLI `kill -l` not implemented

[Unreleased]: https://github.com/3leaps/sysprims/compare/v0.1.10...HEAD
[0.1.10]: https://github.com/3leaps/sysprims/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/3leaps/sysprims/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/3leaps/sysprims/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/3leaps/sysprims/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/3leaps/sysprims/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/3leaps/sysprims/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/3leaps/sysprims/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/3leaps/sysprims/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/3leaps/sysprims/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/3leaps/sysprims/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/3leaps/sysprims/releases/tag/v0.1.0
