# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

---

## v0.1.11 - 2026-02-04

**Status:** macOS Port Discovery & Bun Runtime Support Release

Fixes `listeningPorts()` returning empty results on macOS, adds a new `ports` CLI command, and enables Bun runtime support for TypeScript bindings.

### Highlights

- **macOS Port Discovery Fixed**: `listeningPorts()` now works on macOS (was returning empty)
- **New CLI Command**: `sysprims ports` for listing listening port bindings
- **Bun Runtime Support**: TypeScript bindings now work under Bun

### New CLI Command: `sysprims ports`

List listening port bindings with optional filtering:

```bash
# Table output for human inspection
sysprims ports --table

# Filter by protocol
sysprims ports --protocol tcp --table

# Filter by specific port (JSON output)
sysprims ports --protocol tcp --local-port 8080 --json
```

Output includes full process details (name, PID, exe_path, cmdline, user).

### macOS Port Discovery Fix

The `listeningPorts()` function was returning empty results on macOS due to SDK struct layout mismatches in socket fdinfo parsing. This release fixes the issue:

**Before (v0.1.10):**
```
Found 0 bindings
```

**After (v0.1.11):**
```
Found 68 bindings
  tcp *:9999 -> pid=54659 (bun)
  tcp 127.0.0.1:8080 -> pid=40672 (namelens)
  ...
```

**Technical changes:**
- UID filtering: scans current-user processes only (reduces SIP/TCC permission errors)
- Heuristic vinfo_stat size detection (136/144 bytes) for SDK compatibility
- Offset-based parsing instead of fixed struct layout
- Strict TCP listener filtering (`TSI_S_LISTEN` state only)

### Bun Runtime Support

TypeScript bindings now work under Bun. The explicit Bun block has been removed:

```typescript
// REMOVED in v0.1.11:
if (process.versions?.bun) {
  throw new Error("sysprims TypeScript bindings are not yet validated on Bun...");
}
```

Validated functionality under Bun:

| Feature | Status |
|---------|--------|
| Module loading | Works |
| `procGet()` | Works |
| `terminate()` | Works |
| `listeningPorts()` | Works (with macOS fix) |

### Upgrade Notes

- No breaking changes
- macOS users will now see port bindings that were previously invisible
- Bun users can use sysprims directly without workarounds

---

## v0.1.10 - 2026-02-03

**Status:** Go Shared Library Mode Polish Release

Fast-follow polish release improving Go shared-library mode developer experience and clarifying multi-Rust FFI collision guidance.

### Highlights

- **`sysprims_shared_local` Tag**: New opt-in build tag for local development workflows
- **Cleaner Default Shared Mode**: `sysprims_shared` no longer references non-existent local paths
- **Clearer Multi-Rust Guidance**: README explicitly documents duplicate symbol `_rust_eh_personality` failure mode

### New Build Tag: `sysprims_shared_local`

For developers who need to link against locally-built shared libraries:

```bash
# Local development with custom shared libs
# (libs must be in bindings/go/sysprims/lib-shared/local/<platform>/)
go test -v -tags="sysprims_shared,sysprims_shared_local" ./...
```

This tag re-enables the local override paths that were previously searched by default, which caused confusing linker warnings when the directory didn't exist.

### Cleaner Default: `sysprims_shared`

The default shared mode now only searches shipped prebuilt libraries:

```bash
# Standard shared mode (no local paths searched)
# glibc/macOS/Windows
go test -v -tags=sysprims_shared ./...

# Alpine/musl
go test -v -tags="musl,sysprims_shared" ./...
```

This eliminates the linker warnings that previously appeared when `lib-shared/local/` didn't exist.

### Multi-Rust FFI Collision Guidance

The README now explicitly documents the "multiple Rust FFI libs in one Go binary" failure mode:

**Symptom:** Link errors mentioning duplicate symbols like `_rust_eh_personality`

**Cause:** Linking multiple Rust static libraries (via cgo `//#cgo LDFLAGS: -l...`) in a single Go binary causes duplicate Rust runtime symbols.

**Solution:** Use sysprims as a shared library:

| Platform | Build Tags |
|----------|-----------|
| glibc/macOS/Windows | `-tags=sysprims_shared` |
| Alpine/musl | `-tags="musl,sysprims_shared"` |
| Local dev override | `-tags="sysprims_shared,sysprims_shared_local"` |

### Upgrade Notes

- **No breaking changes** for existing `sysprims_shared` workflows using prebuilt libraries
- If you were relying on `lib-shared/local/...` implicitly, add the `sysprims_shared_local` tag explicitly
- No changes needed for standard consumers using shipped prebuilt libs

### References

- Commit: `3b004b7` - adds `sysprims_shared_local`, removes local-path warnings, updates docs

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

