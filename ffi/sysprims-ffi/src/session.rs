//! Session and process-group introspection FFI functions.
//!
//! This module provides small, non-signal-sending primitives that are useful for
//! "self" / runtime introspection scenarios (e.g. diagnostics, supervision).

use std::os::raw::c_uint;

use crate::error::{clear_error_state, set_error, SysprimsErrorCode};
use sysprims_core::SysprimsError;

/// Get the current process group ID (PGID).
///
/// On Unix, this calls `getpgid(0)`.
/// On Windows, this returns `SYSPRIMS_ERR_NOT_SUPPORTED`.
///
/// # Safety
///
/// - `pgid_out` must be a valid pointer to a `u32`.
#[no_mangle]
pub unsafe extern "C" fn sysprims_self_getpgid(pgid_out: *mut c_uint) -> SysprimsErrorCode {
    clear_error_state();

    if pgid_out.is_null() {
        let err = SysprimsError::invalid_argument("pgid_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    #[cfg(unix)]
    {
        match sysprims_session::getpgid(0) {
            Ok(pgid) => {
                *pgid_out = pgid;
                SysprimsErrorCode::Ok
            }
            Err(e) => {
                set_error(&e);
                SysprimsErrorCode::from(&e)
            }
        }
    }

    #[cfg(windows)]
    {
        let err = SysprimsError::not_supported("getpgid", "windows");
        set_error(&err);
        SysprimsErrorCode::NotSupported
    }
}

/// Get the current session ID (SID).
///
/// On Unix, this calls `getsid(0)`.
/// On Windows, this returns `SYSPRIMS_ERR_NOT_SUPPORTED`.
///
/// # Safety
///
/// - `sid_out` must be a valid pointer to a `u32`.
#[no_mangle]
pub unsafe extern "C" fn sysprims_self_getsid(sid_out: *mut c_uint) -> SysprimsErrorCode {
    clear_error_state();

    if sid_out.is_null() {
        let err = SysprimsError::invalid_argument("sid_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    #[cfg(unix)]
    {
        match sysprims_session::getsid(0) {
            Ok(sid) => {
                *sid_out = sid;
                SysprimsErrorCode::Ok
            }
            Err(e) => {
                set_error(&e);
                SysprimsErrorCode::from(&e)
            }
        }
    }

    #[cfg(windows)]
    {
        let err = SysprimsError::not_supported("getsid", "windows");
        set_error(&err);
        SysprimsErrorCode::NotSupported
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_self_getpgid_null_out() {
        let code = unsafe { sysprims_self_getpgid(std::ptr::null_mut()) };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
    }

    #[test]
    fn test_self_getsid_null_out() {
        let code = unsafe { sysprims_self_getsid(std::ptr::null_mut()) };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
    }

    #[test]
    #[cfg(unix)]
    fn test_self_getpgid_ok() {
        let mut pgid: c_uint = 0;
        let code = unsafe { sysprims_self_getpgid(&mut pgid) };
        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(pgid > 0);
    }

    #[test]
    #[cfg(unix)]
    fn test_self_getsid_ok() {
        let mut sid: c_uint = 0;
        let code = unsafe { sysprims_self_getsid(&mut sid) };
        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(sid > 0);
    }

    #[test]
    #[cfg(windows)]
    fn test_self_session_ids_not_supported() {
        let mut pgid: c_uint = 0;
        let code = unsafe { sysprims_self_getpgid(&mut pgid) };
        assert_eq!(code, SysprimsErrorCode::NotSupported);

        let mut sid: c_uint = 0;
        let code = unsafe { sysprims_self_getsid(&mut sid) };
        assert_eq!(code, SysprimsErrorCode::NotSupported);
    }
}
