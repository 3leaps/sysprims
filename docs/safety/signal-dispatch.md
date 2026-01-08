# Signal Dispatch Safety Guide

> **Audience**: sysprims users and contributors
> **Related ADR**: [ADR-0011: PID Validation Safety](../architecture/adr/0011-pid-validation-safety.md)

## Overview

sysprims is a process control library. By design, it can terminate processes. This power comes with responsibility - incorrect use can kill unintended processes, including critical system components.

This document explains:
1. POSIX signal semantics that make certain PID values dangerous
2. How sysprims protects against accidental misuse
3. What you need to know when using the library

## POSIX Signal Semantics

The `kill(2)` system call has special behavior for certain PID values:

| PID Value | What Happens |
|-----------|--------------|
| `> 0` | Signal sent to that specific process |
| `0` | Signal sent to **all processes in caller's process group** |
| `-1` | Signal sent to **ALL processes** caller can signal |
| `< -1` | Signal sent to all processes in process group `abs(pid)` |

### The Danger of kill(-1, sig)

`kill(-1, SIGTERM)` sends `SIGTERM` to every process you have permission to signal. On a typical workstation, this includes:

- Your terminal emulator
- Your file manager (Finder, Nautilus, etc.)
- Your IDE
- Your browser
- Your music player
- Hundreds of other processes

**This will crash your desktop session.**

### How Integer Overflow Creates This Danger

sysprims uses `u32` for PIDs (for cross-platform FFI compatibility). POSIX uses `pid_t`, which is `i32` on all mainstream platforms.

When a large `u32` is cast to `i32`:

```
u32::MAX (4294967295) as i32 = -1
(i32::MAX as u32 + 1) as i32 = -2147483648
```

If your code accidentally passes `u32::MAX` to a signal function, and that value reaches the kernel as `-1`, you've just signaled everything.

## sysprims Protections

### Automatic PID Validation

All sysprims signal functions validate PIDs before any system call:

```rust
// These are rejected at the API boundary:
kill(0, SIGTERM);        // Error: pid must be > 0
kill(u32::MAX, SIGTERM); // Error: pid exceeds maximum safe value

// This is allowed (will likely return NotFound):
kill(99999, SIGTERM);    // OK: valid PID, probably doesn't exist
```

### The MAX_SAFE_PID Constant

```rust
pub const MAX_SAFE_PID: u32 = i32::MAX as u32; // 2,147,483,647
```

PIDs above this value are rejected because they would overflow to negative values.

### Clear Error Messages

When validation fails, the error message explains why:

```
Invalid argument: pid 4294967295 exceeds maximum safe value 2147483647;
larger values overflow to negative PIDs with dangerous POSIX semantics
(see docs/safety/signal-dispatch.md)
```

## Safe Usage Patterns

### Validating External Input

If you receive PIDs from external sources (CLI, config files, APIs), validate them:

```rust
use sysprims_signal::{kill, MAX_SAFE_PID, SIGTERM};

fn terminate_from_config(pid_str: &str) -> Result<(), Box<dyn Error>> {
    let pid: u32 = pid_str.parse()?;

    // sysprims will validate, but you may want to check earlier:
    if pid == 0 || pid > MAX_SAFE_PID {
        return Err("Invalid PID".into());
    }

    kill(pid, SIGTERM)?;
    Ok(())
}
```

### Handling Parse Errors

Be careful with integer parsing - overflow is silent:

```rust
// DANGEROUS: silently wraps on overflow
let pid: u32 = some_i64_value as u32;

// SAFE: explicit conversion with bounds checking
let pid: u32 = u32::try_from(some_i64_value)?;
```

### Testing Process Control Code

Never test with arbitrary PIDs on your host system:

```rust
// DANGEROUS: What if 99999 exists and is important?
kill(99999, SIGTERM)?;

// SAFER: Use your own PID for error path testing
let my_pid = std::process::id();
let result = kill(my_pid, -1); // Invalid signal, won't actually kill
assert!(result.is_err());

// SAFEST: Use container-based test harness (see below)
```

## Container-Based Testing

For tests that need to exercise dangerous behaviors, use the container test harness:

```bash
# Run "diabolical" tests in a disposable container
make test-diabolical
```

This is the only safe way to test:
- Broadcast signal behavior (`kill -1`)
- Process group signal behavior (`killpg`)
- Tree escape scenarios
- Orphan process cleanup

## Platform Differences

### macOS

- PID 1 is `launchd` (the init system)
- Signaling PID 1 may have unexpected effects
- Maximum PID is typically around 99999

### Linux

- PID 1 is typically `systemd` or `init`
- `pid_max` is configurable (default: 32768, max: 4194304)
- PIDs above `pid_max` don't exist but are "safe" to signal (returns `ESRCH`)

### Windows

- PIDs are actually handles, not small integers
- `kill()` maps to `TerminateProcess()` for SIGTERM/SIGKILL
- `killpg()` returns `NotSupported`
- No broadcast signal semantics
- `SIGINT` uses `GenerateConsoleCtrlEvent` and is best-effort; it only works
  when the target is in the same console/process group and may fail otherwise

## Summary

| Situation | sysprims Behavior |
|-----------|-------------------|
| `pid == 0` | Rejected with `InvalidArgument` |
| `pid > i32::MAX` | Rejected with `InvalidArgument` |
| `pid` doesn't exist | Returns `NotFound` |
| No permission to signal `pid` | Returns `PermissionDenied` |
| Valid `pid`, valid signal | Signal sent |

**sysprims makes it impossible to accidentally broadcast signals.** This is intentional and non-negotiable. If you genuinely need `kill(-1, sig)` semantics, call libc directly - but understand the consequences.

## Further Reading

- [ADR-0011: PID Validation Safety](../architecture/adr/0011-pid-validation-safety.md) - Full rationale
- [POSIX kill(2)](https://pubs.opengroup.org/onlinepubs/9699919799/functions/kill.html) - Official specification
- [ADR-0003: Group-by-Default](../architecture/adr/0003-group-by-default.md) - Why sysprims exists
