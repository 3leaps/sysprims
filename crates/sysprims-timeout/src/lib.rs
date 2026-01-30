//! sysprims-timeout: Process timeout with group-by-default tree management.
//!
//! This crate provides:
//! - Process execution with timeout ([`run_with_timeout`])
//! - Group-by-default semantics (entire process tree killed on timeout)
//! - Signal escalation (SIGTERM → SIGKILL after configurable delay)
//! - Observable fallback status for tree-kill reliability
//!
//! # Group-by-Default
//!
//! The core differentiator of sysprims over GNU timeout. When a command times
//! out, the **entire process tree** is killed, not just the direct child:
//!
//! - **Unix**: Creates a new process group; child is group leader
//! - **Windows**: Creates a Job Object with `KILL_ON_JOB_CLOSE`
//!
//! This prevents orphaned processes that ignore SIGTERM or attempt to escape.
//!
//! # Example
//!
//! ```no_run
//! use std::time::Duration;
//! use sysprims_timeout::{run_with_timeout, TimeoutConfig, TimeoutOutcome};
//!
//! let result = run_with_timeout(
//!     "sleep",
//!     &["60"],
//!     Duration::from_secs(5),
//!     TimeoutConfig::default(),
//! ).unwrap();
//!
//! match result {
//!     TimeoutOutcome::Completed { exit_status } => {
//!         println!("Command completed: {:?}", exit_status);
//!     }
//!     TimeoutOutcome::TimedOut { signal_sent, escalated, .. } => {
//!         println!("Timed out, sent signal {}, escalated: {}", signal_sent, escalated);
//!     }
//! }
//! ```

use std::process::ExitStatus;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sysprims_core::schema::TERMINATE_TREE_RESULT_V1;
use sysprims_core::{get_platform, SysprimsError, SysprimsResult};
use sysprims_proc::wait_pid;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

// Re-export signal constants for convenience
pub use sysprims_signal::{SIGKILL, SIGTERM};

/// Process grouping strategy.
///
/// Controls whether timeout creates a process group (Unix) or Job Object
/// (Windows) to enable tree-kill on timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupingMode {
    /// Create new process group (Unix) or Job Object (Windows).
    /// Kill entire tree on timeout. **This is the default.**
    #[default]
    GroupByDefault,

    /// Run in foreground. Only kills direct child on timeout.
    /// Use when the child must inherit the parent's process group.
    Foreground,
}

/// Configuration for timeout execution.
#[derive(Debug, Clone)]
pub struct TimeoutConfig {
    /// Initial signal to send on timeout.
    ///
    /// Default: `SIGTERM` (15)
    pub signal: i32,

    /// Delay before escalating to SIGKILL if process doesn't exit.
    ///
    /// Default: 10 seconds
    pub kill_after: Duration,

    /// Process grouping strategy.
    ///
    /// Default: `GroupByDefault`
    pub grouping: GroupingMode,

    /// Propagate child exit code when command completes normally.
    ///
    /// When `true`, the timeout exit code matches the child's exit code.
    /// When `false`, successful completion returns exit code 0.
    ///
    /// Default: `false`
    pub preserve_status: bool,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            signal: SIGTERM,
            kill_after: Duration::from_secs(10),
            grouping: GroupingMode::GroupByDefault,
            preserve_status: false,
        }
    }
}

/// Reliability of tree-kill operation.
///
/// Indicates whether the timeout was able to guarantee killing the entire
/// process tree, or had to fall back to best-effort single-process kill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeKillReliability {
    /// Tree-kill guaranteed. Process group (Unix) or Job Object (Windows)
    /// was successfully created and used.
    Guaranteed,

    /// Best-effort only. Process group or Job Object creation failed;
    /// only the direct child was killed. Grandchildren may have escaped.
    BestEffort,
}

// =============================================================================
// Terminate Tree (PID-based)
// =============================================================================

/// Configuration for PID-based terminate-tree.
///
/// This is intentionally small and conservative; higher-level spawn-in-group
/// APIs can provide stronger guarantees.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminateTreeConfig {
    /// Timeout before escalation.
    #[serde(default = "default_grace_timeout_ms")]
    pub grace_timeout_ms: u64,

    /// Timeout to wait after escalation signal.
    #[serde(default = "default_kill_timeout_ms")]
    pub kill_timeout_ms: u64,

    /// Signal to send first (default SIGTERM).
    #[serde(default = "default_grace_signal")]
    pub signal: i32,

    /// Signal to send on escalation (default SIGKILL).
    #[serde(default = "default_kill_signal")]
    pub kill_signal: i32,
}

