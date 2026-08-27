---
title: "sysprims-timeout Module Spec"
module: "sysprims-timeout"
version: "1.2"
status: "Active"
last_updated: "2026-08-27"
adr_refs: ["ADR-0003", "ADR-0005", "ADR-0007", "ADR-0008", "ADR-0011"]
---

# sysprims-timeout Module Spec

## 1) Overview

**Purpose:** Provide a library-first `run_with_timeout()` primitive and thin
CLI wrapper that match widely expected `timeout` semantics while acquiring and
signaling a process group or Job by default, with observable fallbacks.

**Core differentiator (ADR-0003):** Unlike GNU timeout, which targets only the
direct child, sysprims-timeout acquires and signals a cooperative process group
or owned Job. `Guaranteed` means race-free spawn-time acquisition and
group/job-signaling eligibility. It does not claim that Unix descendants cannot
later leave a process group.

**In scope (v0.1.0):**

- Duration parsing (e.g., `250ms`, `2s`, `5m`, `1h`)
- Run command with deadline
- Choose initial signal (default SIGTERM)
- Escalate to SIGKILL after `kill_after` delay
- `--preserve-status` behavior (propagate child exit code on normal completion)
- Group-by-default process control:
  - Unix: owned `setsid` acquisition with a sealed same-spawn receipt and
    `killpg()`; legacy PID-returning paths are explicitly `best_effort`
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

| Exit Code | Condition                                    |
| --------- | -------------------------------------------- |
| 124       | Command timed out                            |
| 125       | `timeout` itself failed                      |
| 126       | Command found but cannot be invoked          |
| 127       | Command not found                            |
| 128+N     | Command killed by signal N                   |
| Other     | Child's exit code (with `--preserve-status`) |

## 4) sysprims Required Interface (Rust)

### 4.1 Core Types

```rust
/// Process grouping strategy (ADR-0003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupingMode {
    /// Create new process group (Unix) or Job Object (Windows).
    /// Signal the acquired group/job on timeout. **This is the default.**
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
    /// Spawn-time acquisition and group/job signaling eligibility were proven.
    /// This is not a Unix descendant non-escape guarantee.
    Guaranteed,
    /// An owned containment capability was attached after spawn.
    Unproven,
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

/// Spawn a process in a new process group (v0.1.6+; Unix compatibility API).
///
/// Creates a child process in a new Unix process group.
/// Does not retain ownership and therefore reports best-effort reliability.
pub fn spawn_in_group(config: SpawnInGroupConfig) -> SysprimsResult<SpawnInGroupResult>;

/// Spawn a child with an owned process-group or Job Object guard.
pub fn spawn_contained(command: Command)
    -> Result<ContainmentGuard<Child>, ContainmentSpawnError>;

/// Adopt an externally spawned child while retaining its reap capability.
pub fn adopt_contained<C: ContainmentChild>(child: C)
    -> Result<ContainmentGuard<C>, ContainmentAdoptionError<C>>;

/// Consume an opaque same-spawn Unix session receipt with its owned child.
pub unsafe fn contain_acquired_session<C: ContainmentChild>(
    child: C,
    receipt: sysprims_session::UnixSessionReceipt,
) -> Result<ContainmentGuard<C>, ContainmentAdoptionError<C>>;

/// Observe normal leader exit without reaping, then clean descendants and reap.
impl<C: ContainmentChild> ContainmentGuard<C> {
    pub fn try_complete(&mut self, config: TerminateTreeConfig)
        -> SysprimsResult<Option<ContainmentOutcome>>;
}

/// Terminate a process by PID with graceful-then-kill escalation (v0.1.6+).
///
/// Works on arbitrary PIDs and reports best-effort reliability.
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

    /// Reliability of the PID-only grouped-spawn compatibility result.
    pub tree_kill_reliability: TreeKillReliability,

    /// Platform-specific warnings (grouping failures, permission limits, etc.)
    pub warnings: Vec<String>,
}
```

**Platform notes:**

- **Unix**: `spawn_contained` uses the `sysprims-session` `setsid` hook and
  sealed acknowledgement receipt. `spawn_in_group` remains a PID-returning
  compatibility API using pre-exec `setpgid(0, 0)` and reports `BestEffort`
  because it cannot retain the receipt and owned child.
