---
title: "sysprims-proc Module Spec"
module: "sysprims-proc"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
adr_refs: ["ADR-0005", "ADR-0007", "ADR-0008"]
---

# sysprims-proc Module Spec

## 1) Overview

**Purpose:** Provide a stable, embeddable process enumeration/inspection API with strict input validation and schema-versioned JSON output for automation.

**In scope (v0.1.0):**

- Process snapshot listing with required minimum fields (`pid`, `name`)
- Single process inspection by PID
- Strict filter validation with unknown key rejection (`deny_unknown_fields`)
- CPU% normalization policy (0-100 across all cores)
- JSON output with `schema_id` per ADR-0005

**Out of scope (v0.1.0):**

- Full `ps` flag parity across platforms
- Interactive TUI (`top`-like)
- Extended process info (environment, threads, file descriptors)

**Supported platforms:**

- Linux (x64, musl + glibc)
- macOS (x64, arm64)
- Windows (x64)

## 2) Normative References

### POSIX Reference

**`ps` utility:**
- https://pubs.opengroup.org/onlinepubs/9699919799/utilities/ps.html

**Note:** POSIX `ps` has many implementation-defined aspects. sysprims defines its own stable subset contract rather than claiming full POSIX compatibility.

### Platform Implementation References

| Platform | API | Reference |
|----------|-----|-----------|
| Linux | `/proc` filesystem | `proc(5)` man page |
| macOS | `libproc` | Darwin headers |
| Windows | Toolhelp32 API | MSDN documentation |

## 3) Literal Interface Reference (POSIX ps)

POSIX `ps` supports options such as:

- `-a`, `-A`, `-e` — process selection
- `-f`, `-l` — output format
- `-o format` — custom output columns

Standard `-o` field names:

- `pid`, `ppid`, `pgid`, `user`, `pcpu`, `vsz`, `args`, `state`

**sysprims contract:** We provide a stable subset of these fields with explicit semantics, not full POSIX `ps` compatibility.

## 4) sysprims Required Interface (Rust)

### 4.1 Core Types

```rust
/// Snapshot of all processes at a point in time.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessSnapshot {
    /// Schema identifier for version detection.
    pub schema_id: &'static str,

    /// Timestamp of snapshot (ISO 8601).
    pub timestamp: String,

    /// List of processes.
    pub processes: Vec<ProcessInfo>,
}

/// Information about a single process.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    /// Process ID (required).
    pub pid: u32,

    /// Parent process ID.
    pub ppid: u32,

    /// Process name (required, best-effort length; may be truncated by platform).
    pub name: String,

    /// Owner username (None if unavailable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,

    /// CPU usage normalized 0-100 across all cores.
    pub cpu_percent: f64,

    /// Memory usage in kilobytes.
    pub memory_kb: u64,

    /// Seconds since process start.
    pub elapsed_seconds: u64,

    /// Process state.
    pub state: ProcessState,

    /// Command line arguments (may be empty if unreadable).
    pub cmdline: Vec<String>,
}

/// Process state (cross-platform mapping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    Running,
    Sleeping,
    Stopped,
    Zombie,
    Unknown,
}

/// Filter for process queries.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessFilter {
    pub name_contains: Option<String>,
    pub name_equals: Option<String>,
    pub user_equals: Option<String>,
    pub pid_in: Option<Vec<u32>>,
    pub state_in: Option<Vec<ProcessState>>,
    pub cpu_above: Option<f64>,        // Must be 0-100
    pub memory_above_kb: Option<u64>,
}
```

### 4.2 Public Functions

```rust
/// Get snapshot of all processes.
pub fn snapshot() -> SysprimsResult<ProcessSnapshot>;

/// Get snapshot with filter applied.
pub fn snapshot_filtered(filter: &ProcessFilter) -> SysprimsResult<ProcessSnapshot>;

/// Get information for a single process.
pub fn get_process(pid: u32) -> SysprimsResult<ProcessInfo>;
```

### 4.3 Error Handling

Per ADR-0008, this module returns:

| Error | Condition |
|-------|-----------|
| `InvalidArgument` | PID is 0, filter has unknown fields, cpu_above out of range |
| `NotFound` | Process does not exist |
| `PermissionDenied` | Process cannot be read due to permissions |

### 4.4 Invariants

1. **Required fields:** `pid` and `name` are always present.

