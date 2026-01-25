//! sysprims-ffi: C-ABI exports for sysprims
//!
//! This crate provides a stable C-ABI interface for sysprims functionality.
//!
//! # Memory Management
//!
//! All strings returned by this API must be freed with `sysprims_free_string()`.
//! Do not use `free()` or any other deallocator.
//!
//! # Error Handling
//!
//! Functions return `SysprimsErrorCode`. On error, detailed information is
//! available via:
//! - `sysprims_last_error_code()` - Get error code
//! - `sysprims_last_error()` - Get error message
//! - `sysprims_clear_error()` - Clear error state
//!
//! Error state is thread-local.
//!
//! # ABI Version
//!
//! Check `sysprims_abi_version()` for ABI compatibility. The ABI version
//! increments when breaking changes are made to the FFI interface.

use std::ffi::CString;
use std::os::raw::c_char;

use sysprims_core::get_platform;

// Modules
mod error;
mod proc;
mod session;
mod signal;
mod spawn;
mod timeout;

// Re-export error types at crate root
pub use error::SysprimsErrorCode;

// Re-export FFI functions from submodules
pub use error::{sysprims_clear_error, sysprims_last_error, sysprims_last_error_code};
pub use proc::{
    sysprims_proc_get, sysprims_proc_list, sysprims_proc_listening_ports, sysprims_proc_wait_pid,
};
pub use session::{sysprims_self_getpgid, sysprims_self_getsid};
pub use signal::{
    sysprims_force_kill, sysprims_signal_send, sysprims_signal_send_group, sysprims_terminate,
};
pub use spawn::sysprims_spawn_in_group;
pub use timeout::{
    sysprims_terminate_tree, sysprims_timeout_run, SysprimsGroupingMode, SysprimsTimeoutConfig,
};

// ============================================================================
// Version Constants
// ============================================================================

/// Library version string (e.g., "0.1.0").
///
/// This matches the version in Cargo.toml.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// ABI version number.
///
/// Incremented when breaking changes are made to the FFI interface:
/// - Function signatures change
/// - Struct layouts change
/// - Enum values change
/// - Error code semantics change
///
/// Minor additions (new functions) do not increment the ABI version.
const ABI_VERSION: u32 = 1;

// ============================================================================
// Version Functions
// ============================================================================

/// Get the library version string.
///
/// Returns a static string like "0.1.0". The pointer is valid for the
/// lifetime of the library and should NOT be freed.
///
/// # Example (C)
///
/// ```c
/// const char* ver = sysprims_version();
/// printf("sysprims version: %s\n", ver);
/// // Do NOT call sysprims_free_string(ver)
/// ```
#[no_mangle]
pub extern "C" fn sysprims_version() -> *const c_char {
    // Use a static CString to ensure the pointer remains valid.
    // This is safe because VERSION is a compile-time constant.
    static VERSION_CSTR: std::sync::OnceLock<CString> = std::sync::OnceLock::new();
    VERSION_CSTR
        .get_or_init(|| CString::new(VERSION).expect("VERSION should not contain null bytes"))
        .as_ptr()
}

/// Get the ABI version number.
///
/// Use this to check compatibility between the library and bindings.
/// If the ABI version differs from what your bindings expect, the
/// bindings may not work correctly.
///
/// # Example (C)
///
/// ```c
/// uint32_t abi = sysprims_abi_version();
/// if (abi != EXPECTED_ABI_VERSION) {
///     fprintf(stderr, "ABI mismatch: expected %u, got %u\n",
///             EXPECTED_ABI_VERSION, abi);
/// }
/// ```
#[no_mangle]
pub extern "C" fn sysprims_abi_version() -> u32 {
    ABI_VERSION
}

// ============================================================================
// Platform Detection
// ============================================================================

/// Returns the current platform name as a C string.
///
/// Returns one of: "linux", "macos", "windows", "freebsd", etc.
///
/// # Safety
///
/// The returned pointer must be freed with `sysprims_free_string()`.
/// Do not use `free()` or any other deallocator.
#[no_mangle]
pub extern "C" fn sysprims_get_platform() -> *mut c_char {
    let platform = get_platform();
    let c_platform = CString::new(platform).unwrap();
    c_platform.into_raw()
}

// ============================================================================
// Memory Management
// ============================================================================

/// Frees a string allocated by sysprims functions.
///
/// # Safety
///
/// The pointer must have been returned by a sysprims function that
/// allocates strings (e.g., `sysprims_get_platform()`, `sysprims_last_error()`).
/// Passing null is safe and will be a no-op.
///
/// Do NOT pass pointers returned by `sysprims_version()` - those are
/// static strings that should not be freed.
///
/// # C Usage
///
/// ```c
/// char* platform = sysprims_get_platform();
/// // use platform...
/// sysprims_free_string(platform);  // Must free with this function
/// ```
#[no_mangle]
pub unsafe extern "C" fn sysprims_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: Caller guarantees `s` was allocated by a sysprims function
    // (e.g., `sysprims_get_platform`). The pointer is valid, properly aligned,
    // and was created via `CString::into_raw`.
    let _ = CString::from_raw(s);
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::ffi::CStr;

    #[test]
    fn test_version_returns_valid_string() {
        let version_ptr = sysprims_version();
        assert!(!version_ptr.is_null());

        // SAFETY: sysprims_version returns a static pointer
        let version = unsafe { CStr::from_ptr(version_ptr).to_str().unwrap() };
        assert!(!version.is_empty());
        // Version should have at least one dot (semver)
        assert!(
            version.contains('.'),
            "Version should be semver: {}",
            version
        );
    }

    #[test]
    fn test_version_is_stable_pointer() {
        // Multiple calls should return the same pointer
        let ptr1 = sysprims_version();
        let ptr2 = sysprims_version();
        assert_eq!(ptr1, ptr2, "sysprims_version should return stable pointer");
    }

    #[test]
    fn test_abi_version_is_positive() {
        let abi = sysprims_abi_version();
        assert!(abi > 0, "ABI version should be > 0");
    }

    #[test]
    fn test_get_platform_returns_valid_string() {
        let platform_ptr = sysprims_get_platform();
        assert!(!platform_ptr.is_null());

        // SAFETY: We just created this pointer and know it's valid
        let platform = unsafe { CStr::from_ptr(platform_ptr).to_str().unwrap() };
        assert_eq!(platform, env::consts::OS);

        // SAFETY: Free the string we allocated above
        unsafe { sysprims_free_string(platform_ptr) };
    }

    #[test]
    fn test_free_null_is_safe() {
        // Should not panic or crash
        // SAFETY: null is explicitly handled by sysprims_free_string
        unsafe { sysprims_free_string(std::ptr::null_mut()) };
    }
}
