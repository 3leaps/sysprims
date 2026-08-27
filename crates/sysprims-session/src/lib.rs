//! GPL-free session and process group management.
//!
//! This crate provides cross-platform primitives for session and process group
//! management, replacing GPL-licensed tools like `setsid` (util-linux) and
//! `nohup` (coreutils).
//!
//! # Provenance
//!
//! All implementations are derived from POSIX specifications and BSD/MIT-licensed
//! references. See `docs/design/sysprims-session-behavioral-comparison.md` for the
//! full provenance record.
//!
//! # Features
//!
//! - [`run_setsid`] - Run a command in a new session
//! - [`run_nohup`] - Run a command immune to SIGHUP
//! - [`prepare_session_acquisition`] - Prepare a Unix spawn-time session
//!   acquisition hook and sealed same-spawn receipt
//!
//! # Example
//!
//! ```no_run
//! use sysprims_session::{run_setsid, SetsidConfig};
//!
//! // Run a command in a new session (detached from terminal)
//! let result = run_setsid("sleep", &["60"], SetsidConfig::default());
//! ```

use std::process::ExitStatus;
use std::{collections::BTreeMap, path::PathBuf};
use sysprims_core::SysprimsResult;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::{
    prepare_session_acquisition, PendingUnixSessionReceipt, UnixSessionAcquisitionHook,
    UnixSessionReceipt,
};

// ============================================================================
// setsid - Create New Session
// ============================================================================

/// Configuration for setsid execution.
#[derive(Debug, Clone, Default)]
pub struct SetsidConfig {
    /// Wait for the child process to exit and return its status.
    ///
    /// When `false` (default), returns immediately after spawning.
    /// When `true`, waits for child and returns its exit status.
    pub wait: bool,

    /// Create a controlling terminal (ctty) for the new session.
    ///
    /// This is a no-op placeholder for compatibility with util-linux setsid -c.
    /// Most use cases don't need this.
    pub ctty: bool,

    /// Working directory for the child process.
    ///
    /// `None` means inherit the caller's current working directory.
    pub cwd: Option<PathBuf>,

    /// Environment overrides for the child process.
    ///
    /// Values are merged into the inherited environment. v1 does not provide a
    /// replace-or-clear environment mode.
    pub env: Option<BTreeMap<String, String>>,
}

/// Outcome of setsid execution.
#[derive(Debug)]
pub enum SetsidOutcome {
    /// Child spawned successfully in new session.
    ///
    /// When `wait: false`, the child continues running detached.
    Spawned {
        /// PID of the child process in the new session.
        child_pid: u32,
    },

    /// Child completed (when `wait: true`).
    Completed {
        /// PID of the child process in the new session.
        child_pid: u32,
        /// Exit status of the child.
        exit_status: ExitStatus,
    },
}

/// Run a command in a new session.
///
/// Creates a new session with the command as session leader, detaching it
/// from the controlling terminal and parent process group.
///
/// This is equivalent to the `setsid` command from util-linux, but GPL-free.
///
/// # Arguments
///
/// * `command` - Command to execute
/// * `args` - Command arguments
/// * `config` - Execution configuration
///
/// # Returns
///
/// * `Ok(SetsidOutcome::Spawned { .. })` - Child started in new session
/// * `Ok(SetsidOutcome::Completed { .. })` - Child finished (if `wait: true`)
/// * `Err(SysprimsError)` - Failed to spawn or setsid failed
///
/// # Example
///
/// ```no_run
/// use sysprims_session::{run_setsid, SetsidConfig};
///
/// // Spawn detached
/// let result = run_setsid("sleep", &["300"], SetsidConfig::default())?;
///
/// // Wait for completion
/// let result = run_setsid("make", &["build"], SetsidConfig { wait: true, ..Default::default() })?;
/// # Ok::<(), sysprims_core::SysprimsError>(())
/// ```
pub fn run_setsid(
    command: &str,
    args: &[&str],
    config: SetsidConfig,
) -> SysprimsResult<SetsidOutcome> {
    #[cfg(unix)]
    return unix::run_setsid_impl(command, args, &config);

    #[cfg(windows)]
    {
        let _ = (command, args, config); // Unused on Windows
        return Err(sysprims_core::SysprimsError::not_supported(
            "setsid", "windows",
        ));
    }
}

