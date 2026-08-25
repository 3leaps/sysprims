use std::path::PathBuf;
use std::time::Duration;

use napi_derive::napi;
use sysprims_core::schema::{
    RUN_NOHUP_CONFIG_V1, RUN_SETSID_CONFIG_V1, SESSION_SPAWN_RESULT_V1, SPAWN_IN_GROUP_CONFIG_V1,
    TERMINATE_TREE_CONFIG_V1,
};
use sysprims_core::SysprimsError;
use sysprims_proc::{
    ancestors, descendants_with_config_and_options, guard_step, CpuMode, DescendantsConfig,
    FdFilter, GuardAction, GuardConfig, GuardRule, PortFilter, ProcessFilter, ProcessOptions,
};
use sysprims_timeout::{spawn_in_group, terminate_tree, SpawnInGroupConfig, TerminateTreeConfig};

const MAX_SAFE_PID: u32 = i32::MAX as u32;

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

fn validate_number(value: f64, name: &str, min: f64, max: f64) -> Result<f64, SysprimsError> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(SysprimsError::invalid_argument(format!(
            "{name} must be a finite integer"
        )));
    }
    if value < min || value > max {
        return Err(SysprimsError::invalid_argument(format!(
            "{name} must be between {min:.0} and {max:.0}"
        )));
    }
    Ok(value)
}

fn validate_pid_number(value: f64, name: &str) -> Result<u32, SysprimsError> {
    validate_number(value, name, 1.0, MAX_SAFE_PID as f64).map(|value| value as u32)
}

fn validate_u32_number(value: f64, name: &str) -> Result<u32, SysprimsError> {
    validate_number(value, name, 0.0, u32::MAX as f64).map(|value| value as u32)
}

fn validate_i32_number(value: f64, name: &str) -> Result<i32, SysprimsError> {
    validate_number(value, name, i32::MIN as f64, i32::MAX as f64).map(|value| value as i32)
}

#[napi]
pub fn sysprims_abi_version() -> u32 {
    1
}

// -----------------------------------------------------------------------------
// Process Inspection
// -----------------------------------------------------------------------------

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ProcessOptionsWire {
    include_env: bool,
    include_threads: bool,
}

fn parse_process_options(options_json: &str) -> Result<ProcessOptions, SysprimsError> {
    if options_json.is_empty() || options_json == "{}" {
        return Ok(ProcessOptions::default());
    }

    let wire: ProcessOptionsWire = serde_json::from_str(options_json)
        .map_err(|e| SysprimsError::invalid_argument(format!("invalid options JSON: {}", e)))?;

    Ok(ProcessOptions {
        include_env: wire.include_env,
        include_threads: wire.include_threads,
    })
}

#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum CpuModeWire {
    #[default]
    Lifetime,
    Monitor,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DescendantsConfigWire {
    #[serde(flatten)]
    filter: ProcessFilter,
    cpu_mode: CpuModeWire,
    sample_duration_ms: Option<u64>,
    cascade: bool,
}

#[derive(Debug, Default)]
struct ParsedDescendantsConfig {
    filter: Option<ProcessFilter>,
    cpu_mode: CpuMode,
    sample_duration: Option<Duration>,
    cascade: bool,
}

#[derive(Debug, Clone, Copy, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum GuardActionKindWire {
    #[default]
    KillDescendants,
}

#[derive(Debug, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct GuardRuleWire {
    root_pid: u32,
    max_levels: u32,
    #[serde(flatten)]
    filter: ProcessFilter,
    cpu_mode: CpuModeWire,
    sample_duration_ms: Option<u64>,
}

