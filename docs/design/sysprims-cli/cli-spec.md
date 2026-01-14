---
title: "sysprims-cli Module Spec"
module: "sysprims-cli"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
adr_refs: ["ADR-0005", "ADR-0008"]
---

# sysprims-cli Module Spec

## 1) Overview

**Purpose:** Provide a unified CLI entry point that exposes all sysprims functionality as subcommands. The CLI is a thin wrapper over the library crates.

**Design principle:** CLI is a test vehicle and convenience wrapper. All logic lives in library crates; CLI only handles argument parsing and output formatting.

**Subcommands (v0.1.0):**

| Command | Library | Purpose |
|---------|---------|---------|
| `kill` | sysprims-signal | Send signal to process |
| `timeout` | sysprims-timeout | Run command with timeout |
| `pstat` | sysprims-proc | Process listing and inspection |

**Planned (v0.2.0):**

| Command | Library | Purpose |
|---------|---------|---------|
| `setsid` | sysprims-session | Run in new session |
| `nohup` | sysprims-session | Run immune to SIGHUP |

## 2) Binary Name

**Binary:** `sysprims`

Installed as single binary with subcommand dispatch.

## 3) Global Options

| Option | Description | Default |
|--------|-------------|---------|
| `--log-format <FORMAT>` | Log output format (text, json) | text |
| `--log-level <LEVEL>` | Minimum log level | info |
| `--version` | Print version | - |
| `--help` | Print help | - |

## 4) Subcommand Contracts

### 4.1 sysprims kill

```
sysprims kill [-s SIGNAL] <PID>
```

**Options:**

| Option | Description | Default |
|--------|-------------|---------|
| `-s, --signal <SIG>` | Signal name or number | TERM |

**Exit codes:** Per sysprims-signal spec (0 success, 1 error).

### 4.2 sysprims timeout

```
sysprims timeout [OPTIONS] <DURATION> <COMMAND> [ARGS...]
```

**Options:**

| Option | Description | Default |
|--------|-------------|---------|
| `-s, --signal <SIG>` | Signal on timeout | TERM |
| `-k, --kill-after <DUR>` | Delay before SIGKILL | 10s |
| `--foreground` | Don't create process group | false |
| `--preserve-status` | Propagate child exit code | false |

**Exit codes:** Per sysprims-timeout spec (124 timeout, 125 error, 126/127 command errors).

### 4.3 sysprims pstat

```
sysprims pstat [OPTIONS]
```

**Options:**

| Option | Description | Default |
|--------|-------------|---------|
| `--json` | JSON output with schema_id | true* |
| `--table` | Human-readable table | false |
| `--pid <PID>` | Show specific process | - |
| `--name <NAME>` | Filter by name | - |
| `--user <USER>` | Filter by user | - |
| `--cpu-above <PERCENT>` | Filter by CPU | - |
| `--memory-above <KB>` | Filter by memory | - |
| `--sort <FIELD>` | Sort by field | pid |

*Default output is JSON for automation.

**Exit codes:** Per sysprims-proc spec (0 success, 1 error).

## 5) Duration Parsing

Both `timeout` duration and `--kill-after` support:

| Format | Example | Duration |
|--------|---------|----------|
| Plain number | `5` | 5 seconds |
| Milliseconds | `500ms` | 500ms |
| Seconds | `5s` | 5 seconds |
| Minutes | `2m` | 2 minutes |
| Hours | `1h` | 1 hour |
| Decimal | `1.5s` | 1.5 seconds |

## 6) Exit Code Summary

| Code | Meaning | Source |
|------|---------|--------|
| 0 | Success | All commands |
| 1 | General error | All commands |
| 124 | Timeout occurred | timeout |
| 125 | Tool error | timeout |
| 126 | Command not executable | timeout |
| 127 | Command not found | timeout |
| 128+N | Killed by signal N | timeout |

## 7) Output Formats

### JSON (default for pstat)

All JSON output includes `schema_id` per ADR-0005.

### Table (--table)

Human-readable columnar output for terminal use.

### Logging

Structured logging via tracing with configurable format and level.

## 8) Traceability Matrix

| Requirement | Spec Section | Implementation | Tests | Status |
|-------------|--------------|----------------|-------|--------|
| Subcommand dispatch | §1 | `clap::Subcommand` | integration | Pass |
| kill delegates to signal | §4.1 | `sysprims_signal::kill_by_name` | integration | Pass |
| timeout delegates to timeout | §4.2 | `sysprims_timeout::run_with_timeout` | integration | Pass |
| pstat delegates to proc | §4.3 | `sysprims_proc::snapshot*` | integration | Pass |
| Duration parsing | §5 | `parse_duration` | unit | Pass |
| Exit codes match spec | §6 | main.rs exit handling | integration | Pass |

---

*Spec version: 1.0*
*Last updated: 2026-01-09*
