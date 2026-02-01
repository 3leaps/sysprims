---
title: "sysprims-signal Module Spec"
module: "sysprims-signal"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
adr_refs: ["ADR-0007", "ADR-0008", "ADR-0011"]
---

# sysprims-signal Module Spec

## 1) Overview

**Purpose:** Provide a stable, embeddable signaling/termination API and thin CLI wrapper compatible with POSIX `kill` where applicable, while being honest about Windows limitations.

**Safety critical (ADR-0011):** This module validates all PIDs at the API boundary to prevent dangerous POSIX signal semantics:

- `kill(0, sig)` — signals caller's process group
- `kill(-1, sig)` — signals ALL processes the caller can reach
- `u32::MAX as i32 = -1` — integer overflow creates catastrophic behavior

**In scope (v0.1.0):**

- Send signal to individual process by PID (`kill`)
- Send signal to process group by PGID (`killpg`, Unix-only)
- Parse signals by name/number (via rsfulmen catalog)
- List/match supported signals
- Clear error mapping (NotFound, PermissionDenied, NotSupported)
- Convenience wrappers (`terminate`, `force_kill`)

**Out of scope (v0.1.0):**

- Pattern/name-based killing (`pkill` semantics)
- Process group killing on Windows (returns NotSupported)

## 2) Normative References

### POSIX (authoritative for Unix)

**`kill` utility:**
- https://pubs.opengroup.org/onlinepubs/9699919799/utilities/kill.html

**`kill()` function:**
- https://pubs.opengroup.org/onlinepubs/9699919799/functions/kill.html
- Linux man page: https://man7.org/linux/man-pages/man2/kill.2.html

### Key POSIX Semantics

| PID Value | Semantics |
|-----------|-----------|
| `> 0` | Signal sent to that specific process |
| `0` | Signal sent to all processes in caller's process group |
| `-1` | Signal sent to **ALL** processes caller has permission to signal |
| `< -1` | Signal sent to all processes in process group `abs(pid)` |

### Windows

**TerminateProcess:**
- https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-terminateprocess

**GenerateConsoleCtrlEvent (for SIGINT):**
- https://learn.microsoft.com/en-us/windows/console/generateconsolectrlevent

## 3) Literal Interface Reference (POSIX kill)

### Synopsis

```
kill [-s signal_name] pid...
kill -l [exit_status]
```

### Default behavior

- Default signal: SIGTERM
- Signal 0 tests process existence without sending a signal

### Exit status

- 0 — at least one signal was sent successfully
- >0 — an error occurred

## 4) sysprims Required Interface (Rust)

### 4.1 Constants

```rust
/// Maximum valid PID value (i32::MAX).
/// PIDs above this overflow to negative when cast to pid_t.
pub const MAX_SAFE_PID: u32 = i32::MAX as u32; // 2,147,483,647
```

### 4.2 Public Functions

```rust
/// Send signal to a process.
///
/// # Errors
/// - InvalidArgument: pid == 0 or pid > MAX_SAFE_PID
/// - NotFound: process doesn't exist
/// - PermissionDenied: insufficient privileges
/// - NotSupported: signal not supported on platform (Windows)
pub fn kill(pid: u32, signal: i32) -> SysprimsResult<()>;

/// Send signal to a process, resolving signal by name.
/// Accepts: "SIGTERM", "TERM", "term", "15"
pub fn kill_by_name(pid: u32, signal_name: &str) -> SysprimsResult<()>;

/// Send signal to a process group (Unix only).
/// Returns NotSupported on Windows.
pub fn killpg(pgid: u32, signal: i32) -> SysprimsResult<()>;

/// Return signal names matching a glob pattern.
/// Supports * (any sequence) and ? (single char).
pub fn match_signal_names(pattern: &str) -> Vec<&'static str>;

// Convenience wrappers
pub fn terminate(pid: u32) -> SysprimsResult<()>;      // SIGTERM
pub fn force_kill(pid: u32) -> SysprimsResult<()>;     // SIGKILL
pub fn terminate_group(pgid: u32) -> SysprimsResult<()>; // Unix only
pub fn force_kill_group(pgid: u32) -> SysprimsResult<()>; // Unix only
```

