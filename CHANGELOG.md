# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** This file maintains the latest 10 releases in reverse chronological order.
> Older releases are archived in `docs/releases/`.

## [Unreleased]

### Added
- Port-to-PID lookup via `sysprims-proc` listening ports (best-effort, schema-backed)
- Go bindings: `ListeningPorts()` wrapper for port-to-process mapping

### Changed
- Release asset integrity: checksum manifests now include published headers and other sidecar assets

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

[Unreleased]: https://github.com/3leaps/sysprims/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/3leaps/sysprims/releases/tag/v0.1.0