impl Default for GuardRuleWire {
    fn default() -> Self {
        Self {
            root_pid: 0,
            max_levels: u32::MAX,
            filter: ProcessFilter::default(),
            cpu_mode: CpuModeWire::Lifetime,
            sample_duration_ms: None,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct GuardActionWire {
    kind: GuardActionKindWire,
    signal: i32,
    cascade: bool,
}

impl Default for GuardActionWire {
    fn default() -> Self {
        Self {
            kind: GuardActionKindWire::KillDescendants,
            signal: 15,
            cascade: false,
        }
    }
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct GuardConfigWire {
    rule: GuardRuleWire,
    action: GuardActionWire,
    action_enabled: bool,
    max_targets: Option<u32>,
}

fn process_filter_has_criteria(filter: &ProcessFilter) -> bool {
    filter.name_contains.is_some()
        || filter.name_equals.is_some()
        || filter.user_equals.is_some()
        || filter.pid_in.is_some()
        || filter.ppid.is_some()
        || filter.state_in.is_some()
        || filter.cpu_above.is_some()
        || filter.memory_above_kb.is_some()
        || filter.running_for_at_least_secs.is_some()
}

fn wire_cpu_mode_to_proc(mode: CpuModeWire) -> CpuMode {
    match mode {
        CpuModeWire::Lifetime => CpuMode::Lifetime,
        CpuModeWire::Monitor => CpuMode::Monitor,
    }
}

fn parse_descendants_config(config_json: &str) -> Result<ParsedDescendantsConfig, SysprimsError> {
    if config_json.is_empty() || config_json == "{}" {
        return Ok(ParsedDescendantsConfig::default());
    }

    let wire: DescendantsConfigWire = serde_json::from_str(config_json)
        .map_err(|e| SysprimsError::invalid_argument(format!("invalid config JSON: {}", e)))?;

    let filter = if process_filter_has_criteria(&wire.filter) {
        Some(wire.filter)
    } else {
        None
    };

    Ok(ParsedDescendantsConfig {
        filter,
        cpu_mode: wire_cpu_mode_to_proc(wire.cpu_mode),
        sample_duration: wire.sample_duration_ms.map(Duration::from_millis),
        cascade: wire.cascade,
    })
}

fn parse_guard_config(config_json: &str) -> Result<GuardConfig, SysprimsError> {
    if config_json.is_empty() || config_json == "{}" {
        return Err(SysprimsError::invalid_argument(
            "guard config JSON cannot be empty",
        ));
    }

    let wire: GuardConfigWire = serde_json::from_str(config_json).map_err(|e| {
        SysprimsError::invalid_argument(format!("invalid guard config JSON: {}", e))
    })?;

    let filter = if process_filter_has_criteria(&wire.rule.filter) {
        Some(wire.rule.filter)
    } else {
        None
    };

    let action = match wire.action.kind {
        GuardActionKindWire::KillDescendants => GuardAction::KillDescendants {
            signal: wire.action.signal,
            cascade: wire.action.cascade,
        },
    };
    let max_targets = match wire.max_targets {
        Some(0) => return Err(SysprimsError::invalid_argument("max_targets must be >= 1")),
        Some(value) => value,
        None => 64,
    };

    Ok(GuardConfig {
        rule: GuardRule {
            root_pid: wire.rule.root_pid,
            max_levels: wire.rule.max_levels,
            filter,
            cpu_mode: wire_cpu_mode_to_proc(wire.rule.cpu_mode),
            sample_duration: wire.rule.sample_duration_ms.map(Duration::from_millis),
        },
        action,
        action_enabled: wire.action_enabled,
        max_targets,
    })
}

#[napi]
pub fn sysprims_proc_get(pid: f64) -> SysprimsCallJsonResult {
    sysprims_proc_get_ex(pid, String::new())
}

#[napi]
pub fn sysprims_proc_get_ex(pid: f64, options_json: String) -> SysprimsCallJsonResult {
    let pid = match validate_pid_number(pid, "pid") {
        Ok(value) => value,
        Err(e) => return err_json(e),
    };
    let options = match parse_process_options(&options_json) {
        Ok(o) => o,
        Err(e) => return err_json(e),
    };

    match sysprims_proc::get_process_with_options(pid, options) {
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
    sysprims_proc_list_ex(filter_json, String::new())
}

#[napi]
pub fn sysprims_proc_list_ex(filter_json: String, options_json: String) -> SysprimsCallJsonResult {
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

    let options = match parse_process_options(&options_json) {
        Ok(o) => o,
        Err(e) => return err_json(e),
    };

    match sysprims_proc::snapshot_filtered_with_options(&filter, options) {
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
pub fn sysprims_proc_list_fds(pid: f64, filter_json: String) -> SysprimsCallJsonResult {
    let pid = match validate_pid_number(pid, "pid") {
        Ok(value) => value,
        Err(e) => return err_json(e),
    };
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
pub fn sysprims_proc_wait_pid(pid: f64, timeout_ms: f64) -> SysprimsCallJsonResult {
    let pid = match validate_pid_number(pid, "pid") {
        Ok(value) => value,
        Err(e) => return err_json(e),
    };
    let timeout_ms = match validate_u32_number(timeout_ms, "timeout_ms") {
        Ok(value) => value,
        Err(e) => return err_json(e),
    };
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
// Descendants
// -----------------------------------------------------------------------------

#[napi]
pub fn sysprims_proc_descendants(
    root_pid: f64,
    max_levels: f64,
    config_json: String,
    options_json: Option<String>,
) -> SysprimsCallJsonResult {
    let root_pid = match validate_pid_number(root_pid, "root_pid") {
        Ok(value) => value,
        Err(e) => return err_json(e),
    };
    let max_levels = match validate_u32_number(max_levels, "max_levels") {
        Ok(value) => value,
        Err(e) => return err_json(e),
    };
    let parsed = match parse_descendants_config(&config_json) {
        Ok(c) => c,
        Err(e) => return err_json(e),
    };
    let options = match parse_process_options(options_json.as_deref().unwrap_or_default()) {
        Ok(o) => o,
        Err(e) => return err_json(e),
    };

    let config = DescendantsConfig {
        root_pid,
        max_levels: Some(max_levels),
        filter: parsed.filter,
        cpu_mode: parsed.cpu_mode,
        sample_duration: parsed.sample_duration,
    };

    match descendants_with_config_and_options(config, options) {
        Ok(result) => match serde_json::to_string(&result) {
            Ok(json) => ok_json(json),
            Err(e) => err_json(SysprimsError::internal(format!(
                "failed to serialize descendants result: {}",
                e
            ))),
        },
        Err(e) => err_json(e),
    }
}

#[napi]
pub fn sysprims_proc_kill_descendants(
    root_pid: f64,
    max_levels: f64,
    signal: f64,
    config_json: String,
) -> SysprimsCallJsonResult {
    let root_pid = match validate_pid_number(root_pid, "root_pid") {
        Ok(value) => value,
        Err(e) => return err_json(e),
    };
    let max_levels = match validate_u32_number(max_levels, "max_levels") {
        Ok(value) => value,
        Err(e) => return err_json(e),
    };
    let signal = match validate_i32_number(signal, "signal") {
        Ok(value) => value,
        Err(e) => return err_json(e),
    };
    let parsed = match parse_descendants_config(&config_json) {
        Ok(c) => c,
        Err(e) => return err_json(e),
    };

    let config = DescendantsConfig {
        root_pid,
        max_levels: Some(max_levels),
        filter: None,
        cpu_mode: parsed.cpu_mode,
        sample_duration: parsed.sample_duration,
    };

    // Traverse descendants before sending any signal.
    let desc_result = match descendants_with_config_and_options(config, ProcessOptions::default()) {
        Ok(r) => r,
        Err(e) => return err_json(e),
    };

    // Select matched targets (optionally expanded to matched subtrees).
    let mut target_pids = sysprims_proc::select_descendant_targets(
        &desc_result,
        parsed.filter.as_ref(),
        parsed.cascade,
    );

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
        (Vec::new(), Vec::<KillDescendantsFailureWire>::new())
    } else {
        match sysprims_signal::kill_many(&target_pids, signal) {
            Ok(batch) => {
                let failed_entries: Vec<KillDescendantsFailureWire> = batch
                    .failed
                    .iter()
                    .map(|f| KillDescendantsFailureWire {
                        pid: f.pid,
                        error: f.error.to_string(),
                    })
                    .collect();
                (batch.succeeded, failed_entries)
            }
            Err(e) => return err_json(e),
        }
    };

    let result = KillDescendantsResultWire {
        schema_id: sysprims_core::schema::BATCH_KILL_RESULT_V1.to_string(),
        signal_sent: signal,
        root_pid,
        succeeded,
        failed,
        skipped_safety,
    };

    match serde_json::to_string(&result) {
        Ok(json) => ok_json(json),
        Err(e) => err_json(SysprimsError::internal(format!(
            "failed to serialize kill-descendants result: {}",
            e
        ))),
    }
}

#[napi]
pub fn sysprims_proc_guard_step(config_json: String) -> SysprimsCallJsonResult {
    let config = match parse_guard_config(&config_json) {
        Ok(c) => c,
        Err(e) => return err_json(e),
    };

    match guard_step(config) {
        Ok(event) => match serde_json::to_string(&event) {
            Ok(json) => ok_json(json),
            Err(e) => err_json(SysprimsError::internal(format!(
                "failed to serialize guard event: {}",
                e
            ))),
        },
        Err(e) => err_json(e),
    }
}

#[napi]
pub fn sysprims_proc_ancestors(
    pid: f64,
    max_depth: f64,
    options_json: String,
) -> SysprimsCallJsonResult {
    let pid = match validate_pid_number(pid, "pid") {
        Ok(value) => value,
        Err(e) => return err_json(e),
    };
    let max_depth = match validate_u32_number(max_depth, "max_depth") {
        Ok(value) => value,
        Err(e) => return err_json(e),
    };
    let options = if options_json.is_empty() {
        ProcessOptions::default()
    } else {
        match serde_json::from_str::<ProcessOptionsWire>(&options_json) {
            Ok(w) => {
                let mut opts = ProcessOptions::default();
                if w.include_env {
                    opts = opts.with_env();
                }
                if w.include_threads {
                    opts = opts.with_threads();
                }
                opts
            }
            Err(e) => {
                return err_json(SysprimsError::invalid_argument(format!(
                    "invalid options JSON: {}",
                    e
                )))
            }
        }
    };

    match ancestors(pid, max_depth, options) {
        Ok(result) => match serde_json::to_string(&result) {
            Ok(json) => ok_json(json),
            Err(e) => err_json(SysprimsError::internal(format!(
                "failed to serialize ancestors result: {}",
                e
            ))),
        },
        Err(e) => err_json(e),
    }
}

#[derive(serde::Serialize)]
struct KillDescendantsFailureWire {
    pid: u32,
    error: String,
}

#[derive(serde::Serialize)]
struct KillDescendantsResultWire {
    schema_id: String,
    signal_sent: i32,
    root_pid: u32,
    succeeded: Vec<u32>,
    failed: Vec<KillDescendantsFailureWire>,
    skipped_safety: usize,
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
pub fn sysprims_signal_send(pid: f64, signal: f64) -> SysprimsCallVoidResult {
    let pid = match validate_pid_number(pid, "pid") {
        Ok(value) => value,
        Err(e) => return err_void(e),
    };
    let signal = match validate_i32_number(signal, "signal") {
        Ok(value) => value,
        Err(e) => return err_void(e),
    };
    match sysprims_signal::kill(pid, signal) {
        Ok(()) => ok_void(),
        Err(e) => err_void(e),
    }
}

#[napi]
pub fn sysprims_signal_send_group(pgid: f64, signal: f64) -> SysprimsCallVoidResult {
    let pgid = match validate_pid_number(pgid, "pgid") {
        Ok(value) => value,
        Err(e) => return err_void(e),
    };
    let signal = match validate_i32_number(signal, "signal") {
        Ok(value) => value,
        Err(e) => return err_void(e),
    };
    match sysprims_signal::killpg(pgid, signal) {
        Ok(()) => ok_void(),
        Err(e) => err_void(e),
    }
}

#[napi]
pub fn sysprims_terminate(pid: f64) -> SysprimsCallVoidResult {
    let pid = match validate_pid_number(pid, "pid") {
        Ok(value) => value,
        Err(e) => return err_void(e),
    };
    match sysprims_signal::terminate(pid) {
        Ok(()) => ok_void(),
        Err(e) => err_void(e),
    }
}

#[napi]
pub fn sysprims_force_kill(pid: f64) -> SysprimsCallVoidResult {
    let pid = match validate_pid_number(pid, "pid") {
        Ok(value) => value,
        Err(e) => return err_void(e),
    };
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
pub fn sysprims_terminate_tree(pid: f64, config_json: String) -> SysprimsCallJsonResult {
    let pid = match validate_pid_number(pid, "pid") {
        Ok(value) => value,
        Err(e) => return err_json(e),
    };
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
// Session Spawn
// -----------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRunSetsidConfig {
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
#[cfg_attr(not(unix), allow(dead_code))]
#[serde(deny_unknown_fields)]
struct WireRunNohupConfig {
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
struct SessionSpawnResultWire {
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

#[napi]
pub fn sysprims_run_setsid(config_json: String) -> SysprimsCallJsonResult {
    if config_json.is_empty() {
        return err_json(SysprimsError::invalid_argument(
            "config_json cannot be empty",
        ));
    }

    let wire = match serde_json::from_str::<WireRunSetsidConfig>(&config_json) {
        Ok(v) => v,
        Err(e) => {
            return err_json(SysprimsError::invalid_argument(format!(
                "invalid config JSON: {}",
                e
            )))
        }
    };

    if wire.schema_id != RUN_SETSID_CONFIG_V1 {
        return err_json(SysprimsError::invalid_argument(format!(
            "invalid schema_id (expected {})",
            RUN_SETSID_CONFIG_V1
        )));
    }

    let (command, args) = match split_argv(&wire.argv) {
        Ok(v) => v,
        Err(e) => return err_json(e),
    };

    let config = sysprims_session::SetsidConfig {
        wait: wire.wait,
        ctty: false,
        cwd: wire.cwd.map(PathBuf::from),
        env: wire.env,
    };

    match sysprims_session::run_setsid(command, &args, config)
        .and_then(session_setsid_result_from_outcome)
    {
        Ok(result) => match serde_json::to_string(&result) {
            Ok(json) => ok_json(json),
            Err(e) => err_json(SysprimsError::internal(format!(
                "failed to serialize session spawn result: {}",
                e
            ))),
        },
        Err(e) => err_json(e),
    }
}

#[napi]
pub fn sysprims_run_nohup(config_json: String) -> SysprimsCallJsonResult {
    if config_json.is_empty() {
        return err_json(SysprimsError::invalid_argument(
            "config_json cannot be empty",
        ));
    }

    let wire = match serde_json::from_str::<WireRunNohupConfig>(&config_json) {
        Ok(v) => v,
        Err(e) => {
            return err_json(SysprimsError::invalid_argument(format!(
                "invalid config JSON: {}",
                e
            )))
        }
    };

    if wire.schema_id != RUN_NOHUP_CONFIG_V1 {
        return err_json(SysprimsError::invalid_argument(format!(
            "invalid schema_id (expected {})",
            RUN_NOHUP_CONFIG_V1
        )));
    }

    let (command, args) = match split_argv(&wire.argv) {
        Ok(v) => v,
        Err(e) => return err_json(e),
    };

    #[cfg(not(unix))]
    {
        let _ = (command, args);
        err_json(SysprimsError::not_supported("nohup", std::env::consts::OS))
    }

    #[cfg(unix)]
    {
        let caller_sid = match sysprims_session::getsid(0) {
            Ok(sid) => sid,
            Err(e) => return err_json(e),
        };
        let caller_pgid = match sysprims_session::getpgid(0) {
            Ok(pgid) => pgid,
            Err(e) => return err_json(e),
        };

        let config = sysprims_session::NohupConfig {
            wait: wire.wait,
            output_file: wire.output_file,
            cwd: wire.cwd.map(PathBuf::from),
            env: wire.env,
        };

        match sysprims_session::run_nohup(command, &args, config)
            .and_then(|outcome| session_nohup_result_from_outcome(outcome, caller_sid, caller_pgid))
        {
            Ok(result) => match serde_json::to_string(&result) {
                Ok(json) => ok_json(json),
                Err(e) => err_json(SysprimsError::internal(format!(
                    "failed to serialize session spawn result: {}",
                    e
                ))),
            },
            Err(e) => err_json(e),
        }
    }
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

fn session_setsid_result_from_outcome(
    outcome: sysprims_session::SetsidOutcome,
) -> Result<SessionSpawnResultWire, SysprimsError> {
    match outcome {
        sysprims_session::SetsidOutcome::Spawned { child_pid } => {
            validate_output_pid(child_pid, "child_pid")?;
            Ok(SessionSpawnResultWire {
                schema_id: SESSION_SPAWN_RESULT_V1,
                timestamp: sysprims_core::time::now_rfc3339(),
                platform: sysprims_core::get_platform(),
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
        sysprims_session::SetsidOutcome::Completed {
            child_pid,
            exit_status,
        } => {
            validate_output_pid(child_pid, "child_pid")?;
            Ok(SessionSpawnResultWire {
                schema_id: SESSION_SPAWN_RESULT_V1,
                timestamp: sysprims_core::time::now_rfc3339(),
                platform: sysprims_core::get_platform(),
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
fn session_nohup_result_from_outcome(
    outcome: sysprims_session::NohupOutcome,
    caller_sid: u32,
    caller_pgid: u32,
) -> Result<SessionSpawnResultWire, SysprimsError> {
    validate_output_pid(caller_sid, "caller_sid")?;
    validate_output_pid(caller_pgid, "caller_pgid")?;

    match outcome {
        sysprims_session::NohupOutcome::Spawned {
            child_pid,
            output_file,
        } => {
            validate_output_pid(child_pid, "child_pid")?;
            Ok(SessionSpawnResultWire {
                schema_id: SESSION_SPAWN_RESULT_V1,
                timestamp: sysprims_core::time::now_rfc3339(),
                platform: sysprims_core::get_platform(),
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
        sysprims_session::NohupOutcome::Completed {
            child_pid,
            exit_status,
            output_file,
        } => {
            validate_output_pid(child_pid, "child_pid")?;
            Ok(SessionSpawnResultWire {
                schema_id: SESSION_SPAWN_RESULT_V1,
                timestamp: sysprims_core::time::now_rfc3339(),
                platform: sysprims_core::get_platform(),
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

#[cfg(unix)]
fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn exit_signal(_status: &std::process::ExitStatus) -> Option<i32> {
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_boundary_rejects_lossy_js_numbers() {
        for value in [f64::NAN, f64::INFINITY, -1.0, 1.5, 4_294_967_297.0] {
            assert!(validate_pid_number(value, "pid").is_err());
        }
        assert!(validate_u32_number(4_294_967_297.0, "depth").is_err());
        assert!(validate_i32_number(2_147_483_648.0, "signal").is_err());
        assert_eq!(validate_pid_number(1.0, "pid").unwrap(), 1);
        assert_eq!(
            validate_pid_number(MAX_SAFE_PID as f64, "pid").unwrap(),
            MAX_SAFE_PID
        );
    }

    #[test]
    fn guard_max_targets_distinguishes_omitted_from_zero() {
        let defaulted = parse_guard_config(&format!(
            r#"{{"rule":{{"root_pid":{}}},"action_enabled":false}}"#,
            std::process::id()
        ))
        .unwrap();
        assert_eq!(defaulted.max_targets, 64);

        let explicit_zero = parse_guard_config(&format!(
            r#"{{"rule":{{"root_pid":{}}},"action_enabled":false,"max_targets":0}}"#,
            std::process::id()
        ));
        assert!(explicit_zero.is_err());
    }
}
