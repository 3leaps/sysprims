---
title: "sysprims-signal Provenance"
module: "sysprims-signal"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
---

# Provenance: sysprims-signal

This document records the sources consulted for implementing `sysprims-signal`, ensuring clear provenance for all functionality.

## Policy

- POSIX and platform specifications are the primary reference
- BSD/MIT/Apache licensed implementations may be consulted for understanding
- Behavioral comparison against tools (via subprocess) is permitted for testing
- Source code of restrictively-licensed tools (GPL/LGPL/AGPL) is NOT consulted

## Signal Dispatch (kill)

### Specification Sources (Primary)

1. **POSIX.1-2017 kill utility**
   - URL: https://pubs.opengroup.org/onlinepubs/9699919799/utilities/kill.html
   - License: Specification (no license restriction)
   - Used for: CLI semantics, exit codes, signal name resolution

2. **POSIX.1-2017 kill() function**
   - URL: https://pubs.opengroup.org/onlinepubs/9699919799/functions/kill.html
   - License: Specification (no license restriction)
   - Used for: API semantics, error codes (ESRCH, EPERM, EINVAL)

3. **Linux kill(2) man page**
   - URL: https://man7.org/linux/man-pages/man2/kill.2.html
   - License: Various (documentation)
   - Used for: Linux-specific behavior details

4. **Windows TerminateProcess**
   - URL: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-terminateprocess
   - License: Microsoft documentation
   - Used for: Windows termination semantics

### Implementation References (Consulted)

1. **rsfulmen** (MIT/Apache-2.0)
   - URL: https://github.com/fulmenhq/rsfulmen
   - License: MIT/Apache-2.0
   - Consulted for: Signal constants, cross-platform signal catalog

2. **libc crate** (MIT/Apache-2.0)
   - URL: https://github.com/rust-lang/libc
   - License: MIT/Apache-2.0
   - Used for: POSIX signal function bindings

### NOT Consulted

- util-linux kill (GPL-2.0) - NOT consulted
- GNU coreutils kill (GPL-3.0) - NOT consulted
- procps-ng (GPL-2.0) - NOT consulted

### Implementation Notes

The implementation is straightforward from POSIX spec:

**Unix:**
1. Validate PID (0 and overflow checks per ADR-0011)
2. Call `libc::kill(pid as i32, signal)`
3. Map errno to SysprimsError

**Windows:**
1. Validate PID
2. Open process handle with PROCESS_TERMINATE
3. Call TerminateProcess for TERM/KILL
4. Return NotSupported for other signals

## Process Group Dispatch (killpg)

### Specification Sources (Primary)

1. **POSIX.1-2017 killpg()**
   - URL: https://pubs.opengroup.org/onlinepubs/9699919799/functions/killpg.html
   - License: Specification (no license restriction)
   - Used for: Process group signal semantics

### Implementation Notes

**Unix:**
1. Validate PGID (same checks as PID)
2. Call `libc::killpg(pgid as i32, signal)`
3. Map errno to SysprimsError

**Windows:**
- Returns `NotSupported` - Windows has no equivalent concept

## PID Validation (ADR-0011)

### Design Decision

Reject PID 0 and PIDs > i32::MAX at the API boundary.

### Rationale

From POSIX kill(2):
- `kill(0, sig)` signals all processes in the caller's process group
- `kill(-1, sig)` signals ALL processes the caller can reach

When `u32::MAX` is cast to `i32`, it becomes `-1`. This means:
```rust
kill(u32::MAX, SIGTERM)  // becomes kill(-1, SIGTERM)
                         // which terminates EVERYTHING
```

This is a real incident documented in ADR-0011 where a test using `u32::MAX` crashed a desktop session.

### Implementation

```rust
fn validate_pid(pid: u32, param_name: &str) -> SysprimsResult<()> {
    if pid == 0 {
        return Err(SysprimsError::invalid_argument(...));
    }
    if pid > MAX_SAFE_PID {
        return Err(SysprimsError::invalid_argument(...));
    }
    Ok(())
}
```

This is simple range checking derived from first principles, not copied from any implementation.

## Behavioral Testing

Tests may compare output against system `kill` via subprocess invocation:

```rust
#[test]
#[cfg(unix)]
fn compare_with_system_kill() {
    // Spawn a child process
    let child = Command::new("sleep").arg("60").spawn().unwrap();
    let pid = child.id();

    // Our implementation
    sysprims_signal::terminate(pid).unwrap();

    // Verify process is gone (behavior comparison)
    let result = sysprims_signal::kill(pid, 0);  // Signal 0 = existence check
    assert!(result.is_err());
}
```

## Certification

This module's implementation is derived from:

- [x] Public specifications (POSIX)
- [x] Permissively-licensed references (rsfulmen, libc)
- [x] Original implementation for validation logic

No GPL/LGPL/AGPL source code was consulted during development.

---

*Provenance version: 1.0*
*Last updated: 2026-01-09*
*Maintainer: sysprims team*
