//! Signal dispatch FFI functions.
//!
//! Thin wrappers around `sysprims_signal` functions for C-ABI export.

use crate::error::{clear_error_state, set_error, SysprimsErrorCode};

/// Send a signal to a process.
///
/// On Unix, this calls `kill(pid, signal)`.
/// On Windows, SIGTERM and SIGKILL are mapped to `TerminateProcess`.
/// Other signals return `SYSPRIMS_ERR_NOT_SUPPORTED`.
///
/// # Arguments
///
/// * `pid` - Process ID (must be > 0 and <= i32::MAX)
/// * `signal` - Signal number (e.g., 15 for SIGTERM, 9 for SIGKILL)
///
/// # Returns
///
/// * `SYSPRIMS_OK` on success
/// * `SYSPRIMS_ERR_INVALID_ARGUMENT` if pid is 0 or > i32::MAX
/// * `SYSPRIMS_ERR_NOT_FOUND` if process doesn't exist
/// * `SYSPRIMS_ERR_PERMISSION_DENIED` if not permitted
/// * `SYSPRIMS_ERR_NOT_SUPPORTED` for unsupported signals on Windows
///
/// # Example (C)
///
/// ```c
/// SysprimsErrorCode err = sysprims_signal_send(pid, 15); // SIGTERM
/// if (err != SYSPRIMS_OK) {
///     char* msg = sysprims_last_error();
///     fprintf(stderr, "Error: %s\n", msg);
///     sysprims_free_string(msg);
/// }
/// ```
#[no_mangle]
pub extern "C" fn sysprims_signal_send(pid: u32, signal: i32) -> SysprimsErrorCode {
    clear_error_state();

    match sysprims_signal::kill(pid, signal) {
        Ok(()) => SysprimsErrorCode::Ok,
        Err(e) => {
            set_error(&e);
            SysprimsErrorCode::from(&e)
        }
    }
}

/// Send a signal to a process group.
///
/// On Unix, this calls `killpg(pgid, signal)`.
/// On Windows, this always returns `SYSPRIMS_ERR_NOT_SUPPORTED`.
///
/// # Arguments
///
/// * `pgid` - Process group ID (must be > 0 and <= i32::MAX)
/// * `signal` - Signal number
///
/// # Returns
///
/// * `SYSPRIMS_OK` on success
/// * `SYSPRIMS_ERR_INVALID_ARGUMENT` if pgid is invalid
/// * `SYSPRIMS_ERR_NOT_SUPPORTED` on Windows
///
/// # Example (C)
///
/// ```c
/// #ifdef _WIN32
/// // Not supported on Windows
/// #else
/// SysprimsErrorCode err = sysprims_signal_send_group(pgid, 15);
/// #endif
/// ```
#[no_mangle]
pub extern "C" fn sysprims_signal_send_group(pgid: u32, signal: i32) -> SysprimsErrorCode {
    clear_error_state();

    match sysprims_signal::killpg(pgid, signal) {
        Ok(()) => SysprimsErrorCode::Ok,
        Err(e) => {
            set_error(&e);
            SysprimsErrorCode::from(&e)
        }
    }
}

/// Send SIGTERM to a process.
///
/// Convenience wrapper for `sysprims_signal_send(pid, SIGTERM)`.
///
/// On Windows, this calls `TerminateProcess`.
///
/// # Arguments
///
/// * `pid` - Process ID
///
/// # Returns
///
/// * `SYSPRIMS_OK` on success
/// * Error code on failure (see `sysprims_signal_send`)
#[no_mangle]
pub extern "C" fn sysprims_terminate(pid: u32) -> SysprimsErrorCode {
    clear_error_state();

    match sysprims_signal::terminate(pid) {
        Ok(()) => SysprimsErrorCode::Ok,
        Err(e) => {
            set_error(&e);
            SysprimsErrorCode::from(&e)
        }
    }
}

/// Send SIGKILL to a process (force kill).
///
/// Convenience wrapper for `sysprims_signal_send(pid, SIGKILL)`.
///
/// On Unix, this sends SIGKILL which cannot be caught or ignored.
/// On Windows, this calls `TerminateProcess`.
///
/// # Arguments
///
/// * `pid` - Process ID
///
/// # Returns
///
/// * `SYSPRIMS_OK` on success
/// * Error code on failure (see `sysprims_signal_send`)
#[no_mangle]
pub extern "C" fn sysprims_force_kill(pid: u32) -> SysprimsErrorCode {
    clear_error_state();

    match sysprims_signal::force_kill(pid) {
        Ok(()) => SysprimsErrorCode::Ok,
        Err(e) => {
            set_error(&e);
            SysprimsErrorCode::from(&e)
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysprims_last_error_code;

    #[test]
    fn test_signal_send_rejects_pid_zero() {
        let result = sysprims_signal_send(0, 15);
        assert_eq!(result, SysprimsErrorCode::InvalidArgument);
        assert_eq!(
            sysprims_last_error_code(),
            SysprimsErrorCode::InvalidArgument
        );
    }

    #[test]
    fn test_signal_send_rejects_pid_overflow() {
        // u32::MAX would overflow to -1 as i32, triggering kill(-1, sig)
        let result = sysprims_signal_send(u32::MAX, 15);
        assert_eq!(result, SysprimsErrorCode::InvalidArgument);
    }

    #[test]
    fn test_terminate_rejects_pid_zero() {
        let result = sysprims_terminate(0);
        assert_eq!(result, SysprimsErrorCode::InvalidArgument);
    }

    #[test]
    fn test_force_kill_rejects_pid_zero() {
        let result = sysprims_force_kill(0);
        assert_eq!(result, SysprimsErrorCode::InvalidArgument);
    }

    #[test]
    fn test_signal_send_nonexistent_pid() {
        // Use a very high PID that shouldn't exist
        let result = sysprims_signal_send(99999999, 15);
        // Should be NotFound (process doesn't exist)
        assert!(
            result == SysprimsErrorCode::NotFound || result == SysprimsErrorCode::PermissionDenied,
            "Expected NotFound or PermissionDenied, got {:?}",
            result
        );
    }

    #[test]
    #[cfg(windows)]
    fn test_signal_send_group_not_supported_on_windows() {
        let result = sysprims_signal_send_group(1234, 15);
        assert_eq!(result, SysprimsErrorCode::NotSupported);
    }
}
