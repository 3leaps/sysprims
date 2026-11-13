//! sysprims-signal: Cross-platform signal dispatch.
//!
//! This crate provides:
//! - Process signal dispatch by PID ([`kill`])
//! - Process group signal dispatch by PGID ([`killpg`], Unix-only)
//! - Convenience wrappers ([`terminate`], [`force_kill`], etc.)
//!
//! Errors use the canonical [`sysprims_core::SysprimsError`] type.
//!
//! # Safety
//!
//! This crate validates PIDs to prevent dangerous POSIX signal semantics:
//!
//! - **PID 0 rejected**: `kill(0, sig)` would signal the caller's process group
//! - **PID > i32::MAX rejected**: Large u32 values wrap to negative i32, and
//!   `kill(-1, sig)` signals ALL processes the caller can reach
//!
//! See `docs/safety/signal-dispatch.md` for full details on POSIX signal
//! semantics and why these restrictions exist.

use sysprims_core::{SysprimsError, SysprimsResult};

/// Maximum valid PID value.
///
/// PIDs above this value would overflow to negative when cast to `pid_t` (i32),
/// which has special POSIX semantics:
/// - `kill(-1, sig)` = signal ALL processes the caller can reach
/// - `kill(-pgid, sig)` = signal process group `pgid`
///
/// We reject these at the API boundary to prevent accidental mass termination.
pub const MAX_SAFE_PID: u32 = i32::MAX as u32;

pub(crate) const TERMINATE_SIGNAL: i32 = SIGTERM;
pub(crate) const FORCE_KILL_SIGNAL: i32 = SIGKILL;

pub mod handle;
pub use handle::{
    open_process_handle, open_process_handle_verified, with_forced_checked_numeric_for_test,
    ProcessHandle, SignalBinding,
};

/// Validate that a parameter (`pid` or `pgid`) is safe to use with POSIX signal functions.
///
/// Returns an error if:
/// - `pid == 0`: Would signal the caller's process group
/// - `pid > i32::MAX`: Would overflow to negative, triggering broadcast semantics
fn validate_pid_param(pid: u32, param_name: &str) -> SysprimsResult<()> {
    if pid == 0 {
        return Err(SysprimsError::invalid_argument(format!(
            "{param_name} must be > 0"
        )));
    }
    if pid > MAX_SAFE_PID {
        return Err(SysprimsError::invalid_argument(format!(
            "{param_name} {} exceeds maximum safe value {}; \
             larger values overflow to negative PIDs with dangerous POSIX semantics \
             (see docs/safety/signal-dispatch.md)",
            pid, MAX_SAFE_PID
        )));
    }
    Ok(())
}

/// Validate that a PID is safe (`1..=MAX_SAFE_PID`) per ADR-0011.
pub fn validate_pid(pid: u32) -> SysprimsResult<()> {
    validate_pid_param(pid, "pid")
}

/// Validate that a PGID is safe (`1..=MAX_SAFE_PID`) per ADR-0011.
pub fn validate_pgid(pgid: u32) -> SysprimsResult<()> {
    validate_pid_param(pgid, "pgid")
}

#[derive(Debug)]
pub struct BatchKillFailure {
    pub pid: u32,
    pub error: SysprimsError,
}

#[derive(Debug, Default)]
pub struct BatchKillResult {
    pub succeeded: Vec<u32>,
    pub failed: Vec<BatchKillFailure>,
}

fn validate_pid_list(pids: &[u32], param_name: &str) -> SysprimsResult<()> {
    if pids.is_empty() {
        return Err(SysprimsError::invalid_argument(format!(
            "{param_name} must not be empty"
        )));
    }

    for &pid in pids {
        validate_pid_param(pid, param_name)?;
    }

    Ok(())
}

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

// Re-export rsfulmen signal constants and helpers for convenience.
//
// This crate is explicitly about signals, so re-exporting these at the crate
// root is intentional and ergonomic.
pub use rsfulmen::foundry::signals::*;

/// User-defined signal 1 (`30` on macOS, `10` on Linux and other platforms).
#[cfg(target_os = "macos")]
pub const SIGUSR1: i32 = 30;
/// User-defined signal 1 (`30` on macOS, `10` on Linux and other platforms).
#[cfg(not(target_os = "macos"))]
pub const SIGUSR1: i32 = 10;

