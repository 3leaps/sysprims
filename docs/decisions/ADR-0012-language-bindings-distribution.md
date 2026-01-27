# ADR-0012: Language Bindings Distribution

> **Status**: Accepted
> **Date**: 2026-01-15
> **Authors**: devlead, Architecture Council

## Context

sysprims provides a C-ABI FFI layer (ADR-0004) that enables bindings for multiple languages. We need to decide:

1. **Repository structure**: In-repo bindings vs. separate repositories
2. **Distribution model**: Prebuilt libraries vs. source-only
3. **Platform support**: Which platforms to support and how
4. **Version synchronization**: How binding versions relate to core versions
5. **CI/CD strategy**: How to build and test across platforms

### Current State

- FFI library builds for multiple platform targets (see `docs/standards/platform-support.md`)
- C header generated via cbindgen at release time
- No language bindings exist yet
- seekable-zstd provides a working reference implementation

### Target Languages

| Language | FFI Mechanism | Priority |
|----------|---------------|----------|
| Go | CGo | v0.1.x (phased) |
| Python | cffi/PyO3 | v0.1.x (phased) |
| TypeScript | napi-rs (Node-API addon) | v0.1.x (phased) |

**Release intent**: v0.1.x is considered feature-complete only once Go, Python, and
TypeScript bindings are all available. Patch releases may be used to phase
delivery across languages while keeping the same major/minor line.

## Decision

### 1. In-Repo Bindings

Language bindings live in the sysprims repository under `bindings/<language>/`.

```
sysprims/
├── bindings/
│   ├── go/
│   │   └── sysprims/
│   ├── python/
│   └── typescript/
├── crates/
├── ffi/
└── ...
```

**Rationale**: Single source of truth, atomic versioning, simplified CI.

### 2. Prebuilt Static Libraries

Prebuilt static libraries are committed to the repository at each release tag.

```
bindings/go/sysprims/lib/
├── darwin-arm64/
│   └── libsysprims_ffi.a
├── linux-amd64/
│   └── libsysprims_ffi.a
├── linux-amd64-musl/
│   └── libsysprims_ffi.a
├── linux-arm64/
│   └── libsysprims_ffi.a
├── linux-arm64-musl/
│   └── libsysprims_ffi.a
├── windows-amd64/
│   └── libsysprims_ffi.a
└── local/            # Gitignored, for local development
```

**Rationale**:
- Users don't need Rust toolchain to use bindings
- Go modules work via `go get` without build steps
- Reproducible builds from tagged commits
- Follows seekable-zstd proven pattern

### 3. Platform Support Matrix

| Platform | Architecture | C Library | Go | Python | TypeScript |
|----------|--------------|-----------|:--:|:------:|:----------:|
| Linux | x86_64 | glibc 2.17+ | ✅ | ✅ | ✅ |
| Linux | x86_64 | musl | ✅ | ❌ | ✅ |
| Linux | aarch64 | glibc 2.17+ | ✅ | ✅ | ✅ |
| Linux | aarch64 | musl | ✅ | ❌ | ✅ |
| macOS | x86_64 | - | ❌ | ❌ | ❌ |
| macOS | aarch64 | - | ✅ | ✅ | ✅ |
| Windows | x86_64 | GNU (Go) / MSVC (shared) | ✅ | ✅ | ✅ |

| Windows | arm64 | MSVC | ❌ | ❌ | ✅ |

**Notes**:
- Linux musl is supported by Go and TypeScript (via Node-API addon); Python musl is deferred.
- macOS x64 (Intel) is not supported as of v0.1.7.
- Windows arm64 Go bindings are not supported (CGo requires MinGW; arm64 requires llvm-mingw).
- Windows arm64 Python bindings are not yet supported.

### 4. Library Naming Convention

sysprims supports multiple binding consumers with different toolchain needs.

| Consumer | Platform | Primary Artifact | Notes |
|----------|----------|------------------|-------|
| Go (cgo) | Linux/macOS | `libsysprims_ffi.a` | Static linking |
| Go (cgo) | Windows | `libsysprims_ffi.a` | Built for `x86_64-pc-windows-gnu` (MinGW) |
| Python (runtime load) | Linux | `libsysprims_ffi.so` | Shared library |
| Python (runtime load) | macOS | `libsysprims_ffi.dylib` | Shared library |
| Python (runtime load) | Windows | `sysprims_ffi.dll` | Built for `x86_64-pc-windows-msvc` |
| TypeScript (Node-API addon) | All | `sysprims.<platform>.node` | Published as npm platform packages |

### 5. CGo Link Flags by Platform

| Platform | LDFLAGS |
|----------|---------|
| Linux (glibc) | `-lm -lpthread -ldl` |
| Linux (musl) | `-lm -lpthread` |
| macOS | `-lm -lpthread` |
| Windows | `-lws2_32 -luserenv -lbcrypt` |

**Rationale**: These are Rust std library dependencies for each platform.

### 6. Version Synchronization

- Binding versions track the core sysprims version (same X.Y.Z)
- ABI compatibility is checked at runtime via `sysprims_abi_version()`

