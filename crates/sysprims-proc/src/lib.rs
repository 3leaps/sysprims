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
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;
use sysprims_core::schema::{
    DESCENDANTS_RESULT_V1, FD_SNAPSHOT_V1, PORT_BINDINGS_V1, PORT_FILTER_V1, PROCESS_INFO_V1,
    WAIT_PID_RESULT_V1,
};
use sysprims_core::{get_platform, SysprimsError, SysprimsResult};

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

/// Result of waiting for a PID to exit.
///
/// Best-effort cross-platform semantics:
/// - On Unix, this uses a polling strategy (we are not necessarily the parent).
/// - On Windows, this uses process wait APIs when available.
#[derive(Debug, Clone, Serialize)]
pub struct WaitPidResult {
    /// Schema identifier for version detection.
    pub schema_id: &'static str,

    /// Timestamp of result creation (ISO 8601).
    pub timestamp: String,

    /// Current platform (e.g., "linux", "macos", "windows").
    pub platform: &'static str,

    /// PID waited on.
    pub pid: u32,

    /// True if the process was observed to have exited.
    pub exited: bool,

    /// True if the wait timed out before exit was observed.
    pub timed_out: bool,

    /// Exit code when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,

    /// Warnings about degraded visibility.
    pub warnings: Vec<String>,
}

/// Snapshot of listening ports at a point in time.
#[derive(Debug, Clone, Serialize)]
pub struct PortBindingsSnapshot {
    /// Schema identifier for version detection.
    pub schema_id: &'static str,

    /// Timestamp of snapshot (ISO 8601).
    pub timestamp: String,

    /// Current platform (e.g., "linux", "macos", "windows").
    pub platform: &'static str,

    /// List of socket bindings.
    pub bindings: Vec<PortBinding>,

    /// Warnings about partial visibility or skipped entries.
    pub warnings: Vec<String>,
}

/// Listening socket binding information.
#[derive(Debug, Clone, Serialize)]
pub struct PortBinding {
    /// Protocol for the socket.
    pub protocol: Protocol,

    /// Local address (None if unknown).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_addr: Option<IpAddr>,

    /// Local port for the socket.
    pub local_port: u16,

    /// Socket state (e.g., "listen" for TCP).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,

    /// Owning process ID (None if attribution not available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,

    /// Owning process info (best-effort).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessInfo>,

    /// Socket inode when available (Linux-only).
    #[serde(skip)]
    pub inode: Option<u64>,
}

/// Protocol for a socket binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Tcp,
    Udp,
}

/// Filter for port queries.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortFilter {
    /// Filter by protocol (tcp/udp).
    pub protocol: Option<Protocol>,

    /// Filter by local port.
    pub local_port: Option<u16>,
}

/// File descriptor kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FdKind {
    File,
    Socket,
    Pipe,
    Unknown,
}

/// Filter for fd queries.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FdFilter {
    /// Filter by fd kind.
    pub kind: Option<FdKind>,
}

impl FdFilter {
    pub fn validate(&self) -> SysprimsResult<()> {
        // No numeric ranges for now.
        Ok(())
    }
}

/// Information about a single file descriptor.
#[derive(Debug, Clone, Serialize)]
pub struct FdInfo {
    /// File descriptor number.
    pub fd: u32,

    /// Best-effort fd classification.
    pub kind: FdKind,

    /// Best-effort resolved path/target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Snapshot of open file descriptors for a process.
#[derive(Debug, Clone, Serialize)]
pub struct FdSnapshot {
    /// Schema identifier for version detection.
    pub schema_id: &'static str,

    /// Timestamp of snapshot (ISO 8601).
    pub timestamp: String,

    /// Current platform (e.g., "linux", "macos", "windows").
    pub platform: &'static str,

    /// Target PID.
    pub pid: u32,

    /// List of open file descriptors.
    pub fds: Vec<FdInfo>,

    /// Warnings about partial visibility.
    pub warnings: Vec<String>,
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

    /// Process start time (Unix epoch milliseconds), when available.
    ///
    /// Best-effort: omitted if the platform cannot provide it or access is denied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time_unix_ms: Option<u64>,

    /// Executable path (absolute), when available.
    ///
    /// Best-effort: omitted if the platform cannot provide it or access is denied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exe_path: Option<String>,

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

