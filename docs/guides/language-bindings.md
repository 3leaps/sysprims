# Language Bindings Guide

This guide covers building and using sysprims language bindings (Go, Python, TypeScript).

## Overview

sysprims provides language bindings via prebuilt FFI libraries:

- **Go**: Static libraries (`libsysprims_ffi.a`) linked at compile time
- **TypeScript**: Shared libraries (`.so`/`.dylib`/`.dll`) loaded at runtime via koffi

Each binding ships with prebuilt libraries for all supported platforms and provides idiomatic API for the target language.

## Platform Matrix

| Platform | Rust Target | Library Name | Linker Flags |
|----------|-------------|--------------|--------------|
| Linux x64 (glibc) | `x86_64-unknown-linux-gnu` | `libsysprims_ffi.a` | `-lm -lpthread -ldl` |
| Linux x64 (musl) | `x86_64-unknown-linux-musl` | `libsysprims_ffi.a` | `-lm -lpthread` |
| Linux arm64 (glibc) | `aarch64-unknown-linux-gnu` | `libsysprims_ffi.a` | `-lm -lpthread -ldl` |
| Linux arm64 (musl) | `aarch64-unknown-linux-musl` | `libsysprims_ffi.a` | `-lm -lpthread` |
| macOS x64 | `x86_64-apple-darwin` | `libsysprims_ffi.a` | `-lm -lpthread` |
| macOS arm64 | `aarch64-apple-darwin` | `libsysprims_ffi.a` | `-lm -lpthread` |
| Windows x64 | `x86_64-pc-windows-gnu` | `libsysprims_ffi.a` | `-lws2_32 -luserenv -lbcrypt` |

## Windows: MinGW Requirement (Go Bindings)

**Important**: Go bindings on Windows use the GNU target (`x86_64-pc-windows-gnu`), not MSVC.

### Why MinGW for Go?

Go's CGo requires MinGW (GCC) on Windows. MSVC-produced `.lib` files are not compatible with MinGW's linker:

- CGo uses MinGW/GCC toolchain on Windows
- MSVC `.lib` files use different ABI/format than GNU `.a` files
- Mixing MSVC static libs with MinGW linker fails

### What This Means for Go Users

- The FFI library is `libsysprims_ffi.a` (not `.lib`) on Windows
- Go binaries built with these bindings are **native Windows executables**
- The Go binary does NOT require MinGW at runtime
- The MinGW requirement only affects the build toolchain

### TypeScript on Windows

TypeScript bindings use MSVC-built shared libraries (`sysprims_ffi.dll`). No MinGW is required for TypeScript users.

### Licensing

MinGW-w64 runtime licensing is GPL-free:

| Component | License | Static Link Safe? |
|-----------|---------|-------------------|
| MinGW-w64 runtime | ZPL / Public Domain / BSD | ✅ Yes |
| Wine-imported headers | LGPL | ✅ Headers only - no effect |
| GCC compiler | GPL | ✅ Output not covered |

No GPL license toxicity issues with static linking.

## Go Bindings

For port-to-process mapping (listening ports), see `docs/guides/port-bindings-getting-started.md`.

### Local Development

```bash
# Build FFI for your platform
make build-local-go

# Run tests
make go-test
```

### Using Prebuilt Libraries

Prebuilt libraries are committed to the repository at release tags:

```
bindings/go/sysprims/lib/
├── darwin-amd64/libsysprims_ffi.a
├── darwin-arm64/libsysprims_ffi.a
├── linux-amd64/libsysprims_ffi.a
├── linux-amd64-musl/libsysprims_ffi.a
├── linux-arm64/libsysprims_ffi.a
├── linux-arm64-musl/libsysprims_ffi.a
└── windows-amd64/libsysprims_ffi.a
```

### CGo Configuration

Each platform has a dedicated CGo file with correct build tags and linker flags:

```go
//go:build darwin && arm64

package sysprims

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib/local/darwin-arm64 -L${SRCDIR}/lib/darwin-arm64 -lsysprims_ffi -lm -lpthread
#include "sysprims.h"
*/
import "C"
```

The `lib/local/` path is checked first (for development), then `lib/<platform>/` (prebuilt).

## CI/CD Integration

### CI: Testing Go Bindings

Note: sysprims is a Rust repo with Go bindings in a submodule (`bindings/go/sysprims`).
We keep a minimal root `go.mod` plus a `go.work` that lists `bindings/go/sysprims` so
repo-root tooling (e.g. goneat / golangci-lint) can lint and typecheck the Go module.

We also include a tiny placeholder Go package under `internal/gowork/` so repo-root
`./...` patterns resolve to at least one package.