fn default_grace_timeout_ms() -> u64 {
    10_000
}

fn default_kill_timeout_ms() -> u64 {
    2_000
}

fn default_grace_signal() -> i32 {
    SIGTERM
}

fn default_kill_signal() -> i32 {
    SIGKILL
}

impl Default for TerminateTreeConfig {
    fn default() -> Self {
        Self {
            grace_timeout_ms: default_grace_timeout_ms(),
            kill_timeout_ms: default_kill_timeout_ms(),
            signal: default_grace_signal(),
            kill_signal: default_kill_signal(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TerminateTreeResult {
    pub schema_id: &'static str,
    pub timestamp: String,
    pub platform: &'static str,
    pub pid: u32,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pgid: Option<u32>,

    pub signal_sent: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kill_signal: Option<i32>,
    pub escalated: bool,
    pub exited: bool,
    pub timed_out: bool,
    pub tree_kill_reliability: String,
    pub warnings: Vec<String>,
}

// =============================================================================
// Spawn In Group / Job
// =============================================================================

/// Spawn a process in a new process group (Unix) or Job Object (Windows).
///
/// This is designed for supervisors that want kill-tree-safe jobs without
/// using `run_with_timeout`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnInGroupConfig {
    /// argv[0] is the command, argv[1..] are args.
    pub argv: Vec<String>,

    /// Optional working directory.
    #[serde(default)]
    pub cwd: Option<String>,

    /// Environment variable overrides/additions.
    ///
    /// By default the child inherits the parent's environment.
    #[serde(default)]
    pub env: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpawnInGroupResult {
    pub schema_id: &'static str,
    pub timestamp: String,
    pub platform: &'static str,
    pub pid: u32,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pgid: Option<u32>,

    pub tree_kill_reliability: String,
    pub warnings: Vec<String>,
}

pub fn spawn_in_group(config: SpawnInGroupConfig) -> SysprimsResult<SpawnInGroupResult> {
    if config.argv.is_empty() {
        return Err(SysprimsError::invalid_argument("argv must not be empty"));
    }

    #[cfg(unix)]
    return unix::spawn_in_group_impl(config);

    #[cfg(windows)]
    return windows::spawn_in_group_impl(config);
}

pub(crate) fn current_timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Terminate a process (and best-effort tree) with escalation.
///
/// PID-only API: if the target PID is a process group leader (Unix only), this will
/// prefer group kill for better coverage. Otherwise it signals the PID directly.
pub fn terminate_tree(
    pid: u32,
    config: TerminateTreeConfig,
) -> SysprimsResult<TerminateTreeResult> {
    if pid == 0 {
        return Err(SysprimsError::invalid_argument("pid must be > 0"));
    }

    // Defense in depth: avoid unsafe casts on Unix.
    // See ADR-0011 (PID Validation Safety).
    if pid > sysprims_signal::MAX_SAFE_PID {
        return Err(SysprimsError::invalid_argument(format!(
            "pid {} exceeds maximum safe value {}",
            pid,
            sysprims_signal::MAX_SAFE_PID
        )));
    }

    let mut warnings: Vec<String> = Vec::new();
    let mut pgid: Option<u32> = None;
    let mut reliability = TreeKillReliability::BestEffort;

    // Decide whether we can safely use group kill (Unix only).
    #[cfg(unix)]
    {
        use sysprims_signal::MAX_SAFE_PID;
        if pid <= MAX_SAFE_PID {
            let pid_i32 = pid as i32;
            let self_pgid = unsafe { libc::getpgid(0) };
            let target_pgid = unsafe { libc::getpgid(pid_i32) };

            if target_pgid == -1 {
                warnings.push("Could not determine process group for pid".to_string());
            } else if target_pgid == pid_i32 {
                // Target is a group leader. Only use killpg if it isn't our own group.
                if self_pgid != -1 && target_pgid == self_pgid {
                    warnings.push(
                        "Target pid is in caller's process group; refusing group kill".to_string(),
                    );
                } else {
                    pgid = Some(target_pgid as u32);
                    reliability = TreeKillReliability::Guaranteed;
                }
            } else {
                warnings
                    .push("Target pid is not a process group leader; using pid kill".to_string());
            }
        } else {
            warnings.push("pid exceeds max safe pid for POSIX kill".to_string());
        }
    }

    #[cfg(windows)]
    {
        // If this PID was spawned via spawn_in_group_impl(), we may have a Job Object.
        // Prefer terminating the Job Object for better tree coverage.
        if crate::windows::terminate_job_for_pid(pid).is_some() {
            warnings.push("Terminated via Job Object (spawn_in_group)".to_string());

            let grace_wait = wait_pid(pid, Duration::from_millis(config.grace_timeout_ms))?;
            return Ok(TerminateTreeResult {
                schema_id: TERMINATE_TREE_RESULT_V1,
                timestamp: current_timestamp(),
                platform: get_platform(),
                pid,
                pgid: None,
                signal_sent: config.signal,
                kill_signal: None,
                escalated: false,
                exited: grace_wait.exited,
                timed_out: grace_wait.timed_out,
                tree_kill_reliability: "guaranteed".to_string(),
                warnings,
            });
        }

        warnings.push("Windows PID termination is best-effort without Job Object".to_string());
    }

    // Step 1: send graceful signal
    // If group kill fails (e.g. permission-limited), fall back to PID kill.
    if let Some(g) = pgid {
        match sysprims_signal::killpg(g, config.signal) {
            Ok(()) => {}
            Err(SysprimsError::PermissionDenied { .. }) => {
                warnings.push(
                    "Permission denied signaling process group; falling back to pid".to_string(),
                );
                pgid = None;
                reliability = TreeKillReliability::BestEffort;
                sysprims_signal::kill(pid, config.signal)?;
            }
            Err(e) => return Err(e),
        }
    } else {
        sysprims_signal::kill(pid, config.signal)?;
    }

    // Step 2: wait for exit
    let grace = Duration::from_millis(config.grace_timeout_ms);
    let grace_wait = wait_pid(pid, grace)?;
    if grace_wait.exited {
        return Ok(TerminateTreeResult {
            schema_id: TERMINATE_TREE_RESULT_V1,
            timestamp: current_timestamp(),
            platform: get_platform(),
            pid,
            pgid,
            signal_sent: config.signal,
            kill_signal: None,
            escalated: false,
            exited: true,
            timed_out: false,
            tree_kill_reliability: match reliability {
                TreeKillReliability::Guaranteed => "guaranteed".to_string(),
                TreeKillReliability::BestEffort => "best_effort".to_string(),
            },
            warnings,
        });
    }

    // Step 3: escalate
    if let Some(g) = pgid {
        match sysprims_signal::killpg(g, config.kill_signal) {
            Ok(()) => {}
            Err(SysprimsError::PermissionDenied { .. }) => {
                warnings.push(
                    "Permission denied signaling process group (kill); falling back to pid"
                        .to_string(),
                );
                pgid = None;
                reliability = TreeKillReliability::BestEffort;
                sysprims_signal::kill(pid, config.kill_signal)?;
            }
            Err(e) => return Err(e),
        }
    } else {
        sysprims_signal::kill(pid, config.kill_signal)?;
    }

    let kill_wait = wait_pid(pid, Duration::from_millis(config.kill_timeout_ms))?;
    let mut exited = kill_wait.exited;
    let mut timed_out = kill_wait.timed_out;

    // If we timed out, attempt one final best-effort confirmation.
    // On some platforms/permission contexts, a process may become unobservable
    // (or a zombie) even after it has exited.
    if timed_out {
        match sysprims_proc::get_process(pid) {
            Ok(_) => {
                // Still observable -> treat as still running.
            }
            Err(SysprimsError::NotFound { .. }) => {
                exited = true;
                timed_out = false;
                warnings.push("PID no longer found after timeout; treating as exited".to_string());
            }
            Err(SysprimsError::PermissionDenied { .. }) => {
                warnings.push("Permission denied while confirming exit after timeout".to_string());
            }
            Err(e) => {
                warnings.push(format!("Failed to confirm exit after timeout: {}", e));
            }
        }
    }

    Ok(TerminateTreeResult {
        schema_id: TERMINATE_TREE_RESULT_V1,
        timestamp: current_timestamp(),
        platform: get_platform(),
        pid,
        pgid,
        signal_sent: config.signal,
        kill_signal: Some(config.kill_signal),
        escalated: true,
        exited,
        timed_out,
        tree_kill_reliability: match reliability {
            TreeKillReliability::Guaranteed => "guaranteed".to_string(),
            TreeKillReliability::BestEffort => "best_effort".to_string(),
        },
        warnings,
    })
}

/// Outcome of timeout execution.
#[derive(Debug)]
pub enum TimeoutOutcome {
    /// Command completed within timeout.
    Completed {
        /// Exit status of the child process.
        exit_status: ExitStatus,
    },

    /// Command timed out and was killed.
    TimedOut {
        /// Signal that was sent to terminate the process.
        signal_sent: i32,

        /// Whether escalation to SIGKILL occurred.
        ///
        /// `true` if the process didn't exit after receiving `signal_sent`
        /// and had to be forcefully killed with SIGKILL.
        escalated: bool,

        /// Whether tree-kill was reliable.
        ///
        /// `Guaranteed` if process group/Job Object worked.
        /// `BestEffort` if only the direct child was killed.
        tree_kill_reliability: TreeKillReliability,
    },
}

/// Run a command with timeout.
///
/// Spawns the command and waits for it to complete or timeout. If the command
/// times out, the entire process tree is killed (when using `GroupByDefault`).
///
/// # Arguments
///
/// * `command` - Command to execute (path or name in PATH)
/// * `args` - Command arguments
/// * `timeout` - Maximum duration to wait for command completion
/// * `config` - Timeout configuration (signal, escalation, grouping)
///
/// # Returns
///
/// * `Ok(TimeoutOutcome::Completed { .. })` - Command finished within timeout
/// * `Ok(TimeoutOutcome::TimedOut { .. })` - Command was killed due to timeout
/// * `Err(SysprimsError)` - Failed to spawn or fatal error
///
/// # Example
///
/// ```no_run
/// use std::time::Duration;
/// use sysprims_timeout::{run_with_timeout, TimeoutConfig};
///
/// let result = run_with_timeout(
///     "make",
///     &["build"],
///     Duration::from_secs(300),
///     TimeoutConfig::default(),
/// );
/// ```
pub fn run_with_timeout(
    command: &str,
    args: &[&str],
    timeout: Duration,
    config: TimeoutConfig,
) -> SysprimsResult<TimeoutOutcome> {
    #[cfg(unix)]
    return unix::run_with_timeout_impl(command, args, timeout, &config);

    #[cfg(windows)]
    return windows::run_with_timeout_impl(command, args, timeout, &config);
}

/// Run a command with timeout using default configuration.
///
/// Equivalent to `run_with_timeout(command, args, timeout, TimeoutConfig::default())`.
///
/// Default configuration:
/// - Signal: SIGTERM
/// - Kill after: 10 seconds
/// - Grouping: GroupByDefault
/// - Preserve status: false
pub fn run_with_timeout_default(
    command: &str,
    args: &[&str],
    timeout: Duration,
) -> SysprimsResult<TimeoutOutcome> {
    run_with_timeout(command, args, timeout, TimeoutConfig::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn default_config_uses_sigterm() {
        let config = TimeoutConfig::default();
        assert_eq!(config.signal, SIGTERM);
    }

    #[test]
    fn default_config_uses_group_by_default() {
        let config = TimeoutConfig::default();
        assert_eq!(config.grouping, GroupingMode::GroupByDefault);
    }

    #[test]
    fn default_config_kill_after_is_10_seconds() {
        let config = TimeoutConfig::default();
        assert_eq!(config.kill_after, Duration::from_secs(10));
    }

    #[test]
    fn default_config_does_not_preserve_status() {
        let config = TimeoutConfig::default();
        assert!(!config.preserve_status);
    }

    #[test]
    fn terminate_tree_rejects_pid_zero() {
        let err = terminate_tree(0, TerminateTreeConfig::default()).unwrap_err();
        assert!(matches!(err, SysprimsError::InvalidArgument { .. }));
    }

    #[test]
    #[cfg(unix)]
    fn terminate_tree_kills_spawned_child() {
        // SAFETY: We spawn this process ourselves and control its PID.
        let mut child = Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to spawn sleep process");

        let pid = child.id();
        let result = terminate_tree(
            pid,
            TerminateTreeConfig {
                grace_timeout_ms: 100,
                kill_timeout_ms: 5000,
                ..TerminateTreeConfig::default()
            },
        )
        .expect("terminate_tree should succeed");

        assert_eq!(result.pid, pid);
        assert!(
            result.exited,
            "expected child to be exited, got: {result:?}"
        );
        assert!(!result.timed_out, "unexpected timeout: {result:?}");

        let _ = child.wait();
    }

    #[test]
    #[cfg(windows)]
    fn terminate_tree_kills_spawned_child() {
        // SAFETY: We spawn this process ourselves and control its PID.
        let mut child = Command::new("cmd")
            .args(["/C", "ping -n 60 127.0.0.1 >NUL"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to spawn ping sleep process");

        let pid = child.id();
        let result = terminate_tree(
            pid,
            TerminateTreeConfig {
                grace_timeout_ms: 100,
                kill_timeout_ms: 5000,
                ..TerminateTreeConfig::default()
            },
        )
        .expect("terminate_tree should succeed");

        assert_eq!(result.pid, pid);
        let _ = child.wait();
    }
}
