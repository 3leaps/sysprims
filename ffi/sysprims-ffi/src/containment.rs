//! Managed containment C ABI: generation-checked capability tokens.

use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use serde::Deserialize;
use sysprims_core::schema::CONTAINMENT_SPAWN_CONFIG_V1;
use sysprims_core::SysprimsError;
use sysprims_timeout::{
    containment_close, containment_identity, containment_poll, containment_spawn,
    containment_terminate, containment_wait, ManagedSnapshot, ManagedSpawnRequest,
};

use crate::error::{clear_error_state, set_error, SysprimsErrorCode};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSpawnConfig {
    schema_id: String,
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Option<BTreeMap<String, String>>,
    #[serde(default)]
    execution_timeout_ms: Option<u64>,
    #[serde(default)]
    grace_timeout_ms: Option<u64>,
    #[serde(default)]
    kill_timeout_ms: Option<u64>,
    #[serde(default)]
    signal: Option<i32>,
    #[serde(default)]
    kill_signal: Option<i32>,
}

fn write_json(snapshot: &ManagedSnapshot, out: *mut *mut c_char) -> SysprimsErrorCode {
    let json = match serde_json::to_string(snapshot) {
        Ok(json) => json,
        Err(error) => {
            let err = SysprimsError::internal(format!("failed to serialize snapshot: {error}"));
            set_error(&err);
            return SysprimsErrorCode::Internal;
        }
    };
    let c_json = match CString::new(json) {
        Ok(value) => value,
        Err(error) => {
            let err = SysprimsError::internal(format!("JSON contains null byte: {error}"));
            set_error(&err);
            return SysprimsErrorCode::Internal;
        }
    };
    unsafe {
        *out = c_json.into_raw();
    }
    SysprimsErrorCode::Ok
}

fn map_err(error: SysprimsError) -> SysprimsErrorCode {
    set_error(&error);
    SysprimsErrorCode::from(&error)
}

fn read_utf8<'a>(ptr: *const c_char, name: &str) -> Result<&'a str, SysprimsErrorCode> {
    if ptr.is_null() {
        let err = SysprimsError::invalid_argument(format!("{name} cannot be null"));
        set_error(&err);
        return Err(SysprimsErrorCode::InvalidArgument);
    }
    match unsafe { CStr::from_ptr(ptr) }.to_str() {
        Ok(value) => Ok(value),
        Err(_) => {
            let err = SysprimsError::invalid_argument(format!("{name} is not valid UTF-8"));
            set_error(&err);
            Err(SysprimsErrorCode::InvalidArgument)
        }
    }
}