/// User-defined signal 2 (`31` on macOS, `12` on Linux and other platforms).
#[cfg(target_os = "macos")]
pub const SIGUSR2: i32 = 31;
/// User-defined signal 2 (`31` on macOS, `12` on Linux and other platforms).
#[cfg(not(target_os = "macos"))]
pub const SIGUSR2: i32 = 12;

fn resolve_signal_number(signal_name: &str) -> Option<i32> {
    let trimmed = signal_name.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(num) = get_signal_number(trimmed) {
        return Some(num);
    }

    let upper = trimmed.to_ascii_uppercase();
    if let Some(num) = get_signal_number(&upper) {
        return Some(num);
    }

    if !upper.starts_with("SIG") {
        let with_sig = format!("SIG{upper}");
        if let Some(num) = get_signal_number(&with_sig) {
            return Some(num);
        }
    }

    let lower = trimmed.to_ascii_lowercase();
    lookup_signal_by_id(&lower).and_then(|signal| get_signal_number(&signal.name))
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    let mut p_idx = 0;
    let mut t_idx = 0;
    let mut star_idx: Option<usize> = None;
    let mut match_idx = 0;

    while t_idx < text.len() {
        if p_idx < pattern.len() && (pattern[p_idx] == b'?' || pattern[p_idx] == text[t_idx]) {
            p_idx += 1;
            t_idx += 1;
            continue;
        }
        if p_idx < pattern.len() && pattern[p_idx] == b'*' {
            star_idx = Some(p_idx);
            match_idx = t_idx;
            p_idx += 1;
            continue;
        }
        if let Some(star) = star_idx {
            p_idx = star + 1;
            match_idx += 1;
            t_idx = match_idx;
            continue;
        }
        return false;
    }

    while p_idx < pattern.len() && pattern[p_idx] == b'*' {
        p_idx += 1;
    }

    p_idx == pattern.len()
}

/// Send a signal to a process.
///
/// # Errors
///
/// Returns [`SysprimsError::InvalidArgument`] if:
/// - `pid == 0`: Would signal the caller's process group (use [`killpg`] explicitly)
/// - `pid > MAX_SAFE_PID`: Would overflow to negative, triggering POSIX broadcast
///
/// See [ADR-0011](https://github.com/3leaps/sysprims/blob/main/docs/decisions/ADR-0011-pid-validation-safety.md)
/// for rationale.
/// Prefer this over shelling out to `kill -TERM <pid>` and parsing command failures.
///
/// # Examples
///
/// ```rust,no_run
/// use sysprims_signal::SIGTERM;
///
/// // Replaces: kill -TERM 4242
/// sysprims_signal::kill(4242, SIGTERM).ok();
/// ```
pub fn kill(pid: u32, signal: i32) -> SysprimsResult<()> {
    validate_pid(pid)?;

    #[cfg(unix)]
    return unix::kill_impl(pid, signal);

    #[cfg(windows)]
    return windows::kill_impl(pid, signal);
}

/// Send a signal to multiple processes.
///
/// PID validation happens for the entire slice before any signals are sent.
/// Individual send failures are collected and returned in the aggregate result.
///
/// # Errors
///
/// Returns [`SysprimsError::InvalidArgument`] if:
/// - `pids` is empty
/// - any PID in `pids` is invalid (e.g. 0 or > [`MAX_SAFE_PID`])
///
/// # Examples
///
/// ```rust,no_run
/// use sysprims_signal::SIGTERM;
///
/// // Replaces: kill -TERM 1234 && kill -TERM 5678
/// let result = sysprims_signal::kill_many(&[1234, 5678], SIGTERM).unwrap();
/// println!("sent to {}", result.succeeded.len());
/// ```
pub fn kill_many(pids: &[u32], signal: i32) -> SysprimsResult<BatchKillResult> {
    validate_pid_list(pids, "pids")?;

    let mut result = BatchKillResult::default();
    for &pid in pids {
        match kill(pid, signal) {
            Ok(()) => result.succeeded.push(pid),
            Err(error) => result.failed.push(BatchKillFailure { pid, error }),
        }
    }

    Ok(result)
}

