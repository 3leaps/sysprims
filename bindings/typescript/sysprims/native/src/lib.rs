use std::time::Duration;

use napi_derive::napi;
use sysprims_core::schema::{SPAWN_IN_GROUP_CONFIG_V1, TERMINATE_TREE_CONFIG_V1};
use sysprims_core::SysprimsError;
use sysprims_proc::{FdFilter, PortFilter, ProcessFilter};
use sysprims_timeout::{spawn_in_group, terminate_tree, SpawnInGroupConfig, TerminateTreeConfig};

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysprimsErrorCode {
    Ok = 0,
    InvalidArgument = 1,
    SpawnFailed = 2,
    Timeout = 3,
    PermissionDenied = 4,
    NotFound = 5,
    NotSupported = 6,
    GroupCreationFailed = 7,
    System = 8,
    Internal = 99,
}

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

#[napi(object)]
pub struct SysprimsCallJsonResult {
    pub code: i32,
    pub json: Option<String>,
    pub message: Option<String>,
}

#[napi(object)]
pub struct SysprimsCallU32Result {
    pub code: i32,
    pub value: Option<u32>,
    pub message: Option<String>,
}

#[napi(object)]
pub struct SysprimsCallVoidResult {
    pub code: i32,
    pub message: Option<String>,
}

fn ok_json(json: String) -> SysprimsCallJsonResult {
    SysprimsCallJsonResult {
        code: SysprimsErrorCode::Ok as i32,
        json: Some(json),
        message: None,
    }
}

fn err_json(err: SysprimsError) -> SysprimsCallJsonResult {
    SysprimsCallJsonResult {
        code: SysprimsErrorCode::from(&err) as i32,
        json: None,
        message: Some(err.to_string()),
    }
}

#[cfg(unix)]
fn ok_u32(value: u32) -> SysprimsCallU32Result {
    SysprimsCallU32Result {
        code: SysprimsErrorCode::Ok as i32,
        value: Some(value),
        message: None,
    }
}

fn err_u32(err: SysprimsError) -> SysprimsCallU32Result {
    SysprimsCallU32Result {
        code: SysprimsErrorCode::from(&err) as i32,
        value: None,
        message: Some(err.to_string()),
    }
}

fn ok_void() -> SysprimsCallVoidResult {
    SysprimsCallVoidResult {
        code: SysprimsErrorCode::Ok as i32,
        message: None,
    }
}

fn err_void(err: SysprimsError) -> SysprimsCallVoidResult {
    SysprimsCallVoidResult {
        code: SysprimsErrorCode::from(&err) as i32,
        message: Some(err.to_string()),
    }
}

#[napi]
pub fn sysprims_abi_version() -> u32 {
    1
}

// -----------------------------------------------------------------------------
// Process Inspection
// -----------------------------------------------------------------------------

#[napi]
pub fn sysprims_proc_get(pid: u32) -> SysprimsCallJsonResult {
    match sysprims_proc::get_process(pid) {
        Ok(info) => match serde_json::to_string(&info) {
            Ok(json) => ok_json(json),
            Err(e) => err_json(SysprimsError::internal(format!(
                "failed to serialize process info: {}",
                e
            ))),
        },
        Err(e) => err_json(e),
    }
}

#[napi]
pub fn sysprims_proc_list(filter_json: String) -> SysprimsCallJsonResult {
    let filter = if filter_json.is_empty() || filter_json == "{}" {
        ProcessFilter::default()
    } else {
        match serde_json::from_str::<ProcessFilter>(&filter_json) {
            Ok(f) => f,
            Err(e) => {
                return err_json(SysprimsError::invalid_argument(format!(
                    "invalid filter JSON: {}",
                    e
                )))
            }
        }
    };

    if let Err(e) = filter.validate() {
        return err_json(e);
    }

    match sysprims_proc::snapshot_filtered(&filter) {
        Ok(snapshot) => match serde_json::to_string(&snapshot) {
            Ok(json) => ok_json(json),
            Err(e) => err_json(SysprimsError::internal(format!(
                "failed to serialize snapshot: {}",
                e
            ))),
        },
        Err(e) => err_json(e),
    }
}

#[napi]
pub fn sysprims_proc_listening_ports(filter_json: String) -> SysprimsCallJsonResult {
    let filter = if filter_json.is_empty() || filter_json == "{}" {
        PortFilter::default()
    } else {
        match serde_json::from_str::<PortFilter>(&filter_json) {
            Ok(f) => f,
            Err(e) => {
                return err_json(SysprimsError::invalid_argument(format!(
                    "invalid filter JSON: {}",
                    e
                )))
            }
        }
    };

    if let Err(e) = filter.validate() {
        return err_json(e);
    }

    match sysprims_proc::listening_ports(Some(&filter)) {
        Ok(snapshot) => match serde_json::to_string(&snapshot) {
            Ok(json) => ok_json(json),
            Err(e) => err_json(SysprimsError::internal(format!(
                "failed to serialize port bindings: {}",
                e
            ))),
        },
        Err(e) => err_json(e),
    }
}

#[napi]
pub fn sysprims_proc_list_fds(pid: u32, filter_json: String) -> SysprimsCallJsonResult {
    let filter = if filter_json.is_empty() || filter_json == "{}" {
        FdFilter::default()
    } else {
        match serde_json::from_str::<FdFilter>(&filter_json) {
            Ok(f) => f,
            Err(e) => {
                return err_json(SysprimsError::invalid_argument(format!(
                    "invalid filter JSON: {}",
                    e
                )))
            }
        }
    };

    if let Err(e) = filter.validate() {
        return err_json(e);
    }

    match sysprims_proc::list_fds(pid, Some(&filter)) {
        Ok(snapshot) => match serde_json::to_string(&snapshot) {
            Ok(json) => ok_json(json),
            Err(e) => err_json(SysprimsError::internal(format!(
                "failed to serialize fd snapshot: {}",
                e
            ))),
        },
        Err(e) => err_json(e),
    }
}