    /// Filter by parent PID.
    pub ppid: Option<u32>,

    /// Filter by process state.
    pub state_in: Option<Vec<ProcessState>>,

    /// Filter by minimum CPU usage (0-100).
    pub cpu_above: Option<f64>,

    /// Filter by minimum memory usage in KB.
    pub memory_above_kb: Option<u64>,

    /// Filter by minimum process age in seconds.
    ///
    /// Uses `elapsed_seconds` (best-effort, already cross-platform).
    pub running_for_at_least_secs: Option<u64>,
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
}

impl PortFilter {
    /// Validate filter values.
    pub fn validate(&self) -> SysprimsResult<()> {
        if let Some(port) = self.local_port {
            if port == 0 {
                return Err(SysprimsError::invalid_argument(
                    "local_port must be between 1 and 65535",
                ));
            }
        }
        Ok(())
    }

    pub fn schema_id() -> &'static str {
        PORT_FILTER_V1
    }
}

impl FdInfo {
    fn matches(&self, filter: &FdFilter) -> bool {
        if let Some(kind) = filter.kind {
            if self.kind != kind {
                return false;
            }
        }

        true
    }
}

impl PortBinding {
    fn matches(&self, filter: &PortFilter) -> bool {
        if let Some(protocol) = filter.protocol {
            if self.protocol != protocol {
                return false;
            }
        }

        if let Some(port) = filter.local_port {
            if self.local_port != port {
                return false;
            }
        }

        true
    }
}

impl ProcessFilter {
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