/// Convenience wrapper: send `SIGTERM` to multiple processes.
///
/// # Examples
///
/// ```rust,no_run
/// // Replaces: xargs -n1 kill -TERM
/// let _ = sysprims_signal::terminate_many(&[1234, 5678]);
/// ```
pub fn terminate_many(pids: &[u32]) -> SysprimsResult<BatchKillResult> {
    kill_many(pids, SIGTERM)
}

/// Convenience wrapper: send `SIGKILL` to multiple processes.
///
/// # Examples
///
/// ```rust,no_run
/// // Replaces: xargs -n1 kill -KILL
/// let _ = sysprims_signal::force_kill_many(&[1234, 5678]);
/// ```
pub fn force_kill_many(pids: &[u32]) -> SysprimsResult<BatchKillResult> {
    kill_many(pids, SIGKILL)
}

/// Resolve a signal number by name (`SIGTERM`, `TERM`, `sigterm`, or short ID `term`).
pub fn resolve_signal_name(signal_name: &str) -> SysprimsResult<i32> {
    resolve_signal_number(signal_name).ok_or_else(|| {
        SysprimsError::invalid_argument(format!("unknown signal name: {signal_name}"))
    })
}

/// Send a signal to a process, resolving the signal number by name.
///
/// This uses rsfulmen's catalog plus a small normalization layer:
/// - Accepts `SIGTERM`, `TERM`, or `sigterm`
/// - Accepts short IDs like `term` or `int`
///
/// # Examples
///
/// ```rust,no_run
/// // Replaces: kill -TERM 1234
/// sysprims_signal::kill_by_name(1234, "TERM").ok();
/// ```
pub fn kill_by_name(pid: u32, signal_name: &str) -> SysprimsResult<()> {
    let signal = resolve_signal_name(signal_name)?;
    kill(pid, signal)
}

/// Send a signal to a process group, resolving the signal number by name.
pub fn kill_group_by_name(pgid: u32, signal_name: &str) -> SysprimsResult<()> {
    let signal = resolve_signal_name(signal_name)?;
    killpg(pgid, signal)
}

/// Send a signal to a process group.
///
/// On Windows, this always returns `NotSupported`.
///
/// # Errors
///
/// Returns [`SysprimsError::InvalidArgument`] if:
/// - `pgid == 0`: Would signal the caller's process group
/// - `pgid > MAX_SAFE_PID`: Would overflow to negative
///
/// See [ADR-0011](https://github.com/3leaps/sysprims/blob/main/docs/decisions/ADR-0011-pid-validation-safety.md)
/// for rationale.
///
/// # Examples
///
/// ```rust,no_run
/// use sysprims_signal::SIGTERM;
///
/// // Replaces: kill -TERM -- -4242
/// sysprims_signal::killpg(4242, SIGTERM).ok();
/// ```
pub fn killpg(pgid: u32, signal: i32) -> SysprimsResult<()> {
    validate_pgid(pgid)?;

    #[cfg(unix)]
    return unix::killpg_impl(pgid, signal);

    #[cfg(windows)]
    {
        let _ = signal; // Unused on Windows
        return Err(SysprimsError::not_supported("killpg", "windows"));
    }
}

/// Return signal names that match a simple glob pattern.
///
/// Supports `*` (any sequence) and `?` (single char). Matching is
/// ASCII case-insensitive and checks both the signal name and short ID.
///
/// # Examples
///
/// ```rust
/// // Replaces: kill -l | grep -i term
/// let matches = sysprims_signal::match_signal_names("*term*");
/// assert!(matches.iter().any(|name| *name == "SIGTERM"));
/// ```
pub fn match_signal_names(pattern: &str) -> Vec<&'static str> {
    let trimmed = pattern.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let pattern = trimmed.to_ascii_lowercase();
    let mut matches = Vec::new();

    for signal in list_signals() {
        let name = signal.name.to_ascii_lowercase();
        let id = signal.id.to_ascii_lowercase();
        if (glob_match(&pattern, &name) || glob_match(&pattern, &id))
            && !matches.iter().any(|&item| item == signal.name)
        {
            matches.push(signal.name.as_str());
        }
    }

    matches
}

