---
title: "sysprims-session Equivalence Test Protocol"
module: "sysprims-session"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
---

# sysprims-session Equivalence Test Protocol

## 1) Purpose

Validate that sysprims-session:

1. **Implements correct setsid semantics** per POSIX
2. **Implements correct nohup semantics** per POSIX
3. **Produces equivalent behavior** to system tools
4. **Returns NotSupported on Windows**

## 2) Reference Implementations

### Unix

| Tool            | License             | Usage                      |
| --------------- | ------------------- | -------------------------- |
| System `setsid` | Various (often GPL) | Subprocess comparison only |
| System `nohup`  | Various (often GPL) | Subprocess comparison only |

**Note:** We invoke system tools as subprocesses only. No source code reading.

**Availability:**

- Linux: util-linux (setsid), coreutils (nohup)
- macOS: BSD implementations
- Windows: Not available

## 3) Test Matrix

### Platforms

| Platform | CI Runner      | Reference Tools | Notes                   |
| -------- | -------------- | --------------- | ----------------------- |
| Linux    | ubuntu-latest  | setsid, nohup   | Primary equivalence     |
| macOS    | macos-latest   | nohup           | setsid may need install |
| Windows  | windows-latest | N/A             | NotSupported tests only |

## 4) Test Categories

### Category A: setsid Semantics

| Test Case                          | Expected                |
| ---------------------------------- | ----------------------- |
| Child becomes session leader       | SID = child PID         |
| Child becomes process group leader | PGID = child PID        |
| Child detaches from terminal       | No controlling terminal |
| Fork if caller is pgrp leader      | setsid succeeds         |
| Wait mode returns exit status      | ExitStatus propagated   |

**Verification method:**

```bash
# Our implementation
sysprims setsid sleep 60 &
PID=$!
ps -o pid,pgid,sid -p $PID
# Verify: PID == PGID == SID
```

### Category B: nohup Semantics

| Test Case                      | Expected                             |
| ------------------------------ | ------------------------------------ |
| SIGHUP ignored                 | Process survives SIGHUP              |
| Output redirected to nohup.out | File created when stdout is terminal |
| Custom output file             | Uses specified path                  |
| Non-terminal stdout            | No redirection                       |

**Verification method:**

```bash
# Our implementation
sysprims nohup ./long-job.sh &
kill -HUP $!
# Verify: process still running
```

### Category C: Error Handling

| Test Case              | Expected Error              |
| ---------------------- | --------------------------- |
| Command not found      | NotFound (exit 127)         |
| Command not executable | PermissionDenied (exit 126) |
| setsid on Windows      | NotSupported                |
| nohup on Windows       | NotSupported                |

### Category D: Behavioral Equivalence

**setsid comparison:**

```bash
# System tool
setsid sleep 60 &
SYSTEM_PID=$!
ps -o pid,pgid,sid -p $SYSTEM_PID

# Our implementation
sysprims setsid sleep 60 &
OUR_PID=$!
ps -o pid,pgid,sid -p $OUR_PID

# Both should show: PID == PGID == SID
```

**nohup comparison:**

```bash
# System tool
nohup ./test.sh &
ls nohup.out

# Our implementation
sysprims nohup ./test.sh &
ls nohup.out
```

| Aspect                        | Must Match        |
| ----------------------------- | ----------------- |
| Session leader (setsid)       | SID = child PID   |
| Process group leader (setsid) | PGID = child PID  |
| SIGHUP immunity (nohup)       | Process survives  |
| Output redirection (nohup)    | nohup.out created |

## 5) Determinism and Flake Policy

### Process State Timing

- Allow brief delay for process state to stabilize
- Use `ps` after short sleep to verify state

### Test Fixtures

| Fixture         | Purpose                 | Behavior              |
| --------------- | ----------------------- | --------------------- |
| `sleep N`       | Simple detached process | Sleeps then exits     |
| `signal-logger` | Signal verification     | Logs received signals |

## 6) Test Locations

| Test Type           | Location                               |
| ------------------- | -------------------------------------- |
| Unit tests (config) | `crates/sysprims-session/src/lib.rs`   |
| Integration tests   | `crates/sysprims-session/tests/`       |
| Equivalence harness | `tests/equivalence/session/` (planned) |

## 7) Traceability to Spec

| Spec Requirement         | Test Category | Test IDs                 |
| ------------------------ | ------------- | ------------------------ |
| Session leader semantics | A             | integration              |
| Process group leader     | A             | integration              |
| SIGHUP immunity          | B             | integration              |
| Output redirection       | B             | integration              |
| Windows NotSupported     | C             | unit                     |
| Fork if pgrp leader      | A             | integration              |
| Default wait=false       | A             | `setsid_config_defaults` |

---

_Protocol version: 1.0_
_Last updated: 2026-01-09_
