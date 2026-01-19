# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

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

### What's Next

- Python bindings (cffi + wheel packaging)
- TypeScript bindings (C-ABI approach)

---

## v0.1.2 - 2026-01-19

**Status:** Security & CI/CD Maintenance Release

Security patch addressing a high-severity vulnerability in CI/CD dependencies, plus infrastructure improvements.

### Security

- **GHSA-cxww-7g56-2vh6** (High): Updated `actions/download-artifact` from `@v4` to `@v4.1.3`
  - Path traversal vulnerability in GitHub Actions artifact downloads
  - Impact: CI/CD pipeline only; no impact on library code or released binaries

### CI/CD Improvements

- Renamed `RELEASE_TAG` to `SYSPRIMS_RELEASE_TAG` to prevent cross-repo confusion
- Added goneat/grype integration for SBOM-based vulnerability scanning
- Updated `GONEAT_VERSION` to v0.5.1

---

## v0.1.1 - 2026-01-17

**Status:** First Language Bindings Release

Completes the FFI surface and ships Go bindings with full cross-platform support.

### Highlights

- **Complete FFI Surface**: 14 C-ABI functions covering error handling, signals, process inspection, and timeout execution
- **Go Bindings**: First-class Go bindings with prebuilt static libraries for 7 platforms
- **Port-to-PID Lookup**: New `ListeningPorts()` API maps listening sockets to owning processes
- **CLI Enhancements**: `kill -l` signal listing, `kill --group` for process groups

### Go Bindings

Install via Go modules (requires CGo):

```go
import "github.com/3leaps/sysprims/bindings/go/sysprims"
```

**Key APIs:**

| Function | Description |
|----------|-------------|
| `Kill(pid, signal)` | Send signal to process |
| `Terminate(pid)` | Send SIGTERM |
| `ForceKill(pid)` | Send SIGKILL |
| `ProcessList(filter)` | List processes with optional filter |
| `ListeningPorts(filter)` | Map listening ports to PIDs (best-effort) |
| `RunWithTimeout(cmd, args, timeout, config)` | Run command with timeout and tree-kill |

**Note:** v0.1.1 Go bindings had empty lib directories. Use v0.1.3+ for working `go get`.

---

*For older releases, see [CHANGELOG.md](CHANGELOG.md) or individual release notes in `docs/releases/`.*