#[napi]
pub fn sysprims_proc_wait_pid(pid: u32, timeout_ms: u32) -> SysprimsCallJsonResult {
    match sysprims_proc::wait_pid(pid, Duration::from_millis(timeout_ms as u64)) {
        Ok(result) => match serde_json::to_string(&result) {
            Ok(json) => ok_json(json),
            Err(e) => err_json(SysprimsError::internal(format!(
                "failed to serialize wait result: {}",
                e
            ))),
        },
        Err(e) => err_json(e),
    }
}

// -----------------------------------------------------------------------------
// Self Introspection
// -----------------------------------------------------------------------------

#[napi]
pub fn sysprims_self_getpgid() -> SysprimsCallU32Result {
    #[cfg(unix)]
    {
        match sysprims_session::getpgid(0) {
            Ok(v) => ok_u32(v),
            Err(e) => err_u32(e),
        }
    }

    #[cfg(windows)]
    {
        err_u32(SysprimsError::not_supported("getpgid", "windows"))
    }
}

#[napi]
pub fn sysprims_self_getsid() -> SysprimsCallU32Result {
    #[cfg(unix)]
    {
        match sysprims_session::getsid(0) {
            Ok(v) => ok_u32(v),
            Err(e) => err_u32(e),
        }
    }

    #[cfg(windows)]
    {
        err_u32(SysprimsError::not_supported("getsid", "windows"))
    }
}

// -----------------------------------------------------------------------------
// Signals
// -----------------------------------------------------------------------------

#[napi]
pub fn sysprims_signal_send(pid: u32, signal: i32) -> SysprimsCallVoidResult {
    match sysprims_signal::kill(pid, signal) {
        Ok(()) => ok_void(),
        Err(e) => err_void(e),
    }
}

#[napi]
pub fn sysprims_signal_send_group(pgid: u32, signal: i32) -> SysprimsCallVoidResult {
    match sysprims_signal::killpg(pgid, signal) {
        Ok(()) => ok_void(),
        Err(e) => err_void(e),
    }
}

#[napi]
pub fn sysprims_terminate(pid: u32) -> SysprimsCallVoidResult {
    match sysprims_signal::terminate(pid) {
        Ok(()) => ok_void(),
        Err(e) => err_void(e),
    }
}

#[napi]
pub fn sysprims_force_kill(pid: u32) -> SysprimsCallVoidResult {
    match sysprims_signal::force_kill(pid) {
        Ok(()) => ok_void(),
        Err(e) => err_void(e),
    }
}

// -----------------------------------------------------------------------------
// Terminate Tree
// -----------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTerminateTreeConfig {
    #[serde(default = "default_terminate_tree_schema_id")]
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

fn default_terminate_tree_schema_id() -> String {
    TERMINATE_TREE_CONFIG_V1.to_string()
}

impl From<WireTerminateTreeConfig> for TerminateTreeConfig {
    fn from(value: WireTerminateTreeConfig) -> Self {
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

#[napi]
pub fn sysprims_terminate_tree(pid: u32, config_json: String) -> SysprimsCallJsonResult {
    let cfg = if config_json.is_empty() || config_json == "{}" {
        TerminateTreeConfig::default()
    } else {
        let wire = match serde_json::from_str::<WireTerminateTreeConfig>(&config_json) {
            Ok(v) => v,
            Err(e) => {
                return err_json(SysprimsError::invalid_argument(format!(
                    "invalid config JSON: {}",
                    e
                )))
            }
        };

        if wire.schema_id != TERMINATE_TREE_CONFIG_V1 {
            return err_json(SysprimsError::invalid_argument(format!(
                "invalid schema_id (expected {})",
                TERMINATE_TREE_CONFIG_V1
            )));
        }

        wire.into()
    };

    match terminate_tree(pid, cfg) {
        Ok(result) => match serde_json::to_string(&result) {
            Ok(json) => ok_json(json),
            Err(e) => err_json(SysprimsError::internal(format!(
                "failed to serialize terminate result: {}",
                e
            ))),
        },
        Err(e) => err_json(e),
    }
}

// -----------------------------------------------------------------------------
// Spawn In Group
// -----------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSpawnInGroupConfig {
    schema_id: String,
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Option<std::collections::BTreeMap<String, String>>,
}

#[napi]
pub fn sysprims_spawn_in_group(config_json: String) -> SysprimsCallJsonResult {
    if config_json.is_empty() {
        return err_json(SysprimsError::invalid_argument(
            "config_json cannot be empty",
        ));
    }

    let wire = match serde_json::from_str::<WireSpawnInGroupConfig>(&config_json) {
        Ok(v) => v,
        Err(e) => {
            return err_json(SysprimsError::invalid_argument(format!(
                "invalid config JSON: {}",
                e
            )))
        }
    };

    if wire.schema_id != SPAWN_IN_GROUP_CONFIG_V1 {
        return err_json(SysprimsError::invalid_argument(format!(
            "invalid schema_id (expected {})",
            SPAWN_IN_GROUP_CONFIG_V1
        )));
    }

    let cfg = SpawnInGroupConfig {
        argv: wire.argv,
        cwd: wire.cwd,
        env: wire.env,
    };

    match spawn_in_group(cfg) {
        Ok(result) => match serde_json::to_string(&result) {
            Ok(json) => ok_json(json),
            Err(e) => err_json(SysprimsError::internal(format!(
                "failed to serialize spawn result: {}",
                e
            ))),
        },
        Err(e) => err_json(e),
    }
}