        // Parent PID
        if let Some(ppid) = self.ppid {
            if proc.ppid != ppid {
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

        // Process age (minimum elapsed seconds)
        if let Some(min_secs) = self.running_for_at_least_secs {
            if proc.elapsed_seconds < min_secs {
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

/// Get total CPU time consumed by a process (kernel + user) in nanoseconds.
///
/// This is a best-effort value used for sampling-based CPU calculations.
///
/// Notes:
/// - The existing `ProcessInfo.cpu_percent` is a lifetime-average estimate.
/// - For near-instant CPU usage, callers should sample this value twice and
///   compute a rate over an interval.
pub fn cpu_total_time_ns(pid: u32) -> SysprimsResult<u64> {
    platform::cpu_total_time_ns_impl(pid)
}

/// Get a snapshot of listening ports.
pub fn listening_ports(filter: Option<&PortFilter>) -> SysprimsResult<PortBindingsSnapshot> {
    let filter = filter.cloned().unwrap_or_default();
    filter.validate()?;

    let mut snapshot = platform::listening_ports_impl()?;
    if filter.protocol.is_some() || filter.local_port.is_some() {
        snapshot.bindings.retain(|binding| binding.matches(&filter));
    }

    if snapshot.bindings.is_empty() && snapshot.warnings.is_empty() {
        let platform = get_platform();
        return Err(SysprimsError::not_supported("port bindings", platform));
    }

    Ok(snapshot)
}

/// List open file descriptors for a PID.
///
/// Best-effort cross-platform behavior:
/// - Linux: enumerates `/proc/<pid>/fd` symlinks.
/// - macOS: enumerates via libproc (`proc_pidinfo(PROC_PIDLISTFDS)`) and attempts path recovery.
/// - Windows: returns NotSupported.
pub fn list_fds(pid: u32, filter: Option<&FdFilter>) -> SysprimsResult<FdSnapshot> {
    // Safety: avoid negative pid_t casting semantics on Unix.
    const MAX_SAFE_PID: u32 = i32::MAX as u32;
    if pid == 0 {
        return Err(SysprimsError::invalid_argument("PID 0 is not valid"));
    }
    if pid > MAX_SAFE_PID {
        return Err(SysprimsError::invalid_argument(format!(
            "PID {} exceeds maximum safe value {}",
            pid, MAX_SAFE_PID
        )));
    }

    let filter = filter.cloned().unwrap_or_default();
    filter.validate()?;

    let (mut fds, mut warnings) = platform::list_fds_impl(pid)?;
    if filter.kind.is_some() {
        fds.retain(|fd| fd.matches(&filter));
    }

    // Best-effort: provide a helpful warning if nothing visible.
    if fds.is_empty() {
        warnings.push("No file descriptors visible".to_string());
    }

    Ok(make_fd_snapshot(pid, fds, warnings))
}

/// Resolve a process by port and protocol.
pub fn process_by_port(port: u16, protocol: Protocol) -> SysprimsResult<ProcessInfo> {
    if port == 0 {
        return Err(SysprimsError::invalid_argument(
            "port must be between 1 and 65535",
        ));
    }

    let filter = PortFilter {
        protocol: Some(protocol),
        local_port: Some(port),
    };
    let snapshot = listening_ports(Some(&filter))?;
    let binding = snapshot
        .bindings
        .into_iter()
        .find(|binding| binding.pid.is_some())
        .ok_or_else(|| SysprimsError::not_found(port as u32))?;

    match binding.process {
        Some(process) => Ok(process),
        None => binding
            .pid
            .map(get_process)
            .unwrap_or_else(|| Err(SysprimsError::not_found(port as u32))),
    }
}

#[cfg(unix)]
fn aggregate_permission_warning(skipped: usize, label: &str) -> Option<String> {
    if skipped == 0 {
        None
    } else {
        Some(format!(
            "Skipped {} {} due to permission errors",
            skipped, label
        ))
    }
}

fn aggregate_error_warning(skipped: usize, label: &str) -> Option<String> {
    if skipped == 0 {
        None
    } else {
        Some(format!("Skipped {} {} due to read errors", skipped, label))
    }
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
// Descendants API
// ============================================================================

/// A single level in a descendants result.
#[derive(Debug, Clone, Serialize)]
pub struct DescendantsLevel {
    /// Depth level (1 = direct children, 2 = grandchildren, etc.).
    pub level: u32,

    /// Processes at this level.
    pub processes: Vec<ProcessInfo>,
}

/// Result of a descendants traversal.
#[derive(Debug, Clone, Serialize)]
pub struct DescendantsResult {
    /// Schema identifier for version detection.
    pub schema_id: &'static str,

    /// Root PID that was traversed from.
    pub root_pid: u32,

    /// Maximum depth that was requested.
    pub max_levels: u32,

    /// Processes grouped by depth level.
    pub levels: Vec<DescendantsLevel>,

    /// Total number of descendant processes found (before filtering).
    pub total_found: usize,

    /// Number of processes matching the filter (after filtering).
    pub matched_by_filter: usize,

    /// Timestamp (ISO 8601).
    pub timestamp: String,

    /// Platform identifier.
    pub platform: &'static str,
}

/// Get descendants of a process using BFS traversal.
///
/// Performs a single snapshot and builds a parent→children map, then traverses
/// from `root_pid` up to `max_levels` deep. An optional filter is applied to
/// the results after traversal.
///
/// # Arguments
///
/// * `root_pid` - PID to start traversal from (must exist).
/// * `max_levels` - Maximum depth (1 = children only, u32::MAX = all).
/// * `filter` - Optional filter applied to descendant processes.
///
/// # Safety
///
/// Validates `root_pid` against ADR-0011 forbidden values (0, > i32::MAX).
pub fn descendants(
    root_pid: u32,
    max_levels: u32,
    filter: Option<&ProcessFilter>,
) -> SysprimsResult<DescendantsResult> {
    const MAX_SAFE_PID: u32 = i32::MAX as u32;

    if root_pid == 0 {
        return Err(SysprimsError::invalid_argument("PID 0 is not valid"));
    }
    if root_pid > MAX_SAFE_PID {
        return Err(SysprimsError::invalid_argument(format!(
            "PID {} exceeds maximum safe value {}",
            root_pid, MAX_SAFE_PID
        )));
    }

    // Verify root exists.
    let _ = get_process(root_pid)?;

    // Single snapshot for consistent traversal.
    let snap = snapshot()?;

    // Build parent → children map.
    let mut children_map: HashMap<u32, Vec<ProcessInfo>> = HashMap::new();
    for proc in snap.processes {
        children_map.entry(proc.ppid).or_default().push(proc);
    }

    // BFS traversal.
    let mut levels: Vec<DescendantsLevel> = Vec::new();
    let mut current_pids = vec![root_pid];
    let mut total_found: usize = 0;

    for depth in 1..=max_levels {
        let mut level_procs = Vec::new();
        let mut next_pids = Vec::new();

        for &pid in &current_pids {
            if let Some(children) = children_map.get(&pid) {
                for child in children {
                    level_procs.push(child.clone());
                    next_pids.push(child.pid);
                }
            }
        }

        if level_procs.is_empty() {
            break;
        }

        total_found += level_procs.len();
        levels.push(DescendantsLevel {
            level: depth,
            processes: level_procs,
        });
        current_pids = next_pids;
    }

    // Apply filter if provided.
    let mut matched_by_filter = total_found;
    if let Some(f) = filter {
        for level in &mut levels {
            level.processes.retain(|p| f.matches(p));
        }
        matched_by_filter = levels.iter().map(|l| l.processes.len()).sum();
        // Remove empty levels after filtering.
        levels.retain(|l| !l.processes.is_empty());
    }

    Ok(DescendantsResult {
        schema_id: DESCENDANTS_RESULT_V1,
        root_pid,
        max_levels,
        levels,
        total_found,
        matched_by_filter,
        timestamp: current_timestamp(),
        platform: get_platform(),
    })
}

/// Wait for a PID to exit, up to the provided timeout.
///
/// Returns:
/// - `Ok` with `timed_out=true` if the process did not exit in time.
/// - `Ok` with `exited=true` if the process was observed to have exited.
/// - `Err(NotFound)` if the PID does not exist at the time of the first check.
/// - `Err(PermissionDenied)` if the platform forbids even querying liveness.
pub fn wait_pid(pid: u32, timeout: Duration) -> SysprimsResult<WaitPidResult> {
    if pid == 0 {
        return Err(SysprimsError::invalid_argument("PID 0 is not valid"));
    }
    platform::wait_pid_impl(pid, timeout)
}

// ============================================================================
// Helpers
// ============================================================================

fn make_port_snapshot(bindings: Vec<PortBinding>, warnings: Vec<String>) -> PortBindingsSnapshot {
    let mut warnings = warnings;
    if bindings.is_empty() {
        warnings.push("No listening ports found".to_string());
    }

    PortBindingsSnapshot {
        schema_id: PORT_BINDINGS_V1,
        timestamp: current_timestamp(),
        platform: get_platform(),
        bindings,
        warnings,
    }
}

fn make_fd_snapshot(pid: u32, fds: Vec<FdInfo>, warnings: Vec<String>) -> FdSnapshot {
    FdSnapshot {
        schema_id: FD_SNAPSHOT_V1,
        timestamp: current_timestamp(),
        platform: get_platform(),
        pid,
        fds,
        warnings,
    }
}

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

fn make_wait_pid_result(
    pid: u32,
    exited: bool,
    timed_out: bool,
    exit_code: Option<i32>,
    warnings: Vec<String>,
) -> WaitPidResult {
    WaitPidResult {
        schema_id: WAIT_PID_RESULT_V1,
        timestamp: current_timestamp(),
        platform: get_platform(),
        pid,
        exited,
        timed_out,
        exit_code,
        warnings,
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

        // Identity fields are best-effort; if present, they should be reasonable.
        if let Some(ms) = info.start_time_unix_ms {
            assert!(ms > 0);
        }
        if let Some(ref exe) = info.exe_path {
            assert!(!exe.is_empty());
        }

        // State should be running (we're executing)
        // Note: Windows reports Unknown as it doesn't expose process state simply
        assert!(
            info.state == ProcessState::Running
                || info.state == ProcessState::Sleeping
                || info.state == ProcessState::Unknown,
            "Test process should be running, sleeping, or unknown (Windows)"
        );
    }

    #[test]
    fn test_wait_pid_self_times_out() {
        let pid = std::process::id();
        let r = wait_pid(pid, Duration::from_millis(1)).unwrap();
        assert_eq!(r.pid, pid);
        assert!(r.timed_out);
        assert!(!r.exited);
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
    fn test_port_filter_unknown_field_rejected() {
        let json = r#"{"unknown_field": true}"#;
        let result: Result<PortFilter, _> = serde_json::from_str(json);
        assert!(result.is_err(), "Unknown fields should be rejected");
    }

    #[test]
    fn test_port_filter_schema_id() {
        assert!(PortFilter::schema_id().contains("port-filter"));
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
