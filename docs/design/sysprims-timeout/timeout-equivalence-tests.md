---
title: "sysprims-timeout Equivalence Test Protocol"
module: "sysprims-timeout"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
---

# sysprims-timeout Equivalence Test Protocol

## 1) Purpose

Validate that sysprims-timeout:

1. **Matches GNU timeout exit codes** (124, 125, 126, 127)
2. **Implements group-by-default** tree kill (ADR-0003)
3. **Reports observable fallbacks** when tree-kill cannot be guaranteed
4. **Handles signal escalation** correctly (TERM → KILL)

## 2) Reference Implementations

### GNU timeout

| Tool                    | Version | License | Usage                      |
| ----------------------- | ------- | ------- | -------------------------- |
| GNU coreutils `timeout` | 8.x+    | GPL-3.0 | Subprocess comparison only |

**Note:** We invoke GNU timeout as subprocess only. No source code reading.

**Availability:**

- Linux: coreutils package
- macOS: `brew install coreutils` (as `gtimeout`)
- Windows: Not available (skip equivalence tests)

## 3) Test Matrix

### Platforms

| Platform | CI Runner      | GNU timeout        | Notes                 |
| -------- | -------------- | ------------------ | --------------------- |
| Linux    | ubuntu-latest  | `/usr/bin/timeout` | Primary equivalence   |
| macOS    | macos-latest   | `gtimeout`         | Via Homebrew          |
| Windows  | windows-latest | N/A                | Self-consistency only |

## 4) Test Categories

### Category A: Exit Codes

| Test Case              | Command                                        | Expected Exit |
| ---------------------- | ---------------------------------------------- | ------------- |
| Normal completion      | `timeout 5s true`                              | 0             |
| Timeout occurs         | `timeout 1s sleep 60`                          | 124           |
| Command not found      | `timeout 5s /nonexistent`                      | 127           |
| Command not executable | `timeout 5s /etc/passwd`                       | 126           |
| Preserve-status        | `timeout --preserve-status 5s sh -c 'exit 42'` | 42            |

### Category B: Signal Handling

| Test Case                 | Expected                          |
| ------------------------- | --------------------------------- |
| Default signal is SIGTERM | Process receives SIGTERM          |
| Custom signal via `-s`    | Process receives specified signal |
| Kill-after escalation     | SIGKILL sent after delay          |
| Signal-resistant process  | Escalation to SIGKILL occurs      |

### Category C: Group-by-Default (ADR-0003)

**Critical test:** Tree escape prevention.

| Test Case               | Expected                              |
| ----------------------- | ------------------------------------- |
| Parent spawns child     | Both killed on timeout                |
| Child spawns grandchild | All three killed                      |
| Child attempts setpgid  | Still killed (in job/group)           |
| Child attempts setsid   | Grandchildren may escape (BestEffort) |

**Tree escape test fixture:**

```bash
#!/bin/bash
# tree-escape-attempt.sh
(sleep 60 &)  # Grandchild attempts to background
sleep 60      # Child waits
```

### Category D: Observable Fallback

| Test Case     | Reliability                      | Notes                    |
| ------------- | -------------------------------- | ------------------------ |
| Normal spawn  | Guaranteed                       | Group/job created        |
| setpgid fails | BestEffort                       | Only direct child killed |
| JSON output   | Contains `tree_kill_reliability` |                          |

### Category E: Duration Parsing

| Input   | Expected Duration |
| ------- | ----------------- |
| `5`     | 5 seconds         |
| `5s`    | 5 seconds         |
| `500ms` | 500 milliseconds  |
| `2m`    | 2 minutes         |
| `1h`    | 1 hour            |
| `1.5s`  | 1.5 seconds       |

### Category F: Equivalence with GNU timeout

**Run both implementations, compare behavior:**

```bash
# GNU timeout
timeout 2s sleep 10
echo "GNU exit: $?"

# sysprims
sysprims timeout 2s -- sleep 10
echo "sysprims exit: $?"
```

| Aspect                      | Must Match      |
| --------------------------- | --------------- |
| Exit code on timeout        | 124             |
| Exit code on success        | 0 or child code |
| Exit code on not-found      | 127             |
| Exit code on not-executable | 126             |

## 5) Determinism and Flake Policy

### Timing Sensitivity

- Use generous timeouts (2s+) to avoid timing flakes
- Allow ±100ms tolerance on duration measurements
- Use dedicated test fixtures with predictable behavior

### Test Fixtures

| Fixture          | Purpose               | Behavior              |
| ---------------- | --------------------- | --------------------- |
| `sleep N`        | Simple delay          | Sleeps then exits 0   |
| `signal-catcher` | Signal verification   | Logs received signals |
| `tree-escape`    | Group-by-default test | Spawns grandchildren  |

## 6) Test Locations

| Test Type            | Location                               |
| -------------------- | -------------------------------------- |
| Unit tests (config)  | `crates/sysprims-timeout/src/lib.rs`   |
| Integration tests    | `crates/sysprims-timeout/tests/`       |
| Equivalence harness  | `tests/equivalence/timeout/` (planned) |
| Tree-escape fixtures | `tests/fixtures/timeout/` (planned)    |

## 7) Traceability to Spec

| Spec Requirement           | Test Category | Test IDs                      |
| -------------------------- | ------------- | ----------------------------- |
| Exit 124 on timeout        | A             | integration                   |
| Exit 127 on not-found      | A             | integration                   |
| Exit 126 on not-executable | A             | integration                   |
| Group-by-default           | C             | tree-escape tests             |
| Observable fallback        | D             | JSON output tests             |
| Signal escalation          | B             | kill-after tests              |
| Default SIGTERM            | B             | `default_config_uses_sigterm` |
| Default 10s kill_after     | B             | `default_config_kill_after_*` |

---

_Protocol version: 1.0_
_Last updated: 2026-01-09_