2. **Optional fields:** Fields that cannot be read (permissions, platform limits) are set to default/None. Never faked with placeholder data.

3. **CPU normalization:** `cpu_percent` is normalized 0-100 across all cores (not 0-N*100).

4. **Filter strictness:**
   - Unknown JSON keys → `InvalidArgument`
   - `cpu_above` not in 0-100 → `InvalidArgument`

5. **Schema ID:** All JSON output includes `schema_id` field.

6. **PID 0 rejected:** `get_process(0)` returns `InvalidArgument`.

## 5) CLI Contract

**Subcommand:** `sysprims pstat`

### Synopsis

```
sysprims pstat [OPTIONS]
```

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `--json` | Output as JSON with schema_id | true |
| `--table` | Output as human-readable table | false |
| `--pid <PID>` | Show only specific process | - |
| `--name <NAME>` | Filter by name (substring, case-insensitive) | - |
| `--user <USER>` | Filter by username | - |
| `--cpu-above <PERCENT>` | Filter by minimum CPU (0-100) | - |
| `--memory-above <KB>` | Filter by minimum memory in KB | - |
| `--sort <FIELD>` | Sort by: pid, name, cpu, memory | pid |

### Exit Codes

| Condition | Exit Code |
|-----------|-----------|
| Success | 0 |
| Invalid argument | 1 |
| Process not found | 1 |

### Output Formats

**Human readable (default/`--table`):**

```
    PID    PPID   CPU%    MEM(KB)    STATE USER             NAME
--------------------------------------------------------------------------------
   1234       1    2.5      51200        R www-data         nginx
```

**JSON (`--json`):**

```json
{
  "schema_id": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/process-info.schema.json",
  "timestamp": "2026-01-09T12:00:00Z",
  "processes": [...]
}
```

## 6) FFI Contract

```c
// List processes with optional filter
SysprimsErrorCode sysprims_proc_list(
    const char* filter_json,  // NULL for no filter
    char** result_json_out    // Caller must free via sysprims_free_string
);

// Get single process
SysprimsErrorCode sysprims_proc_get(
    uint32_t pid,
    char** result_json_out    // Caller must free via sysprims_free_string
);
```

**Memory ownership:** Caller owns returned strings; must free via `sysprims_free_string()`.

## 7) Platform Implementation Notes

| Feature | Linux | macOS | Windows |
|---------|-------|-------|---------|
| Enumeration | `/proc` readdir | `proc_listpids` | `CreateToolhelp32Snapshot` |
| Process info | `/proc/[pid]/*` | `proc_pidinfo` | `OpenProcess` + queries |
| CPU usage | `/proc/[pid]/stat` | `proc_pidinfo` | `GetProcessTimes` |
| Memory | `/proc/[pid]/statm` | `proc_pidinfo` | `GetProcessMemoryInfo` |
| cmdline | `/proc/[pid]/cmdline` | Best-effort (name only) | `QueryFullProcessImageName` |
| User | `/proc/[pid]/status` Uid | `proc_pidinfo` | Token queries |

## 8) Traceability Matrix

| Requirement | Reference | Rust API | CLI | Tests | Evidence |
|-------------|-----------|----------|-----|-------|----------|
| Required fields pid+name | spec §4.4 | `ProcessInfo` | `--json` | `test_get_self` | CI |
| Strict filter schema | ADR-0005 | `ProcessFilter` | `--name` | `test_filter_unknown_field_rejected` | CI |
| Unknown filter keys rejected | spec §4.4 | `deny_unknown_fields` | - | `test_filter_unknown_field_rejected` | CI |
| cpu% normalized 0-100 | spec §4.4 | `cpu_percent` | `--json` | `test_cpu_normalized` | CI |
| cpu_above range validation | spec §4.4 | `ProcessFilter::validate` | `--cpu-above` | `test_filter_validation_cpu_range` | CI |
| schema_id embedded | ADR-0005 | `PROCESS_INFO_V1` | `--json` | `test_snapshot_has_schema_id` | CI |
| No fake data | spec §4.4 | optional fields | any | `test_get_self_has_valid_fields` | CI |
| PID 0 rejected | spec §4.4 | `get_process` | `--pid 0` | `test_invalid_pid_zero` | CI |

---

*Spec version: 1.0*
*Last updated: 2026-01-09*
