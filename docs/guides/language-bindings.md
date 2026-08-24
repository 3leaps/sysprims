# Language Bindings Guide

This guide covers building and using sysprims language bindings (Go, Python, TypeScript).

## Overview

sysprims provides language bindings via prebuilt FFI libraries:

- **Go**: Static libraries (`libsysprims_ffi.a`) linked at compile time
- **TypeScript**: Node-API (N-API) native addon (`.node`) loaded by Node.js

Each binding ships with prebuilt libraries for all supported platforms and provides idiomatic API for the target language.

## Platform Matrix

See [Platform Support Matrix](../standards/platform-support.md) for the canonical reference.

| Platform            | Rust Target                  | Library Name        | Linker Flags                  |
| ------------------- | ---------------------------- | ------------------- | ----------------------------- |
| Linux x64 (glibc)   | `x86_64-unknown-linux-gnu`   | `libsysprims_ffi.a` | `-lm -lpthread -ldl`          |
| Linux x64 (musl)    | `x86_64-unknown-linux-musl`  | `libsysprims_ffi.a` | `-lm -lpthread`               |
| Linux arm64 (glibc) | `aarch64-unknown-linux-gnu`  | `libsysprims_ffi.a` | `-lm -lpthread -ldl`          |
| Linux arm64 (musl)  | `aarch64-unknown-linux-musl` | `libsysprims_ffi.a` | `-lm -lpthread`               |
| macOS arm64         | `aarch64-apple-darwin`       | `libsysprims_ffi.a` | `-lm -lpthread`               |
| Windows x64         | `x86_64-pc-windows-gnu`      | `libsysprims_ffi.a` | `-lws2_32 -luserenv -lbcrypt` |
| Windows arm64       | `aarch64-pc-windows-gnullvm` | `libsysprims_ffi.a` | `-lws2_32 -luserenv -lbcrypt` |

**Not supported**: macOS x64 (Intel Macs) - end-of-life hardware as of v0.1.7.

## Windows: Go cgo toolchain

**Important**: Go bindings on Windows use a GNU-ABI C toolchain, not MSVC. Which GNU toolchain depends on the architecture:

