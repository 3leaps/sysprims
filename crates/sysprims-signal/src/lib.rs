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

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

// Re-export rsfulmen signal constants and helpers for convenience.
//
// This crate is explicitly about signals, so re-exporting these at the crate
// root is intentional and ergonomic.
pub use rsfulmen::foundry::signals::*;

/// Send a signal to a process.
///
/// # Errors
///
/// Returns [`SysprimsError::InvalidArgument`] if:
/// - `pid == 0`: Would signal the caller's process group (use [`killpg`] explicitly)
/// - `pid > MAX_SAFE_PID`: Would overflow to negative, triggering POSIX broadcast
///
/// See [ADR-0011](docs/architecture/adr/0011-pid-validation-safety.md) for rationale.
pub fn kill(pid: u32, signal: i32) -> SysprimsResult<()> {
    validate_pid(pid, "pid")?;

    #[cfg(unix)]
    return unix::kill_impl(pid, signal);

    #[cfg(windows)]
    return windows::kill_impl(pid, signal);
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
/// See [ADR-0011](docs/architecture/adr/0011-pid-validation-safety.md) for rationale.
pub fn killpg(pgid: u32, signal: i32) -> SysprimsResult<()> {
    validate_pid(pgid, "pgid")?;

    #[cfg(unix)]
    return unix::killpg_impl(pgid, signal);

    #[cfg(windows)]
    return Err(SysprimsError::not_supported("killpg", "windows"));
}

/// Convenience wrapper: send `SIGTERM` (or Windows terminate).
pub fn terminate(pid: u32) -> SysprimsResult<()> {
    kill(pid, SIGTERM)
}

/// Convenience wrapper: send `SIGKILL` (or Windows terminate).
pub fn force_kill(pid: u32) -> SysprimsResult<()> {
    kill(pid, SIGKILL)
}

/// Convenience wrapper: send `SIGTERM` to a process group.
pub fn terminate_group(pgid: u32) -> SysprimsResult<()> {
    killpg(pgid, SIGTERM)
}

/// Convenience wrapper: send `SIGKILL` to a process group.
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
        assert!(!matches!(result, Err(SysprimsError::InvalidArgument { .. })));
    }

    #[test]
    fn max_safe_pid_is_i32_max() {
        assert_eq!(MAX_SAFE_PID, i32::MAX as u32);
        assert_eq!(MAX_SAFE_PID, 2147483647);
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
    // Platform-Specific Tests
    // ========================================================================

    #[test]
    #[cfg(windows)]
    fn killpg_is_not_supported_on_windows() {
        let err = killpg(1234, SIGTERM).unwrap_err();
        assert!(matches!(err, SysprimsError::NotSupported { .. }));
    }
}
