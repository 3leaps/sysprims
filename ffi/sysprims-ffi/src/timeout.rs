//! Timeout execution FFI functions.
//!
//! Provides process execution with timeout via C-ABI.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::time::Duration;

use serde::Serialize;
use sysprims_core::schema::{TERMINATE_TREE_CONFIG_V1, TIMEOUT_RESULT_V1};
use sysprims_core::SysprimsError;
use sysprims_timeout::{
    terminate_tree, GroupingMode, TerminateTreeConfig, TimeoutConfig, TimeoutOutcome,
    TreeKillReliability,
};

use crate::error::{clear_error_state, set_error, SysprimsErrorCode};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SysprimsTerminateTreeConfig {
    #[serde(default = "default_config_schema_id")]
    schema_id: String,

    #[serde(default)]
    grace_timeout_ms: Option<u64>,
    #[serde(default)]
    kill_timeout_ms: Option<u64>,
    #[serde(default)]
    signal: Option<i32>,
    #[serde(default)]
    kill_signal: Option<i32>,
}

fn default_config_schema_id() -> String {
    TERMINATE_TREE_CONFIG_V1.to_string()
}

impl From<SysprimsTerminateTreeConfig> for TerminateTreeConfig {
    fn from(value: SysprimsTerminateTreeConfig) -> Self {
        let mut cfg = TerminateTreeConfig::default();
        if let Some(v) = value.grace_timeout_ms {
            cfg.grace_timeout_ms = v;
        }
        if let Some(v) = value.kill_timeout_ms {
            cfg.kill_timeout_ms = v;
        }
        if let Some(v) = value.signal {
            cfg.signal = v;
        }
        if let Some(v) = value.kill_signal {
            cfg.kill_signal = v;
        }
        cfg
    }
}

/// Process grouping mode for timeout execution.
///
/// Controls whether timeout creates a process group (Unix) or Job Object
/// (Windows) to enable tree-kill on timeout.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysprimsGroupingMode {
    /// Create new process group (Unix) or Job Object (Windows).
    /// Kill entire tree on timeout. This is the recommended default.
    GroupByDefault = 0,
    /// Run in foreground. Only kills direct child on timeout.
    Foreground = 1,
}

// C-friendly constants (see `ffi/sysprims-ffi/src/error.rs` for rationale).
#[allow(dead_code)] // exported for cbindgen-generated C header
pub const SYSPRIMS_GROUP_BY_DEFAULT: SysprimsGroupingMode = SysprimsGroupingMode::GroupByDefault;
#[allow(dead_code)] // exported for cbindgen-generated C header
pub const SYSPRIMS_FOREGROUND: SysprimsGroupingMode = SysprimsGroupingMode::Foreground;

impl From<SysprimsGroupingMode> for GroupingMode {
    fn from(mode: SysprimsGroupingMode) -> Self {
        match mode {
            SysprimsGroupingMode::GroupByDefault => GroupingMode::GroupByDefault,
            SysprimsGroupingMode::Foreground => GroupingMode::Foreground,
        }
    }
}

/// Configuration for timeout execution.
///
/// All string pointers must be valid UTF-8 C strings.
#[repr(C)]
#[derive(Debug)]
pub struct SysprimsTimeoutConfig {
    /// Command to execute (must not be NULL).
    pub command: *const c_char,

    /// Argument array (may be NULL for no arguments).
    pub args: *const *const c_char,

    /// Number of arguments in `args` array.
    pub args_len: usize,

    /// Timeout duration in milliseconds.
    pub timeout_ms: u64,

    /// Delay before escalating to SIGKILL, in milliseconds.
    /// Set to 0 for immediate escalation (no grace period).
    pub kill_after_ms: u64,

    /// Signal to send on timeout (e.g., 15 for SIGTERM).
    pub signal: i32,

    /// Process grouping mode.
    pub grouping: SysprimsGroupingMode,

    /// Whether to preserve the child's exit code.
    pub preserve_status: bool,
}

/// Result of timeout execution.
///
/// JSON-serializable structure returned by `sysprims_timeout_run`.
#[derive(Debug, Serialize)]
struct SysprimsTimeoutResult {
    /// Schema ID for this output.
    pub schema_id: &'static str,
    /// Whether the command completed or timed out.
    pub status: String,

    /// Exit code if command completed (None if timed out).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,

    /// Signal sent if command timed out (None if completed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_sent: Option<i32>,

    /// Whether escalation to SIGKILL occurred.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escalated: Option<bool>,

    /// Tree-kill reliability: "guaranteed" or "best_effort".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree_kill_reliability: Option<String>,
}

