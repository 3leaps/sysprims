# ADR-0014: FFI Artifact Groups and Binding Consumers

> **Status**: Proposed
> **Date**: 2026-01-21
> **Authors**: entarch, ffiarch

## Context

sysprims is a Rust library with multiple distribution endpoints:

- **CLI** (`sysprims`) distributed as platform archives
- **C-ABI FFI** (`sysprims-ffi`) distributed as a release bundle
- **Go bindings** (`bindings/go/sysprims`) distributed as a Go submodule with committed prebuilt static libs
- **TypeScript bindings** (planned) distributed as an npm package
- **Python bindings** (planned) distributed as wheels/sdist

These consumers do not all consume the same binary format:

- Go (cgo) consumes **static libraries** and has Windows toolchain constraints.
- Node.js and Python (when using dynamic loading strategies such as koffi/cffi) consume **shared libraries**.

On Windows, ecosystem expectations differ:

- Go/cgo requires a GNU-compatible toolchain and prefers `x86_64-pc-windows-gnu` artifacts.
- Node.js and Python ecosystems are best served by MSVC-built shared libraries (`x86_64-pc-windows-msvc`).

We need an explicit, stable artifact strategy that supports multiple endpoints without ambiguity.

## Decision

### 1) Define Artifact Groups

We standardize the sysprims FFI deliverables into explicit artifact groups.

1. **FFI static** (link-time)
   - Intended consumers: Go (cgo), other static link consumers
   - Output:
     - Linux/macOS: `libsysprims_ffi.a`
     - Windows (GNU): `libsysprims_ffi.a`

2. **FFI shared** (runtime loading)
   - Intended consumers: TypeScript (koffi), Python (cffi), other runtime `dlopen`/`LoadLibrary` consumers
   - Output:
     - Linux: `libsysprims_ffi.so`
     - macOS: `libsysprims_ffi.dylib`
     - Windows (MSVC): `sysprims_ffi.dll`

3. **FFI header** (binding surface)
   - Intended consumers: all language bindings
   - Output: `sysprims.h` generated via cbindgen

### 2) Windows Toolchain Split (By Consumer)

Windows artifacts are produced in two lanes with different goals:

- **Go lane (Windows GNU)**:
  - Target: `x86_64-pc-windows-gnu`
  - Output: `libsysprims_ffi.a` (static)
  - Rationale: cgo uses MinGW/GCC conventions; this is the only broadly reliable path.

- **Shared-lib lane (Windows MSVC)**:
  - Target: `x86_64-pc-windows-msvc`
  - Output: `sysprims_ffi.dll` (shared)
  - Rationale: Node/Python on Windows are MSVC-first; shipping MSVC binaries avoids MinGW runtime/tooling assumptions.

We explicitly do **not** ship MinGW-built shared libraries for Node/Python.

### 3) Release Packaging

We continue to publish a single `sysprims-ffi-<version>-libs.tar.gz` release asset, but it MUST include
both static and shared groups.

Archive layout (inside the tarball):

```
include/
  sysprims.h
lib/
  <platform>/
    static/
      <static lib>
    shared/
      <shared lib>
```

Platform identifiers match existing sysprims conventions:
- `linux-amd64`, `linux-arm64`, `darwin-amd64`, `darwin-arm64`, `windows-amd64`

### 4) Binding Packaging Responsibilities

- Go bindings continue to vendor **FFI static** libs into:
  - `bindings/go/sysprims/lib/<platform>/libsysprims_ffi.a`

- TypeScript bindings will vendor **FFI shared** libs into:
  - `bindings/typescript/sysprims/_lib/<platform>/<shared lib>`
  - The loader MUST validate `sysprims_abi_version()` and fail fast on mismatch.

- Python bindings (future) will consume **FFI shared** libs as wheel platform assets.

### 5) Lessons Learned (seekable-zstd)

We follow the proven split used by seekable-zstd:

- Go distributes Windows libs using `x86_64-pc-windows-gnu` static `.a` artifacts.
- Node distributes Windows binaries as MSVC (`*-win32-x64-msvc.*`).

sysprims adopts the same consumer-driven toolchain split.

## Consequences

### Positive

- Supports TypeScript/Python without requiring Rust toolchains.
- Makes platform/toolchain differences explicit (no silent compatibility assumptions).
- Preserves Go's existing consumer story (including musl variants).
- Aligns with the repo's integrity model (ADR-0013): new assets are covered by signed checksums.

### Negative

- CI matrix expands (additional Windows MSVC build lane).
- Release bundle grows (shared + static artifacts).
- More surface area to document and test across platforms.

### Neutral

- This does not change the C-ABI design (ADR-0004) or schema contract principles; it changes how we ship artifacts.
- Consumers must handle `NotSupported` for platform-limited features (ADR-0007).

## Alternatives Considered

### Alternative 1: Static-only FFI artifacts

Rejected: Node/Python need runtime-loadable libraries; static-only is not workable without a native addon build.

### Alternative 2: GNU shared libraries on Windows

Rejected: adds runtime/tooling ambiguity for Node/Python consumers; MSVC is the dominant convention in those ecosystems.

### Alternative 3: Use napi-rs (native Node addon) instead of C-ABI shared libs

Deferred: we explicitly want a single C-ABI surface shared across Go/Python/TypeScript (ADR-0004) and want to
avoid a second binding strategy.

## References

- ADR-0004: FFI Design (`docs/architecture/adr/0004-ffi-design.md`)
- ADR-0007: Platform Abstraction Strategy (`docs/architecture/adr/0007-platform-abstraction.md`)
- ADR-0012: Language Bindings Distribution (`docs/architecture/adr/0012-language-bindings-distribution.md`)
- ADR-0013: Release Asset Publishing and Verification (`docs/architecture/adr/0013-release-asset-publishing-and-verification.md`)
- seekable-zstd reference implementation (Windows split: Go GNU vs Node MSVC)
