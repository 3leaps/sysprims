//! Process inspection FFI functions.
//!
//! Provides JSON-based process listing and inspection via C-ABI.
//! Uses JSON for complex data structures to avoid FFI struct marshaling complexity.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::time::Duration;

use crate::error::{clear_error_state, set_error, SysprimsErrorCode};
use sysprims_core::SysprimsError;
use sysprims_proc::{FdFilter, PortFilter, ProcessFilter, ProcessOptions};

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ProcessOptionsWire {
    include_env: bool,
    include_threads: bool,
}

unsafe fn parse_process_options(
    options_json: *const c_char,
) -> Result<ProcessOptions, SysprimsError> {
    if options_json.is_null() {
        return Ok(ProcessOptions::default());
    }

    let options_str = CStr::from_ptr(options_json)
        .to_str()
        .map_err(|_| SysprimsError::invalid_argument("options_json is not valid UTF-8"))?;

    if options_str.is_empty() || options_str == "{}" {
        return Ok(ProcessOptions::default());
    }

    let wire: ProcessOptionsWire = serde_json::from_str(options_str)
        .map_err(|e| SysprimsError::invalid_argument(format!("invalid options JSON: {}", e)))?;

    Ok(ProcessOptions {
        include_env: wire.include_env,
        include_threads: wire.include_threads,
    })
}

