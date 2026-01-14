---
title: "sysprims-cli Equivalence Test Protocol"
module: "sysprims-cli"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
---

# sysprims-cli Equivalence Test Protocol

## 1) Purpose

Validate that sysprims CLI:

1. **Correctly dispatches to library crates**
2. **Parses arguments as documented**
3. **Returns correct exit codes**
4. **Produces expected output formats**

## 2) Test Strategy

The CLI is a thin wrapper. Most functionality is tested in the underlying library crates. CLI tests focus on:

- Argument parsing
- Exit code mapping
- Output formatting
- Integration between subcommands and libraries

## 3) Test Categories

### Category A: Argument Parsing

| Test Case | Command | Expected |
|-----------|---------|----------|
| Default signal | `sysprims kill 1234` | Uses TERM |
| Custom signal | `sysprims kill -s INT 1234` | Uses INT |
| Duration seconds | `sysprims timeout 5s true` | 5 second timeout |
| Duration minutes | `sysprims timeout 2m true` | 2 minute timeout |
| Duration milliseconds | `sysprims timeout 500ms true` | 500ms timeout |
| Plain number | `sysprims timeout 5 true` | 5 second timeout |

### Category B: Exit Codes

| Test Case | Command | Expected Exit |
|-----------|---------|---------------|
| Successful command | `sysprims timeout 5s true` | 0 |
| Timeout occurs | `sysprims timeout 1s sleep 60` | 124 |
| Command not found | `sysprims timeout 5s /nonexistent` | 127 |
| Signal success | `sysprims kill -s 0 $$` | 0 |
| Signal not found | `sysprims kill 99999999` | 1 |
| pstat success | `sysprims pstat --pid $$` | 0 |

### Category C: Output Formats

| Test Case | Command | Expected |
|-----------|---------|----------|
| pstat JSON default | `sysprims pstat` | JSON with schema_id |
| pstat table | `sysprims pstat --table` | Tabular output |
| pstat single PID | `sysprims pstat --pid 1` | Single process |

### Category D: Filter Options

| Test Case | Command | Expected |
|-----------|---------|----------|
| Name filter | `sysprims pstat --name nginx` | Filtered list |
| CPU filter | `sysprims pstat --cpu-above 50` | High-CPU processes |
| Sort option | `sysprims pstat --sort cpu` | Sorted by CPU |

## 4) Test Locations

| Test Type | Location |
|-----------|----------|
| Unit tests | `crates/sysprims-cli/src/` (if any) |
| Integration tests | `crates/sysprims-cli/tests/` (planned) |
| Equivalence tests | Via library crate tests |

## 5) Traceability to Spec

| Spec Requirement | Test Category | Test Method |
|------------------|---------------|-------------|
| Subcommand dispatch | A | CLI invocation tests |
| Duration parsing | A | Various duration formats |
| Exit codes | B | Exit status verification |
| JSON output | C | Schema validation |
| Filter options | D | Filter verification |

---

*Protocol version: 1.0*
*Last updated: 2026-01-09*
