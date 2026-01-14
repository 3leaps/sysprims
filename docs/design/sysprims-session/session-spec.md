---
title: "sysprims-session Module Spec"
module: "sysprims-session"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
adr_refs: ["ADR-0007"]
---

# sysprims-session Module Spec

## 1) Overview

**Purpose:** Provide session and process group management primitives, implemented from POSIX specifications.

**In scope (v0.1.0):**

- `run_setsid()` - Run command in new session (session leader)
- `run_nohup()` - Run command immune to SIGHUP
- Low-level APIs: `setsid()`, `getsid()`, `setpgid()`, `getpgid()`
- Unix-only (Windows returns `NotSupported`)

**Out of scope (v0.1.0):**

- Windows session management
- Controlling terminal allocation (`--ctty` is placeholder)
- Process group management utilities beyond basic primitives

## 2) Normative References

### setsid

| Reference | URL | License |
|-----------|-----|---------|
| POSIX.1-2017 setsid(2) | https://pubs.opengroup.org/onlinepubs/9699919799/functions/setsid.html | Spec |
| FreeBSD setsid(2) man | https://www.freebsd.org/cgi/man.cgi?query=setsid&sektion=2 | BSD |

### nohup

| Reference | URL | License |
|-----------|-----|---------|
| POSIX.1-2017 nohup utility | https://pubs.opengroup.org/onlinepubs/9699919799/utilities/nohup.html | Spec |
| FreeBSD nohup(1) man | https://www.freebsd.org/cgi/man.cgi?query=nohup | BSD |
| OpenBSD nohup.c | https://cvsweb.openbsd.org/src/usr.bin/nohup/ | ISC |

## 3) sysprims Required Interface (Rust)

### 3.1 High-Level APIs

```rust
/// Configuration for setsid execution.
#[derive(Debug, Clone, Default)]
pub struct SetsidConfig {
    /// Wait for child to exit (default: false = detach immediately)
    pub wait: bool,

    /// Create controlling terminal (placeholder, no-op in v0.1)
    pub ctty: bool,
}

/// Outcome of setsid execution.
#[derive(Debug)]
pub enum SetsidOutcome {
    /// Child spawned in new session (when wait: false)
    Spawned { child_pid: u32 },

    /// Child completed (when wait: true)
    Completed { exit_status: ExitStatus },
}

/// Run command in a new session.
pub fn run_setsid(
    command: &str,
    args: &[&str],
    config: SetsidConfig,
) -> SysprimsResult<SetsidOutcome>;
```

```rust
/// Configuration for nohup execution.
#[derive(Debug, Clone, Default)]
pub struct NohupConfig {
    /// Output file for stdout redirect (default: nohup.out)
    pub output_file: Option<String>,

    /// Wait for child to exit
    pub wait: bool,
}

/// Outcome of nohup execution.
#[derive(Debug)]
pub enum NohupOutcome {
    /// Child spawned with SIGHUP ignored
    Spawned {
        child_pid: u32,
        output_file: Option<String>,
    },

    /// Child completed (when wait: true)
    Completed { exit_status: ExitStatus },
}

/// Run command immune to SIGHUP.
pub fn run_nohup(
    command: &str,
    args: &[&str],
    config: NohupConfig,
) -> SysprimsResult<NohupOutcome>;
```

### 3.2 Low-Level APIs (Unix only)

```rust
/// Create new session for current process.
#[cfg(unix)]
pub fn setsid() -> SysprimsResult<u32>;

/// Get session ID for process (0 = current).
#[cfg(unix)]
pub fn getsid(pid: u32) -> SysprimsResult<u32>;

/// Set process group ID.
#[cfg(unix)]
pub fn setpgid(pid: u32, pgid: u32) -> SysprimsResult<()>;

/// Get process group ID.
#[cfg(unix)]
pub fn getpgid(pid: u32) -> SysprimsResult<u32>;
```

### 3.3 Platform Support

| Function | Unix | Windows |
|----------|------|---------|
| `run_setsid` | Full | NotSupported |
| `run_nohup` | Full | NotSupported |
| `setsid` | Full | Not compiled |
| `getsid` | Full | Not compiled |
| `setpgid` | Full | Not compiled |
| `getpgid` | Full | Not compiled |

### 3.4 Invariants

1. **setsid semantics:** Child process becomes session leader, detached from controlling terminal
2. **nohup semantics:** SIGHUP is ignored; stdout redirected if terminal
3. **Fork if leader:** `run_setsid` forks if caller is process group leader
4. **Output redirection:** `run_nohup` redirects to `nohup.out` or `$HOME/nohup.out` if stdout is terminal

## 4) CLI Contract

### sysprims setsid

```
sysprims setsid [OPTIONS] -- <COMMAND> [ARGS...]

OPTIONS:
    --wait     Wait for child to complete
    --ctty     Allocate controlling terminal (placeholder)
```

### sysprims nohup

```
sysprims nohup [OPTIONS] -- <COMMAND> [ARGS...]

OPTIONS:
    --output <FILE>  Redirect stdout to FILE
    --wait           Wait for child to complete
```

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Internal error |
| 126 | Command not executable |
| 127 | Command not found |
| 128+N | Child killed by signal N (when --wait) |

## 5) Traceability Matrix

| Requirement | Reference | Rust API | CLI | Tests | Status |
|-------------|-----------|----------|-----|-------|--------|
| New session creation | POSIX setsid(2) | `run_setsid` | `sysprims setsid` | integration | Pass |
| Session leader semantics | POSIX setsid(2) | `setsid()` | `sysprims setsid` | integration | Pass |
| SIGHUP immunity | POSIX nohup | `run_nohup` | `sysprims nohup` | integration | Pass |
| Output redirection | POSIX nohup | `run_nohup` | `sysprims nohup` | integration | Pass |
| Windows NotSupported | Platform contract | all | all | unit | Pass |
| Fork if pgrp leader | POSIX setsid(2) | `run_setsid` | `sysprims setsid` | integration | Pass |
| Default wait=false | spec | `SetsidConfig::default()` | default | `setsid_config_defaults` | Pass |
| Default output=nohup.out | spec | `NohupConfig::default()` | default | `nohup_config_defaults` | Pass |

---

*Spec version: 1.0*
*Last updated: 2026-01-09*
