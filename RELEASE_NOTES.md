# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

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

The new [runaway process diagnosis guide](docs/guides/runaway-process-diagnosis.md) documents two approaches:

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

- **New Guide**: `docs/guides/runaway-process-diagnosis.md`
  - Real-world walkthrough with VSCodium/Electron plugin helper scenario
  - Investigation workflow using `pstat` and `lsof`
  - Decision framework for surgical vs tree termination

### Coming in v0.1.9

- **`sysprims fds`**: Open file descriptor inspection (Linux/macOS)
- **Multi-PID kill**: `sysprims kill <PID> <PID> ...` batch operations

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

## v0.1.6 - 2026-01-25

**Status:** Supervisor & Job Manager Primitives Release

This release delivers process management primitives for long-running supervisors and job managers. Teams building systems like gonimbus or rampart lifecycle can now spawn kill-tree-safe jobs, detect PID reuse, and cleanly terminate process trees—without coupling to the timeout API.

### Highlights

- **PID Reuse Guard**: New `start_time_unix_ms` and `exe_path` fields in `ProcessInfo` enable detection of PID reuse
- **Spawn In Group**: Create processes in a new process group (Unix) or Job Object (Windows)
- **Wait With Timeout**: Poll for process exit with configurable timeout
- **Terminate Tree**: Graceful-then-kill tree termination as a standalone primitive

### New Primitives

| Primitive | Rust | FFI | Go | TypeScript |
|-----------|------|-----|-----|------------|
| Process identity | `ProcessInfo` | `sysprims_proc_get` | `ProcessGet` | `procGet` |
| Spawn in group | `spawn_in_group()` | `sysprims_spawn_in_group` | `SpawnInGroup` | `spawnInGroup` |
| Wait PID with timeout | `wait_pid()` | `sysprims_proc_wait_pid` | `WaitPID` | `waitPID` |
| Terminate tree | `terminate_tree()` | `sysprims_terminate_tree` | `TerminateTree` | `terminateTree` |

### Process Identity (PID Reuse Guard)

Long-running supervisors can now detect whether a stored PID still refers to the expected process:

```rust
use sysprims_proc::get_process;

// Store identity at job creation
let info = get_process(pid)?;
let identity = (info.pid, info.start_time_unix_ms);

// Later, verify before signaling
let current = get_process(pid)?;
if current.start_time_unix_ms != identity.1 {
    // PID was reused—don't signal!
}
```

### Spawn In Group

Create processes in a new process group or Job Object for reliable tree termination:

```rust
use sysprims_timeout::{spawn_in_group, SpawnInGroupConfig};

let outcome = spawn_in_group(SpawnInGroupConfig {
    argv: vec!["./worker.sh".into(), "--id".into(), "42".into()],
    cwd: None,
    env: None, // inherits parent env by default
})?;

println!("Spawned PID {}", outcome.pid);
#[cfg(unix)]
println!("Process group: {}", outcome.pgid.unwrap());
```

### Wait PID With Timeout

Wait for a process to exit without blocking forever:

```rust
use sysprims_proc::wait_pid;
use std::time::Duration;

let outcome = wait_pid(pid, Duration::from_secs(10))?;
if outcome.timed_out {
    println!("Process {} did not exit in 10s", pid);
} else {
    println!("Process {} exited with code {:?}", pid, outcome.exit_code);
}
```

### Terminate Tree

One-call process tree termination with graceful-then-kill escalation:

```rust
use sysprims_timeout::{terminate_tree, TerminateTreeConfig};

let outcome = terminate_tree(pid, TerminateTreeConfig {
    grace_timeout_ms: 5000,
    kill_timeout_ms: 2000,
    ..Default::default()
})?;

if outcome.escalated {
    println!("Had to escalate to SIGKILL");
}
```

### Documentation

- Added Job Object registry documentation for Windows platform behavior

---

*For older releases, see [CHANGELOG.md](CHANGELOG.md) or individual release notes in `docs/releases/`.*