// ============================================================================
// nohup - Ignore SIGHUP
// ============================================================================

/// Configuration for nohup execution.
#[derive(Debug, Clone, Default)]
pub struct NohupConfig {
    /// File to redirect stdout to when stdout is a terminal.
    ///
    /// Default: "nohup.out" in current directory, falls back to $HOME/nohup.out
    pub output_file: Option<String>,

    /// Wait for the child process to exit.
    pub wait: bool,

    /// Working directory for the child process.
    ///
    /// `None` means inherit the caller's current working directory.
    pub cwd: Option<PathBuf>,

    /// Environment overrides for the child process.
    ///
    /// Values are merged into the inherited environment. v1 does not provide a
    /// replace-or-clear environment mode.
    pub env: Option<BTreeMap<String, String>>,
}

/// Outcome of nohup execution.
#[derive(Debug)]
pub enum NohupOutcome {
    /// Child spawned successfully with SIGHUP ignored.
    Spawned {
        /// PID of the child process.
        child_pid: u32,
        /// Output file if stdout was redirected.
        output_file: Option<String>,
    },

    /// Child completed (when `wait: true`).
    Completed {
        /// PID of the child process.
        child_pid: u32,
        /// Exit status of the child.
        exit_status: ExitStatus,
        /// Output file if stdout/stderr was redirected.
        output_file: Option<String>,
    },
}

/// Run a command immune to SIGHUP.
///
/// Sets SIGHUP to be ignored before executing the command, allowing it to
/// continue running after the terminal is closed.
///
/// If stdout is a terminal, output is redirected to `nohup.out`.
///
/// This is equivalent to the `nohup` command from coreutils, but GPL-free.
///
/// # Arguments
///
/// * `command` - Command to execute
/// * `args` - Command arguments
/// * `config` - Execution configuration
///
/// # Example
///
/// ```no_run
/// use sysprims_session::{run_nohup, NohupConfig};
///
/// let result = run_nohup("./long-running-job.sh", &[], NohupConfig::default())?;
/// # Ok::<(), sysprims_core::SysprimsError>(())
/// ```
pub fn run_nohup(
    command: &str,
    args: &[&str],
    config: NohupConfig,
) -> SysprimsResult<NohupOutcome> {
    #[cfg(unix)]
    return unix::run_nohup_impl(command, args, &config);

    #[cfg(windows)]
    {
        let _ = (command, args, config); // Unused on Windows
        return Err(sysprims_core::SysprimsError::not_supported(
            "nohup", "windows",
        ));
    }
}

// ============================================================================
// Low-level APIs
// ============================================================================

/// Create a new session for the current process.
///
/// This is a thin wrapper around the `setsid(2)` system call.
///
/// # Safety
///
/// This must not be called if the current process is a process group leader,
/// as `setsid()` will fail with EPERM. Fork first if needed.
///
/// # Returns
///
/// The new session ID (which equals the process ID) on success.
#[cfg(unix)]
pub fn setsid() -> SysprimsResult<u32> {
    unix::setsid_impl()
}

/// Get the session ID for a process.
///
/// # Arguments
///
/// * `pid` - Process ID (0 for current process)
#[cfg(unix)]
pub fn getsid(pid: u32) -> SysprimsResult<u32> {
    unix::getsid_impl(pid)
}

/// Set the process group ID for a process.
///
/// # Arguments
///
/// * `pid` - Process ID (0 for current process)
/// * `pgid` - Process group ID (0 to use pid as pgid)
#[cfg(unix)]
pub fn setpgid(pid: u32, pgid: u32) -> SysprimsResult<()> {
    unix::setpgid_impl(pid, pgid)
}

/// Get the process group ID for a process.
///
/// # Arguments
///
/// * `pid` - Process ID (0 for current process)
#[cfg(unix)]
pub fn getpgid(pid: u32) -> SysprimsResult<u32> {
    unix::getpgid_impl(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setsid_config_defaults() {
        let config = SetsidConfig::default();
        assert!(!config.wait);
        assert!(!config.ctty);
    }

    #[test]
    fn nohup_config_defaults() {
        let config = NohupConfig::default();
        assert!(config.output_file.is_none());
        assert!(!config.wait);
    }
}