| Arch   | Rust target                   | Toolchain                                | Consumer install                                                      |
| ------ | ----------------------------- | ---------------------------------------- | --------------------------------------------------------------------- |
| x86_64 | `x86_64-pc-windows-gnu`       | msys2/MinGW-w64 (`mingw-w64-x86_64-gcc`) | `pacman -S mingw-w64-x86_64-gcc` in msys2                             |
| arm64  | `aarch64-pc-windows-gnullvm`  | [llvm-mingw](https://github.com/mstorsjo/llvm-mingw) (`aarch64-w64-mingw32-gcc`) | Download `*-ucrt-aarch64.zip` release, add `bin/` to PATH (since v0.1.16) |

### Why a GNU toolchain for Go?

Go's cgo driver on Windows is GCC-compatible. MSVC-produced `.lib` files use a different ABI/format than GNU `.a` files and won't link with cgo. msys2/MinGW-w64 ships the x86_64 toolchain but does not provide an aarch64 target; llvm-mingw fills that gap with an LLVM-based GNU-ABI toolchain for both architectures.

### What this means for Go users

- The FFI library is `libsysprims_ffi.a` (not `.lib`) on Windows, for both architectures.
- The Go binary you build is a native Windows executable; the GNU toolchain is only required at build time.
- To build against sysprims on Windows, install the matching toolchain and ensure the GCC driver is on `PATH`.

### TypeScript on Windows

TypeScript bindings use a Node-API native addon and do not require a GNU toolchain — they're built with MSVC on both x64 and arm64.

### Licensing

The GNU-ABI toolchains are GPL-free for our use:

| Component                 | License                   | Static Link Safe?           |
| ------------------------- | ------------------------- | --------------------------- |
| MinGW-w64 runtime         | ZPL / Public Domain / BSD | ✅ Yes                      |
| llvm-mingw runtime (LLVM) | Apache 2.0 with exception | ✅ Yes                      |
| Wine-imported headers     | LGPL                      | ✅ Headers only - no effect |
| GCC / clang compiler      | GPL / Apache 2.0          | ✅ Output not covered       |

No GPL license toxicity with static linking.

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
├── darwin-arm64/libsysprims_ffi.a
├── linux-amd64/libsysprims_ffi.a
├── linux-amd64-musl/libsysprims_ffi.a
├── linux-arm64/libsysprims_ffi.a
├── linux-arm64-musl/libsysprims_ffi.a
├── windows-amd64/libsysprims_ffi.a
└── windows-arm64/libsysprims_ffi.a
```

**Note**: macOS x64 (darwin-amd64) is not supported as of v0.1.7.

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
        - os: windows-latest # Uses MinGW via msys2/setup-msys2
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

See `docs/decisions/ADR-0012-language-bindings-distribution.md` for the policy.

## TypeScript Bindings

TypeScript bindings use a Node-API (N-API) native addon (napi-rs).

### Platform Support

See [Platform Support Matrix](../standards/platform-support.md) for the canonical reference.

| Platform            | Status    |
| ------------------- | --------- |
| Linux x64 (glibc)   | Supported |
| Linux x64 (musl)    | Supported |
| Linux arm64 (glibc) | Supported |
| Linux arm64 (musl)  | Supported |
| macOS arm64         | Supported |
| Windows x64         | Supported |
| Windows arm64       | Supported |

**Not supported**: macOS x64 (Intel Macs) - end-of-life hardware as of v0.1.7.

Runtime support covers Node.js >= 18 and Bun >= 1.3. Bun uses the same Node-API
binding surface as Node.js; prebuilt package availability still follows the
platform matrix above.

### Installation

**From git checkout / local path (current):**

When installing from a git checkout or local path, the addon is built from source:

```bash
cd bindings/typescript/sysprims
npm install
npm run build:native  # Builds the N-API addon
```

**Requirements for building from source:**

- Rust toolchain (1.88+)
- C/C++ compiler
- Node.js 18+

**From npm (future):**

When npm publishing is enabled, prebuilt platform packages will be installed automatically:

```bash
npm install @3leaps/sysprims
# Prebuilt .node binary selected based on platform
```

No build tools required when using npm prebuilds.

### API Surface

The TypeScript bindings provide parity with Go bindings:

| Function                        | Description                                                         |
| ------------------------------- | ------------------------------------------------------------------- |
| `procGet(pid)`                  | Get process info by PID (includes `start_time_unix_ms`, `exe_path`) |
| `processList(filter?)`          | List processes with optional filtering                              |
| `listeningPorts(filter?)`       | Map listening ports to processes                                    |
| `selfPGID()`                    | Get current process group ID (Unix)                                 |
| `selfSID()`                     | Get current session ID (Unix)                                       |
| `runSetsid(config)`             | Spawn a command in a new POSIX session                              |
| `runNohup(config)`              | Spawn a SIGHUP-ignoring command in the inherited session            |
| `signalSend(pid, signal)`       | Send signal to process                                              |
| `signalSendGroup(pgid, signal)` | Send signal to process group (Unix)                                 |
| `terminate(pid)`                | Graceful termination                                                |
| `forceKill(pid)`                | Immediate kill                                                      |
| `waitPID(pid, timeoutMs)`       | Wait for process exit with timeout (v0.1.6+)                        |
| `spawnInGroup(config)`          | Spawn in a new Unix process group; unsupported on Windows           |
| `terminateTree(pid, config?)`   | Graceful-then-kill tree termination (v0.1.6+)                       |

### Filter Conventions

Filter fields use **snake_case** to match FFI/schema conventions directly:

```typescript
// ProcessFilter
const filter = {
  name_contains: "nginx", // substring match
  cpu_above: 50, // percentage
  memory_above_kb: 100000, // kilobytes
};

// PortFilter
const portFilter = {
  protocol: "tcp",
  local_port: 8080,
};
```

### v0.1.6 Supervisor Primitives

**waitPID** - Wait for a process to exit with timeout:

```typescript
import { waitPID } from "@3leaps/sysprims";

const outcome = waitPID(pid, 10000); // 10 seconds
if (outcome.timed_out) {
  console.log("Process did not exit in time");
} else {
  console.log(`Exited with code ${outcome.exit_code}`);
}
```

**spawnInGroup** - Spawn process in a new Unix process group:

```typescript
import { spawnInGroup } from "@3leaps/sysprims";

const result = spawnInGroup({
  argv: ["./worker.sh", "--id", "42"],
  cwd: "/app",
  env: { LOG_LEVEL: "debug" },
});

console.log(`Spawned PID ${result.pid}`);
```

`spawnInGroup` fails closed on Windows because the binding returns only a PID
and cannot retain the Job handle required for later tree cleanup. Windows
callers should not treat PID-only `terminateTree` as Job-backed containment.

**terminateTree** - Graceful-then-kill tree termination:

```typescript
import { terminateTree } from "@3leaps/sysprims";

const outcome = terminateTree(pid, {
  grace_timeout_ms: 5000,
  kill_timeout_ms: 2000,
});

if (outcome.escalated) {
  console.log("Had to escalate to kill");
}
```

### Config Types (v0.1.6+)

**SpawnInGroupConfig:**

- `argv`: Command and arguments (required)
- `cwd`: Working directory (optional)
- `env`: Environment overrides (optional)

**TerminateTreeConfig:**

- `signal`: Initial signal (default: SIGTERM/15)
- `grace_timeout_ms`: Wait before escalation (default: 10000)
- `kill_signal`: Kill signal (default: SIGKILL/9)
- `kill_timeout_ms`: Wait after kill (default: 2000)

### How It Works

At load time, the binding:

1. Detects the current platform (`process.platform` + `process.arch`)
2. Loads the appropriate `.node` binary (either locally built or a prebuilt binary)
3. Exposes typed functions to JavaScript

### Local Development

```bash
cd bindings/typescript/sysprims
npm install
npm run build
npm run build:native
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

Ensure you're using the GNU-ABI target and matching toolchain:

- Windows x64: `x86_64-pc-windows-gnu` + msys2/MinGW-w64 (`mingw-w64-x86_64-gcc`)
- Windows arm64: `aarch64-pc-windows-gnullvm` + [llvm-mingw](https://github.com/mstorsjo/llvm-mingw) (`aarch64-w64-mingw32-gcc`)

See the [Windows: Go cgo toolchain](#windows-go-cgo-toolchain) section above.

### CGo can't find library

Check that the library is in `lib/local/<platform>/` or `lib/<platform>/`.
Verify build tags match your platform.

## References

- [ADR-0004: FFI Design](../architecture/adr/0004-ffi-design.md)
- [ADR-0012: Language Bindings Distribution](../architecture/adr/0012-language-bindings-distribution.md)
