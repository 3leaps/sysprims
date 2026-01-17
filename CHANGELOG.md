# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** This file maintains the latest 10 releases in reverse chronological order.
> Older releases are archived in `docs/releases/`.

## [Unreleased]

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

[Unreleased]: https://github.com/3leaps/sysprims/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/3leaps/sysprims/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/3leaps/sysprims/releases/tag/v0.1.0
