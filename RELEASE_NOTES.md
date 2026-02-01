# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

---

## v0.1.9 - 2026-02-01

**Status:** Process Visibility & Batch Operations Release

This release adds process visibility (`sysprims fds`) and batch signal operations (`sysprims kill` with multiple PIDs), completing the diagnostic and remediation toolkit for runaway process management.

### Highlights

- **`sysprims fds`**: Inspect open file descriptors (the `lsof` use-case, GPL-free)
- **Multi-PID Kill**: Batch signal delivery with per-PID result tracking
- **Complete Workflow**: From diagnosis (`pstat` → `fds`) to remediation (`kill` → `terminate-tree`)
- **Go Shared Library Mode**: Alpine/musl support via `-tags=sysprims_shared` for consumers linking multiple Rust staticlibs

### New CLI Commands

#### `sysprims fds`

Inspect open file descriptors for any process:

```bash
# Table output for human inspection
sysprims fds --pid 88680 --table

# JSON output for scripting
sysprims fds --pid 88680 --json

# Filter by resource type
sysprims fds --pid 88680 --kind file --table
sysprims fds --pid 88680 --kind socket --json
```

Output (JSON):

```json
{
  "schema_id": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/fd-snapshot.schema.json",
  "timestamp": "2026-02-01T17:40:10.702011Z",
  "platform": "macos",
  "pid": 88680,
  "fds": [
    {
      "fd": 5,
      "kind": "file",
      "path": "/Users/.../extension/state.json"
    },
    {
      "fd": 6,
      "kind": "socket"
    }
  ],
  "warnings": []
}
```

Options:

| Option | Description |
|--------|-------------|
| `--pid <PID>` | Process to inspect (required) |
| `--kind <TYPE>` | Filter by: `file`, `socket`, `pipe`, `unknown` |
| `--json` | Output as JSON (default) |
| `--table` | Human-readable table format |

**Platform Support:**
- **Linux**: Full file path resolution via `/proc/<pid>/fd/`
- **macOS**: Best-effort path recovery via `proc_pidinfo()`
- **Windows**: Returns error (not supported; requires elevated privileges)

#### Multi-PID Kill

Send signals to multiple processes in one call:

```bash
# Kill multiple specific PIDs
sysprims kill 88680 88681 88682 -s TERM

# With JSON output showing per-PID results
sysprims kill 88680 88681 88682 -s TERM --json
```

Output (JSON):

```json
{
  "schema_id": "https://schemas.3leaps.dev/sysprims/signal/v1.0.0/batch-kill-result.schema.json",
  "signal_sent": 15,
  "succeeded": [88680, 88681],
  "failed": [
    { "pid": 88682, "error": "Process not found" }
  ]
}
```

Exit codes:
- `0`: All PIDs signaled successfully
- `1`: Some PIDs failed (partial success)
- `2`: All PIDs failed or validation error

**Key behavior:** All PIDs are validated before any signals are sent. Individual failures don't abort the batch.

### Complete Runaway Process Workflow

```bash
# 1. Find high-CPU processes
sysprims pstat --cpu-mode monitor --cpu-above 50 --sort cpu --table

# 2. Inspect what files they have open
sysprims fds --pid <PID> --kind file --json

# 3. Kill runaway processes (surgical)
sysprims kill <PID> <PID> ... -s TERM

# 4. If that doesn't work, terminate the tree
sysprims terminate-tree <PARENT_PID> --require-exe-path <PATH>
```

See the updated [runaway process diagnosis guide](docs/guides/runaway-process-diagnosis.md) for the full walkthrough.

### Documentation

- **New App Note**: `docs/appnotes/fds-validation/`
  - Synthetic test cases for validating `sysprims fds` output
  - Demonstrates platform differences (Linux full paths vs macOS best-effort)
- **Updated Guide**: `docs/guides/runaway-process-diagnosis.md`
  - Now includes `sysprims fds` for root cause identification
  - Documents the complete diagnostic workflow

### Library API (Rust)

```rust
use sysprims_proc::{list_fds, FdFilter, FdKind};
use sysprims_signal::kill_many;

// Inspect file descriptors
let filter = FdFilter { kind: Some(FdKind::File) };
let snapshot = list_fds(pid, Some(&filter))?;

// Batch signal delivery
let result = kill_many(&[pid1, pid2, pid3], SIGTERM)?;
```

### Language Bindings

Both features are available in Go and TypeScript:

**Go:**
```go
// List file descriptors
snapshot, err := sysprims.ListFds(pid, &sysprims.FdFilter{Kind: sysprims.FdKindFile})

// Batch kill
result, err := sysprims.KillMany([]uint32{pid1, pid2, pid3}, sysprims.SIGTERM)
```

**TypeScript:**
```typescript
// List file descriptors
const snapshot = listFds(pid, { kind: 'file' });

// Batch kill
const result = killMany([pid1, pid2, pid3], SIGTERM);
```

### Go Shared Library Mode

For Go applications that link multiple Rust static libraries (which can cause symbol collisions), sysprims now supports shared library mode on all platforms except Windows ARM64:

**Default (static linking):**
```bash
go test ./...
```

**Shared mode (where supported):**
```bash
# Standard platforms (darwin, linux-glibc, windows-amd64)
go test -tags=sysprims_shared ./...

# Alpine/musl (added in this release)
go test -tags="musl,sysprims_shared" ./...
```

**Platform Support:**