impl From<TimeoutOutcome> for SysprimsTimeoutResult {
    fn from(outcome: TimeoutOutcome) -> Self {
        match outcome {
            TimeoutOutcome::Completed { exit_status } => SysprimsTimeoutResult {
                schema_id: TIMEOUT_RESULT_V1,
                status: "completed".to_string(),
                exit_code: exit_status.code(),
                signal_sent: None,
                escalated: None,
                tree_kill_reliability: None,
            },
            TimeoutOutcome::TimedOut {
                signal_sent,
                escalated,
                tree_kill_reliability,
            } => SysprimsTimeoutResult {
                schema_id: TIMEOUT_RESULT_V1,
                status: "timed_out".to_string(),
                exit_code: None,
                signal_sent: Some(signal_sent),
                escalated: Some(escalated),
                tree_kill_reliability: Some(match tree_kill_reliability {
                    TreeKillReliability::Guaranteed => "guaranteed".to_string(),
                    TreeKillReliability::BestEffort => "best_effort".to_string(),
                }),
            },
        }
    }
}

/// Run a command with timeout.
///
/// Spawns the command and waits for it to complete or timeout. If the command
/// times out, the entire process tree is killed (when using GroupByDefault).
///
/// # Arguments
///
/// * `config` - Timeout configuration (must not be NULL)
/// * `result_json_out` - Output pointer for result JSON string
///
/// # Result JSON Format
///
/// ```json
/// // Completed:
/// {
///   "status": "completed",
///   "exit_code": 0
/// }
///
/// // Timed out:
/// {
///   "status": "timed_out",
///   "signal_sent": 15,
///   "escalated": false,
///   "tree_kill_reliability": "guaranteed"
/// }
/// ```
///
/// # Returns
///
/// * `SYSPRIMS_OK` on success (result written to `result_json_out`)
/// * `SYSPRIMS_ERR_INVALID_ARGUMENT` if config is invalid
/// * `SYSPRIMS_ERR_SPAWN_FAILED` if command couldn't be spawned
/// * `SYSPRIMS_ERR_NOT_FOUND` if command doesn't exist
/// * `SYSPRIMS_ERR_PERMISSION_DENIED` if command isn't executable
///
/// # Safety
///
/// * `config` must be a valid pointer to `SysprimsTimeoutConfig`
/// * `config.command` must be a valid, non-null C string
/// * `config.args` may be null (no arguments) or a valid array
/// * `result_json_out` must be a valid pointer to a `char*`
/// * The result string must be freed with `sysprims_free_string()`
///
/// # Example (C)
///
/// ```c
/// SysprimsTimeoutConfig config = {
///     .command = "/bin/sleep",
///     .args = (const char*[]){ "10", NULL },
///     .args_len = 1,
///     .timeout_ms = 5000,
///     .kill_after_ms = 2000,
///     .signal = 15,  // SIGTERM
///     .grouping = SYSPRIMS_GROUP_BY_DEFAULT,
///     .preserve_status = false,
/// };
///
/// char* result = NULL;
/// SysprimsErrorCode err = sysprims_timeout_run(&config, &result);
/// if (err == SYSPRIMS_OK) {
///     printf("%s\n", result);
///     sysprims_free_string(result);
/// }
/// ```
#[no_mangle]
pub unsafe extern "C" fn sysprims_timeout_run(
    config: *const SysprimsTimeoutConfig,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();

    // Validate pointers
    if config.is_null() {
        let err = SysprimsError::invalid_argument("config cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    if result_json_out.is_null() {
        let err = SysprimsError::invalid_argument("result_json_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    // SAFETY: We verified config is not null
    let cfg = &*config;

    // Validate command
    if cfg.command.is_null() {
        let err = SysprimsError::invalid_argument("command cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    // Parse command
    let command = match CStr::from_ptr(cfg.command).to_str() {
        Ok(s) => s,
        Err(_) => {
            let err = SysprimsError::invalid_argument("command is not valid UTF-8");
            set_error(&err);
            return SysprimsErrorCode::InvalidArgument;
        }
    };

    if command.is_empty() {
        let err = SysprimsError::invalid_argument("command cannot be empty");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    // Parse arguments
    let mut args: Vec<&str> = Vec::new();
    if cfg.args.is_null() && cfg.args_len > 0 {
        let err = SysprimsError::invalid_argument("args cannot be null when args_len > 0");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }
    if !cfg.args.is_null() && cfg.args_len > 0 {
        for i in 0..cfg.args_len {
            let arg_ptr = *cfg.args.add(i);
            if arg_ptr.is_null() {
                // Stop at first null (allows null-terminated arrays)
                break;
            }
            match CStr::from_ptr(arg_ptr).to_str() {
                Ok(s) => args.push(s),
                Err(_) => {
                    let err =
                        SysprimsError::invalid_argument(format!("arg[{}] is not valid UTF-8", i));
                    set_error(&err);
                    return SysprimsErrorCode::InvalidArgument;
                }
            }
        }
    }

    // Validate timeout
    if cfg.timeout_ms == 0 {
        let err = SysprimsError::invalid_argument("timeout_ms must be > 0");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    // Build configuration
    let timeout_config = TimeoutConfig {
        signal: cfg.signal,
        kill_after: Duration::from_millis(cfg.kill_after_ms),
        grouping: GroupingMode::from(cfg.grouping),
        preserve_status: cfg.preserve_status,
    };

    let timeout = Duration::from_millis(cfg.timeout_ms);

    // Run with timeout
    let outcome = match sysprims_timeout::run_with_timeout(command, &args, timeout, timeout_config)
    {
        Ok(o) => o,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    };

    // Convert to result
    let result = SysprimsTimeoutResult::from(outcome);

    // Serialize to JSON
    let json = match serde_json::to_string(&result) {
        Ok(j) => j,
        Err(e) => {
            let err = SysprimsError::internal(format!("failed to serialize result: {}", e));
            set_error(&err);
            return SysprimsErrorCode::Internal;
        }
    };

    // Convert to C string
    let c_json = match CString::new(json) {
        Ok(c) => c,
        Err(e) => {
            let err = SysprimsError::internal(format!("JSON contains null byte: {}", e));
            set_error(&err);
            return SysprimsErrorCode::Internal;
        }
    };

    // SAFETY: We verified result_json_out is not null above
    *result_json_out = c_json.into_raw();
    SysprimsErrorCode::Ok
}

/// Terminate a process (best-effort tree) with escalation.
///
/// Returns a JSON object matching `terminate-tree-result.schema.json`.
///
/// # Arguments
///
/// * `pid` - Process ID to terminate (must be > 0)
/// * `config_json` - Optional JSON config (NULL/empty/"{}" for defaults)
/// * `result_json_out` - Output pointer for result JSON string
///
/// # Safety
///
/// * `result_json_out` must be a valid pointer to a `char*`
/// * The result string must be freed with `sysprims_free_string()`
#[no_mangle]
pub unsafe extern "C" fn sysprims_terminate_tree(
    pid: u32,
    config_json: *const c_char,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();

    if result_json_out.is_null() {
        let err = SysprimsError::invalid_argument("result_json_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    let cfg = if config_json.is_null() {
        TerminateTreeConfig::default()
    } else {
        let cfg_str = match CStr::from_ptr(config_json).to_str() {
            Ok(s) => s,
            Err(_) => {
                let err = SysprimsError::invalid_argument("config_json is not valid UTF-8");
                set_error(&err);
                return SysprimsErrorCode::InvalidArgument;
            }
        };

        if cfg_str.is_empty() || cfg_str == "{}" {
            TerminateTreeConfig::default()
        } else {
            let parsed = match serde_json::from_str::<SysprimsTerminateTreeConfig>(cfg_str) {
                Ok(p) => p,
                Err(e) => {
                    let err =
                        SysprimsError::invalid_argument(format!("invalid config JSON: {}", e));
                    set_error(&err);
                    return SysprimsErrorCode::InvalidArgument;
                }
            };

            if parsed.schema_id != TERMINATE_TREE_CONFIG_V1 {
                let err = SysprimsError::invalid_argument(format!(
                    "invalid schema_id (expected {})",
                    TERMINATE_TREE_CONFIG_V1
                ));
                set_error(&err);
                return SysprimsErrorCode::InvalidArgument;
            }

            parsed.into()
        }
    };

    let result = match terminate_tree(pid, cfg) {
        Ok(r) => r,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    };

    let json = match serde_json::to_string(&result) {
        Ok(j) => j,
        Err(e) => {
            let err =
                SysprimsError::internal(format!("failed to serialize terminate result: {}", e));
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysprims_free_string;
    use std::ffi::CStr;
    use std::ptr;

    fn make_config(command: &CString, timeout_ms: u64) -> SysprimsTimeoutConfig {
        SysprimsTimeoutConfig {
            command: command.as_ptr(),
            args: std::ptr::null(),
            args_len: 0,
            timeout_ms,
            kill_after_ms: 2000,
            signal: 15, // SIGTERM
            grouping: SysprimsGroupingMode::GroupByDefault,
            preserve_status: false,
        }
    }

    #[test]
    fn test_timeout_null_config() {
        let mut result: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_timeout_run(ptr::null(), &mut result) };

        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
        assert!(result.is_null());
    }

    #[test]
    fn test_timeout_null_output() {
        let cmd = CString::new("sleep").unwrap();
        let config = make_config(&cmd, 1000);

        let code = unsafe { sysprims_timeout_run(&config, ptr::null_mut()) };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
    }

    #[test]
    fn test_timeout_null_command() {
        let config = SysprimsTimeoutConfig {
            command: ptr::null(),
            args: ptr::null(),
            args_len: 0,
            timeout_ms: 1000,
            kill_after_ms: 2000,
            signal: 15,
            grouping: SysprimsGroupingMode::GroupByDefault,
            preserve_status: false,
        };

        let mut result: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_timeout_run(&config, &mut result) };

        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
    }

    #[test]
    fn test_timeout_zero_timeout() {
        let cmd = CString::new("sleep").unwrap();
        let mut config = make_config(&cmd, 1000);
        config.timeout_ms = 0;

        let mut result: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_timeout_run(&config, &mut result) };

        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
    }

    #[test]
    fn test_timeout_null_args_with_nonzero_len() {
        let cmd = CString::new("echo").unwrap();
        let config = SysprimsTimeoutConfig {
            command: cmd.as_ptr(),
            args: ptr::null(), // NULL args
            args_len: 2,       // but non-zero length
            timeout_ms: 1000,
            kill_after_ms: 2000,
            signal: 15,
            grouping: SysprimsGroupingMode::GroupByDefault,
            preserve_status: false,
        };

        let mut result: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_timeout_run(&config, &mut result) };

        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
        assert!(result.is_null());
    }

    // Platform-specific command for quick completion test
    #[cfg(unix)]
    const TRUE_CMD: &str = "true";
    #[cfg(windows)]
    const TRUE_CMD: &str = "cmd";

    #[test]
    fn test_timeout_command_completes() {
        let cmd = CString::new(TRUE_CMD).unwrap();

        #[cfg(windows)]
        let args_raw: Vec<CString> =
            vec![CString::new("/c").unwrap(), CString::new("exit 0").unwrap()];
        #[cfg(unix)]
        let args_raw: Vec<CString> = vec![];

        let args_ptrs: Vec<*const c_char> = args_raw.iter().map(|s| s.as_ptr()).collect();

        let config = SysprimsTimeoutConfig {
            command: cmd.as_ptr(),
            args: if args_ptrs.is_empty() {
                ptr::null()
            } else {
                args_ptrs.as_ptr()
            },
            args_len: args_ptrs.len(),
            timeout_ms: 10000,
            kill_after_ms: 2000,
            signal: 15,
            grouping: SysprimsGroupingMode::GroupByDefault,
            preserve_status: false,
        };

        let mut result: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_timeout_run(&config, &mut result) };

        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(!result.is_null());

        // SAFETY: We just allocated this
        let json = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        assert!(json.contains("\"status\":\"completed\""), "JSON: {}", json);
        // Verify schema_id matches expected constant (per ADR-0005)
        assert!(
            json.contains(&format!("\"schema_id\":\"{}\"", TIMEOUT_RESULT_V1)),
            "Result JSON should contain schema_id={}: {}",
            TIMEOUT_RESULT_V1,
            json
        );

        unsafe { sysprims_free_string(result) };
    }

    #[test]
    fn test_terminate_tree_rejects_pid_zero() {
        let mut result: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_terminate_tree(0, ptr::null(), &mut result) };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
        assert!(result.is_null());
    }

    #[test]
    fn test_terminate_tree_kills_spawned_child() {
        #[cfg(unix)]
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("Failed to spawn sleep");

        #[cfg(windows)]
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 60 127.0.0.1 >NUL"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("Failed to spawn ping");

        let pid = child.id();

        let cfg = CString::new(format!(
            r#"{{"schema_id":"{}","grace_timeout_ms":100,"kill_timeout_ms":5000}}"#,
            TERMINATE_TREE_CONFIG_V1
        ))
        .unwrap();

        let mut result: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_terminate_tree(pid, cfg.as_ptr(), &mut result) };
        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(!result.is_null());

        let json = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        assert!(json.contains("\"schema_id\""));
        assert!(json.contains("\"tree_kill_reliability\""));

        unsafe { sysprims_free_string(result) };
        let _ = child.wait();
    }

    #[test]
    fn test_timeout_nonexistent_command() {
        let cmd = CString::new("/nonexistent/command/that/does/not/exist").unwrap();
        let config = make_config(&cmd, 1000);

        let mut result: *mut c_char = ptr::null_mut();
        let code = unsafe { sysprims_timeout_run(&config, &mut result) };

        // Should fail to spawn
        assert!(
            code == SysprimsErrorCode::NotFound || code == SysprimsErrorCode::SpawnFailed,
            "Expected NotFound or SpawnFailed, got {:?}",
            code
        );
    }

    #[test]
    fn test_grouping_mode_conversion() {
        assert_eq!(
            GroupingMode::from(SysprimsGroupingMode::GroupByDefault),
            GroupingMode::GroupByDefault
        );
        assert_eq!(
            GroupingMode::from(SysprimsGroupingMode::Foreground),
            GroupingMode::Foreground
        );
    }
}
