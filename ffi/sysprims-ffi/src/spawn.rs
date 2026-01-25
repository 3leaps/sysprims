//! Spawn primitives exposed over the C-ABI.
//!
//! These functions accept JSON inputs and return JSON outputs to avoid complex
//! FFI struct marshaling.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::error::{clear_error_state, set_error, SysprimsErrorCode};
use sysprims_core::schema::SPAWN_IN_GROUP_CONFIG_V1;
use sysprims_core::SysprimsError;
use sysprims_timeout::{spawn_in_group, SpawnInGroupConfig};

/// Spawn a process in a new process group (Unix) or Job Object (Windows).
///
/// Returns a JSON object matching `spawn-in-group-result.schema.json`.
///
/// # Arguments
///
/// * `config_json` - Spawn config JSON (must not be NULL)
/// * `result_json_out` - Output pointer for result JSON string
///
/// # Safety
///
/// * `config_json` must point to a valid UTF-8 C string
/// * `result_json_out` must be a valid pointer to a `char*`
/// * The result string must be freed with `sysprims_free_string()`
#[no_mangle]
pub unsafe extern "C" fn sysprims_spawn_in_group(
    config_json: *const c_char,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();

    if result_json_out.is_null() {
        let err = SysprimsError::invalid_argument("result_json_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    if config_json.is_null() {
        let err = SysprimsError::invalid_argument("config_json cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    let cfg_str = match CStr::from_ptr(config_json).to_str() {
        Ok(s) => s,
        Err(_) => {
            let err = SysprimsError::invalid_argument("config_json is not valid UTF-8");
            set_error(&err);
            return SysprimsErrorCode::InvalidArgument;
        }
    };

    if cfg_str.is_empty() {
        let err = SysprimsError::invalid_argument("config_json cannot be empty");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireConfig {
        schema_id: String,
        argv: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        env: Option<std::collections::BTreeMap<String, String>>,
    }

    let wire = match serde_json::from_str::<WireConfig>(cfg_str) {
        Ok(c) => c,
        Err(e) => {
            let err = SysprimsError::invalid_argument(format!("invalid config JSON: {}", e));
            set_error(&err);
            return SysprimsErrorCode::InvalidArgument;
        }
    };

    if wire.schema_id != SPAWN_IN_GROUP_CONFIG_V1 {
        let err = SysprimsError::invalid_argument(format!(
            "invalid schema_id (expected {})",
            SPAWN_IN_GROUP_CONFIG_V1
        ));
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    let cfg = SpawnInGroupConfig {
        argv: wire.argv,
        cwd: wire.cwd,
        env: wire.env,
    };

    let result = match spawn_in_group(cfg) {
        Ok(r) => r,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    };

    let json = match serde_json::to_string(&result) {
        Ok(j) => j,
        Err(e) => {
            let err = SysprimsError::internal(format!("failed to serialize spawn result: {}", e));
            set_error(&err);
            return SysprimsErrorCode::Internal;
        }
    };

    let c_json = match CString::new(json) {
        Ok(c) => c,
        Err(e) => {
            let err = SysprimsError::internal(format!("JSON contains null byte: {}", e));
            set_error(&err);
            return SysprimsErrorCode::Internal;
        }
    };

    *result_json_out = c_json.into_raw();
    SysprimsErrorCode::Ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysprims_free_string;
    use std::ptr;

    #[test]
    fn test_spawn_in_group_rejects_null_config() {
        let mut result: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_spawn_in_group(ptr::null(), &mut result) };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
        assert!(result.is_null());
    }

    #[test]
    fn test_spawn_in_group_rejects_empty_config() {
        let cfg = CString::new("").unwrap();
        let mut result: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_spawn_in_group(cfg.as_ptr(), &mut result) };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
        assert!(result.is_null());
    }

    #[test]
    fn test_spawn_in_group_basic() {
        #[cfg(unix)]
        let argv = ["sleep", "0"];
        #[cfg(windows)]
        let argv = ["cmd", "/C", "exit 0"];

        let cfg = CString::new(format!(
            r#"{{"schema_id":"{}","argv":[{}]}}"#,
            SPAWN_IN_GROUP_CONFIG_V1,
            argv.iter()
                .map(|s| format!("\"{}\"", s))
                .collect::<Vec<_>>()
                .join(",")
        ))
        .unwrap();

        let mut result: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_spawn_in_group(cfg.as_ptr(), &mut result) };
        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(!result.is_null());

        unsafe { sysprims_free_string(result) };
    }
}
