# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** This file maintains the latest 10 releases in reverse chronological order.
> Older releases are archived in `docs/releases/`.

## [Unreleased]

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

[Unreleased]: https://github.com/3leaps/sysprims/compare/v0.1.5...HEAD
[0.1.5]: https://github.com/3leaps/sysprims/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/3leaps/sysprims/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/3leaps/sysprims/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/3leaps/sysprims/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/3leaps/sysprims/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/3leaps/sysprims/releases/tag/v0.1.0
