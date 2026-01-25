---
title: "sysprims-timeout Module Spec"
module: "sysprims-timeout"
version: "1.1"
status: "Active"
last_updated: "2026-01-25"
adr_refs: ["ADR-0003", "ADR-0005", "ADR-0007", "ADR-0008", "ADR-0011"]
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

/// Spawn a process in a new process group/Job Object (v0.1.6+).
///
/// Creates a child process with reliable tree-kill capability.
/// Does not wait for the process to exit.
pub fn spawn_in_group(config: SpawnInGroupConfig) -> SysprimsResult<SpawnInGroupResult>;

/// Terminate a process tree with graceful-then-kill escalation (v0.1.6+).
///
/// Works on arbitrary PIDs, not just spawned children.
pub fn terminate_tree(pid: u32, config: TerminateTreeConfig) -> SysprimsResult<TerminateTreeResult>;
```

### 4.3 SpawnInGroup Types (v0.1.6+)

```rust
/// Configuration for spawn_in_group (v0.1.6+).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnInGroupConfig {
    /// Command and arguments (required).
    pub argv: Vec<String>,

    /// Working directory (optional; inherits parent if None).
    #[serde(default)]
    pub cwd: Option<String>,

    /// Environment overrides/additions (optional; inherits parent env by default).
    #[serde(default)]
    pub env: Option<std::collections::BTreeMap<String, String>>,
}

/// Result of spawn_in_group.
#[derive(Debug, Clone, Serialize)]
pub struct SpawnInGroupResult {
    /// Schema identifier for version detection.
    pub schema_id: &'static str,

    /// Timestamp (RFC3339).
    pub timestamp: String,

    /// Platform identifier.
    pub platform: &'static str,

    /// Child process ID.
    pub pid: u32,

    /// Process group ID (Unix only; null/None on Windows).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pgid: Option<u32>,

    /// Reliability of tree-kill capability.
    pub tree_kill_reliability: TreeKillReliability,

    /// Platform-specific warnings (grouping failures, permission limits, etc.)
    pub warnings: Vec<String>,
}
```

**Platform notes:**
- **Unix**: Creates new process group via `setpgid(0, 0)`. Returns `pgid`.
- **Windows**: Uses Job Objects when possible; `pgid` is not applicable (null/None). v0.1.6 does not expose a stable Job handle/token yet (planned follow-on).
- **Degradation**: If grouping fails (nested Job Objects, privilege limits), returns `tree_kill_reliability: BestEffort`.

### 4.4 TerminateTree Types (v0.1.6+)

```rust
/// Configuration for terminate_tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminateTreeConfig {
    /// Initial signal (default: SIGTERM).
    #[serde(default = "default_signal")]
    pub signal: i32,

    /// Grace timeout in milliseconds before escalating to kill (default: 10000).
    #[serde(default = "default_grace_timeout_ms")]
    pub grace_timeout_ms: u64,

    /// Kill signal (default: SIGKILL).
    #[serde(default = "default_kill_signal")]
    pub kill_signal: i32,

    /// Timeout after kill signal in milliseconds (default: 2000).
    #[serde(default = "default_kill_timeout_ms")]
    pub kill_timeout_ms: u64,
}

/// Outcome of terminate_tree (schema-backed).
#[derive(Debug, Clone, Serialize)]
pub struct TerminateTreeResult {
    /// Schema identifier for version detection.
    pub schema_id: &'static str,

    /// Timestamp (RFC3339).
    pub timestamp: String,

    /// Platform identifier.
    pub platform: &'static str,

    /// PID that was terminated.
    pub pid: u32,

    /// Process group ID if available (Unix only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pgid: Option<u32>,

    /// Signal that was sent for graceful termination.
    pub signal_sent: i32,

    /// Kill signal sent during escalation (if escalated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kill_signal: Option<i32>,

    /// True if escalation to kill signal was required.
    pub escalated: bool,

    /// True if the process tree exited.
    pub exited: bool,

    /// True if overall operation timed out.
    pub timed_out: bool,

    /// Reliability of tree-kill operation ("guaranteed" or "best_effort").
    pub tree_kill_reliability: String,

    /// Platform-specific warnings.
    pub warnings: Vec<String>,
}
```

**Platform notes:**
- **Unix**: Sends signal to process group if pgid available, otherwise direct PID.
- **Windows**: Terminates Job Object if available, otherwise direct TerminateProcess. v0.1.6 does not expose a stable Job token in the API surface yet; tree kill without a Job is best-effort.

### 4.5 Error Handling

Per ADR-0008:

| Error | Condition | CLI Exit |
|-------|-----------|----------|
| `NotFound` | Command not found | 127 |
| `PermissionDenied` | Command not executable | 126 |
| `SpawnFailed` | Failed to spawn process | 125 |
| `GroupCreationFailed` | Process group/job creation failed | (continues with best-effort) |

**Note:** The "CLI Exit" column applies to the `sysprims timeout` CLI contract only. Library APIs (`spawn_in_group`, `terminate_tree`) return `SysprimsError` directly and do not define exit codes.

### 4.6 Invariants

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
| spawn_in_group (v0.1.6) | spec §4.3 | `spawn_in_group` | - | `test_spawn_in_group_*` | Pass |
| spawn_in_group PID validation | ADR-0011 | `spawn_in_group` | - | implicit | Pass |
| terminate_tree (v0.1.6) | spec §4.4 | `terminate_tree` | - | `test_terminate_tree_*` | Pass |
| terminate_tree PID validation | ADR-0011 | `terminate_tree` | - | `test_terminate_tree_invalid_pid` | Pass |
| terminate_tree escalation | spec §4.4 | `TerminateTreeResult::escalated` | - | `test_terminate_tree_escalates` | Pass |

---

*Spec version: 1.1*
*Last updated: 2026-01-25*