| Platform | Static | Shared | Build Tags |
|----------|--------|--------|------------|
| macOS (arm64) | ✓ | ✓ | `sysprims_shared` |
| Linux glibc | ✓ | ✓ | `sysprims_shared` |
| Linux musl | ✓ | ✓ | `musl,sysprims_shared` |
| Windows | ✓ | ✓ | `sysprims_shared` |
| Windows ARM64 | ✓ | ✗ | N/A |

Shared libraries use rpath for runtime resolution and are validated in CI via Alpine containers for musl support.

---

## v0.1.8 - 2026-01-29

**Status:** CLI Tree Termination Release

This release adds the `terminate-tree` CLI subcommand for safe, structured termination of existing process trees. Combined with enhanced `pstat` sampling, sysprims now provides a complete workflow for diagnosing and cleaning up runaway processes.

### Highlights

- **`sysprims terminate-tree`**: Terminate process trees with graceful-then-kill escalation
- **PID Reuse Protection**: `--require-start-time-ms` and `--require-exe-path` guards
- **CLI Safety**: Refuses to terminate PID 1, self, or parent without `--force`
- **`pstat` Sampling**: `--sample` and `--top` flags for "what's burning CPU" investigation

### New CLI Commands

#### `sysprims terminate-tree`

Terminate an existing process tree by PID:

```bash
# Basic usage
sysprims terminate-tree 26021

# With PID reuse protection (recommended for automation)
sysprims terminate-tree 26021 \
  --require-exe-path "/Applications/VSCodium.app/Contents/MacOS/Electron" \
  --require-start-time-ms 1769432792261

# JSON output for scripting
sysprims terminate-tree 26021 --json
```

Output (JSON):

```json
{
  "schema_id": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/terminate-tree-result.schema.json",
  "pid": 26021,
  "pgid": 26021,
  "signal_sent": 15,
  "escalated": false,
  "exited": true,
  "tree_kill_reliability": "guaranteed"
}
```

Options:

| Option | Description |
|--------|-------------|
| `--grace <DURATION>` | Grace period before escalation (default: 5s) |
| `--kill-after <DURATION>` | Send kill signal if still running (default: 10s) |
| `--signal <SIGNAL>` | Grace period signal (default: TERM) |
| `--kill-signal <SIGNAL>` | Forced termination signal (default: KILL) |
| `--require-start-time-ms <MS>` | Refuse if PID start time doesn't match |
| `--require-exe-path <PATH>` | Refuse if PID exe path doesn't match |
| `--force` | Override safety checks (PID 1, self, parent) |
| `--json` | Output as JSON |

#### `pstat` Sampling Enhancements

Find processes consuming CPU right now:

```bash
# Sample CPU over 250ms, show top 5
sysprims pstat --sample 250ms --top 5 --sort cpu --table

# Find VSCodium helpers burning CPU
sysprims pstat --name "VSCodium Helper" --sample 500ms --cpu-above 50 --json
```

### Surgical vs Tree Termination

The [runaway process diagnosis guide](docs/guides/runaway-process-diagnosis.md) documents two approaches:

**Option A: Surgical Strike** (try first)
```bash
sysprims kill 8436 -s TERM   # or -s KILL if ignored
```
- Kills individual helpers while preserving parent windows
- SIGTERM may be ignored by runaway processes; escalate to SIGKILL

**Option B: Tree Termination** (if surgical fails)
```bash
sysprims terminate-tree 26021 --require-exe-path "..."
```
- Terminates parent and all descendants
- Closes all windows managed by that process

### Documentation

- **Guide**: `docs/guides/runaway-process-diagnosis.md`
  - Real-world walkthrough with VSCodium/Electron plugin helper scenario
  - Investigation workflow using `pstat` and `fds`
  - Decision framework for surgical vs tree termination

---

## v0.1.7 - 2026-01-26

**Status:** TypeScript Bindings Infrastructure Release

This release migrates TypeScript bindings from koffi FFI to a Node-API (N-API) native addon via napi-rs. The primary user-facing outcome: TypeScript bindings now work in Alpine/musl containers.

### Highlights

- **Node-API Migration**: TypeScript bindings now use napi-rs instead of koffi + vendored shared libraries
- **Alpine/musl Support**: Linux musl containers (including Alpine) now supported for TypeScript
- **No API Changes**: Existing imports and function calls remain unchanged
- **npm Publishing Deferred**: Prebuilt npm packages planned for future release

### What Changed (Implementation Detail)

| Aspect | v0.1.6 (koffi) | v0.1.7 (N-API) |
|--------|----------------|----------------|
| Native loading | `koffi.load()` C-ABI shared lib | `require()` N-API `.node` addon |
| Library location | `_lib/<platform>/libsysprims_ffi.*` | `native/<platform>/sysprims.*.node` |
| Build requirement | None (prebuilt libs vendored) | Rust toolchain (when building from source) |
| Alpine support | No | Yes |

### Installation Modes

**From git checkout / local path (current):**
- Requires Rust toolchain and C/C++ build tools
- Run `npm run build:native` after install

**From npm (future):**
- Prebuilt platform packages will install automatically
- No build tools required

### Breaking Changes

None. The JavaScript API surface is unchanged.

### Adoption Notes

- Pin `@3leaps/sysprims` to exact version for initial rollouts
- Add smoke test that calls `procGet(process.pid)` to validate addon loading
- Keep fallback implementations for locked-down environments where native addons may fail

---

