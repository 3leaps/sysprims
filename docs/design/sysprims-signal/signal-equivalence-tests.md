---
title: "sysprims-signal Equivalence Test Protocol"
module: "sysprims-signal"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
---

# sysprims-signal Equivalence Test Protocol

## 1) Purpose

Validate that sysprims-signal:

1. **Matches POSIX `kill` behavior** where applicable (Unix platforms)
2. **Has stable, predictable error mapping** across platforms
3. **Is honest about Windows limitations** (returns NotSupported, not fake success)
4. **Prevents dangerous operations** (PID 0, PID overflow per ADR-0011)

## 2) Reference Implementations

### Unix

| Tool                  | License                         | Usage                 |
| --------------------- | ------------------------------- | --------------------- |
| System `kill` utility | POSIX (various implementations) | Subprocess comparison |

**Note:** We compare _behavior_, not code. System `kill` may be from util-linux (GPL), BSD, or busybox — we invoke it as subprocess only.

### Windows

No POSIX `kill` equivalent. Tests verify:

- Self-consistency with documented behavior
- TerminateProcess semantics
- Proper NotSupported errors for unsupported signals

## 3) Test Matrix

### Platforms

| Platform      | CI Runner        | Notes              |
| ------------- | ---------------- | ------------------ |
| Linux (glibc) | ubuntu-latest    | POSIX reference    |
| Linux (musl)  | alpine container | Static binary      |
| macOS (arm64) | macos-latest     | BSD-derived        |
| Windows (x64) | windows-latest   | NotSupported paths |

**Privilege:** Non-root in standard CI.

## 4) Test Categories

### Category A: Signal Parsing

| Test Case  | Input     | Expected |
| ---------- | --------- | -------- |
| Full name  | `SIGTERM` | 15       |
| Short name | `TERM`    | 15       |
| Lowercase  | `term`    | 15       |
| Short ID   | `int`     | 2        |
| Invalid    | `SIGFAKE` | Error    |
| Empty      | ``        | Error    |

### Category B: Signal Delivery

| Test Case                    | Expected                | Notes            |
| ---------------------------- | ----------------------- | ---------------- |
| Send TERM to running process | Process receives signal | Use test fixture |
| Send KILL to running process | Process terminated      | Unconditional    |
| Send TERM to own process     | Allowed                 | Self-signal      |

### Category C: Error Cases

| Test Case               | Expected Error   | Code |
| ----------------------- | ---------------- | ---- |
| PID does not exist      | NotFound         | 5    |
| PID owned by other user | PermissionDenied | 4    |
| PID 0                   | InvalidArgument  | 1    |
| PID u32::MAX            | InvalidArgument  | 1    |
| PID i32::MAX + 1        | InvalidArgument  | 1    |

**Critical (ADR-0011):** PID validation tests are safety-critical:

```rust
// These MUST fail validation before reaching kernel
kill(0, SIGTERM)       // Would signal caller's group
kill(u32::MAX, SIGTERM) // Would signal ALL processes
```

### Category D: Windows-Specific

| Test Case | Expected             | Notes                 |
| --------- | -------------------- | --------------------- |
| SIGTERM   | Process terminated   | Via TerminateProcess  |
| SIGKILL   | Process terminated   | Via TerminateProcess  |
| SIGINT    | Best-effort or error | Console-dependent     |
| SIGHUP    | NotSupported         | No Windows equivalent |
| killpg    | NotSupported         | No process groups     |

### Category E: Process Group (Unix-only)

| Test Case               | Expected                   | Platform |
| ----------------------- | -------------------------- | -------- |
| killpg to child's group | All group members signaled | Unix     |
| killpg on Windows       | NotSupported error         | Windows  |

### Category F: Signal Listing/Matching

| Test Case  | Pattern   | Expected               |
| ---------- | --------- | ---------------------- |
| Glob match | `SIGT*`   | SIGTERM, SIGTSTP, etc. |
| No match   | `SIGFOO*` | Empty                  |

## 5) Determinism and Flake Policy

### Signal Delivery Timing

- Signal delivery is asynchronous; tests must wait for effect
- Use poll loops with reasonable timeout
- Accept that signal delivery order is not guaranteed

### Acceptable Variations

| Aspect         | Tolerance                                           |
| -------------- | --------------------------------------------------- |
| Error messages | Content may vary; error type must match             |
| Signal numbers | SIGUSR1/SIGUSR2 differ by platform (10/12 vs 30/31) |

## 6) Test Locations

| Test Type                   | Location                                  |
| --------------------------- | ----------------------------------------- |
| Unit tests (PID validation) | `crates/sysprims-signal/src/lib.rs`       |
| Integration tests           | `crates/sysprims-signal/tests/` (planned) |
| Equivalence harness         | `tests/equivalence/signal/` (planned)     |

## 7) Traceability to Spec

| Spec Requirement            | Test Category | Test IDs                             |
| --------------------------- | ------------- | ------------------------------------ |
| Parse signal names          | A             | `resolve_signal_number_*`            |
| PID 0 rejected              | C             | `kill_rejects_pid_zero`              |
| PID overflow rejected       | C             | `kill_rejects_pid_exceeding_*`       |
| MAX_SAFE_PID boundary       | C             | `kill_accepts_pid_at_max_safe`       |
| Windows killpg NotSupported | D             | `killpg_is_not_supported_on_windows` |
| Error mapping               | C             | integration tests                    |

---

_Protocol version: 1.0_
_Last updated: 2026-01-09_
