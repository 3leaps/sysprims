//! Thread-local error handling for FFI.
//!
//! FFI functions return error codes and store detailed error information
//! in thread-local storage. Callers retrieve error details via:
//! - `sysprims_last_error_code()` - Get error code
//! - `sysprims_last_error()` - Get error message (must free with `sysprims_free_string`)
//! - `sysprims_clear_error()` - Clear error state

use std::cell::RefCell;
use std::ffi::CString;
use std::os::raw::c_char;

use sysprims_core::SysprimsError;

/// FFI error codes.
///
/// These map directly to `SysprimsError` variants. See `sysprims_core::error`
/// for the authoritative error taxonomy.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysprimsErrorCode {
    /// No error - operation succeeded.
    Ok = 0,
    /// Invalid argument provided.
    InvalidArgument = 1,
    /// Failed to spawn child process.
    SpawnFailed = 2,
    /// Operation timed out.
    Timeout = 3,
    /// Permission denied for operation.
    PermissionDenied = 4,
    /// Process or command not found.
    NotFound = 5,
    /// Operation not supported on this platform.
    NotSupported = 6,
    /// Failed to create process group or job object.
    GroupCreationFailed = 7,
    /// System-level error (errno/GetLastError).
    System = 8,
    /// Internal error (bug in sysprims).
    Internal = 99,
}

// ----------------------------------------------------------------------------
// C-friendly constants
// ----------------------------------------------------------------------------
//
// cbindgen's enum variant naming is not guaranteed to match the exact
// `SYSPRIMS_*` names used throughout our docs. We export these constants so the
// generated `sysprims.h` can provide stable, idiomatic C names.

/// No error - operation succeeded.
#[allow(dead_code)] // exported for cbindgen-generated C header
pub const SYSPRIMS_OK: SysprimsErrorCode = SysprimsErrorCode::Ok;
/// Invalid argument provided.
#[allow(dead_code)] // exported for cbindgen-generated C header
pub const SYSPRIMS_ERR_INVALID_ARGUMENT: SysprimsErrorCode = SysprimsErrorCode::InvalidArgument;
/// Failed to spawn child process.
#[allow(dead_code)] // exported for cbindgen-generated C header
pub const SYSPRIMS_ERR_SPAWN_FAILED: SysprimsErrorCode = SysprimsErrorCode::SpawnFailed;
/// Operation timed out.
#[allow(dead_code)] // exported for cbindgen-generated C header
pub const SYSPRIMS_ERR_TIMEOUT: SysprimsErrorCode = SysprimsErrorCode::Timeout;
/// Permission denied for operation.
#[allow(dead_code)] // exported for cbindgen-generated C header
pub const SYSPRIMS_ERR_PERMISSION_DENIED: SysprimsErrorCode = SysprimsErrorCode::PermissionDenied;
/// Process or command not found.
#[allow(dead_code)] // exported for cbindgen-generated C header
pub const SYSPRIMS_ERR_NOT_FOUND: SysprimsErrorCode = SysprimsErrorCode::NotFound;
/// Operation not supported on this platform.
#[allow(dead_code)] // exported for cbindgen-generated C header
pub const SYSPRIMS_ERR_NOT_SUPPORTED: SysprimsErrorCode = SysprimsErrorCode::NotSupported;
/// Failed to create process group or job object.
#[allow(dead_code)] // exported for cbindgen-generated C header
pub const SYSPRIMS_ERR_GROUP_CREATION_FAILED: SysprimsErrorCode =
    SysprimsErrorCode::GroupCreationFailed;
/// System-level error (errno/GetLastError).
#[allow(dead_code)] // exported for cbindgen-generated C header
pub const SYSPRIMS_ERR_SYSTEM: SysprimsErrorCode = SysprimsErrorCode::System;
/// Internal error (bug in sysprims).
#[allow(dead_code)] // exported for cbindgen-generated C header
pub const SYSPRIMS_ERR_INTERNAL: SysprimsErrorCode = SysprimsErrorCode::Internal;

impl From<&SysprimsError> for SysprimsErrorCode {
    fn from(err: &SysprimsError) -> Self {
        match err.error_code() {
            1 => SysprimsErrorCode::InvalidArgument,
            2 => SysprimsErrorCode::SpawnFailed,
            3 => SysprimsErrorCode::Timeout,
            4 => SysprimsErrorCode::PermissionDenied,
            5 => SysprimsErrorCode::NotFound,
            6 => SysprimsErrorCode::NotSupported,
            7 => SysprimsErrorCode::GroupCreationFailed,
            8 => SysprimsErrorCode::System,
            _ => SysprimsErrorCode::Internal,
        }
    }
}

/// Thread-local error state.
struct ErrorState {
    code: SysprimsErrorCode,
    message: Option<String>,
}

impl Default for ErrorState {
    fn default() -> Self {
        Self {
            code: SysprimsErrorCode::Ok,
            message: None,
        }
    }
}

thread_local! {
    static LAST_ERROR: RefCell<ErrorState> = RefCell::new(ErrorState::default());
}

