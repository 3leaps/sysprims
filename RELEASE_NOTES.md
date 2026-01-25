# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

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

### Bug Fixes

- Windows TypeScript tests now pass (cross-platform build scripts)
- Fixed parallel test flakiness in tree_escape tests

---

*For older releases, see [CHANGELOG.md](CHANGELOG.md) or individual release notes in `docs/releases/`.*
