# sysprims-session

Session and process group acquisition primitives for sysprims.

See the [sysprims repository](https://github.com/3leaps/sysprims) for
documentation, release notes, and supported platform policy.

GPL-free session and process group management primitives.

## Overview

This crate provides cross-platform primitives for session and process group
management, replacing GPL-licensed tools:

| Function | Replaces          | Original License |
| -------- | ----------------- | ---------------- |
| `setsid` | util-linux setsid | GPL-2.0          |
| `nohup`  | coreutils nohup   | GPL-3.0          |

## Cleanroom Implementation

This implementation follows a cleanroom approach:

1. **Specification-driven**: All behavior derived from POSIX specifications
2. **Permissive-only references**: Only BSD/MIT code consulted for understanding
3. **No GPL code reviewed**: util-linux and coreutils sources NOT consulted
4. **Behavioral validation**: Tests compare against system tools (shell-out)

See [Provenance Documentation](#provenance) below for detailed source records.

## Platform Support

| Platform | setsid | nohup | setpgid | getpgid | getsid | Notes                   |
| -------- | ------ | ----- | ------- | ------- | ------ | ----------------------- |
| Linux    | Y      | Y     | Y       | Y       | Y      | Full support via POSIX  |
| macOS    | Y      | Y     | Y       | Y       | Y      | Full support via POSIX  |
| FreeBSD  | Y      | Y     | Y       | Y       | Y      | Full support via POSIX  |
| Windows  | -      | -     | -       | -       | -      | No equivalent concept\* |

\* Windows uses different process isolation model (Job Objects, sessions via
`CreateProcessAsUser`). These are not direct equivalents and would be a
different abstraction. WSL provides POSIX session semantics.

## API

### High-Level Functions

```rust
use sysprims_session::{run_setsid, run_nohup, SetsidConfig, NohupConfig};

// Run command in new session (detached from terminal)
let result = run_setsid("sleep", &["300"], SetsidConfig::default())?;

// Run command immune to SIGHUP
let result = run_nohup("./build.sh", &[], NohupConfig::default())?;

// Wait for completion
let result = run_setsid("make", &["build"], SetsidConfig { wait: true, ..Default::default() })?;
```

### Low-Level Functions

```rust
use sysprims_session::{setsid, getsid, setpgid, getpgid};

// Get current session ID
let sid = getsid(0)?;  // 0 = current process

// Get current process group
let pgid = getpgid(0)?;

// Create new process group (self as leader)
setpgid(0, 0)?;
```

### Spawn-Time Acquisition Receipt

External spawners that own the child and terminal handles can prepare a
single-use session hook before fork, install it in place of any other
`setsid`/`setpgid` acquirer, validate its positive acknowledgement, and pass the
receipt to the timeout crate's explicit external-child ownership boundary:

```rust
use std::os::unix::process::CommandExt;
use std::process::Command;
use sysprims_session::prepare_session_acquisition;

let (hook, pending) = prepare_session_acquisition()?;
let mut command = Command::new("worker");
unsafe {
    command.pre_exec(move || hook.acquire());
}
let child = command.spawn()?;
let receipt = pending.into_receipt(child.id())?;
```

The prepared hook carries a shared one-byte token, so reusing the configured
command for a second spawn fails closed before a second `setsid`. Dropping the
pending receipt closes its private descriptors. Linux creates those descriptors
with atomic close-on-exec/nonblocking flags; other Unix targets use the
platform `pipe`/`fcntl` fallback.

The hook performs one `setsid` and one fixed-size nonblocking write in the
post-fork/pre-exec child. It performs no allocation, locking, formatting,
blocking, or panic. The opaque receipt is not cloneable and cannot be minted
from a PID or post-spawn observation. Consumers pair it with the owned child via
`sysprims_timeout::contain_acquired_session`.

## Design Notes

### setsid

**POSIX Specification**: [setsid(2)](https://pubs.opengroup.org/onlinepubs/9699919799/functions/setsid.html)

The `setsid()` function creates a new session if the calling process is not a
process group leader. The calling process becomes:

1. The session leader of the new session
2. The process group leader of a new process group
3. Has no controlling terminal

**Implementation approach**:

```
Parent Process
    |
    +-- fork() --> Child Process
                       |
                       +-- setsid()  // Creates new session
                       |
                       +-- exec(command)
```

The fork is implicit in `Command::spawn()`. We use `pre_exec` to call `setsid()`
in the child after fork but before exec.

**Why fork first?**: A process group leader cannot call `setsid()` (EPERM).
By forking, the child is guaranteed not to be a process group leader.

### nohup

**POSIX Specification**: [nohup](https://pubs.opengroup.org/onlinepubs/9699919799/utilities/nohup.html)

The `nohup` utility invokes a command with SIGHUP ignored:

1. Set SIGHUP disposition to SIG_IGN
2. If stdout is terminal, redirect to `nohup.out` or `$HOME/nohup.out`
3. If stderr is terminal and stdout was redirected, redirect stderr too
4. Execute the command

**Implementation approach**:

```rust
// In pre_exec (after fork, before exec):
signal(SIGHUP, SIG_IGN);

// Before spawn (in parent):
if stdout.is_tty() {
    cmd.stdout(File::create("nohup.out"));
}
```

## Provenance

### setsid

**Primary Sources (Specifications)**:

- POSIX.1-2017 setsid(2): https://pubs.opengroup.org/onlinepubs/9699919799/functions/setsid.html
- FreeBSD setsid(2) man page (BSD License)
- Apple Darwin setsid(2) (APSL)

**Reference Implementations (Consulted for Understanding)**:

- [setsid-macosx](https://github.com/tzvetkoff/setsid-macosx) - BSD-2-Clause
- [ersatz-setsid](https://github.com/jerrykuch/ersatz-setsid) - MIT

**NOT Consulted**:

- util-linux setsid.c (GPL-2.0)

### nohup

**Primary Sources (Specifications)**:

- POSIX.1-2017 nohup utility: https://pubs.opengroup.org/onlinepubs/9699919799/utilities/nohup.html
- FreeBSD nohup(1) man page (BSD License)

**Reference Implementations (Consulted for Understanding)**:

- OpenBSD nohup.c (ISC License)

**NOT Consulted**:

- GNU coreutils nohup.c (GPL-3.0)

## Behavioral Comparison Tests

The test suite includes behavioral comparison against system tools:

```rust
#[test]
#[cfg(target_os = "linux")]
fn setsid_behavior_matches_system() {
    // Our implementation
    let our_result = run_setsid("id", &["-g"], SetsidConfig { wait: true, .. });

    // System implementation (shelling out - no GPL contamination)
    let sys_result = Command::new("/usr/bin/setsid")
        .args(&["id", "-g"])
        .output();

    // Behavioral equivalence (not code comparison)
    assert_eq!(our_result.exit_code, sys_result.exit_code);
}
```

This validates we match expected behavior without reviewing GPL source code.

## Future Work

- [ ] CLI subcommand (`sysprims session setsid <cmd>`)
- [ ] Full `daemonize()` function (double-fork, chdir, umask, close fds)
- [ ] `disown` equivalent (remove from job table)
- [ ] Windows: Document that Job Objects / `CreateProcessAsUser` are different abstractions

## License

MIT OR Apache-2.0 (same as sysprims workspace)
