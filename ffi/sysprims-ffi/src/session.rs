//! Session and process-group FFI functions.
//!
//! This module provides non-signal-sending primitives for "self" runtime
//! introspection and session-spawn operations.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_uint};
use std::path::PathBuf;

use crate::error::{clear_error_state, set_error, SysprimsErrorCode};
use sysprims_core::schema::{RUN_NOHUP_CONFIG_V1, RUN_SETSID_CONFIG_V1, SESSION_SPAWN_RESULT_V1};
use sysprims_core::{get_platform, time::now_rfc3339, SysprimsError};
#[cfg(unix)]
use sysprims_session::NohupOutcome;
use sysprims_session::{SetsidConfig, SetsidOutcome};

const MAX_SAFE_PID: u32 = i32::MAX as u32;

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
// Session Spawn
// ============================================================================

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RunSetsidWireConfig {
    schema_id: String,
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    wait: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(unix), allow(dead_code))]
struct RunNohupWireConfig {
    schema_id: String,
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    wait: bool,
    #[serde(default)]
    output_file: Option<String>,
}

#[derive(serde::Serialize)]
struct SessionSpawnResult {
    schema_id: &'static str,
    timestamp: String,
    platform: &'static str,
    verb: &'static str,
    status: &'static str,
    pid: Option<u32>,
    sid: Option<u32>,
    pgid: Option<u32>,
    session_kind: &'static str,
    identifier_provenance: &'static str,
    exit_code: Option<i32>,
    signal: Option<i32>,
    output_file: Option<String>,
    warnings: Vec<String>,
}

/// Run a command in a new session.
///
/// Returns a JSON object matching `session-spawn-result.schema.json`.
///
/// # Safety
///
/// * `config_json` must point to a valid UTF-8 C string
/// * `result_json_out` must be a valid pointer to a `char*`
/// * The result string must be freed with `sysprims_free_string()`
#[no_mangle]
pub unsafe extern "C" fn sysprims_run_setsid(
    config_json: *const c_char,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();

    let cfg_str = match validate_json_args(config_json, result_json_out) {
        Ok(s) => s,
        Err(code) => return code,
    };
    *result_json_out = std::ptr::null_mut();

    let wire = match serde_json::from_str::<RunSetsidWireConfig>(cfg_str) {
        Ok(c) => c,
        Err(e) => {
            let err = SysprimsError::invalid_argument(format!("invalid config JSON: {}", e));
            set_error(&err);
            return SysprimsErrorCode::InvalidArgument;
        }
    };

    if wire.schema_id != RUN_SETSID_CONFIG_V1 {
        let err = SysprimsError::invalid_argument(format!(
            "invalid schema_id (expected {})",
            RUN_SETSID_CONFIG_V1
        ));
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    let (command, args) = match split_argv(&wire.argv) {
        Ok(v) => v,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::InvalidArgument;
        }
    };

    let config = SetsidConfig {
        wait: wire.wait,
        ctty: false,
        cwd: wire.cwd.map(PathBuf::from),
        env: wire.env,
    };

    let outcome = match sysprims_session::run_setsid(command, &args, config) {
        Ok(r) => r,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    };

    let result = match setsid_result_from_outcome(outcome) {
        Ok(r) => r,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    };

    write_json_result(result, result_json_out)
}

/// Run a command with SIGHUP ignored.
///
/// Returns a JSON object matching `session-spawn-result.schema.json`.
///
/// # Safety
///
/// * `config_json` must point to a valid UTF-8 C string
/// * `result_json_out` must be a valid pointer to a `char*`
/// * The result string must be freed with `sysprims_free_string()`
#[no_mangle]
pub unsafe extern "C" fn sysprims_run_nohup(
    config_json: *const c_char,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();

    let cfg_str = match validate_json_args(config_json, result_json_out) {
        Ok(s) => s,
        Err(code) => return code,
    };
    *result_json_out = std::ptr::null_mut();

    let wire = match serde_json::from_str::<RunNohupWireConfig>(cfg_str) {
        Ok(c) => c,
        Err(e) => {
            let err = SysprimsError::invalid_argument(format!("invalid config JSON: {}", e));
            set_error(&err);
            return SysprimsErrorCode::InvalidArgument;
        }
    };

    if wire.schema_id != RUN_NOHUP_CONFIG_V1 {
        let err = SysprimsError::invalid_argument(format!(
            "invalid schema_id (expected {})",
            RUN_NOHUP_CONFIG_V1
        ));
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    let (command, args) = match split_argv(&wire.argv) {
        Ok(v) => v,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::InvalidArgument;
        }
    };

    #[cfg(not(unix))]
    {
        let _ = (command, args);
        let err = SysprimsError::not_supported("nohup", std::env::consts::OS);
        set_error(&err);
        SysprimsErrorCode::NotSupported
    }

    #[cfg(unix)]
    {
        let caller_sid = match sysprims_session::getsid(0) {
            Ok(sid) => sid,
            Err(e) => {
                set_error(&e);
                return SysprimsErrorCode::from(&e);
            }
        };
        let caller_pgid = match sysprims_session::getpgid(0) {
            Ok(pgid) => pgid,
            Err(e) => {
                set_error(&e);
                return SysprimsErrorCode::from(&e);
            }
        };

        let config = sysprims_session::NohupConfig {
            wait: wire.wait,
            output_file: wire.output_file,
            cwd: wire.cwd.map(PathBuf::from),
            env: wire.env,
        };

        let outcome = match sysprims_session::run_nohup(command, &args, config) {
            Ok(r) => r,
            Err(e) => {
                set_error(&e);
                return SysprimsErrorCode::from(&e);
            }
        };

        let result = match nohup_result_from_outcome(outcome, caller_sid, caller_pgid) {
            Ok(r) => r,
            Err(e) => {
                set_error(&e);
                return SysprimsErrorCode::from(&e);
            }
        };

        write_json_result(result, result_json_out)
    }
}