For TypeScript bindings, we keep a minimal repo-root `package.json` as a tooling shim.
This is not a published npm package; it exists so repo-root tools that invoke npm (e.g.
goneat in `--package-mode`) do not error when run from the repository root.

The CI workflow builds the FFI library and runs Go tests on all platforms:

```yaml
# .github/workflows/ci.yml
test-go:
  strategy:
    matrix:
      include:
        - os: ubuntu-latest
        - os: macos-latest
        - os: windows-latest  # Uses MinGW via msys2/setup-msys2
```

### Release: Updating Prebuilt Libraries

Prebuilt libraries must be present in the repository at the commit a tag points to
so `go get` works without requiring Rust.

The Go bindings prep workflow builds artifacts and creates a PR with updated prebuilt libs:

1. Builds FFI libraries for all 7 platforms
2. Creates a PR with updated prebuilt libs in `bindings/go/sysprims/lib/`
3. PR is reviewed and merged BEFORE tagging so `go get` works at the release tag

After the PR is merged, create the release tag so it points at the commit that contains
the binding artifacts (tags remain immutable; if you already tagged, publish a patch
version that includes the merged artifacts).

### Go Submodule Tags (Required)

Because the Go module lives in a subdirectory (`bindings/go/sysprims`), Go expects a
path-prefixed tag for semantic version resolution.

For every release `vX.Y.Z`, create BOTH tags pointing at the same commit:

- `vX.Y.Z`
- `bindings/go/sysprims/vX.Y.Z`

This is required so consumers can do:

```bash
go get github.com/3leaps/sysprims/bindings/go/sysprims@vX.Y.Z
```

and get a proper semantic version instead of a pseudo-version.

See `docs/architecture/adr/0012-language-bindings-distribution.md` for the policy.

## TypeScript Bindings

TypeScript bindings use [koffi](https://koffi.dev/) to call the sysprims C-ABI shared library.

### Platform Support

| Platform | Status |
|----------|--------|
| Linux x64 (glibc) | Supported |
| Linux arm64 (glibc) | Supported |
| macOS x64 | Supported |
| macOS arm64 | Supported |
| Windows x64 | Supported |
| Linux musl (Alpine) | Not supported |

**Note:** TypeScript bindings require glibc. Linux musl (Alpine) is not supported due to glibc dependencies in koffi.

### Installation

```bash
npm install @3leaps/sysprims
```

### API Surface

The TypeScript bindings provide parity with Go bindings:

| Function | Description |
|----------|-------------|
| `procGet(pid)` | Get process info by PID |
| `processList(filter?)` | List processes with optional filtering |
| `listeningPorts(filter?)` | Map listening ports to processes |
| `selfPGID()` | Get current process group ID (Unix) |
| `selfSID()` | Get current session ID (Unix) |
| `signalSend(pid, signal)` | Send signal to process |
| `signalSendGroup(pgid, signal)` | Send signal to process group (Unix) |
| `terminate(pid)` | Graceful termination |
| `forceKill(pid)` | Immediate kill |

### Filter Conventions

Filter fields use **snake_case** to match FFI/schema conventions directly:

```typescript
// ProcessFilter
const filter = {
  name_contains: "nginx",    // substring match
  cpu_above: 50,             // percentage
  memory_above_kb: 100000    // kilobytes
};

// PortFilter
const portFilter = {
  protocol: "tcp",
  local_port: 8080
};
```

### How It Works

At load time, the binding:

1. Detects the current platform (`process.platform` + `process.arch`)
2. Loads the appropriate shared library from `_lib/<platform>/`
3. Verifies ABI version matches expected value
4. Exposes typed functions to JavaScript

### Local Development

```bash
# Build the shared FFI library for your platform first
make build-local-ffi-shared

# Then build and test the TypeScript bindings
cd bindings/typescript/sysprims
npm install
npm run build
npm test
```

## Adding New Features

When adding new FFI functions:

1. **Rust FFI**: Add function to `ffi/sysprims-ffi/src/`
2. **Regenerate header**: `make cbindgen` or `make header-go`
3. **Go wrapper**: Add wrapper function in appropriate Go file
4. **Tests**: Add tests in `sysprims_test.go`
5. **Documentation**: Update this guide if platform-specific behavior

## Troubleshooting

### "undefined reference" on Linux

Missing system libraries. Ensure linker flags include `-ldl` for glibc targets.

### Windows build fails with MSVC errors

Ensure you're using the GNU target (`x86_64-pc-windows-gnu`) and MinGW toolchain.

### CGo can't find library

Check that the library is in `lib/local/<platform>/` or `lib/<platform>/`.
Verify build tags match your platform.

## References

- [ADR-0004: FFI Design](../architecture/adr/0004-ffi-design.md)
- [ADR-0012: Language Bindings Distribution](../architecture/adr/0012-language-bindings-distribution.md)
