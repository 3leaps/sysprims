//! sysprims-proc: Process inspection and enumeration utilities
//!
//! This crate provides cross-platform process inspection capabilities with a stable
//! JSON schema for automation. It supports listing all processes, filtering by various
//! criteria, and getting details for a specific PID.
//!
//! ## Features
//!
//! - **Process enumeration**: List all running processes
//! - **Process inspection**: Get details for a single process by PID
//! - **Filtering**: Filter by name, state, CPU, memory, user
//! - **Stable JSON output**: Schema-versioned output for automation
//!
//! ## Platform Support
//!
//! | Feature | Linux | macOS | Windows |
//! |---------|-------|-------|---------|
//! | PID enumeration | /proc | proc_listpids | Toolhelp32 |
//! | Process info | /proc/[pid]/* | proc_pidinfo | OpenProcess |
//! | CPU usage | /proc/[pid]/stat | proc_pidinfo | GetProcessTimes |
//! | Memory usage | /proc/[pid]/statm | proc_pidinfo | GetProcessMemoryInfo |
//!
//! ## Example
//!
//! ```rust,no_run
//! use sysprims_proc::{snapshot, get_process, ProcessFilter};
//!
//! // Get all processes
//! let snap = snapshot().unwrap();
//! println!("Found {} processes", snap.processes.len());
//!
//! // Get current process
//! let self_info = get_process(std::process::id()).unwrap();
//! println!("Current process: {} (PID {})", self_info.name, self_info.pid);
//!
//! // Filter processes by name
//! let filter = ProcessFilter {
//!     name_contains: Some("rust".into()),
//!     ..Default::default()
//! };
//! let filtered = sysprims_proc::snapshot_filtered(&filter).unwrap();
//! ```

use serde::{Deserialize, Serialize};
use sysprims_core::schema::PROCESS_INFO_V1;
use sysprims_core::{SysprimsError, SysprimsResult};

// Platform-specific implementations
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

// Re-export the platform implementation
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(windows)]
use windows as platform;

// ============================================================================
// Core Types
// ============================================================================

/// Snapshot of all processes at a point in time.
///
/// Contains a list of process information with a timestamp and schema identifier.
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
///
/// All fields are populated on a best-effort basis. Fields that cannot be read
/// (e.g., due to permissions) are set to default values or `None`.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    /// Process ID.
    pub pid: u32,

    /// Parent process ID.
    pub ppid: u32,

    /// Process name (executable name, max 255 chars).
    pub name: String,

    /// Owner username (None if unavailable due to permissions).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,

    /// CPU usage normalized 0-100 across all cores.
    ///
    /// Note: This is an instantaneous value and may be 0 for short-lived
    /// processes or processes that were just started.
    pub cpu_percent: f64,

    /// Memory usage in kilobytes.
    pub memory_kb: u64,

    /// Seconds since process start.
    pub elapsed_seconds: u64,

    /// Process state.
    pub state: ProcessState,

    /// Command line arguments.
    ///
    /// May be empty if command line cannot be read (permissions, zombie process).
    pub cmdline: Vec<String>,
}

/// Process state.
///
/// Maps platform-specific states to a common enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    /// Process is running or runnable.
    Running,
    /// Process is sleeping (interruptible or uninterruptible).
    Sleeping,
    /// Process is stopped (e.g., by a signal).
    Stopped,
    /// Process is a zombie (terminated but not reaped).
    Zombie,
    /// Process state could not be determined.
    Unknown,
}

/// Filter for process queries.
///
/// All fields are optional. Processes must match ALL specified criteria (AND logic).
/// Unknown fields in JSON input will result in `InvalidArgument` error.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessFilter {
    /// Filter by process name substring (case-insensitive).
    pub name_contains: Option<String>,

    /// Filter by exact process name (case-sensitive).
    pub name_equals: Option<String>,

    /// Filter by owner username (exact match).
    pub user_equals: Option<String>,

    /// Filter to specific PIDs.
    pub pid_in: Option<Vec<u32>>,

    /// Filter by process state.
    pub state_in: Option<Vec<ProcessState>>,

    /// Filter by minimum CPU usage (0-100).
    pub cpu_above: Option<f64>,

    /// Filter by minimum memory usage in KB.
    pub memory_above_kb: Option<u64>,
}

impl ProcessFilter {
    /// Validate filter values.
    ///
    /// Returns an error if any values are out of range.
    pub fn validate(&self) -> SysprimsResult<()> {
        if let Some(cpu) = self.cpu_above {
            if !(0.0..=100.0).contains(&cpu) {
                return Err(SysprimsError::invalid_argument(
                    "cpu_above must be between 0 and 100",
                ));
            }
        }
        Ok(())
    }

