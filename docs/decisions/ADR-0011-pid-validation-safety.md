# ADR-0011: PID Validation Safety

> **Status**: Accepted
> **Date**: 2026-01-08
> **Authors**: Architecture Council, devlead
> **Incident**: .plans/tasks/cargo-crash/incident-2026-01-08.md

## Context

During development of `sysprims-signal`, a test that sent `SIGTERM` to `u32::MAX` caused a macOS Finder crash. Investigation revealed the root cause:

```rust
// Test intended to verify "nonexistent PID" handling
kill_impl(u32::MAX, SIGTERM)
         ↓
libc::kill(u32::MAX as i32, 15)
         ↓
libc::kill(-1, 15)  // u32::MAX wraps to -1 as signed i32!
```

**POSIX defines special semantics for negative PIDs in `kill(2)`:**

| PID Value | Semantics |
|-----------|-----------|
| `> 0` | Send signal to process with that PID |
| `0` | Send signal to all processes in the caller's process group |
| `-1` | Send signal to **ALL processes** the caller has permission to signal |
| `< -1` | Send signal to all processes in process group `abs(pid)` |

The test inadvertently invoked `kill(-1, SIGTERM)`, which sent `SIGTERM` to every user-owned process including Finder, Terminal, and potentially hundreds of other processes.

### Why This Matters for sysprims

sysprims is a process control library. Its users will:
- Parse PIDs from external input (config files, CLI arguments, APIs)
- Store PIDs in various integer types
- Pass PIDs between languages via FFI

Any of these could result in an out-of-range value reaching `kill()`. The consequences of `kill(-1, sig)` are catastrophic and non-obvious. A library designed to kill processes must make it **impossible** to accidentally kill everything.

## Decision

**Validate all PIDs at the public API boundary** to reject values that would overflow to dangerous negative values when cast to `pid_t` (i32).

### Implementation

1. **Define `MAX_SAFE_PID`** as `i32::MAX` (2,147,483,647):
   ```rust
   pub const MAX_SAFE_PID: u32 = i32::MAX as u32;
   ```

2. **Validate in `kill()` and `killpg()`** before any system call:
   ```rust
   fn validate_pid(pid: u32, param_name: &str) -> SysprimsResult<()> {
       if pid == 0 {
           return Err(SysprimsError::invalid_argument(
               format!("{param_name} must be > 0")
           ));
       }
       if pid > MAX_SAFE_PID {
           return Err(SysprimsError::invalid_argument(format!(
               "{param_name} {} exceeds maximum safe value {}; \
                larger values overflow to negative PIDs with dangerous POSIX semantics",
               pid, MAX_SAFE_PID
           )));
       }
       Ok(())
   }
   ```

3. **Error messages reference documentation** explaining the danger.

4. **Comprehensive tests** verify both boundaries:
   - `kill(0, sig)` → rejected (would signal caller's group)
   - `kill(u32::MAX, sig)` → rejected (would broadcast)
   - `kill(i32::MAX + 1, sig)` → rejected (first unsafe value)
   - `kill(i32::MAX, sig)` → allowed (last safe value)

### Rejected PIDs

| Value | Rejection Reason |
|-------|------------------|
| `0` | Would signal caller's process group |
| `> i32::MAX` | Overflows to negative, triggering broadcast or group semantics |

### Accepted PIDs

| Value | Result |
|-------|--------|
| `1` to `i32::MAX` | Passed to kernel, may return `ESRCH` (not found) or `EPERM` (permission denied) |

## Consequences

### Positive

- **Impossible to accidentally broadcast signals** - the most dangerous POSIX behavior is blocked
- **Clear error messages** - users understand why their PID was rejected
- **Defense in depth** - validation at API boundary catches bugs in calling code
- **FFI safety** - protects against integer width mismatches across language boundaries

### Negative

- **Cannot signal "all processes"** - intentional use of `kill(-1, sig)` is blocked
  - Mitigation: This is an administrative operation; users can call libc directly if needed
- **Theoretical valid PIDs rejected** - PIDs above 2^31 are technically possible on some systems
  - Mitigation: No mainstream OS uses PIDs above 2^31; Linux `pid_max` defaults to 32768

### Neutral

- Error code is `InvalidArgument` (FFI code 1), consistent with ADR-0008
- The `MAX_SAFE_PID` constant is public for documentation purposes

## Alternatives Considered

### Alternative 1: Use i32 for PIDs in the API

Reject at compile time by making the PID parameter `i32` instead of `u32`.

**Rejected because:**
- FFI compatibility: C and other languages typically use unsigned types for PIDs
- Rust stdlib uses `u32` for `std::process::id()`
- Negative PIDs have valid but dangerous semantics; accepting them invites misuse

### Alternative 2: Check only at platform implementation level

Validate in `unix::kill_impl()` rather than `lib.rs::kill()`.

**Rejected because:**
- Duplicates validation across platforms
- Platform code receives already-casted values, too late to give good errors
- Public API should document and enforce its contract

### Alternative 3: Allow negative PIDs with explicit opt-in

Provide `kill_raw(pid: i32, sig)` for users who need broadcast semantics.

**Rejected because:**
- Adds API surface for a rare administrative use case
- Users who truly need this can call libc directly
- "Making dangerous things hard" is the design goal

### Alternative 4: Use `pid_t` alias from libc

Make sysprims use `libc::pid_t` directly in its public API.

**Rejected because:**
- `pid_t` is `i32` on all supported platforms, but this is an implementation detail
- Would leak libc types into FFI boundary
- Doesn't solve the negative-value problem; just changes where it surfaces

## Future Work: Container-Based Test Harness

For tests that genuinely need to exercise dangerous POSIX behaviors (e.g., verifying `kill(-1, sig)` semantics), we should provide an **optional container-based test harness**:

```bash
# Run "diabolical" tests safely in a container
make test-diabolical
```

This would:
1. Spin up a disposable Linux container (Docker/Podman)
2. Run tests that intentionally trigger broadcast signals, orphan processes, etc.
3. Verify expected behavior without risking the host system
4. Be opt-in and excluded from normal `cargo test`

Implementation deferred to v0.2.0 when we have the tree-escape integration tests.

## References

- [POSIX kill(2)](https://pubs.opengroup.org/onlinepubs/9699919799/functions/kill.html) - Official semantics for negative PIDs
- [Linux kill(2)](https://man7.org/linux/man-pages/man2/kill.2.html) - Linux-specific behavior
- [Incident Report](.plans/tasks/cargo-crash/incident-2026-01-08.md) - The Finder crash that prompted this ADR
- ADR-0008: Error Handling Strategy - Error type taxonomy
- ADR-0004: FFI Design - Why we use u32 for PIDs at the FFI boundary
