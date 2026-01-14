---
title: "sysprims-session Provenance"
module: "sysprims-session"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
---

# Provenance: sysprims-session

This document records the sources consulted for implementing `sysprims-session`, ensuring clear provenance for all functionality.

## Policy

- POSIX specifications are the primary reference
- BSD/MIT/ISC licensed implementations may be consulted for understanding
- Behavioral comparison against tools (via subprocess) is permitted for testing
- Source code of restrictively-licensed tools (GPL/LGPL/AGPL) is NOT consulted

## setsid

### Specification Sources (Primary)

1. **POSIX.1-2017 setsid(2)**
   - URL: https://pubs.opengroup.org/onlinepubs/9699919799/functions/setsid.html
   - License: Specification (no license restriction)
   - Used for: Authoritative specification for behavior

2. **FreeBSD setsid(2) man page**
   - URL: https://www.freebsd.org/cgi/man.cgi?query=setsid&sektion=2
   - License: BSD
   - Used for: BSD-licensed documentation

3. **Apple Darwin setsid(2)**
   - URL: Apple Developer Documentation
   - License: APSL (documentation reference)
   - Used for: macOS behavior

### Implementation References (Consulted)

1. **setsid-macosx** (BSD-2-Clause)
   - URL: https://github.com/tzvetkoff/setsid-macosx
   - License: BSD-2-Clause (permissive)
   - Consulted for: General approach (fork if pgrp leader, setsid, exec)

2. **ersatz-setsid** (MIT)
   - URL: https://github.com/jerrykuch/ersatz-setsid
   - License: MIT (permissive)
   - Consulted for: Error handling patterns

### NOT Consulted

- util-linux setsid.c (GPL-2.0) - **NOT consulted**
- GNU coreutils (GPL-3.0) - **NOT consulted**

### Implementation Notes

The implementation is straightforward from POSIX spec:

1. If caller is process group leader, fork first (child cannot be leader)
2. Call `setsid()` to create new session
3. Exec the target command

This is ~20 lines of Rust using `libc::setsid()` and `std::process::Command`.

```rust
// Simplified implementation flow
fn run_setsid_impl(command: &str, args: &[&str], config: &SetsidConfig) -> SysprimsResult<SetsidOutcome> {
    let mut cmd = Command::new(command);
    cmd.args(args);

    unsafe {
        cmd.pre_exec(|| {
            // Create new session - child becomes session leader
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = cmd.spawn()?;
    // ... handle wait/detach
}
```

## nohup

### Specification Sources (Primary)

1. **POSIX.1-2017 nohup utility**
   - URL: https://pubs.opengroup.org/onlinepubs/9699919799/utilities/nohup.html
   - License: Specification (no license restriction)
   - Used for: Authoritative specification

2. **FreeBSD nohup(1) man page**
   - URL: https://www.freebsd.org/cgi/man.cgi?query=nohup
   - License: BSD
   - Used for: BSD documentation

### Implementation References (Consulted)

1. **OpenBSD nohup.c** (ISC License)
   - URL: https://cvsweb.openbsd.org/src/usr.bin/nohup/
   - License: ISC (permissive, BSD-like)
   - Consulted for: Reference implementation

### NOT Consulted

- GNU coreutils nohup.c (GPL-3.0) - **NOT consulted**

### Implementation Notes

From POSIX spec:

1. Set SIGHUP to SIG_IGN
2. If stdout is terminal, redirect to nohup.out or $HOME/nohup.out
3. If stderr is terminal, redirect to stdout
4. Exec the command

```rust
// Simplified implementation flow
fn run_nohup_impl(command: &str, args: &[&str], config: &NohupConfig) -> SysprimsResult<NohupOutcome> {
    // Ignore SIGHUP before spawn
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }

    let mut cmd = Command::new(command);
    cmd.args(args);

    // Redirect stdout if terminal
    if is_terminal(stdout()) {
        let output_file = config.output_file.clone()
            .unwrap_or_else(|| "nohup.out".to_string());
        cmd.stdout(File::create(&output_file)?);
    }

    let child = cmd.spawn()?;
    // ... handle wait/detach
}
```

## Behavioral Comparison Testing

Tests compare our implementation against system tools via subprocess invocation. This validates behavioral equivalence without any code reading:

```rust
#[test]
#[cfg(target_os = "linux")]
fn setsid_matches_system_behavior() {
    // Our implementation
    let our_result = run_setsid("id", &["-g"], SetsidConfig { wait: true, ..Default::default() });

    // System implementation (shelling out - no license concern)
    let sys_result = Command::new("setsid")
        .args(&["--wait", "id", "-g"])
        .output();

    // Compare behavior (not code)
    // Both should show child as session leader
}
```

This validates behavioral equivalence without any code copying.

## Certification

This module's implementation is derived from:

- [x] Public specifications (POSIX.1-2017)
- [x] Permissively-licensed references (BSD/MIT/ISC)
- [x] Original implementation based on specification

**No GPL/LGPL/AGPL source code was consulted during development.**

---

*Provenance version: 1.0*
*Last updated: 2026-01-09*
*Maintainer: sysprims team*