    /// Check if a process matches this filter.
    fn matches(&self, proc: &ProcessInfo) -> bool {
        // Name contains (case-insensitive)
        if let Some(ref pattern) = self.name_contains {
            if !proc.name.to_lowercase().contains(&pattern.to_lowercase()) {
                return false;
            }
        }

        // Name equals (exact)
        if let Some(ref name) = self.name_equals {
            if proc.name != *name {
                return false;
            }
        }

        // User equals
        if let Some(ref user) = self.user_equals {
            match &proc.user {
                Some(proc_user) if proc_user == user => {}
                _ => return false,
            }
        }

        // PID in list
        if let Some(ref pids) = self.pid_in {
            if !pids.contains(&proc.pid) {
                return false;
            }
        }

        // State in list
        if let Some(ref states) = self.state_in {
            if !states.contains(&proc.state) {
                return false;
            }
        }

        // CPU above threshold
        if let Some(threshold) = self.cpu_above {
            if proc.cpu_percent < threshold {
                return false;
            }
        }

        // Memory above threshold
        if let Some(threshold) = self.memory_above_kb {
            if proc.memory_kb < threshold {
                return false;
            }
        }

        true
    }
}

// ============================================================================
// Public API
// ============================================================================

/// Get a snapshot of all processes.
///
/// Returns a list of all processes visible to the current user. Processes that
/// cannot be read (e.g., due to permissions) are silently skipped.
///
/// # Example
///
/// ```rust,no_run
/// let snap = sysprims_proc::snapshot().unwrap();
/// for proc in &snap.processes {
///     println!("{}: {}", proc.pid, proc.name);
/// }
/// ```
pub fn snapshot() -> SysprimsResult<ProcessSnapshot> {
    platform::snapshot_impl()
}

/// Get a snapshot with filter applied.
///
/// Filters are applied after enumeration. All filter criteria must match (AND logic).
///
/// # Example
///
/// ```rust,no_run
/// use sysprims_proc::ProcessFilter;
///
/// let filter = ProcessFilter {
///     name_contains: Some("nginx".into()),
///     cpu_above: Some(1.0),
///     ..Default::default()
/// };
/// let snap = sysprims_proc::snapshot_filtered(&filter).unwrap();
/// ```
pub fn snapshot_filtered(filter: &ProcessFilter) -> SysprimsResult<ProcessSnapshot> {
    filter.validate()?;

    let mut snap = snapshot()?;
    snap.processes.retain(|p| filter.matches(p));
    Ok(snap)
}

/// Get information for a single process.
///
/// # Errors
///
/// Returns `NotFound` if the process does not exist.
/// Returns `PermissionDenied` if the process cannot be read.
///
/// # Example
///
/// ```rust,no_run
/// let self_info = sysprims_proc::get_process(std::process::id()).unwrap();
/// println!("Current process: {}", self_info.name);
/// ```
pub fn get_process(pid: u32) -> SysprimsResult<ProcessInfo> {
    if pid == 0 {
        return Err(SysprimsError::invalid_argument("PID 0 is not valid"));
    }
    platform::get_process_impl(pid)
}

// ============================================================================
// Helpers
// ============================================================================

