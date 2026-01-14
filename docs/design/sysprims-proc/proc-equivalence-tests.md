---
title: "sysprims-proc Equivalence Test Protocol"
module: "sysprims-proc"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
adr_refs: ["ADR-0005", "ADR-0007", "ADR-0008"]
---

# sysprims-proc Equivalence Test Protocol

## 1) Purpose

Validate that sysprims-proc provides:

1. **Stable minimum fields** (`pid`, `name`) across all platforms
2. **Strict filter validation** via `#[serde(deny_unknown_fields)]`
3. **Schema-backed JSON output** with embedded `schema_id`
4. **CPU% normalization** invariant (0-100 across all cores)
5. **Reasonable equivalence** with system `ps` where comparison is meaningful

**Note:** We do not claim full POSIX `ps` compatibility. Many `ps` fields are implementation-defined. Our contract defines a stable subset with explicit semantics.

## 2) Reference Implementations

### Unix

**Reference tool:** System `ps` (POSIX utility)

Invocation for comparison:
```bash
ps -o pid,ppid,user,pcpu,vsz,stat,args -p <PID>
```

**Availability:**
- Linux: procps-ng or busybox ps
- macOS: BSD ps

**Invocation method:** Subprocess only. We never read reference tool source code during testing.

### Windows

**Reference:** No POSIX `ps` equivalent.

Comparisons are:
- Self-consistency tests
- OS API expectations (Toolhelp32 returns expected data)
- PowerShell `Get-Process` for cross-validation where useful

## 3) Test Matrix

### Platforms

| Platform | Architecture | CI Available | Privilege |
|----------|--------------|--------------|-----------|
| Linux glibc | x64 | Yes | Non-root |
| Linux musl | x64 | Yes | Non-root |
| macOS | x64, arm64 | Yes | Non-root |
| Windows | x64 | Yes | Non-admin |

**Privilege note:** Process enumeration may be limited by permissions. Tests must account for this and not assume visibility of all system processes.

## 4) Test Categories

### Category A: Required Field Correctness

**Objective:** Verify `pid` and `name` are always present and correct.

| Test ID | Description | Method |
|---------|-------------|--------|
| A.1 | Self-process has correct PID | `get_process(std::process::id())` returns matching PID |
| A.2 | Self-process has non-empty name | `name.len() > 0` |
| A.3 | Test fixture has expected name | Spawn process with known name, verify in snapshot |
| A.4 | Child process appears in snapshot | Spawn child, filter by PID, verify found |

### Category B: Filter Schema Strictness

**Objective:** Verify `ProcessFilter` rejects unknown fields and validates ranges.

| Test ID | Description | Method |
|---------|-------------|--------|
| B.1 | Unknown field rejected | Parse JSON `{"unknown_field": "x"}` → `InvalidArgument` |
| B.2 | Valid filter accepted | Parse `{"name_contains": "test"}` → Ok |
| B.3 | `cpu_above` range validation | `cpu_above: 150.0` → `InvalidArgument` |
| B.4 | `cpu_above` negative rejected | `cpu_above: -1.0` → `InvalidArgument` |
| B.5 | Empty filter valid | `{}` → Ok (matches all) |
| B.6 | Multiple filters AND logic | `{name_contains, cpu_above}` → intersection |

### Category C: Schema Conformance

**Objective:** Verify JSON output matches schema contract.

| Test ID | Description | Method |
|---------|-------------|--------|
| C.1 | Output contains `schema_id` | JSON has field at root |
| C.2 | `schema_id` matches constant | Value equals `PROCESS_INFO_V1` |
| C.3 | Output contains `timestamp` | RFC3339 format validation |
| C.4 | Output contains `processes` array | Type check |
| C.5 | ProcessInfo has required fields | Each entry has `pid`, `name` |
| C.6 | ProcessState serializes snake_case | `Running` → `"running"` |

**Schema ID:** `https://schemas.3leaps.dev/sysprims/process/v1.0.0/process-info.schema.json`

