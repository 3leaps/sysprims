---
title: "Platform Support Matrix"
description: "Canonical reference for supported platforms across all artifacts"
author: "OpenCode"
author_of_record: "Dave Thompson <dave.thompson@3leaps.net>"
supervised_by: "@3leapsdave"
date: "2026-01-27"
status: "active"
---

# Platform Support Matrix

This standard defines the canonical set of supported platforms for all sysprims artifacts.
All CI/CD workflows, language bindings, and release assets MUST conform to this matrix.

## Supported Platforms

| Platform            | Rust Target                                                    | Go GOOS/GOARCH       | Node Platform      | Status                               |
| ------------------- | -------------------------------------------------------------- | -------------------- | ------------------ | ------------------------------------ |
| Linux x64 (glibc)   | `x86_64-unknown-linux-gnu`                                     | `linux/amd64`        | `linux-x64-gnu`    | **Supported**                        |
| Linux x64 (musl)    | `x86_64-unknown-linux-musl`                                    | `linux/amd64` (musl) | `linux-x64-musl`   | **Supported**                        |
| Linux arm64 (glibc) | `aarch64-unknown-linux-gnu`                                    | `linux/arm64`        | `linux-arm64-gnu`  | **Supported**                        |
| Linux arm64 (musl)  | `aarch64-unknown-linux-musl`                                   | `linux/arm64` (musl) | `linux-arm64-musl` | **Supported**                        |
| macOS arm64         | `aarch64-apple-darwin`                                         | `darwin/arm64`       | `darwin-arm64`     | **Supported**                        |
| Windows x64         | `x86_64-pc-windows-msvc` (CLI) / `x86_64-pc-windows-gnu` (FFI) | `windows/amd64`      | `win32-x64-msvc`   | **Supported**                        |
| Windows arm64       | `aarch64-pc-windows-msvc` (CLI) / `aarch64-pc-windows-gnullvm` (FFI) | `windows/arm64`      | `win32-arm64-msvc` | **Supported**                        |

**Note on Windows arm64 Go bindings**: Go cgo requires a GNU-ABI C toolchain on Windows. For arm64 this is
provided by [llvm-mingw](https://github.com/mstorsjo/llvm-mingw) (via the Rust `aarch64-pc-windows-gnullvm`
target), since msys2/MinGW-w64 does not ship an aarch64 toolchain. Consumers building Go code against the
arm64 prebuilt must have llvm-mingw installed and the `aarch64-w64-mingw32-gcc` driver on `PATH`. Available
since v0.1.16.

## Explicitly Unsupported Platforms

| Platform           | Rust Target              | Reason                                                | Since  |
| ------------------ | ------------------------ | ----------------------------------------------------- | ------ |
| macOS x64 (Intel)  | `x86_64-apple-darwin`    | Intel Macs are end-of-life; Apple Silicon is standard | v0.1.7 |
| Linux x86 (32-bit) | `i686-unknown-linux-gnu` | Legacy; no modern use case                            | v0.1.0 |

**Note on macOS x64**: macOS x64 is not supported for sysprims artifacts as of v0.1.7. New adopters should use
Apple Silicon (arm64) Macs.

## Artifact Coverage

### CLI Binaries

Release assets include CLI binaries for all supported platforms:

- `sysprims-<version>-linux-amd64.tar.gz`
- `sysprims-<version>-linux-amd64-musl.tar.gz`
- `sysprims-<version>-linux-arm64.tar.gz`
- `sysprims-<version>-linux-arm64-musl.tar.gz`
- `sysprims-<version>-darwin-arm64.tar.gz`
- `sysprims-<version>-windows-amd64.zip`
- `sysprims-<version>-windows-arm64.zip`

In addition, the release pipeline still produces a legacy artifact that is **not an officially
supported platform**:

- `sysprims-<version>-darwin-amd64.tar.gz` — retained for backward compat only; scheduled for
  removal. Intel Mac consumers should plan migration to Apple Silicon. No guarantees of bug
  fixes or new feature coverage.

### FFI Libraries (Go Bindings)

Static libraries committed to `bindings/go/sysprims/lib/`:

- `darwin-arm64/libsysprims_ffi.a`
- `linux-amd64/libsysprims_ffi.a`
- `linux-amd64-musl/libsysprims_ffi.a`
- `linux-arm64/libsysprims_ffi.a`
- `linux-arm64-musl/libsysprims_ffi.a`
- `windows-amd64/libsysprims_ffi.a` (msys2/MinGW-w64, `x86_64-pc-windows-gnu`)
- `windows-arm64/libsysprims_ffi.a` (llvm-mingw, `aarch64-pc-windows-gnullvm`, since v0.1.16)
- `darwin-amd64/libsysprims_ffi.a` — legacy, not officially supported (see CLI section above)

### TypeScript N-API Prebuilds

Platform packages published to npm (when enabled):

- `@3leaps/sysprims-linux-x64-gnu`
- `@3leaps/sysprims-linux-x64-musl`
- `@3leaps/sysprims-linux-arm64-gnu`
- `@3leaps/sysprims-linux-arm64-musl`
- `@3leaps/sysprims-darwin-arm64`
- `@3leaps/sysprims-win32-x64-msvc`
- `@3leaps/sysprims-win32-arm64-msvc`

Runtime support for the TypeScript binding covers Node.js >= 18 and Bun >= 1.3.
Bun support is verified against the same Node-API binding surface; platform
coverage remains governed by the prebuild target list above.

## CI Runner Matrix

### GitHub Actions Runners

| Platform      | Runner                      | Notes                       |
| ------------- | --------------------------- | --------------------------- |
| Linux x64     | `ubuntu-latest`             | Default glibc builds        |
| Linux arm64   | `ubuntu-latest-arm64-s`     | Native arm64 builds         |
| macOS arm64   | `macos-14`                  | Apple Silicon               |
| Windows x64   | `windows-latest`            | MSVC for CLI, MinGW for FFI |
| Windows arm64 | `windows-latest-arm64-s`    | MSVC for CLI/TS; llvm-mingw for Go FFI |
| Alpine/musl   | `ubuntu-latest` + container | `node:20-alpine` or custom  |

### Cross-Compilation

Zig is used for cross-compiling Linux targets on `ubuntu-latest`:

- `x86_64-unknown-linux-gnu` with `--zig-abi-suffix 2.17` (glibc baseline)
- `x86_64-unknown-linux-musl` with `--zig`
- `aarch64-unknown-linux-musl` with `--zig`

Native arm64-gnu builds are done on `ubuntu-latest-arm64-s` for reliability.

## Validation Checklist

Before any release, verify:

- [ ] All 7 supported platforms have artifacts (Go bindings: 7 platforms including Windows arm64 via llvm-mingw)
- [ ] No unsupported platform artifacts are included
- [ ] CI workflows reference correct runners
- [ ] Package configurations (napi, cgo) match this matrix
- [ ] Release notes document any platform support changes

## Updating This Standard

Changes to platform support require:

1. Update this document
2. Update all affected workflows (`.github/workflows/*.yml`)
3. Update binding configurations:
   - `bindings/typescript/sysprims/package.json` (napi triples)
   - `bindings/go/sysprims/cgo_*.go` (build tags)
4. Update `docs/guides/language-bindings.md`
5. Document in release notes

## References

- [ADR-0007: Platform Abstraction Strategy](../decisions/ADR-0007-platform-abstraction.md)
- [Language Bindings Guide](../guides/language-bindings.md)
- [Release Asset Policy](release-asset-policy.md)