/// Spawn a sysprims-owned contained process and return a capability token.
///
/// `handle_out` receives a non-zero generation-checked `uint64_t`. It is not a
/// PID and must not be treated as a pointer. Failure returns no handle.
///
/// # Safety
///
/// `config_json` and `handle_out` must be valid pointers.
#[no_mangle]
pub unsafe extern "C" fn sysprims_containment_spawn(
    config_json: *const c_char,
    handle_out: *mut u64,
) -> SysprimsErrorCode {
    clear_error_state();
    if handle_out.is_null() {
        let err = SysprimsError::invalid_argument("handle_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }
    unsafe {
        *handle_out = 0;
    }

    let cfg_str = match read_utf8(config_json, "config_json") {
        Ok(value) => value,
        Err(code) => return code,
    };
    if cfg_str.is_empty() {
        let err = SysprimsError::invalid_argument("config_json cannot be empty");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    let wire = match serde_json::from_str::<WireSpawnConfig>(cfg_str) {
        Ok(value) => value,
        Err(error) => {
            let err = SysprimsError::invalid_argument(format!("invalid config JSON: {error}"));
            set_error(&err);
            return SysprimsErrorCode::InvalidArgument;
        }
    };
    if wire.schema_id != CONTAINMENT_SPAWN_CONFIG_V1 {
        let err = SysprimsError::invalid_argument(format!(
            "invalid schema_id (expected {CONTAINMENT_SPAWN_CONFIG_V1})"
        ));
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    let request = ManagedSpawnRequest {
        argv: wire.argv,
        cwd: wire.cwd,
        env: wire.env,
        execution_timeout_ms: wire.execution_timeout_ms,
        grace_timeout_ms: wire.grace_timeout_ms,
        kill_timeout_ms: wire.kill_timeout_ms,
        signal: wire.signal,
        kill_signal: wire.kill_signal,
    };

    match containment_spawn(request) {
        Ok(token) => {
            unsafe {
                *handle_out = token;
            }
            SysprimsErrorCode::Ok
        }
        Err(error) => map_err(error),
    }
}

/// Return immutable identity and reliability evidence for a live or inert handle.
///
/// # Safety
///
/// `result_json_out` must be a valid pointer. Free with `sysprims_free_string`.
#[no_mangle]
pub unsafe extern "C" fn sysprims_containment_identity(
    handle: u64,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();
    if result_json_out.is_null() {
        let err = SysprimsError::invalid_argument("result_json_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }
    unsafe {
        *result_json_out = std::ptr::null_mut();
    }
    match containment_identity(handle) {
        Ok(snapshot) => write_json(&snapshot, result_json_out),
        Err(error) => map_err(error),
    }
}

/// Non-destructive poll. Completes the lifecycle only when the leader has exited.
///
/// # Safety
///
/// `result_json_out` must be a valid pointer. Free with `sysprims_free_string`.
#[no_mangle]
pub unsafe extern "C" fn sysprims_containment_poll(
    handle: u64,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();
    if result_json_out.is_null() {
        let err = SysprimsError::invalid_argument("result_json_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }
    unsafe {
        *result_json_out = std::ptr::null_mut();
    }
    match containment_poll(handle) {
        Ok(snapshot) => write_json(&snapshot, result_json_out),
        Err(error) => map_err(error),
    }
}

/// Wait until the native lifecycle is inert or `wait_timeout_ms` elapses.
///
/// A timeout of 0 waits until a terminal result. Language wait cancellation
/// must not be implemented by interrupting this call mid-kill; abandon the
/// waiter instead.
///
/// # Safety
///
/// `result_json_out` must be a valid pointer. Free with `sysprims_free_string`.
#[no_mangle]
pub unsafe extern "C" fn sysprims_containment_wait(
    handle: u64,
    wait_timeout_ms: u64,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();
    if result_json_out.is_null() {
        let err = SysprimsError::invalid_argument("result_json_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }
    unsafe {
        *result_json_out = std::ptr::null_mut();
    }
    match containment_wait(handle, wait_timeout_ms) {
        Ok(snapshot) => write_json(&snapshot, result_json_out),
        Err(error) => map_err(error),
    }
}

/// Explicitly terminate the owned containment once.
///
/// # Safety
///
/// `result_json_out` must be a valid pointer. Free with `sysprims_free_string`.
#[no_mangle]
pub unsafe extern "C" fn sysprims_containment_terminate(
    handle: u64,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();
    if result_json_out.is_null() {
        let err = SysprimsError::invalid_argument("result_json_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }
    unsafe {
        *result_json_out = std::ptr::null_mut();
    }
    match containment_terminate(handle) {
        Ok(snapshot) => write_json(&snapshot, result_json_out),
        Err(error) => map_err(error),
    }
}

/// Deterministically release the registry entry.
///
/// Active close performs bounded native guard cleanup and recycles the slot
/// only after the child is confirmed reaped. Cleanup failure keeps the same
/// generation and active owner so close is retryable. A stale or foreign token
/// fails closed. Language wrappers must keep their token until this call
/// succeeds; clear it only after success and restore it on error.
#[no_mangle]
pub extern "C" fn sysprims_containment_close(handle: u64) -> SysprimsErrorCode {
    clear_error_state();
    match containment_close(handle) {
        Ok(()) => SysprimsErrorCode::Ok,
        Err(error) => map_err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::sysprims_free_string;
    use std::ptr;

    fn spawn_cfg(argv: &[&str]) -> CString {
        let argv_json: Vec<String> = argv.iter().map(|value| format!("\"{value}\"")).collect();
        CString::new(format!(
            r#"{{"schema_id":"{CONTAINMENT_SPAWN_CONFIG_V1}","argv":[{}],"grace_timeout_ms":50,"kill_timeout_ms":200}}"#,
            argv_json.join(",")
        ))
        .unwrap()
    }

    #[test]
    fn rejects_null_and_zero_handles() {
        let mut handle = 1u64;
        let code = unsafe { sysprims_containment_spawn(ptr::null(), &mut handle) };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
        assert_eq!(handle, 0);

        let mut json: *mut c_char = ptr::null_mut();
        assert_eq!(
            unsafe { sysprims_containment_identity(0, &mut json) },
            SysprimsErrorCode::InvalidArgument
        );
        assert!(json.is_null());
        assert_eq!(
            unsafe { sysprims_containment_poll(u64::MAX, &mut json) },
            SysprimsErrorCode::InvalidArgument
        );
        assert_eq!(
            sysprims_containment_close(0),
            SysprimsErrorCode::InvalidArgument
        );
    }

    #[test]
    fn rejects_malformed_wire_and_null_outputs() {
        for wire in [
            "{",
            r#"{"schema_id":"wrong","argv":["true"]}"#,
            &format!(
                r#"{{"schema_id":"{CONTAINMENT_SPAWN_CONFIG_V1}","argv":["true"],"unexpected":true}}"#
            ),
        ] {
            let config = CString::new(wire).unwrap();
            let mut handle = 99;
            assert_eq!(
                unsafe { sysprims_containment_spawn(config.as_ptr(), &mut handle) },
                SysprimsErrorCode::InvalidArgument
            );
            assert_eq!(handle, 0, "wire rejection must return no capability");
        }
        let config = spawn_cfg(&["true"]);
        assert_eq!(
            unsafe { sysprims_containment_spawn(config.as_ptr(), ptr::null_mut()) },
            SysprimsErrorCode::InvalidArgument
        );
        assert_eq!(
            unsafe { sysprims_containment_identity(0, ptr::null_mut()) },
            SysprimsErrorCode::InvalidArgument
        );
        assert_eq!(
            unsafe { sysprims_containment_poll(0, ptr::null_mut()) },
            SysprimsErrorCode::InvalidArgument
        );
        assert_eq!(
            unsafe { sysprims_containment_wait(0, 10, ptr::null_mut()) },
            SysprimsErrorCode::InvalidArgument
        );
        assert_eq!(
            unsafe { sysprims_containment_terminate(0, ptr::null_mut()) },
            SysprimsErrorCode::InvalidArgument
        );
    }

    #[cfg(unix)]
    #[test]
    fn null_snapshot_outputs_preserve_a_valid_handle() {
        let config = spawn_cfg(&["sleep", "30"]);
        let mut handle = 0;
        assert_eq!(
            unsafe { sysprims_containment_spawn(config.as_ptr(), &mut handle) },
            SysprimsErrorCode::Ok
        );
        let codes = unsafe {
            [
                sysprims_containment_identity(handle, ptr::null_mut()),
                sysprims_containment_poll(handle, ptr::null_mut()),
                sysprims_containment_wait(handle, 10, ptr::null_mut()),
                sysprims_containment_terminate(handle, ptr::null_mut()),
            ]
        };
        let snapshot = containment_identity(handle);
        assert_eq!(sysprims_containment_close(handle), SysprimsErrorCode::Ok);
        assert!(codes
            .iter()
            .all(|code| *code == SysprimsErrorCode::InvalidArgument));
        assert_eq!(snapshot.unwrap().handle_state, "active");
    }

    #[cfg(unix)]
    #[test]
    fn spawn_wait_close_and_stale_token() {
        let cfg = spawn_cfg(&["true"]);
        let mut handle = 0u64;
        let code = unsafe { sysprims_containment_spawn(cfg.as_ptr(), &mut handle) };
        assert_eq!(code, SysprimsErrorCode::Ok);
        assert_ne!(handle, 0);

        let mut json: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_containment_wait(handle, 5_000, &mut json) };
        assert_eq!(code, SysprimsErrorCode::Ok);
        let text = unsafe { CStr::from_ptr(json).to_str().unwrap() };
        assert!(text.contains("\"leader_status\":\"completed\""));
        assert!(text.contains("\"tree_kill_reliability\":\"guaranteed\""));
        assert!(text.contains("\"boundary_strength\":\"cooperative_group\""));
        unsafe { sysprims_free_string(json) };

        assert_eq!(sysprims_containment_close(handle), SysprimsErrorCode::Ok);
        assert_eq!(
            sysprims_containment_close(handle),
            SysprimsErrorCode::InvalidArgument
        );
    }
}
