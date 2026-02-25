---
title: "sysprims-timeout Provenance"
module: "sysprims-timeout"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
---

# Provenance: sysprims-timeout

This document records the sources consulted for implementing `sysprims-timeout`, ensuring clear provenance for all functionality.

## Policy

- POSIX and platform specifications are the primary reference
- BSD/MIT/Apache licensed implementations may be consulted for understanding
- Behavioral comparison against tools (via subprocess) is permitted for testing
- Source code of restrictively-licensed tools (GPL/LGPL/AGPL) is NOT consulted

## Timeout Execution

### Specification Sources (Primary)

1. **GNU coreutils timeout manual** (behavioral reference only)
   - URL: https://www.gnu.org/software/coreutils/manual/html_node/timeout-invocation.html
   - License: GFDL (documentation)
   - Used for: Exit code semantics (124, 125, 126, 127), CLI option design
   - Note: Documentation only, NOT source code

2. **POSIX setpgid()**
   - URL: https://pubs.opengroup.org/onlinepubs/9699919799/functions/setpgid.html
   - License: Specification (no license restriction)
   - Used for: Process group creation semantics

3. **POSIX kill() / killpg()**
   - URL: https://pubs.opengroup.org/onlinepubs/9699919799/functions/kill.html
   - License: Specification (no license restriction)
   - Used for: Process group signaling

4. **Windows Job Objects**
   - URL: https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects
   - License: Microsoft documentation
   - Used for: Windows tree-kill implementation

### Implementation References (Consulted)

1. **Rust std::process** (MIT/Apache-2.0)
   - URL: https://doc.rust-lang.org/std/process/
   - License: MIT/Apache-2.0
   - Used for: Process spawning, exit status handling

2. **libc crate** (MIT/Apache-2.0)
   - URL: https://github.com/rust-lang/libc
   - License: MIT/Apache-2.0
   - Used for: setpgid, killpg bindings

3. **windows crate** (MIT/Apache-2.0)
   - URL: https://github.com/microsoft/windows-rs
   - License: MIT/Apache-2.0
   - Used for: Job Object API bindings

### NOT Consulted

- GNU coreutils timeout.c (GPL-3.0) - NOT consulted
- util-linux (GPL-2.0) - NOT consulted

### Implementation Notes

**Unix implementation:**

1. Spawn child with `pre_exec` hook calling `setpgid(0, 0)`
2. Child becomes process group leader
3. On timeout, call `killpg(-pgid, signal)` to signal entire group
4. Wait for group to exit; escalate to SIGKILL if needed

**Windows implementation:**

1. Create Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
2. Spawn child and assign to job
3. On timeout, job termination kills all processes in job
4. Alternative: `TerminateProcess` for direct child

This is straightforward use of documented platform APIs.

## Group-by-Default Design (ADR-0003)

### Design Decision

Make tree-kill the default, not opt-in.

### Rationale

GNU timeout kills only the direct child. This leaves orphaned processes running when:

- Child spawns grandchildren
- Grandchildren ignore SIGTERM
- Child forks and exits (orphaning the grandchild)

sysprims-timeout uses process groups (Unix) or Job Objects (Windows) to kill the entire tree.

### Observable Fallback

When tree-kill cannot be guaranteed (e.g., setpgid fails), the implementation:

- Falls back to direct child kill
- Reports `tree_kill_reliability: best_effort` in output
- Does NOT silently fail or lie about behavior

This design principle is original to sysprims.

## Exit Code Convention

### Design Decision

Match GNU timeout exit codes for compatibility.

| Exit  | Meaning                |
| ----- | ---------------------- |
| 124   | Timeout occurred       |
| 125   | Internal error         |
| 126   | Command not executable |
| 127   | Command not found      |
| 128+N | Killed by signal N     |

### Rationale

These codes are widely expected by scripts and CI systems. Matching them improves adoption.

### Implementation

Simple mapping in CLI wrapper - no complex logic needed.

## Behavioral Testing

Tests may compare output against GNU timeout via subprocess:

```rust
#[test]
#[cfg(unix)]
fn exit_code_matches_gnu_timeout() {
    // GNU timeout
    let gnu = Command::new("timeout")
        .args(&["1s", "sleep", "60"])
        .status()
        .unwrap();

    // sysprims
    let ours = Command::new("sysprims")
        .args(&["timeout", "1s", "--", "sleep", "60"])
        .status()
        .unwrap();

    assert_eq!(gnu.code(), Some(124));
    assert_eq!(ours.code(), Some(124));
}
```

## Certification

This module's implementation is derived from:

- [x] Public specifications (POSIX, Windows docs)
- [x] GNU timeout documentation (behavioral reference, not source)
- [x] Permissively-licensed references (Rust std, libc, windows crates)
- [x] Original design for group-by-default and observable fallback

No GPL/LGPL/AGPL source code was consulted during development.

---

_Provenance version: 1.0_
_Last updated: 2026-01-09_
_Maintainer: sysprims team_