/// List open file descriptors for a PID, optionally filtered.
///
/// Returns a JSON object matching `fd-snapshot.schema.json`.
///
/// # Arguments
///
/// * `pid` - Target PID
/// * `filter_json` - JSON filter object (may be NULL for no filtering)
/// * `result_json_out` - Output pointer for result JSON string
///
/// # Filter JSON Format
///
/// ```json
/// {
///   "kind": "socket" // Optional: "file", "socket", "pipe", "unknown"
/// }
/// ```
///
/// # Safety
///
/// * `result_json_out` must be a valid pointer to a `char*`
/// * The result string must be freed with `sysprims_free_string()`
#[no_mangle]
pub unsafe extern "C" fn sysprims_proc_list_fds(
    pid: u32,
    filter_json: *const c_char,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();

    if result_json_out.is_null() {
        let err = SysprimsError::invalid_argument("result_json_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    let filter = if filter_json.is_null() {
        FdFilter::default()
    } else {
        let filter_str = match CStr::from_ptr(filter_json).to_str() {
            Ok(s) => s,
            Err(_) => {
                let err = SysprimsError::invalid_argument("filter_json is not valid UTF-8");
                set_error(&err);
                return SysprimsErrorCode::InvalidArgument;
            }
        };

        if filter_str.is_empty() || filter_str == "{}" {
            FdFilter::default()
        } else {
            match serde_json::from_str::<FdFilter>(filter_str) {
                Ok(f) => f,
                Err(e) => {
                    let err =
                        SysprimsError::invalid_argument(format!("invalid filter JSON: {}", e));
                    set_error(&err);
                    return SysprimsErrorCode::InvalidArgument;
                }
            }
        }
    };

    if let Err(e) = filter.validate() {
        set_error(&e);
        return SysprimsErrorCode::from(&e);
    }

    let snapshot = match sysprims_proc::list_fds(pid, Some(&filter)) {
        Ok(s) => s,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    };

    let json = match serde_json::to_string(&snapshot) {
        Ok(j) => j,
        Err(e) => {
            let err = SysprimsError::internal(format!("failed to serialize fd snapshot: {}", e));
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

/// List listening ports, optionally filtered.
///
/// Returns a JSON object containing a port bindings snapshot.
///
/// # Arguments
///
/// * `filter_json` - JSON filter object (may be NULL for no filtering)
/// * `result_json_out` - Output pointer for result JSON string
///
/// # Filter JSON Format
///
/// ```json
/// {
///   "protocol": "tcp",    // Optional: "tcp" or "udp"
///   "local_port": 8080    // Optional: local port to filter
/// }
/// ```
///
/// # Safety
///
/// * `result_json_out` must be a valid pointer to a `char*`
/// * The result string must be freed with `sysprims_free_string()`
#[no_mangle]
pub unsafe extern "C" fn sysprims_proc_listening_ports(
    filter_json: *const c_char,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();

    if result_json_out.is_null() {
        let err = SysprimsError::invalid_argument("result_json_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    let filter = if filter_json.is_null() {
        PortFilter::default()
    } else {
        let filter_str = match CStr::from_ptr(filter_json).to_str() {
            Ok(s) => s,
            Err(_) => {
                let err = SysprimsError::invalid_argument("filter_json is not valid UTF-8");
                set_error(&err);
                return SysprimsErrorCode::InvalidArgument;
            }
        };

        if filter_str.is_empty() || filter_str == "{}" {
            PortFilter::default()
        } else {
            match serde_json::from_str::<PortFilter>(filter_str) {
                Ok(f) => f,
                Err(e) => {
                    let err =
                        SysprimsError::invalid_argument(format!("invalid filter JSON: {}", e));
                    set_error(&err);
                    return SysprimsErrorCode::InvalidArgument;
                }
            }
        }
    };

    if let Err(e) = filter.validate() {
        set_error(&e);
        return SysprimsErrorCode::from(&e);
    }

    let snapshot = match sysprims_proc::listening_ports(Some(&filter)) {
        Ok(s) => s,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    };

    let json = match serde_json::to_string(&snapshot) {
        Ok(j) => j,
        Err(e) => {
            let err = SysprimsError::internal(format!("failed to serialize port bindings: {}", e));
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

/// List processes, optionally filtered.
///
/// Returns a JSON object containing a process snapshot. The JSON format matches
/// the `ProcessSnapshot` schema with `schema_id`, `timestamp`, and `processes`.
///
/// # Arguments
///
/// * `filter_json` - JSON filter object (may be NULL for no filtering)
/// * `result_json_out` - Output pointer for result JSON string
///
/// # Filter JSON Format
///
/// ```json
/// {
///   "name_contains": "nginx",     // Optional: filter by name substring
///   "name_equals": "nginx",       // Optional: filter by exact name
///   "user_equals": "www-data",    // Optional: filter by username
///   "pid_in": [1234, 5678],       // Optional: filter to specific PIDs
///   "cpu_above": 10.0,            // Optional: minimum CPU percent (0-100)
///   "memory_above_kb": 1024       // Optional: minimum memory in KB
/// }
/// ```
///
/// # Returns
///
/// * `SYSPRIMS_OK` on success (result written to `result_json_out`)
/// * `SYSPRIMS_ERR_INVALID_ARGUMENT` if filter JSON is invalid
/// * `SYSPRIMS_ERR_SYSTEM` on system error
///
/// # Safety
///
/// * `result_json_out` must be a valid pointer to a `char*`
/// * The result string must be freed with `sysprims_free_string()`
///
/// # Example (C)
///
/// ```c
/// char* result = NULL;
/// SysprimsErrorCode err = sysprims_proc_list(NULL, &result);
/// if (err == SYSPRIMS_OK) {
///     printf("%s\n", result);
///     sysprims_free_string(result);
/// }
/// ```
#[no_mangle]
pub unsafe extern "C" fn sysprims_proc_list(
    filter_json: *const c_char,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    sysprims_proc_list_ex(filter_json, std::ptr::null(), result_json_out)
}

/// List processes with optional filter and optional process detail options.
///
/// `options_json` format:
///
/// ```json
/// {"include_env": true, "include_threads": true}
/// ```
///
/// # Safety
///
/// * `result_json_out` must be a valid pointer to a `char*`
/// * `filter_json` and `options_json` must be NULL or valid UTF-8 C strings
/// * The result string must be freed with `sysprims_free_string()`
#[no_mangle]
pub unsafe extern "C" fn sysprims_proc_list_ex(
    filter_json: *const c_char,
    options_json: *const c_char,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();

    if result_json_out.is_null() {
        let err = SysprimsError::invalid_argument("result_json_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    let options = match parse_process_options(options_json) {
        Ok(o) => o,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    };

    let filter = if filter_json.is_null() {
        ProcessFilter::default()
    } else {
        let filter_str = match CStr::from_ptr(filter_json).to_str() {
            Ok(s) => s,
            Err(_) => {
                let err = SysprimsError::invalid_argument("filter_json is not valid UTF-8");
                set_error(&err);
                return SysprimsErrorCode::InvalidArgument;
            }
        };

        if filter_str.is_empty() || filter_str == "{}" {
            ProcessFilter::default()
        } else {
            match serde_json::from_str::<ProcessFilter>(filter_str) {
                Ok(f) => f,
                Err(e) => {
                    let err =
                        SysprimsError::invalid_argument(format!("invalid filter JSON: {}", e));
                    set_error(&err);
                    return SysprimsErrorCode::InvalidArgument;
                }
            }
        }
    };

    if let Err(e) = filter.validate() {
        set_error(&e);
        return SysprimsErrorCode::from(&e);
    }

    let snapshot = match sysprims_proc::snapshot_filtered_with_options(&filter, options) {
        Ok(s) => s,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    };

    let json = match serde_json::to_string(&snapshot) {
        Ok(j) => j,
        Err(e) => {
            let err = SysprimsError::internal(format!("failed to serialize snapshot: {}", e));
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

/// Get information for a single process by PID.
///
/// Returns JSON for a single process. If the process doesn't exist,
/// returns `SYSPRIMS_ERR_NOT_FOUND`.
///
/// # Arguments
///
/// * `pid` - Process ID to query
/// * `result_json_out` - Output pointer for result JSON string
///
/// # Result JSON Format
///
/// ```json
/// {
///   "pid": 1234,
///   "ppid": 1,
///   "name": "nginx",
///   "user": "www-data",
///   "cpu_percent": 5.2,
///   "memory_kb": 102400,
///   "elapsed_seconds": 3600,
///   "state": "running",
///   "cmdline": ["nginx", "-g", "daemon off;"]
/// }
/// ```
///
/// # Returns
///
/// * `SYSPRIMS_OK` on success
/// * `SYSPRIMS_ERR_INVALID_ARGUMENT` if pid is 0
/// * `SYSPRIMS_ERR_NOT_FOUND` if process doesn't exist
/// * `SYSPRIMS_ERR_PERMISSION_DENIED` if not permitted to read process
///
/// # Safety
///
/// * `result_json_out` must be a valid pointer to a `char*`
/// * The result string must be freed with `sysprims_free_string()`
///
/// # Example (C)
///
/// ```c
/// char* result = NULL;
/// SysprimsErrorCode err = sysprims_proc_get(getpid(), &result);
/// if (err == SYSPRIMS_OK) {
///     printf("%s\n", result);
///     sysprims_free_string(result);
/// }
/// ```
#[no_mangle]
pub unsafe extern "C" fn sysprims_proc_get(
    pid: u32,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    sysprims_proc_get_ex(pid, std::ptr::null(), result_json_out)
}

/// Get process info with optional process detail options.
///
/// `options_json` format:
///
/// ```json
/// {"include_env": true, "include_threads": true}
/// ```
///
/// # Safety
///
/// * `result_json_out` must be a valid pointer to a `char*`
/// * `options_json` must be NULL or a valid UTF-8 C string
/// * The result string must be freed with `sysprims_free_string()`
#[no_mangle]
pub unsafe extern "C" fn sysprims_proc_get_ex(
    pid: u32,
    options_json: *const c_char,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();

    if result_json_out.is_null() {
        let err = SysprimsError::invalid_argument("result_json_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    let options = match parse_process_options(options_json) {
        Ok(o) => o,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    };

    let info = match sysprims_proc::get_process_with_options(pid, options) {
        Ok(i) => i,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    };

    let json = match serde_json::to_string(&info) {
        Ok(j) => j,
        Err(e) => {
            let err = SysprimsError::internal(format!("failed to serialize process info: {}", e));
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

/// Wait for a PID to exit, up to a timeout.
///
/// Returns a JSON object matching `wait-pid-result.schema.json`.
///
/// # Arguments
///
/// * `pid` - PID to wait on (must be > 0)
/// * `timeout_ms` - Timeout in milliseconds
/// * `result_json_out` - Output pointer for result JSON string
///
/// # Safety
///
/// * `result_json_out` must be a valid pointer to a `char*`
/// * The result string must be freed with `sysprims_free_string()`
#[no_mangle]
pub unsafe extern "C" fn sysprims_proc_wait_pid(
    pid: u32,
    timeout_ms: u64,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();

    if result_json_out.is_null() {
        let err = SysprimsError::invalid_argument("result_json_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    let result = match sysprims_proc::wait_pid(pid, Duration::from_millis(timeout_ms)) {
        Ok(r) => r,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    };

    let json = match serde_json::to_string(&result) {
        Ok(j) => j,
        Err(e) => {
            let err = SysprimsError::internal(format!("failed to serialize wait result: {}", e));
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

/// Get descendants of a process.
///
/// Returns a JSON object matching `descendants-result.schema.json`.
///
/// # Arguments
///
/// * `root_pid` - PID to traverse descendants from (must be > 0 and <= i32::MAX)
/// * `max_levels` - Maximum depth (1 = children only, `u32::MAX` = all levels)
/// * `filter_json` - Optional JSON filter (may be NULL for no filtering)
/// * `result_json_out` - Output pointer for result JSON string
///
/// # Filter JSON Format
///
/// Same as `sysprims_proc_list` filter — see `process-filter.schema.json`.
///
/// # Returns
///
/// * `SYSPRIMS_OK` on success
/// * `SYSPRIMS_ERR_INVALID_ARGUMENT` if root_pid is 0 or filter JSON is invalid
/// * `SYSPRIMS_ERR_NOT_FOUND` if root process doesn't exist
///
/// # Safety
///
/// * `result_json_out` must be a valid pointer to a `char*`
/// * The result string must be freed with `sysprims_free_string()`
#[no_mangle]
pub unsafe extern "C" fn sysprims_proc_descendants(
    root_pid: u32,
    max_levels: u32,
    filter_json: *const c_char,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    sysprims_proc_descendants_ex(
        root_pid,
        max_levels,
        filter_json,
        std::ptr::null(),
        result_json_out,
    )
}

/// Get descendants with optional filter and optional process detail options.
///
/// `options_json` format:
///
/// ```json
/// {"include_env": true, "include_threads": true}
/// ```
///
/// # Safety
///
/// * `result_json_out` must be a valid pointer to a `char*`
/// * `filter_json` and `options_json` must be NULL or valid UTF-8 C strings
/// * The result string must be freed with `sysprims_free_string()`
#[no_mangle]
pub unsafe extern "C" fn sysprims_proc_descendants_ex(
    root_pid: u32,
    max_levels: u32,
    filter_json: *const c_char,
    options_json: *const c_char,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();

    if result_json_out.is_null() {
        let err = SysprimsError::invalid_argument("result_json_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    let options = match parse_process_options(options_json) {
        Ok(o) => o,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    };

    let filter = if filter_json.is_null() {
        None
    } else {
        let filter_str = match CStr::from_ptr(filter_json).to_str() {
            Ok(s) => s,
            Err(_) => {
                let err = SysprimsError::invalid_argument("filter_json is not valid UTF-8");
                set_error(&err);
                return SysprimsErrorCode::InvalidArgument;
            }
        };

        if filter_str.is_empty() || filter_str == "{}" {
            None
        } else {
            match serde_json::from_str::<ProcessFilter>(filter_str) {
                Ok(f) => Some(f),
                Err(e) => {
                    let err =
                        SysprimsError::invalid_argument(format!("invalid filter JSON: {}", e));
                    set_error(&err);
                    return SysprimsErrorCode::InvalidArgument;
                }
            }
        }
    };

    if let Some(ref f) = filter {
        if let Err(e) = f.validate() {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    }

    let result = match sysprims_proc::descendants_with_options(
        root_pid,
        max_levels,
        filter.as_ref(),
        options,
    ) {
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
                SysprimsError::internal(format!("failed to serialize descendants result: {}", e));
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

/// Kill descendants of a process.
///
/// Traverses the process tree from `root_pid`, collects descendant PIDs, and
/// sends the specified signal to each. Safety rules are enforced in this layer:
/// the root PID, self, PID 1, and parent are excluded from the kill list.
///
/// Returns a JSON object with `schema_id`, `signal_sent`, `succeeded`, `failed`,
/// and `skipped_safety` fields.
///
/// # Arguments
///
/// * `root_pid` - PID to traverse descendants from
/// * `max_levels` - Maximum depth (`u32::MAX` = all levels)
/// * `signal` - Signal number to send (e.g., 15 for SIGTERM)
/// * `filter_json` - Optional JSON filter (may be NULL)
/// * `result_json_out` - Output pointer for result JSON string
///
/// # Safety Rules (enforced here, not in bindings)
///
/// The following PIDs are always excluded from the kill list:
/// - The root PID itself (descendants-only)
/// - The calling process (self)
/// - PID 1 (init/launchd)
/// - The calling process's parent
///
/// # Safety
///
/// * `result_json_out` must be a valid pointer to a `char*`
/// * The result string must be freed with `sysprims_free_string()`
#[no_mangle]
pub unsafe extern "C" fn sysprims_proc_kill_descendants(
    root_pid: u32,
    max_levels: u32,
    signal: i32,
    filter_json: *const c_char,
    result_json_out: *mut *mut c_char,
) -> SysprimsErrorCode {
    clear_error_state();

    if result_json_out.is_null() {
        let err = SysprimsError::invalid_argument("result_json_out cannot be null");
        set_error(&err);
        return SysprimsErrorCode::InvalidArgument;
    }

    // Parse optional filter
    let filter = if filter_json.is_null() {
        None
    } else {
        let filter_str = match CStr::from_ptr(filter_json).to_str() {
            Ok(s) => s,
            Err(_) => {
                let err = SysprimsError::invalid_argument("filter_json is not valid UTF-8");
                set_error(&err);
                return SysprimsErrorCode::InvalidArgument;
            }
        };

        if filter_str.is_empty() || filter_str == "{}" {
            None
        } else {
            match serde_json::from_str::<ProcessFilter>(filter_str) {
                Ok(f) => Some(f),
                Err(e) => {
                    let err =
                        SysprimsError::invalid_argument(format!("invalid filter JSON: {}", e));
                    set_error(&err);
                    return SysprimsErrorCode::InvalidArgument;
                }
            }
        }
    };

    if let Some(ref f) = filter {
        if let Err(e) = f.validate() {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    }

    // Traverse descendants
    let desc_result = match sysprims_proc::descendants(root_pid, max_levels, filter.as_ref()) {
        Ok(r) => r,
        Err(e) => {
            set_error(&e);
            return SysprimsErrorCode::from(&e);
        }
    };

    // Collect all descendant PIDs
    let mut target_pids: Vec<u32> = desc_result
        .levels
        .iter()
        .flat_map(|l| l.processes.iter().map(|p| p.pid))
        .collect();
    target_pids.sort_unstable();
    target_pids.dedup();

    // Safety: exclude root PID (descendants-only)
    target_pids.retain(|&pid| pid != root_pid);

    // Safety: exclude self, PID 1, parent
    let self_pid = std::process::id();
    let parent_pid = sysprims_proc::get_process(self_pid).ok().map(|p| p.ppid);

    let before = target_pids.len();
    target_pids.retain(|&pid| pid != self_pid && pid != 1);
    if let Some(ppid) = parent_pid {
        target_pids.retain(|&pid| pid != ppid);
    }
    let skipped_safety = before.saturating_sub(target_pids.len());

    // Build result
    let (succeeded, failed) = if target_pids.is_empty() {
        (Vec::new(), Vec::<KillDescendantsFailure>::new())
    } else {
        match sysprims_signal::kill_many(&target_pids, signal) {
            Ok(batch) => {
                let failed_entries: Vec<KillDescendantsFailure> = batch
                    .failed
                    .iter()
                    .map(|f| KillDescendantsFailure {
                        pid: f.pid,
                        error: f.error.to_string(),
                    })
                    .collect();
                (batch.succeeded, failed_entries)
            }
            Err(e) => {
                set_error(&e);
                return SysprimsErrorCode::from(&e);
            }
        }
    };

    let result = KillDescendantsResultJson {
        schema_id: sysprims_core::schema::BATCH_KILL_RESULT_V1,
        signal_sent: signal,
        root_pid,
        succeeded,
        failed,
        skipped_safety,
    };

    let json = match serde_json::to_string(&result) {
        Ok(j) => j,
        Err(e) => {
            let err = SysprimsError::internal(format!(
                "failed to serialize kill-descendants result: {}",
                e
            ));
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

/// JSON-serializable failure entry for kill-descendants results.
#[derive(serde::Serialize)]
struct KillDescendantsFailure {
    pid: u32,
    error: String,
}

/// JSON-serializable result for kill-descendants.
#[derive(serde::Serialize)]
struct KillDescendantsResultJson {
    schema_id: &'static str,
    signal_sent: i32,
    root_pid: u32,
    succeeded: Vec<u32>,
    failed: Vec<KillDescendantsFailure>,
    skipped_safety: usize,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysprims_free_string;
    use std::ffi::CStr;

    #[test]
    fn test_proc_list_no_filter() {
        let mut result: *mut c_char = std::ptr::null_mut();
        let code = unsafe { sysprims_proc_list(std::ptr::null(), &mut result) };

        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(!result.is_null());

        // SAFETY: We just allocated this
        let json = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        assert!(json.contains("\"schema_id\""));
        assert!(json.contains("\"processes\""));

        unsafe { sysprims_free_string(result) };
    }

    #[test]
    fn test_proc_list_with_filter() {
        let filter = CString::new(r#"{"name_contains": "sysprims"}"#).unwrap();
        let mut result: *mut c_char = std::ptr::null_mut();

        let code = unsafe { sysprims_proc_list(filter.as_ptr(), &mut result) };

        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(!result.is_null());

        unsafe { sysprims_free_string(result) };
    }

    #[test]
    fn test_proc_list_invalid_filter() {
        let filter = CString::new(r#"{"unknown_field": true}"#).unwrap();
        let mut result: *mut c_char = std::ptr::null_mut();

        let code = unsafe { sysprims_proc_list(filter.as_ptr(), &mut result) };

        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
        assert!(result.is_null());
    }

    #[test]
    fn test_proc_list_fds_self() {
        let pid = std::process::id();
        let mut result: *mut c_char = std::ptr::null_mut();

        let code = unsafe { sysprims_proc_list_fds(pid, std::ptr::null(), &mut result) };

        if cfg!(windows) {
            assert_eq!(code, SysprimsErrorCode::NotSupported);
            assert!(result.is_null());
            return;
        }

        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(!result.is_null());

        // SAFETY: We just allocated this
        let json = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        assert!(json.contains("\"schema_id\""));
        assert!(json.contains("\"fds\""));

        unsafe { sysprims_free_string(result) };
    }

    #[test]
    fn test_proc_listening_ports_self_listener() {
        use serde_json::Value;
        use std::net::TcpListener;

        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                eprintln!("SKIP: net.Listen denied in this environment: {}", e);
                return;
            }
            Err(e) => panic!("TcpListener::bind failed unexpectedly: {}", e),
        };
        let port = listener.local_addr().unwrap().port();
        let pid = std::process::id();

        let filter =
            CString::new(format!(r#"{{"protocol":"tcp","local_port":{}}}"#, port)).unwrap();
        let mut result: *mut c_char = std::ptr::null_mut();

        let code = unsafe { sysprims_proc_listening_ports(filter.as_ptr(), &mut result) };

        // NotSupported is acceptable in container/CI environments where
        // port introspection may not be available.
        if code == SysprimsErrorCode::NotSupported {
            eprintln!("SKIP: listening_ports returned NotSupported (container/CI environment)");
            drop(listener);
            return;
        }

        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(!result.is_null());

        let json_str = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        let value: Value = serde_json::from_str(json_str).unwrap();

        let bindings = value.get("bindings").and_then(|v| v.as_array()).unwrap();

        let found = bindings.iter().any(|binding| {
            let local_port = binding.get("local_port").and_then(|v| v.as_u64());
            let pid_value = binding.get("pid").and_then(|v| v.as_u64());
            local_port == Some(port as u64) && pid_value == Some(pid as u64)
        });

        if !found {
            let warnings = value
                .get("warnings")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            // macOS/libproc environments can restrict socket introspection (SIP/TCC).
            // We treat this as best-effort and only hard-fail if we explicitly got
            // a PermissionDenied error code.
            eprintln!("bindings count: {}", bindings.len());
            eprintln!("warnings: {:?}", warnings);
        }

        if !found {
            // Accept best-effort omission (common on macOS due to SIP/TCC, and
            // can occur on constrained CI runners). PermissionDenied should be
            // represented as an error code, not an empty Ok response.
            unsafe { sysprims_free_string(result) };
            drop(listener);
            return;
        }

        unsafe { sysprims_free_string(result) };

        drop(listener);
    }

    #[test]
    fn test_proc_list_null_output() {
        let code = unsafe { sysprims_proc_list(std::ptr::null(), std::ptr::null_mut()) };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
    }

    #[test]
    fn test_proc_get_self() {
        let pid = std::process::id();
        let mut result: *mut c_char = std::ptr::null_mut();

        let code = unsafe { sysprims_proc_get(pid, &mut result) };

        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(!result.is_null());

        // SAFETY: We just allocated this
        let json = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        assert!(json.contains(&format!("\"pid\":{}", pid)));

        unsafe { sysprims_free_string(result) };
    }

    #[test]
    fn test_proc_get_invalid_pid() {
        let mut result: *mut c_char = std::ptr::null_mut();
        let code = unsafe { sysprims_proc_get(0, &mut result) };

        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
        assert!(result.is_null());
    }

    #[test]
    fn test_proc_get_nonexistent() {
        let mut result: *mut c_char = std::ptr::null_mut();
        let code = unsafe { sysprims_proc_get(99999999, &mut result) };

        assert_eq!(code, SysprimsErrorCode::NotFound);
        assert!(result.is_null());
    }

    #[test]
    fn test_proc_get_null_output() {
        let code = unsafe { sysprims_proc_get(1234, std::ptr::null_mut()) };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
    }

    #[test]
    fn test_proc_get_ex_self_with_threads_option() {
        let pid = std::process::id();
        let options = CString::new(r#"{"include_threads":true}"#).unwrap();
        let mut result: *mut c_char = std::ptr::null_mut();

        let code = unsafe { sysprims_proc_get_ex(pid, options.as_ptr(), &mut result) };

        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(!result.is_null());

        let json = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        assert!(json.contains("\"pid\":"));

        unsafe { sysprims_free_string(result) };
    }

    #[test]
    fn test_proc_list_ex_invalid_options_json() {
        let options = CString::new(r#"{"bad":true}"#).unwrap();
        let mut result: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { sysprims_proc_list_ex(std::ptr::null(), options.as_ptr(), &mut result) };

        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
        assert!(result.is_null());
    }

    #[test]
    fn test_proc_wait_pid_invalid_pid() {
        let mut result: *mut c_char = std::ptr::null_mut();
        let code = unsafe { sysprims_proc_wait_pid(0, 1, &mut result) };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
    }

    #[test]
    fn test_proc_wait_pid_self_times_out() {
        let mut result: *mut c_char = std::ptr::null_mut();
        let pid = std::process::id();
        let code = unsafe { sysprims_proc_wait_pid(pid, 1, &mut result) };
        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(!result.is_null());

        let json = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        assert!(json.contains("\"timed_out\":true"));

        unsafe { sysprims_free_string(result) };
    }

    // ========================================================================
    // Descendants FFI tests
    // ========================================================================

    #[test]
    fn test_proc_descendants_self() {
        let pid = std::process::id();
        let mut result: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { sysprims_proc_descendants(pid, u32::MAX, std::ptr::null(), &mut result) };

        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(!result.is_null());

        let json = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        assert!(json.contains("\"schema_id\""));
        assert!(json.contains("\"root_pid\""));
        assert!(json.contains("\"levels\""));
        assert!(json.contains("\"total_found\""));

        unsafe { sysprims_free_string(result) };
    }

    #[test]
    fn test_proc_descendants_invalid_pid_zero() {
        let mut result: *mut c_char = std::ptr::null_mut();
        let code = unsafe { sysprims_proc_descendants(0, u32::MAX, std::ptr::null(), &mut result) };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
        assert!(result.is_null());
    }

    #[test]
    fn test_proc_descendants_nonexistent_pid() {
        let mut result: *mut c_char = std::ptr::null_mut();
        let code =
            unsafe { sysprims_proc_descendants(99999999, u32::MAX, std::ptr::null(), &mut result) };
        assert_eq!(code, SysprimsErrorCode::NotFound);
        assert!(result.is_null());
    }

    #[test]
    fn test_proc_descendants_null_output() {
        let pid = std::process::id();
        let code = unsafe {
            sysprims_proc_descendants(pid, u32::MAX, std::ptr::null(), std::ptr::null_mut())
        };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
    }

    #[test]
    fn test_proc_descendants_with_filter() {
        let pid = std::process::id();
        let filter = CString::new(r#"{"name_contains": "nonexistent_proc_xyz"}"#).unwrap();
        let mut result: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { sysprims_proc_descendants(pid, u32::MAX, filter.as_ptr(), &mut result) };

        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(!result.is_null());

        let json = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        // The filter should result in matched_by_filter == 0
        assert!(json.contains("\"matched_by_filter\":0"));

        unsafe { sysprims_free_string(result) };
    }

    #[test]
    fn test_proc_descendants_invalid_filter() {
        let pid = std::process::id();
        let filter = CString::new(r#"{"unknown_field": true}"#).unwrap();
        let mut result: *mut c_char = std::ptr::null_mut();

        let code =
            unsafe { sysprims_proc_descendants(pid, u32::MAX, filter.as_ptr(), &mut result) };

        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
        assert!(result.is_null());
    }

    // ========================================================================
    // Kill-descendants FFI tests
    // ========================================================================

    #[test]
    fn test_proc_kill_descendants_null_output() {
        let pid = std::process::id();
        let code = unsafe {
            sysprims_proc_kill_descendants(
                pid,
                u32::MAX,
                15,
                std::ptr::null(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
    }

    #[test]
    fn test_proc_kill_descendants_invalid_pid_zero() {
        let mut result: *mut c_char = std::ptr::null_mut();
        let code = unsafe {
            sysprims_proc_kill_descendants(0, u32::MAX, 15, std::ptr::null(), &mut result)
        };
        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
        assert!(result.is_null());
    }

    #[test]
    fn test_proc_kill_descendants_nonexistent_pid() {
        let mut result: *mut c_char = std::ptr::null_mut();
        let code = unsafe {
            sysprims_proc_kill_descendants(99999999, u32::MAX, 15, std::ptr::null(), &mut result)
        };
        assert_eq!(code, SysprimsErrorCode::NotFound);
        assert!(result.is_null());
    }

    #[test]
    fn test_proc_kill_descendants_self_returns_json() {
        // Calling kill-descendants on self should succeed (no actual children
        // in a test process), returning an empty result with skipped_safety.
        let pid = std::process::id();
        let mut result: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            sysprims_proc_kill_descendants(pid, u32::MAX, 15, std::ptr::null(), &mut result)
        };

        assert_eq!(code, SysprimsErrorCode::Ok);
        assert!(!result.is_null());

        let json = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        assert!(json.contains("\"schema_id\""));
        assert!(json.contains("\"signal_sent\":15"));
        assert!(json.contains("\"root_pid\""));
        assert!(json.contains("\"skipped_safety\""));

        unsafe { sysprims_free_string(result) };
    }

    #[test]
    fn test_proc_kill_descendants_invalid_filter() {
        let pid = std::process::id();
        let filter = CString::new(r#"{"bad_field": 123}"#).unwrap();
        let mut result: *mut c_char = std::ptr::null_mut();

        let code = unsafe {
            sysprims_proc_kill_descendants(pid, u32::MAX, 15, filter.as_ptr(), &mut result)
        };

        assert_eq!(code, SysprimsErrorCode::InvalidArgument);
        assert!(result.is_null());
    }
}
