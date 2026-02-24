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

/// Validate that a PID is safe to use with POSIX signal functions.
///
/// Returns an error if:
/// - `pid == 0`: Would signal the caller's process group
/// - `pid > i32::MAX`: Would overflow to negative, triggering broadcast semantics
fn validate_pid(pid: u32, param_name: &str) -> SysprimsResult<()> {
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
        validate_pid(pid, param_name)?;
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
    validate_pid(pid, "pid")?;

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
    let signal = resolve_signal_number(signal_name).ok_or_else(|| {
        SysprimsError::invalid_argument(format!("unknown signal name: {signal_name}"))
    })?;
    kill(pid, signal)
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
    validate_pid(pgid, "pgid")?;

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
}