### 4.3 Signal Constants

Re-exported from rsfulmen:

```rust
pub use rsfulmen::foundry::signals::*;
// Provides: SIGTERM, SIGKILL, SIGINT, SIGHUP, etc.
// Plus: get_signal_number(), list_signals(), lookup_signal_by_id()
```

### 4.4 Error Handling

Per ADR-0008:

| Error | Condition |
|-------|-----------|
| `InvalidArgument` | pid == 0, pid > MAX_SAFE_PID, unknown signal name |
| `NotFound` | Process doesn't exist (ESRCH) |
| `PermissionDenied` | Insufficient privileges (EPERM) |
| `NotSupported` | Signal/operation not available on platform |

### 4.5 Invariants

1. **PID validation (ADR-0011):**
   - `pid == 0` → InvalidArgument
   - `pid > MAX_SAFE_PID` → InvalidArgument (with overflow explanation)

2. **Error mapping stability:**
   - ESRCH → NotFound
   - EPERM → PermissionDenied
   - EINVAL → InvalidArgument

3. **No silent failures:** If signal cannot be delivered, return an error.

4. **Windows honesty:** Unsupported signals return NotSupported, not fake success.

## 5) CLI Contract

**Subcommand:** `sysprims kill`

### Synopsis

```
sysprims kill [-s SIGNAL] [--json] <PID> [PID...]
```

### Options

| Option | Description |
|--------|-------------|
| `-s, --signal <SIG>` | Signal to send (name or number, default: TERM) |
| `-g, --group` | Treat PID as a PGID and signal the process group (Unix-only; requires exactly one PID) |
| `--json` | Print per-PID batch result as JSON |

### Exit Codes

| Code | Condition |
|------|-----------|
| 0 | All targets signaled successfully |
| 1 | Any target failed (or argument/parse error) |

## 6) Platform Signal Mapping

| Signal | Linux | macOS | Windows |
|--------|-------|-------|---------|
| SIGTERM (15) | Native | Native | TerminateProcess |
| SIGKILL (9) | Native | Native | TerminateProcess |
| SIGINT (2) | Native | Native | GenerateConsoleCtrlEvent* |
| SIGHUP (1) | Native | Native | NotSupported |
| SIGUSR1 | Native (10) | Native (30) | NotSupported |
| SIGUSR2 | Native (12) | Native (31) | NotSupported |

*SIGINT on Windows is best-effort and depends on console attachment.

## 7) FFI Contract

```c
// Send signal to process
SysprimsErrorCode sysprims_signal_send(uint32_t pid, int32_t signal);

// Send signal to process group (Unix only)
SysprimsErrorCode sysprims_signal_send_group(uint32_t pgid, int32_t signal);

// Convenience wrappers
SysprimsErrorCode sysprims_terminate(uint32_t pid);
SysprimsErrorCode sysprims_force_kill(uint32_t pid);
```

## 8) Traceability Matrix

| Requirement | Reference | Rust API | CLI | Tests | Status |
|-------------|-----------|----------|-----|-------|--------|
| Parse TERM/SIGTERM/15 | POSIX | `kill_by_name` | `-s` | `resolve_signal_number_*` | Pass |
| Send TERM to PID | POSIX | `kill(pid, SIGTERM)` | default | integration | Pass |
| NotFound for missing PID | POSIX ESRCH | `NotFound` | any | integration | Pass |
| PermissionDenied | POSIX EPERM | `PermissionDenied` | any | integration | Pass |
| Reject PID 0 | ADR-0011 | `validate_pid` | any | `kill_rejects_pid_zero` | Pass |
| Reject PID > MAX_SAFE_PID | ADR-0011 | `validate_pid` | any | `kill_rejects_pid_exceeding_*` | Pass |
| NotSupported for Windows | ADR-0007 | `NotSupported` | any | `killpg_is_not_supported_*` | Pass |

---

*Spec version: 1.0*
*Last updated: 2026-01-09*