/// Convenience wrapper: send `SIGTERM` (or Windows terminate).
///
/// # Examples
///
/// ```rust,no_run
/// // Replaces: kill -TERM 1234
/// sysprims_signal::terminate(1234).ok();
/// ```
pub fn terminate(pid: u32) -> SysprimsResult<()> {
    kill(pid, SIGTERM)
}

/// Convenience wrapper: send `SIGKILL` (or Windows terminate).
///
/// # Examples
///
/// ```rust,no_run
/// // Replaces: kill -KILL 1234
/// sysprims_signal::force_kill(1234).ok();
/// ```
pub fn force_kill(pid: u32) -> SysprimsResult<()> {
    kill(pid, SIGKILL)
}

/// Convenience wrapper: send `SIGTERM` to a process group.
///
/// # Examples
///
/// ```rust,no_run
/// // Replaces: kill -TERM -- -4242
/// sysprims_signal::terminate_group(4242).ok();
/// ```
pub fn terminate_group(pgid: u32) -> SysprimsResult<()> {
    killpg(pgid, SIGTERM)
}

/// Convenience wrapper: send `SIGKILL` to a process group.
///
/// # Examples
///
/// ```rust,no_run
/// // Replaces: kill -KILL -- -4242
/// sysprims_signal::force_kill_group(4242).ok();
/// ```
pub fn force_kill_group(pgid: u32) -> SysprimsResult<()> {
    killpg(pgid, SIGKILL)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // PID Validation Tests (ADR-0011)
    // ========================================================================

    #[test]
    fn kill_rejects_pid_zero() {
        let err = kill(0, SIGTERM).unwrap_err();
        assert!(matches!(err, SysprimsError::InvalidArgument { .. }));
        assert!(err.to_string().contains("must be > 0"));
    }

    #[test]
    fn killpg_rejects_pgid_zero() {
        let err = killpg(0, SIGTERM).unwrap_err();
        assert!(matches!(err, SysprimsError::InvalidArgument { .. }));
        assert!(err.to_string().contains("must be > 0"));
    }

    #[test]
    fn kill_rejects_pid_exceeding_max_safe() {
        // This is the critical safety test per ADR-0011.
        //
        // u32::MAX (4294967295) cast to i32 becomes -1.
        // kill(-1, sig) is POSIX for "signal ALL processes you can reach".
        // This would terminate Finder, Terminal, and everything else!
        let err = kill(u32::MAX, SIGTERM).unwrap_err();
        assert!(matches!(err, SysprimsError::InvalidArgument { .. }));
        assert!(err.to_string().contains("exceeds maximum safe value"));
    }

    #[test]
    fn killpg_rejects_pgid_exceeding_max_safe() {
        let err = killpg(u32::MAX, SIGTERM).unwrap_err();
        assert!(matches!(err, SysprimsError::InvalidArgument { .. }));
        assert!(err.to_string().contains("exceeds maximum safe value"));
    }

    #[test]
    fn kill_rejects_pid_at_boundary() {
        // i32::MAX + 1 is the first unsafe value
        let first_unsafe = (i32::MAX as u32) + 1;
        let err = kill(first_unsafe, SIGTERM).unwrap_err();
        assert!(matches!(err, SysprimsError::InvalidArgument { .. }));
    }

    #[test]
    fn kill_accepts_pid_at_max_safe() {
        // i32::MAX is the last safe value (will return NotFound, not validation error)
        let result = kill(MAX_SAFE_PID, SIGTERM);
        // Should NOT be InvalidArgument - should be NotFound or PermissionDenied
        assert!(!matches!(
            result,
            Err(SysprimsError::InvalidArgument { .. })
        ));
    }

    #[test]
    fn max_safe_pid_is_i32_max() {
        assert_eq!(MAX_SAFE_PID, i32::MAX as u32);
        assert_eq!(MAX_SAFE_PID, 2147483647);
    }

    #[test]
    fn kill_many_rejects_empty_pid_list() {
        let err = kill_many(&[], SIGTERM).unwrap_err();
        assert!(matches!(err, SysprimsError::InvalidArgument { .. }));
        assert!(err.to_string().contains("must not be empty"));
    }

    // ========================================================================
    // rsfulmen Integration Tests
    // ========================================================================

    #[test]
    fn rsfulmen_constants_available() {
        assert_eq!(SIGTERM, 15);
        assert_eq!(SIGKILL, 9);
    }

    // ========================================================================
    // Signal Name Resolution Tests
    // ========================================================================

    #[test]
    fn resolve_signal_number_accepts_common_variants() {
        assert_eq!(resolve_signal_number("SIGTERM"), Some(SIGTERM));
        assert_eq!(resolve_signal_number("term"), Some(SIGTERM));
        assert_eq!(resolve_signal_number("TERM"), Some(SIGTERM));
        assert_eq!(resolve_signal_number(" sigterm "), Some(SIGTERM));
    }

    #[test]
    fn resolve_signal_number_accepts_short_id() {
        assert_eq!(resolve_signal_number("int"), Some(SIGINT));
    }

    #[test]
    fn match_signal_names_glob_matches_names() {
        let matches = match_signal_names("SIGT*");
        assert!(matches.contains(&"SIGTERM"));
    }

    // ========================================================================
    // Platform-Specific Tests
    // ========================================================================

    #[test]
    #[cfg(windows)]
    fn killpg_is_not_supported_on_windows() {
        let err = killpg(1234, SIGTERM).unwrap_err();
        assert!(matches!(err, SysprimsError::NotSupported { .. }));
    }

    // ========================================================================
    // ProcessHandle Tests
    // ========================================================================

    fn spawn_sleep_child() -> std::process::Child {
        #[cfg(unix)]
        {
            std::process::Command::new("sleep")
                .arg("30")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn sleep child")
        }
        #[cfg(windows)]
        {
            std::process::Command::new("ping")
                .args(["-n", "30", "127.0.0.1"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn ping child")
        }
    }

    #[test]
    fn process_handle_rejects_invalid_pids() {
        let err_zero = open_process_handle(0).unwrap_err();
        assert!(matches!(err_zero, SysprimsError::InvalidArgument { .. }));

        let err_max = open_process_handle(u32::MAX).unwrap_err();
        assert!(matches!(err_max, SysprimsError::InvalidArgument { .. }));

        let err_boundary = open_process_handle(MAX_SAFE_PID + 1).unwrap_err();
        assert!(matches!(
            err_boundary,
            SysprimsError::InvalidArgument { .. }
        ));
    }

    #[test]
    fn process_handle_verified_rejects_empty_and_mismatched_start_identity() {
        let self_pid = std::process::id();
        let err_empty = open_process_handle_verified(self_pid, "   ").unwrap_err();
        assert!(matches!(err_empty, SysprimsError::InvalidArgument { .. }));

        let handle = open_process_handle(self_pid).expect("open self handle");
        assert_eq!(handle.pid(), self_pid);
        assert!(!handle.start_identity().is_empty());

        let verified = open_process_handle_verified(self_pid, handle.start_identity())
            .expect("open verified self handle");
        assert_eq!(verified.start_identity(), handle.start_identity());

        let err_mismatch = open_process_handle_verified(self_pid, "0000000000.999999").unwrap_err();
        assert!(matches!(err_mismatch, SysprimsError::NotFound { .. }));
    }

    #[test]
    fn process_handle_lifecycle_and_latched_exit() {
        let mut child = spawn_sleep_child();
        let pid = child.id();

        let handle = open_process_handle(pid).expect("open child handle");
        assert_eq!(handle.pid(), pid);
        assert!(!handle.start_identity().is_empty());
        assert!(!handle.exited().expect("exited check on live child"));

        #[cfg(target_os = "macos")]
        assert_eq!(handle.binding(), SignalBinding::CheckedNumeric);
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        assert_eq!(handle.binding(), SignalBinding::Bound);

        let binding = handle.force_kill().expect("force_kill bound child");
        assert_eq!(binding, handle.binding());

        let _ = child.wait();

        // Wait briefly for OS exit observation if needed, then verify latched true.
        let mut observed_exit = false;
        for _ in 0..50 {
            if handle.exited().expect("exited check after wait") {
                observed_exit = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            observed_exit,
            "expected exited() to become true after child exit"
        );
        assert!(handle.exited().expect("latched exited check"));

        // Second signal after exit must refuse with NotFound without signaling numeric PID.
        let second = handle.force_kill().unwrap_err();
        assert!(matches!(second, SysprimsError::NotFound { .. }));

        // Opening a new handle on the reaped PID (or with the old start_identity) must fail.
        let reopen = open_process_handle_verified(pid, handle.start_identity()).unwrap_err();
        assert!(matches!(reopen, SysprimsError::NotFound { .. }));

        handle.close();
        assert!(handle.is_closed());
        handle.close(); // idempotent
        let err_closed = handle.exited().unwrap_err();
        assert!(matches!(err_closed, SysprimsError::InvalidArgument { .. }));
    }

    #[test]
    fn process_handle_detects_unreaped_zombie_exit_and_refuses_signal() {
        let mut child = spawn_sleep_child();
        let pid = child.id();
        let handle = open_process_handle(pid).expect("open child handle");

        let _ = child.kill();
        // Deliberately do NOT wait() yet so child becomes an unreaped zombie on Unix.
        let mut observed_exit = false;
        for _ in 0..100 {
            if handle.exited().expect("exited poll on zombie") {
                observed_exit = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(observed_exit, "expected exited() on unreaped zombie");
        let sig_err = handle.terminate().unwrap_err();
        assert!(matches!(sig_err, SysprimsError::NotFound { .. }));

        // Opening a fresh ProcessHandle on an unreaped zombie must also return NotFound.
        let zombie_open = open_process_handle(pid).unwrap_err();
        assert!(matches!(zombie_open, SysprimsError::NotFound { .. }));

        let _ = child.wait();
    }

    #[test]
    fn process_handle_error_messages_do_not_leak_paths_or_cmdline() {
        let err = open_process_handle(MAX_SAFE_PID).unwrap_err();
        let msg = err.to_string();
        assert!(!msg.contains("/Users/"));
        assert!(!msg.contains("/home/"));
        assert!(!msg.contains("sleep"));
    }

    #[test]
    #[cfg(unix)]
    fn process_handle_fd_is_cloexec_and_not_inherited_by_exec_children() {
        let baseline = std::process::Command::new("sh")
            .args([
                "-c",
                "ls /dev/fd 2>/dev/null || ls /proc/self/fd 2>/dev/null",
            ])
            .stdin(std::process::Stdio::null())
            .output()
            .expect("list baseline child fds");
        assert!(baseline.status.success());
        let baseline_count = String::from_utf8_lossy(&baseline.stdout)
            .split_whitespace()
            .filter_map(|s| s.parse::<i32>().ok())
            .count();

        let mut handles = Vec::new();
        for _ in 0..8 {
            let h = open_process_handle(std::process::id()).expect("open self handle");
            assert!(!h.exited().expect("self is alive"));
            handles.push(h);
        }

        let after = std::process::Command::new("sh")
            .args([
                "-c",
                "ls /dev/fd 2>/dev/null || ls /proc/self/fd 2>/dev/null",
            ])
            .stdin(std::process::Stdio::null())
            .output()
            .expect("list child fds with 8 open ProcessHandles");
        assert!(after.status.success());
        let after_count = String::from_utf8_lossy(&after.stdout)
            .split_whitespace()
            .filter_map(|s| s.parse::<i32>().ok())
            .count();

        // A non-CLOEXEC kqueue/pidfd opened in the parent would increase the child's
        // open FD count by 8. With FD_CLOEXEC set, the child FD count is unchanged.
        assert_eq!(
            after_count, baseline_count,
            "ProcessHandle FDs must not be inherited across exec"
        );
        drop(handles);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_checked_numeric_fallback_works_when_forced() {
        with_forced_checked_numeric_for_test(|| {
            let mut child = spawn_sleep_child();
            let pid = child.id();
            let handle = open_process_handle(pid).expect("open fallback handle");
            assert_eq!(handle.binding(), SignalBinding::CheckedNumeric);
            assert!(!handle.exited().expect("exited check"));
            let b = handle.force_kill().expect("force_kill via fallback");
            assert_eq!(b, SignalBinding::CheckedNumeric);
            let _ = child.wait();
            assert!(handle.exited().expect("exited after wait"));
        });
    }
}
