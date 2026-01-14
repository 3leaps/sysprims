---
title: "sysprims-timeout Module Spec"
module: "sysprims-timeout"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
adr_refs: ["ADR-0003", "ADR-0005", "ADR-0007", "ADR-0008"]
---

# sysprims-timeout Module Spec

## 1) Overview

**Purpose:** Provide a library-first `run_with_timeout()` primitive and thin CLI wrapper that match widely expected `timeout` semantics while delivering a stronger guarantee: **terminate the whole process tree by default** (group-by-default), with **observable fallbacks** when the OS cannot guarantee it.

**Core differentiator (ADR-0003):** Unlike GNU timeout, which kills only the direct child, sysprims-timeout kills the entire process tree. This prevents orphaned processes that ignore SIGTERM or attempt to escape.

**In scope (v0.1.0):**

- Duration parsing (e.g., `250ms`, `2s`, `5m`, `1h`)
- Run command with deadline
- Choose initial signal (default SIGTERM)
- Escalate to SIGKILL after `kill_after` delay
- `--preserve-status` behavior (propagate child exit code on normal completion)
- Group-by-default process control:
  - Unix: process groups via `setpgid(0, 0)` and `killpg()`
  - Windows: Job Objects with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
- Machine-readable JSON output including `schema_id` and reliability fields

**Out of scope (v0.1.0):**

- Full GNU `timeout` option parity
- Shell parsing / quoting beyond deterministic rules

## 2) Normative References

### Reference Behavior Target (non-POSIX)

`timeout` is NOT standardized by POSIX. We use GNU coreutils `timeout` as the **reference behavior target** for:

- Exit code conventions (124, 125, 126, 127)
- CLI option semantics (`--signal`, `--kill-after`, `--preserve-status`)

**References:**
- GNU coreutils `timeout`: https://www.gnu.org/software/coreutils/manual/html_node/timeout-invocation.html

### OS-Level Normative References

**Unix (process groups):**
- POSIX `setpgid()`: https://pubs.opengroup.org/onlinepubs/9699919799/functions/setpgid.html
- POSIX `killpg()`: equivalent to `kill(-pgrp, sig)`

**Windows (Job Objects):**
- Job Objects: https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects
- `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` semantics

## 3) Literal Interface Reference (Behavior Target)

### GNU timeout CLI synopsis

```
timeout [OPTION] DURATION COMMAND [ARG]...
```

### Core options:

- `--signal=SIGNAL`, `-s SIGNAL` — signal to send on timeout (default: TERM)
- `--kill-after=DURATION`, `-k DURATION` — send KILL if still running after delay
- `--preserve-status` — exit with child's status if command completes normally
- `--foreground` — don't create process group

### Exit codes (behavior target):

| Exit Code | Condition |
|-----------|-----------|
| 124 | Command timed out |
| 125 | `timeout` itself failed |
| 126 | Command found but cannot be invoked |
| 127 | Command not found |
| 128+N | Command killed by signal N |
| Other | Child's exit code (with `--preserve-status`) |

## 4) sysprims Required Interface (Rust)

### 4.1 Core Types

```rust
/// Process grouping strategy (ADR-0003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupingMode {
    /// Create new process group (Unix) or Job Object (Windows).
    /// Kill entire tree on timeout. **This is the default.**
    #[default]
    GroupByDefault,

    /// Run in foreground. Only kills direct child on timeout.
    Foreground,
}

/// Configuration for timeout execution.
pub struct TimeoutConfig {
    /// Initial signal to send on timeout (default: SIGTERM = 15).
    pub signal: i32,

    /// Delay before escalating to SIGKILL (default: 10 seconds).
    pub kill_after: Duration,

    /// Process grouping strategy (default: GroupByDefault).
    pub grouping: GroupingMode,

    /// Propagate child exit code on normal completion (default: false).
    pub preserve_status: bool,
}

/// Reliability of tree-kill operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeKillReliability {
    /// Process group (Unix) or Job Object (Windows) established.
    Guaranteed,
    /// Fallback: only direct child killed. Grandchildren may escape.
    BestEffort,
}

/// Outcome of timeout execution.
pub enum TimeoutOutcome {
    /// Command completed within timeout.
    Completed { exit_status: ExitStatus },

    /// Command timed out and was killed.
    TimedOut {
        signal_sent: i32,
        escalated: bool,
        tree_kill_reliability: TreeKillReliability,
    },
}
```

