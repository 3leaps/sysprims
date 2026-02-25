# Diagnosing and Terminating Runaway Processes

This guide demonstrates using sysprims to identify and terminate runaway processes, using a real-world scenario: multiple VSCodium extension host processes consuming excessive CPU.

## The Scenario

Activity Monitor shows nine `VSCodium Helper (Plugin)` processes, each consuming 95-98% CPU:

| PID   | CPU % | CPU Time |
| ----- | ----- | -------- |
| 88680 | 96.5  | 14:22:21 |
| 87620 | 97.5  | 14:19:06 |
| 87799 | 98.5  | 13:40:15 |
| 57490 | 96.6  | 5:06:52  |
| 13343 | 97.8  | 35:54:31 |
| 49457 | 97.9  | 34:07:26 |
| 87446 | 98.7  | 14:19:34 |
| 18709 | 97.0  | 13:06:52 |
| 49501 | 97.5  | 13:18:29 |

These processes are Electron/Chromium utility processes that host VS Code extension code. The goal: identify ownership, confirm identity, and terminate the correct scope.

## Step 1: Find High-CPU Candidates

Use `sysprims pstat` with `--cpu-mode monitor` to identify processes by instantaneous CPU usage (Activity Monitor / top style):

```bash
sysprims pstat --cpu-mode monitor --name "VSCodium Helper" --cpu-above 50 --sort cpu --table
```

**Note**: In monitor mode, `--cpu-above` filters by sampled CPU which can exceed 100% when processes use multiple cores. The default sample duration is 1 second; use `--sample 250ms` for faster (but noisier) results.

For machine-readable output:

```bash
sysprims pstat --cpu-mode monitor --name "VSCodium Helper" --json
```

The `--cpu-mode monitor` uses the sampled schema (`process-info-sampled.schema.json`) to indicate that CPU values may exceed 100%.

## Step 2: Identify Process Relationships

Inspect individual processes to understand the tree structure:

```bash
sysprims pstat --pid 88680 --json
```

Output:

```json
{
  "pid": 88680,
  "ppid": 26021,
  "name": "VSCodium Helper (Plugin)",
  "cpu_percent": 96.5,
  "memory_kb": 74720,
  "elapsed_seconds": 183818,
  "start_time_unix_ms": 1769537759790,
  "exe_path": "/Applications/VSCodium.app/Contents/Frameworks/VSCodium Helper (Plugin).app/Contents/MacOS/VSCodium Helper (Plugin)",
  "state": "running"
}
```

Key insight: **`ppid: 26021`** - all nine runaway helpers share the same parent. Check the parent:

```bash
sysprims pstat --pid 26021 --json
```

Output:

```json
{
  "pid": 26021,
  "ppid": 1,
  "name": "Electron",
  "exe_path": "/Applications/VSCodium.app/Contents/MacOS/Electron",
  "start_time_unix_ms": 1769432792261,
  "elapsed_seconds": 288531
}
```

The parent is the main VSCodium Electron process, running for 3+ days.

## Step 3: Decide What to Terminate

You have two main options:

### Option A: Surgical Strike - Kill Individual Helpers (Recommended First)

Try the least disruptive approach first - kill only the runaway helpers while preserving the parent application and its windows:

```bash
sysprims kill 88680 -s TERM
```

This terminates only the specific runaway helper. The parent VSCodium window stays open.

**Note: SIGTERM may be ignored.** Runaway Electron/Node processes sometimes ignore SIGTERM, especially if they're stuck in a tight loop or have signal handlers. If the process doesn't terminate, escalate to SIGKILL:

```bash
sysprims kill 88680 -s KILL
```

If you have multiple runaway helpers, you can terminate them in one call:

```bash
sysprims kill 88680 88681 88682 -s TERM
```

In testing, surgical kills successfully terminated runaway helpers while preserving the parent VSCodium windows - no respawns occurred and no work was lost. This makes Option A the preferred starting point.

### Option B: Tree Termination - Kill the Parent

> **Before using terminate-tree, consider:** Have you tried killing the individual runaway processes first? Tree termination is irreversible and closes all windows managed by the parent. If surgical strikes (Option A) can resolve the issue, you preserve your application state and open documents.

If surgical kills don't work (processes respawn immediately, or too many children are affected), terminate the entire tree:

```bash
sysprims terminate-tree 26021 \
  --require-exe-path "/Applications/VSCodium.app/Contents/MacOS/Electron" \
  --json
```

Output:

```json
{
  "schema_id": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/terminate-tree-result.schema.json",
  "timestamp": "2026-01-29T21:23:07.677021Z",
  "platform": "macos",
  "pid": 26021,
  "pgid": 26021,
  "signal_sent": 15,
  "escalated": false,
  "exited": true,
  "timed_out": false,
  "tree_kill_reliability": "guaranteed",
  "warnings": []
}
```

**Important**: Tree termination kills the parent and all descendants. In this case, that means all windows managed by this VSCodium instance are closed.

## Step 4: Verify Termination

Confirm the processes are gone:

```bash
sysprims pstat --pid 26021 --json
# Error: Process 26021 not found

sysprims pstat --pid 88680 --json
# Error: Process 88680 not found
```

## Understanding the Output

### terminate-tree Result Fields

| Field                   | Meaning                                  |
| ----------------------- | ---------------------------------------- |
| `signal_sent`           | Signal used (15 = SIGTERM, 9 = SIGKILL)  |
| `escalated`             | Whether SIGKILL was needed after SIGTERM |
| `exited`                | Process terminated successfully          |
| `tree_kill_reliability` | `"guaranteed"` if PGID kill was used     |
| `warnings`              | Any edge cases encountered               |

### Safety Options

The `--require-exe-path` and `--require-start-time-ms` options protect against PID reuse:

```bash
# Require both exe path and start time to match
sysprims terminate-tree 26021 \
  --require-exe-path "/Applications/VSCodium.app/Contents/MacOS/Electron" \
  --require-start-time-ms 1769432792261 \
  --json
```

If the PID has been reused by a different process since you identified it, the command fails safely instead of terminating the wrong process.

## CLI Footgun Protections

The `terminate-tree` CLI includes safety guards that refuse to proceed without `--force`:

- PID 1 (init/launchd)
- PID of the calling process (self)
- PID of the calling process's parent

These protections are CLI-specific. The underlying library allows these operations for controlled automation scenarios.

## Best Practices

1. **Investigate first**: Use `pstat` to understand the process tree before terminating anything.

2. **Start surgical**: Try killing individual runaway children first (Option A). If the parent respawns them or multiple children are affected, then escalate to tree termination.

3. **Use identity guards**: Always use `--require-exe-path` or `--require-start-time-ms` when automating termination, to protect against PID reuse.

4. **Understand scope**: Tree termination affects all descendants. For applications like VS Code that may have multiple windows under one process, tree termination closes all windows in that instance.

## Identifying the Root Cause

In this scenario, investigation revealed the runaway helpers were all accessing the Cline (claude-dev) extension's state files. Use `sysprims fds` to inspect open file descriptors without requiring external tools:

```bash
# Inspect what files a runaway process has open
sysprims fds --pid 88680 --table

# Filter to only file descriptors
sysprims fds --pid 88680 --kind file --json
```

Example output showing the extension state files:

```json
{
  "schema_id": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/fd-snapshot.schema.json",
  "pid": 88680,
  "fds": [
    {
      "fd": 5,
      "kind": "file",
      "path": "/Users/.../saoudrizwan.claude-dev/state/taskHistory.json"
    }
  ],
  "warnings": []
}
```

**Platform Notes:**

- **Linux**: Full file paths available via `/proc/<pid>/fd/` symlinks
- **macOS**: Best-effort path recovery; some paths may be unavailable
- **Windows**: Not supported (requires elevated privileges; see `docs/appnotes/fds-validation/`)

This identifies which extension or workspace triggered the issue directly within sysprims, without requiring external tools like `lsof`.

## Using sysprims as a Library

### TypeScript Example

```typescript
import { procGet, processList, terminateTree } from "@3leaps/sysprims";

// Find candidates
const helpers = processList({ name_contains: "VSCodium Helper" });
const runaway = helpers.filter((p) => p.cpu_percent > 50);

// Group by parent
const byParent = new Map<number, typeof runaway>();
for (const p of runaway) {
  const group = byParent.get(p.ppid) || [];
  group.push(p);
  byParent.set(p.ppid, group);
}

// Terminate tree with identity verification
for (const [ppid, children] of byParent) {
  const parent = procGet(ppid);

  const result = terminateTree(ppid, {
    require_exe_path: parent.exe_path,
    require_start_time_ms: parent.start_time_unix_ms,
    grace_timeout_ms: 5000,
    kill_timeout_ms: 10000,
  });

  if (result.exited) {
    console.log(
      `Terminated tree rooted at ${ppid} (${children.length} runaway children)`,
    );
  }
}
```

## Summary

| Task                                    | Command                                                       |
| --------------------------------------- | ------------------------------------------------------------- |
| Find high-CPU processes (instantaneous) | `sysprims pstat --cpu-mode monitor --cpu-above 50 --sort cpu` |
| Find high-CPU processes (lifetime avg)  | `sysprims pstat --cpu-above 50 --sort cpu`                    |
| Inspect specific PID                    | `sysprims pstat --pid <PID> --json`                           |
| Inspect open files/sockets              | `sysprims fds --pid <PID> --table`                            |
| Kill single process (try first)         | `sysprims kill <PID> -s TERM`                                 |
| Kill single process (if TERM ignored)   | `sysprims kill <PID> -s KILL`                                 |
| Kill multiple processes                 | `sysprims kill <PID> <PID> ... -s TERM --json`                |
| Terminate process tree (last resort)    | `sysprims terminate-tree <PID> --require-exe-path <PATH>`     |

---

**Version**: sysprims v0.1.9+
**Platforms**: Linux, macOS, Windows