unsafe fn validate_json_args<'a>(
    config_json: *const c_char,
    result_json_out: *mut *mut c_char,
) -> Result<&'a str, SysprimsErrorCode> {
    if result_json_out.is_null() {
        let err = SysprimsError::invalid_argument("result_json_out cannot be null");
        set_error(&err);
        return Err(SysprimsErrorCode::InvalidArgument);
    }

    if config_json.is_null() {
        let err = SysprimsError::invalid_argument("config_json cannot be null");
        set_error(&err);
        return Err(SysprimsErrorCode::InvalidArgument);
    }

    let cfg_str = match CStr::from_ptr(config_json).to_str() {
        Ok(s) => s,
        Err(_) => {
            let err = SysprimsError::invalid_argument("config_json is not valid UTF-8");
            set_error(&err);
            return Err(SysprimsErrorCode::InvalidArgument);
        }
    };

    if cfg_str.is_empty() {
        let err = SysprimsError::invalid_argument("config_json cannot be empty");
        set_error(&err);
        return Err(SysprimsErrorCode::InvalidArgument);
    }

    Ok(cfg_str)
}

fn split_argv(argv: &[String]) -> Result<(&str, Vec<&str>), SysprimsError> {
    let command = argv
        .first()
        .ok_or_else(|| SysprimsError::invalid_argument("argv must not be empty"))?;
    if command.is_empty() {
        return Err(SysprimsError::invalid_argument("argv[0] must not be empty"));
    }
    let args = argv.iter().skip(1).map(String::as_str).collect();
    Ok((command.as_str(), args))
}

fn setsid_result_from_outcome(outcome: SetsidOutcome) -> Result<SessionSpawnResult, SysprimsError> {
    match outcome {
        SetsidOutcome::Spawned { child_pid } => {
            validate_output_pid(child_pid, "child_pid")?;
            Ok(SessionSpawnResult {
                schema_id: SESSION_SPAWN_RESULT_V1,
                timestamp: now_rfc3339(),
                platform: get_platform(),
                verb: "setsid",
                status: "spawned",
                pid: Some(child_pid),
                sid: Some(child_pid),
                pgid: Some(child_pid),
                session_kind: "new_session",
                identifier_provenance: "setsid_structural_child_pid",
                exit_code: None,
                signal: None,
                output_file: None,
                warnings: Vec::new(),
            })
        }
        SetsidOutcome::Completed {
            child_pid,
            exit_status,
        } => {
            validate_output_pid(child_pid, "child_pid")?;
            Ok(SessionSpawnResult {
                schema_id: SESSION_SPAWN_RESULT_V1,
                timestamp: now_rfc3339(),
                platform: get_platform(),
                verb: "setsid",
                status: "completed",
                pid: Some(child_pid),
                sid: Some(child_pid),
                pgid: Some(child_pid),
                session_kind: "new_session",
                identifier_provenance: "setsid_structural_child_pid",
                exit_code: exit_status.code(),
                signal: exit_signal(&exit_status),
                output_file: None,
                warnings: Vec::new(),
            })
        }
    }
}

