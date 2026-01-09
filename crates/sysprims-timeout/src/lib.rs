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

use serde::Serialize;
use sysprims_core::SysprimsResult;

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
}