```go
// Go example
if sysprims.ABIVersion() != expectedABI {
    return errors.New("ABI version mismatch")
}
```

### 6.1 Go Submodule Tagging (Required)

sysprims uses a Go module in a subdirectory:

- Module path: `github.com/3leaps/sysprims/bindings/go/sysprims`

Per the Go module versioning rules for repositories containing modules in subdirectories,
we MUST create an additional tag that is prefixed by the module subdirectory:

- Canonical repo tag: `vX.Y.Z`
- Go module tag: `bindings/go/sysprims/vX.Y.Z`

Rules:
- Both tags MUST point at the same git commit.
- The Go module tag exists solely to let `go get` resolve a semantic version for the submodule.
- If the Go module tag is missing, consumers will fall back to pseudo-versions.

Notes on other bindings:
- Python (PyPI) and TypeScript (npm) do not use git tags for version resolution in the same way.
  They should still use the same X.Y.Z version numbers for ecosystem consistency, but they do not
  require path-prefixed git tags.

### 7. Local Development Flow

Developers building from source use `lib/local/` directory.

Go is an exception in that a non-module repo root breaks common tooling patterns (`./...` in CI, golangci-lint defaults, and multi-tool runners like goneat). We intentionally add a minimal root `go.mod` + `go.work` plus a tiny placeholder package (`internal/gowork/`) so repo-root Go tooling can run without turning sysprims into a Go-first repository.

Python/TypeScript bindings do not require equivalent repo-root module scaffolding for version resolution. However, as we expanded repo-root tooling (goneat `--package-mode`) to include TypeScript checks, we also add a minimal repo-root `package.json` so npm-based tool invocation does not fail with ENOENT when executed from the repository root.

```bash
# Build FFI for current platform
make build-local-go

# Run Go tests
make go-test
```

CGo LDFLAGS search order: `lib/local/<platform>` first, then `lib/<platform>`.

### 8. Release Flow

1. Run the Go bindings prep workflow (`.github/workflows/go-bindings.yml`) to generate prebuilt libs and open a PR
2. Merge the PR so the prebuilt libs are present under `bindings/go/sysprims/lib/`
3. Create the canonical release tag `vX.Y.Z` pointing at that commit (tags remain immutable)
4. Create the Go module tag `bindings/go/sysprims/vX.Y.Z` pointing at the same commit
5. Push tags to trigger the release workflow (`.github/workflows/release.yml`)
6. Go users can `go get github.com/3leaps/sysprims/bindings/go/sysprims@vX.Y.Z` without pseudo-versions

### 9. Module Path Convention

| Language | Module/Package Path |
|----------|---------------------|
| Go | `github.com/3leaps/sysprims/bindings/go/sysprims` |
| Python | `sysprims` (PyPI) |
| TypeScript | `@3leaps/sysprims` (npm) |

### 10. Platform-Specific Behavior Documentation

Each binding must clearly document platform differences:

```go
// KillGroup sends a signal to a process group.
//
// Platform notes:
// - Unix: Calls killpg(pgid, signal)
// - Windows: Returns ErrNotSupported (process groups not supported)
func KillGroup(pgid uint32, signal int) error
```

## Consequences

### Positive

- Single repository for all sysprims code
- No Rust toolchain required for binding users
- Atomic releases (core + bindings in sync)
- Proven pattern from seekable-zstd
- Go modules work out of the box

### Negative

- Repository size increases with prebuilt libs (~100MB total)
- Must rebuild all platforms for each release
- Linux musl users limited to Go bindings initially

### Neutral

- Header committed to bindings directory (not ffi/)
- Local development requires initial `make build-local-go`
- CI must test bindings on all target platforms

## Alternatives Considered

### Alternative 1: Separate Repositories

Each language binding in its own repository (e.g., `sysprims-go`, `sysprims-py`).

**Rejected**:
- Version synchronization complexity
- Multiple CI configurations
- Harder to maintain consistency
- seekable-zstd moved away from this pattern

### Alternative 2: Source-Only Distribution

Users build FFI library from source.

**Rejected**:
- Requires Rust toolchain for all binding users
- Go modules don't support build steps
- Poor developer experience
- Cross-compilation complexity

### Alternative 3: Dynamic Libraries Only

Ship `.so`/`.dylib`/`.dll` instead of static libraries.

**Rejected**:
- Runtime dependency management
- Path configuration complexity
- Static linking preferred for deployment simplicity

### Alternative 4: Central lib/ Directory

Single `lib/` at repo root shared by all bindings.

**Rejected**:
- Go modules require libs relative to go.mod
- Each binding may need different platforms
- seekable-zstd pattern puts libs in binding directory

## References

- [ADR-0004: FFI Design](0004-ffi-design.md)
- [seekable-zstd Go bindings](https://github.com/3leaps/seekable-zstd/tree/main/bindings/go)
- [CGo documentation](https://pkg.go.dev/cmd/cgo)
- [Go Modules Reference](https://go.dev/ref/mod)