#[cfg(unix)]
fn nohup_result_from_outcome(
    outcome: NohupOutcome,
    caller_sid: u32,
    caller_pgid: u32,
) -> Result<SessionSpawnResult, SysprimsError> {
    validate_output_pid(caller_sid, "caller_sid")?;
    validate_output_pid(caller_pgid, "caller_pgid")?;

    match outcome {
        NohupOutcome::Spawned {
            child_pid,
            output_file,
        } => {
            validate_output_pid(child_pid, "child_pid")?;
            Ok(SessionSpawnResult {
                schema_id: SESSION_SPAWN_RESULT_V1,
                timestamp: now_rfc3339(),
                platform: get_platform(),
                verb: "nohup",
                status: "spawned",
                pid: Some(child_pid),
                sid: Some(caller_sid),
                pgid: Some(caller_pgid),
                session_kind: "inherited_session",
                identifier_provenance: "caller_context_before_spawn",
                exit_code: None,
                signal: None,
                output_file,
                warnings: Vec::new(),
            })
        }
        NohupOutcome::Completed {
            child_pid,
            exit_status,
            output_file,
        } => {
            validate_output_pid(child_pid, "child_pid")?;
            Ok(SessionSpawnResult {
                schema_id: SESSION_SPAWN_RESULT_V1,
                timestamp: now_rfc3339(),
                platform: get_platform(),
                verb: "nohup",
                status: "completed",
                pid: Some(child_pid),
                sid: Some(caller_sid),
                pgid: Some(caller_pgid),
                session_kind: "inherited_session",
                identifier_provenance: "caller_context_before_spawn",
                exit_code: exit_status.code(),
                signal: exit_signal(&exit_status),
                output_file,
                warnings: Vec::new(),
            })
        }
    }
}

fn validate_output_pid(value: u32, name: &str) -> Result<(), SysprimsError> {
    if value == 0 || value > MAX_SAFE_PID {
        return Err(SysprimsError::spawn_failed(
            "session spawn",
            format!("{name} {value} is outside [1, {MAX_SAFE_PID}]"),
        ));
    }
    Ok(())
}

fn write_json_result(
    result: SessionSpawnResult,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    let json = match serde_json::to_string(&result) {
        Ok(j) => j,
        Err(e) => {
            let err =
                SysprimsError::internal(format!("failed to serialize session spawn result: {}", e));
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

    unsafe {
        *result_json_out = c_json.into_raw();
    }
    SysprimsErrorCode::Ok
}

#[cfg(unix)]
fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn exit_signal(_status: &std::process::ExitStatus) -> Option<i32> {
    None
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysprims_free_string;
    use std::ptr;
    use sysprims_core::schema::{RUN_NOHUP_CONFIG_V1, RUN_SETSID_CONFIG_V1};

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

    #[test]
    fn test_run_setsid_rejects_null_config() {
        let mut result: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_run_setsid(ptr::null(), &mut result) };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
        assert!(result.is_null());
    }

    #[test]
    fn test_run_setsid_rejects_empty_argv() {
        let cfg = CString::new(format!(
            r#"{{"schema_id":"{}","argv":[]}}"#,
            RUN_SETSID_CONFIG_V1
        ))
        .unwrap();
        let mut result: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_run_setsid(cfg.as_ptr(), &mut result) };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
        assert!(result.is_null());
    }

    #[test]
    #[cfg(unix)]
    fn test_run_setsid_wait_result_has_structural_ids() {
        let cfg = CString::new(format!(
            r#"{{"schema_id":"{}","argv":["sh","-c","exit 0"],"wait":true}}"#,
            RUN_SETSID_CONFIG_V1
        ))
        .unwrap();
        let mut result: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_run_setsid(cfg.as_ptr(), &mut result) };
        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(!result.is_null());

        let json = unsafe { CStr::from_ptr(result).to_str().unwrap().to_string() };
        unsafe { sysprims_free_string(result) };

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["verb"], "setsid");
        assert_eq!(parsed["status"], "completed");
        assert_eq!(parsed["session_kind"], "new_session");
        assert_eq!(
            parsed["identifier_provenance"],
            "setsid_structural_child_pid"
        );
        assert_eq!(parsed["pid"], parsed["sid"]);
        assert_eq!(parsed["pid"], parsed["pgid"]);
        assert_eq!(parsed["exit_code"], 0);
    }

    #[test]
    #[cfg(unix)]
    fn test_run_nohup_spawned_result_has_inherited_context() {
        let caller_sid = sysprims_session::getsid(0).unwrap();
        let caller_pgid = sysprims_session::getpgid(0).unwrap();
        let cfg = CString::new(format!(
            r#"{{"schema_id":"{}","argv":["sleep","0.1"],"output_file":"/dev/null"}}"#,
            RUN_NOHUP_CONFIG_V1
        ))
        .unwrap();
        let mut result: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_run_nohup(cfg.as_ptr(), &mut result) };
        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(!result.is_null());

        let json = unsafe { CStr::from_ptr(result).to_str().unwrap().to_string() };
        unsafe { sysprims_free_string(result) };

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["verb"], "nohup");
        assert_eq!(parsed["status"], "spawned");
        assert_eq!(parsed["session_kind"], "inherited_session");
        assert_eq!(
            parsed["identifier_provenance"],
            "caller_context_before_spawn"
        );
        assert_eq!(parsed["sid"], caller_sid);
        assert_eq!(parsed["pgid"], caller_pgid);
        assert!(parsed["pid"].as_u64().unwrap() > 0);
    }
}