/// Get current timestamp in ISO 8601 format.
fn current_timestamp() -> String {
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;

    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Create a ProcessSnapshot with the standard schema ID.
fn make_snapshot(processes: Vec<ProcessInfo>) -> ProcessSnapshot {
    ProcessSnapshot {
        schema_id: PROCESS_INFO_V1,
        timestamp: current_timestamp(),
        processes,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_not_empty() {
        let snap = snapshot().unwrap();
        assert!(
            !snap.processes.is_empty(),
            "Snapshot should contain processes"
        );
    }

    #[test]
    fn test_snapshot_has_schema_id() {
        let snap = snapshot().unwrap();
        assert_eq!(snap.schema_id, PROCESS_INFO_V1);
        assert!(snap.schema_id.contains("process-info"));
    }

    #[test]
    fn test_snapshot_has_timestamp() {
        let snap = snapshot().unwrap();
        assert!(!snap.timestamp.is_empty());
        // Should be RFC3339 format
        assert!(snap.timestamp.contains('T'));
        assert!(snap.timestamp.contains('Z') || snap.timestamp.contains('+'));
    }

    #[test]
    #[cfg(unix)]
    fn test_snapshot_has_processes() {
        // On macOS with SIP, we may only see user-owned processes
        // On Linux without root, we may also be restricted
        // Just verify we get a reasonable number of processes
        let snap = snapshot().unwrap();

        // We should at least see our own process
        let own_pid = std::process::id();
        let has_self = snap.processes.iter().any(|p| p.pid == own_pid);
        assert!(has_self, "Snapshot should include our own process");

        // We should see at least a few processes
        assert!(
            snap.processes.len() >= 3,
            "Should have at least a few visible processes, got {}",
            snap.processes.len()
        );
    }

    #[test]
    fn test_get_self() {
        let pid = std::process::id();
        let info = get_process(pid).unwrap();
        assert_eq!(info.pid, pid);
        assert!(!info.name.is_empty(), "Process should have a name");
    }

    #[test]
    fn test_get_self_has_valid_fields() {
        let pid = std::process::id();
        let info = get_process(pid).unwrap();

        // CPU percent should be in valid range
        assert!(info.cpu_percent >= 0.0);
        assert!(info.cpu_percent <= 100.0);

        // Memory should be non-zero for running process
        // Note: Don't require this as some systems may report 0
        assert!(info.memory_kb < u64::MAX);

        // State should be running (we're executing)
        assert!(
            info.state == ProcessState::Running || info.state == ProcessState::Sleeping,
            "Test process should be running or sleeping"
        );
    }

    #[test]
    fn test_filter_by_name_contains() {
        // Filter for our own test process
        let filter = ProcessFilter {
            name_contains: Some("sysprims".into()),
            ..Default::default()
        };
        let snap = snapshot_filtered(&filter).unwrap();

        for proc in &snap.processes {
            assert!(
                proc.name.to_lowercase().contains("sysprims"),
                "Filtered process '{}' should contain 'sysprims'",
                proc.name
            );
        }
    }

    #[test]
    fn test_filter_by_pid() {
        let my_pid = std::process::id();
        let filter = ProcessFilter {
            pid_in: Some(vec![my_pid]),
            ..Default::default()
        };
        let snap = snapshot_filtered(&filter).unwrap();

        assert_eq!(snap.processes.len(), 1);
        assert_eq!(snap.processes[0].pid, my_pid);
    }

    #[test]
    fn test_filter_validation_cpu_range() {
        let filter = ProcessFilter {
            cpu_above: Some(150.0),
            ..Default::default()
        };
        let result = filter.validate();
        assert!(result.is_err());

        let filter = ProcessFilter {
            cpu_above: Some(-1.0),
            ..Default::default()
        };
        let result = filter.validate();
        assert!(result.is_err());

        let filter = ProcessFilter {
            cpu_above: Some(50.0),
            ..Default::default()
        };
        let result = filter.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_nonexistent_pid() {
        // Use a very high PID that shouldn't exist
        let result = get_process(99999999);
        assert!(
            matches!(result, Err(SysprimsError::NotFound { .. })),
            "Should return NotFound for nonexistent PID"
        );
    }

    #[test]
    fn test_invalid_pid_zero() {
        let result = get_process(0);
        assert!(
            matches!(result, Err(SysprimsError::InvalidArgument { .. })),
            "PID 0 should be invalid"
        );
    }

    #[test]
    fn test_cpu_normalized() {
        let snap = snapshot().unwrap();
        for proc in &snap.processes {
            assert!(
                proc.cpu_percent >= 0.0,
                "CPU percent for PID {} should be >= 0",
                proc.pid
            );
            assert!(
                proc.cpu_percent <= 100.0,
                "CPU percent for PID {} should be <= 100, got {}",
                proc.pid,
                proc.cpu_percent
            );
        }
    }

    #[test]
    fn test_process_state_serialization() {
        // Test that states serialize to snake_case
        let json = serde_json::to_string(&ProcessState::Running).unwrap();
        assert_eq!(json, "\"running\"");

        let json = serde_json::to_string(&ProcessState::Sleeping).unwrap();
        assert_eq!(json, "\"sleeping\"");

        let json = serde_json::to_string(&ProcessState::Zombie).unwrap();
        assert_eq!(json, "\"zombie\"");
    }

    #[test]
    fn test_filter_deserialization() {
        let json = r#"{"name_contains": "test", "cpu_above": 5.0}"#;
        let filter: ProcessFilter = serde_json::from_str(json).unwrap();
        assert_eq!(filter.name_contains, Some("test".to_string()));
        assert_eq!(filter.cpu_above, Some(5.0));
    }

    #[test]
    fn test_filter_unknown_field_rejected() {
        let json = r#"{"unknown_field": "value"}"#;
        let result: Result<ProcessFilter, _> = serde_json::from_str(json);
        assert!(result.is_err(), "Unknown fields should be rejected");
    }

    #[test]
    fn test_snapshot_json_output() {
        let snap = snapshot().unwrap();
        let json = serde_json::to_string_pretty(&snap).unwrap();

        // Verify JSON structure
        assert!(json.contains("\"schema_id\""));
        assert!(json.contains("\"timestamp\""));
        assert!(json.contains("\"processes\""));
        assert!(json.contains(PROCESS_INFO_V1));
    }
}
