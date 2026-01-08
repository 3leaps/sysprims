//! sysprims-ffi: C-ABI exports for sysprims
//!
//! This crate provides a stable C-ABI interface for sysprims functionality.
//! All strings returned by this API must be freed with `sysprims_free_string()`.

use std::ffi::CString;
use std::os::raw::c_char;
use sysprims_core::get_platform;

/// Returns the current platform name as a C string.
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

/// Frees a string allocated by sysprims functions.
///
/// # Safety
///
/// The pointer must have been returned by a sysprims function that
/// allocates strings (e.g., `sysprims_get_platform()`). Passing null
/// is safe and will be a no-op.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::ffi::CStr;

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