- **Windows**: The PID-returning compatibility API fails closed because it cannot return an owned Job capability. Use `adopt_contained` for explicitly `Unproven` post-spawn ownership. `spawn_contained` remains unsupported until it can assign a suspended child to the Job before execution.
- **Degradation**: `Unproven` means an owned capability exists but post-spawn assignment left an escape window. `BestEffort` means no tree capability exists.

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

    /// Reliability of PID-based termination (always "best_effort").
    pub tree_kill_reliability: String,

    /// Platform-specific warnings.
    pub warnings: Vec<String>,
}
```

**Platform notes:**

- **Unix**: PID-only termination signals the direct PID. Group signaling
  requires an owned containment guard.
- **Windows**: PID-based termination uses direct `TerminateProcess` and is best-effort. Job termination requires the owned containment guard API.

### 4.5 Error Handling

Per ADR-0008:

| Error                 | Condition                         | CLI Exit                     |
| --------------------- | --------------------------------- | ---------------------------- |
| `NotFound`            | Command not found                 | 127                          |
| `PermissionDenied`    | Command not executable            | 126                          |
| `SpawnFailed`         | Failed to spawn process           | 125                          |
| `GroupCreationFailed` | Process group/job creation failed | (continues with best-effort) |

**Note:** The "CLI Exit" column applies to the `sysprims timeout` CLI contract only. Library APIs (`spawn_in_group`, `terminate_tree`) return `SysprimsError` directly and do not define exit codes.

### 4.6 Invariants

1. **Timeout invariant:** If deadline reached, `TimedOut` must be returned and CLI exit must be `124`.

2. **Group-by-default invariant (ADR-0003):**
   - Unix owned spawn: the child acquires `sid == pgid == pid` via one
     async-signal-safe `setsid` hook, and a private fixed-size acknowledgement
     proves hook execution. Generic external-child consumption is unsafe and
     requires same-spawn, exclusive-unreaped ownership; runtime checks remain
     defense-in-depth
   - Unix compatibility spawn: the child runs in its own process group via
     pre-exec `setpgid(0, 0)`
   - Windows: owned adoption assigns the child to a Job Object with `KILL_ON_JOB_CLOSE`; guaranteed owned spawn requires create-suspended assignment
   - Termination targets the group/job, not just the direct child

3. **Observable reliability invariant:** If guaranteed acquisition and
   group/job-signaling eligibility cannot be established:
   - owned post-spawn containment reports `Unproven`
   - direct-child fallback reports `BestEffort`
   - JSON output reflects actual behavior

4. **Guard lifecycle invariant:** The owned guard observes normal leader exit
   without reaping. It revalidates the same-spawn receipt, owned child, captured
   identity, session, and group before signaling; keeps the leader unreaped
   through the final group signal; and only then reaps. After reap the guard is
   inert. Receipt/child mismatch fails closed and returns child ownership.

5. **Signal escalation invariant:** If process doesn't exit after `signal_sent` within `kill_after`, escalate to SIGKILL.

6. **Preserve-status invariant:** `--preserve-status` affects only non-timeout completion.

## 5) CLI Contract

**Subcommand:** `sysprims timeout`

### Synopsis

```
sysprims timeout [OPTIONS] <DURATION> -- <COMMAND> [ARGS...]
```

### Options

| Option                   | Description                | Default |
| ------------------------ | -------------------------- | ------- |
| `-s, --signal <SIG>`     | Signal to send on timeout  | TERM    |
| `-k, --kill-after <DUR>` | Delay before SIGKILL       | 10s     |
| `--preserve-status`      | Propagate child exit code  | false   |
| `--foreground`           | Don't create process group | false   |

### Exit Codes

| Code         | Condition                                           |
| ------------ | --------------------------------------------------- |
| 0            | Command completed normally (no `--preserve-status`) |
| Child's code | Command completed (with `--preserve-status`)        |
| 124          | Command timed out                                   |
| 125          | Internal failure / invalid usage                    |
| 126          | Command found but cannot invoke                     |
| 127          | Command not found                                   |
| 128+N        | Child killed by signal N                            |

## 6) Platform Implementation

| Feature          | Unix                 | Windows                              |
| ---------------- | -------------------- | ------------------------------------ |
| Process grouping | owned `setsid` receipt; compatibility `setpgid(0, 0)` | Job Object |
| Tree kill        | `killpg(-pgid, sig)` | `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` |
| SIGTERM          | Native signal        | TerminateProcess                     |
| SIGKILL          | Native signal        | TerminateProcess                     |

## 7) Traceability Matrix

| Requirement                     | Reference   | Rust API                         | CLI            | Tests                             | Status |
| ------------------------------- | ----------- | -------------------------------- | -------------- | --------------------------------- | ------ |
| Exit code 124 on timeout        | GNU timeout | `TimedOut`                       | exit 124       | integration                       | Pass   |
| Exit code 125 on internal error | GNU timeout | `SysprimsError`                  | exit 125       | integration                       | Pass   |
| Exit code 126 on not executable | GNU timeout | `PermissionDenied`               | exit 126       | integration                       | Pass   |
| Exit code 127 on not found      | GNU timeout | `NotFound`                       | exit 127       | integration                       | Pass   |
| Signal escalation               | GNU timeout | `kill_after`, `escalated`        | `--kill-after` | integration                       | Pass   |
| Group-by-default                | ADR-0003    | `GroupByDefault`                 | default        | tree-escape                       | Pass   |
| Observable fallback             | ADR-0003    | `TreeKillReliability`            | `--json`       | integration                       | Pass   |
| Default SIGTERM                 | spec        | `TimeoutConfig::default()`       | default        | `default_config_*`                | Pass   |
| Default 10s kill_after          | spec        | `TimeoutConfig::default()`       | default        | `default_config_*`                | Pass   |
| spawn_in_group (v0.1.6)         | spec §4.3   | `spawn_in_group`                 | -              | `test_spawn_in_group_*`           | Pass   |
| spawn_in_group PID validation   | ADR-0011    | `spawn_in_group`                 | -              | implicit                          | Pass   |
| terminate_tree (v0.1.6)         | spec §4.4   | `terminate_tree`                 | -              | `test_terminate_tree_*`           | Pass   |
| terminate_tree PID validation   | ADR-0011    | `terminate_tree`                 | -              | `test_terminate_tree_invalid_pid` | Pass   |
| terminate_tree escalation       | spec §4.4   | `TerminateTreeResult::escalated` | -              | `test_terminate_tree_escalates`   | Pass   |

---

_Spec version: 1.2_
_Last updated: 2026-08-27_