### Category D: CPU% Normalization Invariant

**Objective:** Verify `cpu_percent` is always 0-100, normalized across cores.

| Test ID | Description | Method |
|---------|-------------|--------|
| D.1 | All processes have cpu >= 0 | Iterate snapshot, assert range |
| D.2 | All processes have cpu <= 100 | Even on multi-core, never N*100 |
| D.3 | Idle process ~0% | Low-activity test process has low CPU |
| D.4 | Busy process > 0% | Spin loop process shows non-zero |

**Normalization policy:** Single value 0-100 representing percentage of total system capacity, not per-core.

### Category E: Optional Field Handling

**Objective:** Verify optional fields are present when readable, absent/None when not.

| Test ID | Description | Method |
|---------|-------------|--------|
| E.1 | `user` present for own process | Non-None on Unix, may vary on Windows |
| E.2 | `cmdline` readable for self | Non-empty for running test |
| E.3 | `state` valid enum | Not Unknown for running process |
| E.4 | No fake/placeholder data | Fields are None or real, never invented |

### Category F: Comparative Tests vs ps

**Objective:** Where `ps` is available, compare presence and rough equivalence.

| Test ID | Description | Method |
|---------|-------------|--------|
| F.1 | PID matches ps output | `ps -p <PID>` reports same PID |
| F.2 | PPID roughly matches | Within expected process tree |
| F.3 | Name present in both | May differ in truncation |
| F.4 | User matches | `ps -o user` vs our `user` field |

**Comparison notes:**
- cmdline quoting may differ (don't require exact match)
- Memory units may differ (ps uses VSZ in KB)
- CPU sampling windows differ (accept variance)

### Category G: Error Handling

**Objective:** Verify correct error codes per ADR-0008.

| Test ID | Description | Method |
|---------|-------------|--------|
| G.1 | PID 0 is InvalidArgument | `get_process(0)` returns error |
| G.2 | Non-existent PID is NotFound | `get_process(99999999)` returns NotFound |
| G.3 | Invalid filter is InvalidArgument | Bad JSON → InvalidArgument |

## 5) Determinism and Flake Policy

### Fixture Processes

Use dedicated test fixture processes:
- Known stable name
- Known command line arguments
- Spawned by test harness with controlled lifetime

### Avoid Flakes

1. **Don't compare transient processes** - Only test processes we spawn
2. **Don't require specific process count** - System varies
3. **Allow CPU% tolerance** - ±5% for timing jitter
4. **Don't require user field on Windows** - May be unavailable

### Timing Controls

- CPU measurement needs warm-up time
- Snapshot should be taken after process stabilizes
- Use sleep/polling where needed for process visibility

## 6) Test Locations

| Type | Location |
|------|----------|
| Unit tests | `crates/sysprims-proc/src/lib.rs` |
| Platform tests | `crates/sysprims-proc/src/linux.rs`, `crates/sysprims-proc/src/macos.rs`, `crates/sysprims-proc/src/windows.rs` |
| Integration tests | Not yet separated (planned) |
| Equivalence tests | Not yet separated (planned) |
| Golden tests | Not yet used |

## 7) Artifacts

| Artifact | Path | Purpose |
|----------|------|---------|
| Test results | CI job logs | Pass/fail evidence |
| Snapshot outputs | CI artifacts | Schema validation |
| Environment info | CI job metadata | Version tracking |

## 8) Traceability to Spec

| Spec Requirement | Test Category | Test IDs |
|------------------|---------------|----------|
| Required fields pid+name | A | A.1-A.4 |
| Strict filter schema | B | B.1-B.6 |
| Unknown keys rejected | B | B.1 |
| cpu% normalized 0-100 | D | D.1-D.4 |
| schema_id embedded | C | C.1-C.2 |
| No fake data | E | E.4 |
| PID 0 rejected | G | G.1 |
| Error codes per ADR-0008 | G | G.1-G.3 |

---

*Protocol version: 1.0*
*Last updated: 2026-01-09*