/// Set the thread-local error state from a `SysprimsError`.
pub(crate) fn set_error(err: &SysprimsError) {
    LAST_ERROR.with(|state| {
        let mut state = state.borrow_mut();
        state.code = SysprimsErrorCode::from(err);
        state.message = Some(err.to_string());
    });
}

/// Clear the thread-local error state.
pub(crate) fn clear_error_state() {
    LAST_ERROR.with(|state| {
        let mut state = state.borrow_mut();
        state.code = SysprimsErrorCode::Ok;
        state.message = None;
    });
}

// ============================================================================
// FFI Exports
// ============================================================================

/// Get the error code from the last failed operation.
///
/// Returns `SYSPRIMS_OK` (0) if the last operation succeeded.
///
/// # Thread Safety
///
/// Error state is thread-local. Each thread has its own error state.
#[no_mangle]
pub extern "C" fn sysprims_last_error_code() -> SysprimsErrorCode {
    LAST_ERROR.with(|state| state.borrow().code)
}

/// Get the error message from the last failed operation.
///
/// Returns an owned string (must be freed with `sysprims_free_string()`).
///
/// After a successful operation (or after calling `sysprims_clear_error()`),
/// this returns an empty string (`""`).
///
/// # Safety
///
/// The returned pointer must be freed with `sysprims_free_string()`.
/// The caller owns the returned string.
///
/// # Thread Safety
///
/// Error state is thread-local. Each thread has its own error state.
#[no_mangle]
pub extern "C" fn sysprims_last_error() -> *mut c_char {
    LAST_ERROR.with(|state| {
        let state = state.borrow();
        let msg = state.message.as_deref().unwrap_or("");

        // CString::new can fail if the string contains null bytes.
        // In that case, return a sanitized version.
        match CString::new(msg) {
            Ok(c_str) => c_str.into_raw(),
            Err(_) => {
                let sanitized = msg.replace('\0', "?");
                CString::new(sanitized)
                    .map(|s| s.into_raw())
                    .unwrap_or(std::ptr::null_mut())
            }
        }
    })
}

/// Clear the error state for the current thread.
///
/// After calling this function, `sysprims_last_error_code()` will return
/// `SYSPRIMS_OK` and `sysprims_last_error()` will return an empty string.
#[no_mangle]
pub extern "C" fn sysprims_clear_error() {
    clear_error_state();
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn test_initial_state_is_ok() {
        clear_error_state();
        assert_eq!(sysprims_last_error_code(), SysprimsErrorCode::Ok);

        let msg_ptr = sysprims_last_error();
        assert!(!msg_ptr.is_null());
        let msg = unsafe { CStr::from_ptr(msg_ptr).to_str().unwrap() };
        assert!(msg.is_empty());
        unsafe { crate::sysprims_free_string(msg_ptr) };
    }

    #[test]
    fn test_set_and_get_error() {
        let err = SysprimsError::invalid_argument("test error message");
        set_error(&err);

        assert_eq!(
            sysprims_last_error_code(),
            SysprimsErrorCode::InvalidArgument
        );

        let msg_ptr = sysprims_last_error();
        assert!(!msg_ptr.is_null());

        // SAFETY: We just created this pointer and know it's valid
        let msg = unsafe { CStr::from_ptr(msg_ptr).to_str().unwrap() };
        assert!(msg.contains("test error message"));

        // SAFETY: Free the string we allocated
        unsafe { crate::sysprims_free_string(msg_ptr) };
    }

    #[test]
    fn test_clear_error() {
        let err = SysprimsError::not_found(1234);
        set_error(&err);
        assert_eq!(sysprims_last_error_code(), SysprimsErrorCode::NotFound);

        sysprims_clear_error();
        assert_eq!(sysprims_last_error_code(), SysprimsErrorCode::Ok);

        let msg_ptr = sysprims_last_error();
        assert!(!msg_ptr.is_null());
        let msg = unsafe { CStr::from_ptr(msg_ptr).to_str().unwrap() };
        assert!(msg.is_empty());
        unsafe { crate::sysprims_free_string(msg_ptr) };
    }

    #[test]
    fn test_error_code_mapping() {
        let test_cases = [
            (
                SysprimsError::invalid_argument(""),
                SysprimsErrorCode::InvalidArgument,
            ),
            (SysprimsError::Timeout, SysprimsErrorCode::Timeout),
            (
                SysprimsError::permission_denied(1, "op"),
                SysprimsErrorCode::PermissionDenied,
            ),
            (SysprimsError::not_found(1), SysprimsErrorCode::NotFound),
            (
                SysprimsError::not_supported("f", "p"),
                SysprimsErrorCode::NotSupported,
            ),
            (
                SysprimsError::group_creation_failed(""),
                SysprimsErrorCode::GroupCreationFailed,
            ),
            (SysprimsError::system("", 0), SysprimsErrorCode::System),
            (SysprimsError::internal(""), SysprimsErrorCode::Internal),
        ];

        for (err, expected_code) in test_cases {
            assert_eq!(
                SysprimsErrorCode::from(&err),
                expected_code,
                "Error {:?} should map to {:?}",
                err,
                expected_code
            );
        }
    }
}