### 4.2 Public Functions

```rust
/// Run a command with timeout.
pub fn run_with_timeout(
    command: &str,
    args: &[&str],
    timeout: Duration,
    config: TimeoutConfig,
) -> SysprimsResult<TimeoutOutcome>;

/// Run with default configuration.
pub fn run_with_timeout_default(
    command: &str,
    args: &[&str],
    timeout: Duration,
) -> SysprimsResult<TimeoutOutcome>;
```

### 4.3 Error Handling

Per ADR-0008:

| Error | Condition | CLI Exit |
|-------|-----------|----------|
| `NotFound` | Command not found | 127 |
| `PermissionDenied` | Command not executable | 126 |
| `SpawnFailed` | Failed to spawn process | 125 |
| `GroupCreationFailed` | Process group/job creation failed | (continues with best-effort) |

### 4.4 Invariants

1. **Timeout invariant:** If deadline reached, `TimedOut` must be returned and CLI exit must be `124`.

2. **Group-by-default invariant (ADR-0003):**
   - Unix: child runs in its own process group via `setpgid(0, 0)`
   - Windows: child assigned to Job Object with `KILL_ON_JOB_CLOSE`
   - Termination targets the group/job, not just the direct child

3. **Observable fallback invariant:** If guaranteed tree-kill cannot be established:
   - `tree_kill_reliability = BestEffort`
   - JSON output reflects actual behavior

4. **Signal escalation invariant:** If process doesn't exit after `signal_sent` within `kill_after`, escalate to SIGKILL.

5. **Preserve-status invariant:** `--preserve-status` affects only non-timeout completion.

## 5) CLI Contract

**Subcommand:** `sysprims timeout`

### Synopsis

```
sysprims timeout [OPTIONS] <DURATION> -- <COMMAND> [ARGS...]
```

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `-s, --signal <SIG>` | Signal to send on timeout | TERM |
| `-k, --kill-after <DUR>` | Delay before SIGKILL | 10s |
| `--preserve-status` | Propagate child exit code | false |
| `--foreground` | Don't create process group | false |

### Exit Codes

| Code | Condition |
|------|-----------|
| 0 | Command completed normally (no `--preserve-status`) |
| Child's code | Command completed (with `--preserve-status`) |
| 124 | Command timed out |
| 125 | Internal failure / invalid usage |
| 126 | Command found but cannot invoke |
| 127 | Command not found |
| 128+N | Child killed by signal N |

## 6) Platform Implementation

| Feature | Unix | Windows |
|---------|------|---------|
| Process grouping | `setpgid(0, 0)` | Job Object |
| Tree kill | `killpg(-pgid, sig)` | `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` |
| SIGTERM | Native signal | TerminateProcess |
| SIGKILL | Native signal | TerminateProcess |

## 7) Traceability Matrix

| Requirement | Reference | Rust API | CLI | Tests | Status |
|-------------|-----------|----------|-----|-------|--------|
| Exit code 124 on timeout | GNU timeout | `TimedOut` | exit 124 | integration | Pass |
| Exit code 125 on internal error | GNU timeout | `SysprimsError` | exit 125 | integration | Pass |
| Exit code 126 on not executable | GNU timeout | `PermissionDenied` | exit 126 | integration | Pass |
| Exit code 127 on not found | GNU timeout | `NotFound` | exit 127 | integration | Pass |
| Signal escalation | GNU timeout | `kill_after`, `escalated` | `--kill-after` | integration | Pass |
| Group-by-default | ADR-0003 | `GroupByDefault` | default | tree-escape | Pass |
| Observable fallback | ADR-0003 | `TreeKillReliability` | `--json` | integration | Pass |
| Default SIGTERM | spec | `TimeoutConfig::default()` | default | `default_config_*` | Pass |
| Default 10s kill_after | spec | `TimeoutConfig::default()` | default | `default_config_*` | Pass |

---

*Spec version: 1.0*
*Last updated: 2026-01-09*
