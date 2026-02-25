---
title: "sysprims-proc Provenance"
module: "sysprims-proc"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
---

# Provenance: sysprims-proc

This document records the sources consulted for implementing `sysprims-proc`, ensuring clear provenance for all functionality.

## Policy

- POSIX and platform specifications are the primary reference
- BSD/MIT/Apache licensed implementations may be consulted for understanding
- Behavioral comparison against tools (via subprocess) is permitted for testing
- Source code of restrictively-licensed tools (GPL/LGPL/AGPL) is NOT consulted

## Process Enumeration

### Specification Sources (Primary)

1. **POSIX ps utility**
   - URL: https://pubs.opengroup.org/onlinepubs/9699919799/utilities/ps.html
   - License: Specification (no license restriction)
   - Used for: Understanding standard field semantics (pid, ppid, user, etc.)

2. **Linux proc(5) man page**
   - URL: https://man7.org/linux/man-pages/man5/proc.5.html
   - License: GPL (documentation only, not code)
   - Used for: Understanding /proc filesystem structure

3. **Apple libproc documentation**
   - URL: Darwin headers and WWDC documentation
   - License: APSL (documentation reference only)
   - Used for: macOS API semantics

4. **Microsoft Toolhelp32 documentation**
   - URL: https://learn.microsoft.com/en-us/windows/win32/toolhelp/tool-help-library
   - License: Microsoft documentation license
   - Used for: Windows process enumeration API

### Implementation References (Consulted)

1. **sysinfo crate** (MIT)
   - URL: https://github.com/GuillaumeGomez/sysinfo
   - License: MIT
   - Consulted for: Cross-platform API patterns, CPU normalization approach

2. **procfs crate** (MIT/Apache-2.0)
   - URL: https://github.com/eminence/procfs
   - License: MIT/Apache-2.0
   - Consulted for: Linux /proc parsing patterns

### NOT Consulted

- procps-ng (GPL-2.0) - NOT consulted
- GNU coreutils (GPL-3.0) - NOT consulted
- util-linux (GPL-2.0) - NOT consulted

### Implementation Notes

The implementation uses standard platform APIs:

**Linux:**

1. Enumerate PIDs via `/proc` directory listing
2. Read `/proc/[pid]/stat` for process state and CPU times
3. Read `/proc/[pid]/statm` for memory usage
4. Read `/proc/[pid]/status` for user ID
5. Read `/proc/[pid]/cmdline` for command line

**macOS:**

1. Use `proc_listpids()` for enumeration
2. Use `proc_pidinfo()` with `PROC_PIDTASKINFO` for details
3. Use `proc_name()` for process name

**Windows:**

1. Use `CreateToolhelp32Snapshot()` for enumeration
2. Use `OpenProcess()` and related APIs for details
3. Use `GetProcessTimes()` for CPU usage
4. Use `GetProcessMemoryInfo()` for memory

This is straightforward platform API usage - no complex algorithms derived from external code.

## CPU Normalization

### Design Decision

CPU percentage is normalized to 0-100 across all cores (not 0-N\*100).

### Rationale

This matches user expectations and the POSIX `ps -o pcpu` convention where 100% means "one full CPU worth of work."

### Implementation

- Calculate CPU time delta over measurement interval
- Divide by elapsed wall time
- Multiply by 100 for percentage
- Clamp to 0-100 range

This is basic arithmetic derived from first principles, not copied from any implementation.

## Filter Validation

### Design Decision

Use serde's `#[serde(deny_unknown_fields)]` for strict input validation.

### Rationale

Prevents typos in filter JSON from silently being ignored. Matches ADR-0005 principle of explicit contracts.

### Implementation

Standard serde derive macro usage - no external code consulted.

## Behavioral Testing

Tests may compare output against system `ps` via subprocess invocation. This validates behavioral equivalence without any code reading:

```rust
#[test]
#[cfg(unix)]
fn compare_with_system_ps() {
    let our_info = sysprims_proc::get_process(std::process::id()).unwrap();

    // System ps (subprocess - no license concern)
    let ps_output = Command::new("ps")
        .args(&["-p", &our_info.pid.to_string(), "-o", "pid="])
        .output()
        .unwrap();

    let ps_pid: u32 = String::from_utf8_lossy(&ps_output.stdout)
        .trim()
        .parse()
        .unwrap();

    assert_eq!(our_info.pid, ps_pid);
}
```

## Certification

This module's implementation is derived from:

- [x] Public specifications (POSIX, platform docs)
- [x] Permissively-licensed references (MIT/Apache-2.0 crates for patterns)
- [x] Original implementation for platform API calls

No GPL/LGPL/AGPL source code was consulted during development.

---

_Provenance version: 1.0_
_Last updated: 2026-01-09_
_Maintainer: sysprims team_
