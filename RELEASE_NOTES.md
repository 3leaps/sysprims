# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

---

## v0.1.5 - 2026-01-24

**Status:** TypeScript Bindings Parity Release (proc/ports/signals)

Node.js developers now have access to process inspection, port mapping, and signal APIs. This release achieves parity with Go bindings for these core surfaces.

### Highlights

- **TypeScript Parity**: Process listing, port inspection, and signal operations
- **Full Type Definitions**: All schemas have corresponding TypeScript types
- **Windows Stability**: Signal tests no longer flaky on Windows CI

### New TypeScript API

| Function | Description |
|----------|-------------|
| `processList(filter?)` | List running processes with filtering |
| `listeningPorts(filter?)` | Map listening ports to processes |
| `signalSend(pid, signal)` | Send signal to process |
| `signalSendGroup(pgid, signal)` | Send signal to process group (Unix) |
| `terminate(pid)` | Graceful termination (SIGTERM on Unix, TerminateProcess on Windows) |
| `forceKill(pid)` | Immediate kill (SIGKILL on Unix, TerminateProcess on Windows) |

### Example Usage

```typescript
import { processList, listeningPorts, terminate } from '@3leaps/sysprims';

// List all nginx processes
const nginx = processList({ name_contains: "nginx" });
for (const proc of nginx.processes) {
  console.log(`${proc.pid}: ${proc.name} (${proc.cpu_percent}% CPU)`);
}

// Find what's listening on port 8080
const http = listeningPorts({ local_port: 8080 });
for (const binding of http.bindings) {
  console.log(`Port ${binding.local_port}: PID ${binding.pid}`);
}

// Gracefully terminate a process
terminate(1234);
```

### Bug Fixes

- Windows signal tests now use deterministic patterns: reject pid=0, spawn-and-kill for terminate/forceKill

---

## v0.1.4 - 2026-01-22

**Status:** TypeScript Language Bindings Release

Node.js developers can now integrate sysprims directly. This release delivers koffi-based TypeScript bindings with cross-platform support.

### Highlights

- **TypeScript Bindings**: First-class Node.js support via koffi FFI
- **Cross-Platform**: linux-amd64, linux-arm64, darwin-arm64, windows-amd64
- **ABI Verification**: Library loader validates ABI version at startup
- **CI Coverage**: Native ARM64 Linux testing added to CI matrix

### TypeScript Bindings

Install and use in your Node.js projects:

```typescript
import { procGet, selfPGID, selfSID } from '@3leaps/sysprims';

// Get process info by PID
const proc = procGet(process.pid);
console.log(`Process ${proc.pid}: ${proc.name}`);

// Get current process group/session IDs (Unix)
const pgid = selfPGID();
const sid = selfSID();
```

**Platform Support:**

| Platform | Status |
|----------|--------|
| Linux x64 (glibc) | Supported |
| Linux arm64 (glibc) | Supported |
| macOS arm64 | Supported |
| Windows x64 | Supported |
| Linux musl | Not supported |

**Note:** Linux musl (Alpine) is not supported for TypeScript bindings due to glibc dependencies.

### CI Changes

- Replaced darwin-amd64 (Intel Mac) with linux-arm64 in CI matrix
- Intel Mac runners deprecated by GitHub Actions

### Bug Fixes

- Windows TypeScript tests now pass (cross-platform build scripts)
- Fixed parallel test flakiness in tree_escape tests

---

## v0.1.3 - 2026-01-19

**Status:** Go Bindings Infrastructure Release

First fully working Go bindings release. Prebuilt static libraries now included in release tags.

### Highlights

- **Prebuilt Libs in Tags**: Go bindings now ship with static libraries in the tagged commit
- **Manual Prep Workflow**: New `go-bindings.yml` workflow builds libs before tagging
- **Release Gate**: CI verifies Go libs present before publishing
- **Dual-Tag Policy**: Both `vX.Y.Z` and `bindings/go/sysprims/vX.Y.Z` tags required

### Go Bindings (Now Working)

v0.1.1/v0.1.2 had empty lib directories. v0.1.3 is the first release where Go bindings work via `go get`:

```bash
go get github.com/3leaps/sysprims/bindings/go/sysprims@v0.1.3
```

### Release Process Changes

The Go bindings prep is now a manual pre-release step:

1. Run `go-bindings.yml` workflow (manual dispatch)
2. Merge the resulting PR
3. Tag with both `vX.Y.Z` and `bindings/go/sysprims/vX.Y.Z`
4. Push tags; release workflow verifies libs are present

See `RELEASE_CHECKLIST.md` for full instructions.

---

*For older releases, see [CHANGELOG.md](CHANGELOG.md) or individual release notes in `docs/releases/`.*
