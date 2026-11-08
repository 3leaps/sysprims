// SPDX-License-Identifier: MIT OR Apache-2.0

//! Native PyO3 `cp310-abi3` extension module (`sysprims._sysprims`).
//!
//! Exposes `sysprims-core`, `sysprims-proc`, `sysprims-signal`,
//! `sysprims-session`, `sysprims-timeout`, and `sysprims-recurrence` to the
//! typed `sysprims` Python package. Blocking and OS inspection operations
//! detach from the Python GIL via `Python::detach`.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Duration;

use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use serde::{Deserialize, Serialize};
use sysprims_core::schema::{
    BATCH_KILL_RESULT_V1, CONTAINMENT_SPAWN_CONFIG_V1, RUN_NOHUP_CONFIG_V1, RUN_SETSID_CONFIG_V1,
    SESSION_SPAWN_RESULT_V1, SPAWN_IN_GROUP_CONFIG_V1, TERMINATE_TREE_CONFIG_V1,
    TIMEOUT_RESULT_V1_1,
};
use sysprims_core::SysprimsError;
use sysprims_proc::{
    ancestors, descendants_with_config_and_options, guard_step, CpuMode, DescendantsConfig,
    FdFilter, GuardAction, GuardConfig, GuardRule, PortFilter, ProcessFilter, ProcessOptions,
};
use sysprims_recurrence::{
    Instant as RecurrenceInstant, QueryError, RecurrenceRule, Refusal as RecurrenceRefusalErr,
    CONTRACT as RECURRENCE_CONTRACT_STR, CORPUS_DIGEST as RECURRENCE_CORPUS_DIGEST_STR,
};
use sysprims_timeout::{
    containment_close as timeout_containment_close,
    containment_identity as timeout_containment_identity,
    containment_poll as timeout_containment_poll, containment_spawn as timeout_containment_spawn,
    containment_terminate as timeout_containment_terminate,
    containment_wait as timeout_containment_wait, run_with_timeout as timeout_run_with_timeout,
    spawn_in_group as timeout_spawn_in_group, terminate_tree as timeout_terminate_tree,
    GroupingMode, ManagedSpawnRequest, SpawnInGroupConfig, TerminateTreeConfig, TimeoutConfig,
    TimeoutOutcome,
};

const MAX_SAFE_PID: u32 = i32::MAX as u32;

create_exception!(_sysprims, RawSysprimsError, PyException);
create_exception!(_sysprims, RawRecurrenceRefusal, PyException);
create_exception!(_sysprims, RawRecurrenceQueryError, PyException);

fn map_error_code(err: &SysprimsError) -> i32 {
    match err.error_code() {
        1..=8 => err.error_code(),
        _ => 99,
    }
}

fn py_sysprims_err(err: SysprimsError) -> PyErr {
    RawSysprimsError::new_err((map_error_code(&err), err.to_string()))
}

fn py_recurrence_refusal(err: RecurrenceRefusalErr) -> PyErr {
    RawRecurrenceRefusal::new_err((err.reason().as_str(), err.detail()))
}

fn py_recurrence_query_err(err: QueryError) -> PyErr {
    let kind = match err {
        QueryError::ReversedRange => "reversed_range",
        QueryError::SearchLimitExceeded => "search_limit_exceeded",
        _ => "query_error",
    };
    RawRecurrenceQueryError::new_err((kind, err.to_string()))
}

fn serialize_json<T: Serialize>(value: &T, label: &str) -> Result<String, SysprimsError> {
    serde_json::to_string(value)
        .map_err(|err| SysprimsError::internal(format!("failed to serialize {label}: {err}")))
}

fn validate_pid(value: i128, name: &str) -> Result<u32, SysprimsError> {
    if value < 1 || value > i128::from(MAX_SAFE_PID) {
        return Err(SysprimsError::invalid_argument(format!(
            "{name} must be between 1 and {MAX_SAFE_PID}"
        )));
    }
    Ok(value as u32)
}

fn validate_u32(value: i128, name: &str) -> Result<u32, SysprimsError> {
    if value < 0 || value > i128::from(u32::MAX) {
        return Err(SysprimsError::invalid_argument(format!(
            "{name} must be between 0 and {}",
            u32::MAX
        )));
    }
    Ok(value as u32)
}

fn validate_i32(value: i128, name: &str) -> Result<i32, SysprimsError> {
    if value < i128::from(i32::MIN) || value > i128::from(i32::MAX) {
        return Err(SysprimsError::invalid_argument(format!(
            "{name} must be between {} and {}",
            i32::MIN,
            i32::MAX
        )));
    }
    Ok(value as i32)
}

fn validate_u64(value: i128, name: &str) -> Result<u64, SysprimsError> {
    if value < 0 || value > i128::from(u64::MAX) {
        return Err(SysprimsError::invalid_argument(format!(
            "{name} must be between 0 and {}",
            u64::MAX
        )));
    }
    Ok(value as u64)
}

// =============================================================================
// Process Inspection & Guard
// =============================================================================

#[derive(Debug, Default, Deserialize)]
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
        .map_err(|err| SysprimsError::invalid_argument(format!("invalid options JSON: {err}")))?;
    Ok(ProcessOptions {
        include_env: wire.include_env,
        include_threads: wire.include_threads,
    })
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CpuModeWire {
    #[default]
    Lifetime,
    Monitor,
}

#[derive(Debug, Default, Deserialize)]
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

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum GuardActionKindWire {
    #[default]
    KillDescendants,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Default, Deserialize)]
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
        .map_err(|err| SysprimsError::invalid_argument(format!("invalid config JSON: {err}")))?;
    wire.filter.validate()?;
    let filter = process_filter_has_criteria(&wire.filter).then_some(wire.filter);
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
    let wire: GuardConfigWire = serde_json::from_str(config_json).map_err(|err| {
        SysprimsError::invalid_argument(format!("invalid guard config JSON: {err}"))
    })?;
    wire.rule.filter.validate()?;
    let filter = process_filter_has_criteria(&wire.rule.filter).then_some(wire.rule.filter);
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

#[derive(Serialize)]
struct BatchFailureWire {
    pid: u32,
    error: String,
}

#[derive(Serialize)]
struct KillDescendantsResultWire {
    schema_id: &'static str,
    signal_sent: i32,
    root_pid: u32,
    succeeded: Vec<u32>,
    failed: Vec<BatchFailureWire>,
    skipped_safety: usize,
}

#[derive(Serialize)]
struct BatchKillResultWire {
    schema_id: &'static str,
    signal_sent: i32,
    succeeded: Vec<u32>,
    failed: Vec<BatchFailureWire>,
}

#[pyfunction]
fn abi_version() -> u32 {
    1
}

#[pyfunction]
fn platform() -> &'static str {
    sysprims_core::get_platform()
}

#[pyfunction]
fn proc_get(py: Python<'_>, pid: i128, options_json: &str) -> PyResult<String> {
    let pid = validate_pid(pid, "pid").map_err(py_sysprims_err)?;
    let options = parse_process_options(options_json).map_err(py_sysprims_err)?;
    py.detach(|| {
        let info = sysprims_proc::get_process_with_options(pid, options)?;
        serialize_json(&info, "process info")
    })
    .map_err(py_sysprims_err)
}

#[pyfunction]
fn proc_list(py: Python<'_>, filter_json: &str, options_json: &str) -> PyResult<String> {
    let filter = if filter_json.is_empty() || filter_json == "{}" {
        ProcessFilter::default()
    } else {
        serde_json::from_str::<ProcessFilter>(filter_json).map_err(|err| {
            py_sysprims_err(SysprimsError::invalid_argument(format!(
                "invalid filter JSON: {err}"
            )))
        })?
    };
    filter.validate().map_err(py_sysprims_err)?;
    let options = parse_process_options(options_json).map_err(py_sysprims_err)?;
    py.detach(|| {
        let snapshot = sysprims_proc::snapshot_filtered_with_options(&filter, options)?;
        serialize_json(&snapshot, "process snapshot")
    })
    .map_err(py_sysprims_err)
}

#[pyfunction]
fn proc_listening_ports(py: Python<'_>, filter_json: &str) -> PyResult<String> {
    let filter = if filter_json.is_empty() || filter_json == "{}" {
        PortFilter::default()
    } else {
        serde_json::from_str::<PortFilter>(filter_json).map_err(|err| {
            py_sysprims_err(SysprimsError::invalid_argument(format!(
                "invalid filter JSON: {err}"
            )))
        })?
    };
    filter.validate().map_err(py_sysprims_err)?;
    py.detach(|| {
        let snapshot = sysprims_proc::listening_ports(Some(&filter))?;
        serialize_json(&snapshot, "port bindings")
    })
    .map_err(py_sysprims_err)
}

#[pyfunction]
fn proc_list_fds(py: Python<'_>, pid: i128, filter_json: &str) -> PyResult<String> {
    let pid = validate_pid(pid, "pid").map_err(py_sysprims_err)?;
    let filter = if filter_json.is_empty() || filter_json == "{}" {
        FdFilter::default()
    } else {
        serde_json::from_str::<FdFilter>(filter_json).map_err(|err| {
            py_sysprims_err(SysprimsError::invalid_argument(format!(
                "invalid filter JSON: {err}"
            )))
        })?
    };
    filter.validate().map_err(py_sysprims_err)?;
    py.detach(|| {
        let snapshot = sysprims_proc::list_fds(pid, Some(&filter))?;
        serialize_json(&snapshot, "fd snapshot")
    })
    .map_err(py_sysprims_err)
}

#[pyfunction]
fn proc_wait_pid(py: Python<'_>, pid: i128, timeout_ms: i128) -> PyResult<String> {
    let pid = validate_pid(pid, "pid").map_err(py_sysprims_err)?;
    let timeout_ms = validate_u64(timeout_ms, "timeout_ms").map_err(py_sysprims_err)?;
    py.detach(|| {
        let result = sysprims_proc::wait_pid(pid, Duration::from_millis(timeout_ms))?;
        serialize_json(&result, "wait result")
    })
    .map_err(py_sysprims_err)
}

#[pyfunction]
fn proc_descendants(
    py: Python<'_>,
    root_pid: i128,
    max_levels: i128,
    config_json: &str,
    options_json: &str,
) -> PyResult<String> {
    let root_pid = validate_pid(root_pid, "root_pid").map_err(py_sysprims_err)?;
    let max_levels = validate_u32(max_levels, "max_levels").map_err(py_sysprims_err)?;
    let parsed = parse_descendants_config(config_json).map_err(py_sysprims_err)?;
    let options = parse_process_options(options_json).map_err(py_sysprims_err)?;
    let config = DescendantsConfig {
        root_pid,
        max_levels: Some(max_levels),
        filter: parsed.filter,
        cpu_mode: parsed.cpu_mode,
        sample_duration: parsed.sample_duration,
    };
    py.detach(|| {
        let result = descendants_with_config_and_options(config, options)?;
        serialize_json(&result, "descendants result")
    })
    .map_err(py_sysprims_err)
}

#[pyfunction]
fn proc_kill_descendants(
    py: Python<'_>,
    root_pid: i128,
    max_levels: i128,
    signal: i128,
    config_json: &str,
) -> PyResult<String> {
    let root_pid = validate_pid(root_pid, "root_pid").map_err(py_sysprims_err)?;
    let max_levels = validate_u32(max_levels, "max_levels").map_err(py_sysprims_err)?;
    let signal = validate_i32(signal, "signal").map_err(py_sysprims_err)?;
    let parsed = parse_descendants_config(config_json).map_err(py_sysprims_err)?;
    let config = DescendantsConfig {
        root_pid,
        max_levels: Some(max_levels),
        filter: None,
        cpu_mode: parsed.cpu_mode,
        sample_duration: parsed.sample_duration,
    };
    py.detach(|| {
        let desc_result = descendants_with_config_and_options(config, ProcessOptions::default())?;
        let mut target_pids = sysprims_proc::select_descendant_targets(
            &desc_result,
            parsed.filter.as_ref(),
            parsed.cascade,
        );
        let self_pid = std::process::id();
        let parent_pid = sysprims_proc::get_process(self_pid).ok().map(|p| p.ppid);
        let before = target_pids.len();
        target_pids.retain(|&pid| pid != self_pid && pid != 1);
        if let Some(ppid) = parent_pid {
            target_pids.retain(|&pid| pid != ppid);
        }
        let skipped_safety = before.saturating_sub(target_pids.len());
        let (succeeded, failed) = if target_pids.is_empty() {
            (Vec::new(), Vec::new())
        } else {
            let batch = sysprims_signal::kill_many(&target_pids, signal)?;
            let failed_entries = batch
                .failed
                .into_iter()
                .map(|f| BatchFailureWire {
                    pid: f.pid,
                    error: f.error.to_string(),
                })
                .collect();
            (batch.succeeded, failed_entries)
        };
        let result = KillDescendantsResultWire {
            schema_id: BATCH_KILL_RESULT_V1,
            signal_sent: signal,
            root_pid,
            succeeded,
            failed,
            skipped_safety,
        };
        serialize_json(&result, "kill-descendants result")
    })
    .map_err(py_sysprims_err)
}

#[pyfunction]
fn proc_guard_step(py: Python<'_>, config_json: &str) -> PyResult<String> {
    let config = parse_guard_config(config_json).map_err(py_sysprims_err)?;
    py.detach(|| {
        let event = guard_step(config)?;
        serialize_json(&event, "guard event")
    })
    .map_err(py_sysprims_err)
}

#[pyfunction]
fn proc_ancestors(
    py: Python<'_>,
    pid: i128,
    max_depth: i128,
    options_json: &str,
) -> PyResult<String> {
    let pid = validate_pid(pid, "pid").map_err(py_sysprims_err)?;
    let max_depth = validate_u32(max_depth, "max_depth").map_err(py_sysprims_err)?;
    let options = parse_process_options(options_json).map_err(py_sysprims_err)?;
    py.detach(|| {
        let result = ancestors(pid, max_depth, options)?;
        serialize_json(&result, "ancestors result")
    })
    .map_err(py_sysprims_err)
}

// =============================================================================
// Self Introspection & Signals
// =============================================================================

#[pyfunction]
fn self_getpgid() -> PyResult<u32> {
    #[cfg(unix)]
    {
        sysprims_session::getpgid(0).map_err(py_sysprims_err)
    }
    #[cfg(windows)]
    {
        Err(py_sysprims_err(SysprimsError::not_supported(
            "getpgid", "windows",
        )))
    }
}

#[pyfunction]
fn self_getsid() -> PyResult<u32> {
    #[cfg(unix)]
    {
        sysprims_session::getsid(0).map_err(py_sysprims_err)
    }
    #[cfg(windows)]
    {
        Err(py_sysprims_err(SysprimsError::not_supported(
            "getsid", "windows",
        )))
    }
}

#[pyfunction]
fn signal_send(py: Python<'_>, pid: i128, signal: i128) -> PyResult<()> {
    let pid = validate_pid(pid, "pid").map_err(py_sysprims_err)?;
    let signal = validate_i32(signal, "signal").map_err(py_sysprims_err)?;
    py.detach(|| sysprims_signal::kill(pid, signal))
        .map_err(py_sysprims_err)
}

#[pyfunction]
fn signal_send_group(py: Python<'_>, pgid: i128, signal: i128) -> PyResult<()> {
    let pgid = validate_pid(pgid, "pgid").map_err(py_sysprims_err)?;
    let signal = validate_i32(signal, "signal").map_err(py_sysprims_err)?;
    py.detach(|| sysprims_signal::killpg(pgid, signal))
        .map_err(py_sysprims_err)
}

#[pyfunction]
fn terminate(py: Python<'_>, pid: i128) -> PyResult<()> {
    let pid = validate_pid(pid, "pid").map_err(py_sysprims_err)?;
    py.detach(|| sysprims_signal::terminate(pid))
        .map_err(py_sysprims_err)
}

#[pyfunction]
fn force_kill(py: Python<'_>, pid: i128) -> PyResult<()> {
    let pid = validate_pid(pid, "pid").map_err(py_sysprims_err)?;
    py.detach(|| sysprims_signal::force_kill(pid))
        .map_err(py_sysprims_err)
}

#[pyfunction]
fn signal_kill_many(py: Python<'_>, pids: Vec<i128>, signal: i128) -> PyResult<String> {
    if pids.is_empty() {
        return Err(py_sysprims_err(SysprimsError::invalid_argument(
            "pids must be a non-empty sequence",
        )));
    }
    let mut validated = Vec::with_capacity(pids.len());
    for (idx, raw) in pids.into_iter().enumerate() {
        validated.push(validate_pid(raw, &format!("pids[{idx}]")).map_err(py_sysprims_err)?);
    }
    let signal = validate_i32(signal, "signal").map_err(py_sysprims_err)?;
    py.detach(|| {
        let batch = sysprims_signal::kill_many(&validated, signal)?;
        let result = BatchKillResultWire {
            schema_id: BATCH_KILL_RESULT_V1,
            signal_sent: signal,
            succeeded: batch.succeeded,
            failed: batch
                .failed
                .into_iter()
                .map(|f| BatchFailureWire {
                    pid: f.pid,
                    error: f.error.to_string(),
                })
                .collect(),
        };
        serialize_json(&result, "batch kill result")
    })
    .map_err(py_sysprims_err)
}

// =============================================================================
// Terminate Tree & Timeout Execution
// =============================================================================

#[derive(Debug, Deserialize)]
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

#[pyfunction]
fn terminate_tree(py: Python<'_>, pid: i128, config_json: &str) -> PyResult<String> {
    let pid = validate_pid(pid, "pid").map_err(py_sysprims_err)?;
    let cfg = if config_json.is_empty() || config_json == "{}" {
        TerminateTreeConfig::default()
    } else {
        let wire: WireTerminateTreeConfig = serde_json::from_str(config_json).map_err(|err| {
            py_sysprims_err(SysprimsError::invalid_argument(format!(
                "invalid config JSON: {err}"
            )))
        })?;
        if wire.schema_id != TERMINATE_TREE_CONFIG_V1 {
            return Err(py_sysprims_err(SysprimsError::invalid_argument(format!(
                "invalid schema_id (expected {TERMINATE_TREE_CONFIG_V1})"
            ))));
        }
        wire.into()
    };
    py.detach(|| {
        let result = timeout_terminate_tree(pid, cfg)?;
        serialize_json(&result, "terminate-tree result")
    })
    .map_err(py_sysprims_err)
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireGroupingMode {
    #[default]
    GroupByDefault,
    Foreground,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRunWithTimeoutConfig {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    timeout_ms: u64,
    #[serde(default)]
    kill_after_ms: Option<u64>,
    #[serde(default)]
    signal: Option<i32>,
    #[serde(default)]
    grouping: WireGroupingMode,
    #[serde(default)]
    preserve_status: bool,
}

#[derive(Debug, Serialize)]
struct TimeoutResultWire {
    schema_id: &'static str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signal_sent: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    escalated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tree_kill_reliability: Option<String>,
}

#[pyfunction]
fn run_with_timeout(py: Python<'_>, config_json: &str) -> PyResult<String> {
    if config_json.is_empty() {
        return Err(py_sysprims_err(SysprimsError::invalid_argument(
            "config_json cannot be empty",
        )));
    }
    let wire: WireRunWithTimeoutConfig = serde_json::from_str(config_json).map_err(|err| {
        py_sysprims_err(SysprimsError::invalid_argument(format!(
            "invalid timeout config JSON: {err}"
        )))
    })?;
    if wire.command.is_empty() {
        return Err(py_sysprims_err(SysprimsError::invalid_argument(
            "command must not be empty",
        )));
    }
    let mut cfg = TimeoutConfig::default();
    if let Some(kill_after_ms) = wire.kill_after_ms {
        cfg.kill_after = Duration::from_millis(kill_after_ms);
    }
    if let Some(signal) = wire.signal {
        cfg.signal = signal;
    }
    cfg.grouping = match wire.grouping {
        WireGroupingMode::GroupByDefault => GroupingMode::GroupByDefault,
        WireGroupingMode::Foreground => GroupingMode::Foreground,
    };
    cfg.preserve_status = wire.preserve_status;
    let timeout = Duration::from_millis(wire.timeout_ms);
    py.detach(|| {
        let arg_refs: Vec<&str> = wire.args.iter().map(String::as_str).collect();
        let outcome = timeout_run_with_timeout(&wire.command, &arg_refs, timeout, cfg)?;
        let result = match outcome {
            TimeoutOutcome::Completed { exit_status } => TimeoutResultWire {
                schema_id: TIMEOUT_RESULT_V1_1,
                status: "completed",
                exit_code: exit_status.code(),
                signal_sent: None,
                escalated: None,
                tree_kill_reliability: None,
            },
            TimeoutOutcome::TimedOut {
                signal_sent,
                escalated,
                tree_kill_reliability,
            } => TimeoutResultWire {
                schema_id: TIMEOUT_RESULT_V1_1,
                status: "timed_out",
                exit_code: None,
                signal_sent: Some(signal_sent),
                escalated: Some(escalated),
                tree_kill_reliability: Some(tree_kill_reliability.as_str().to_string()),
            },
        };
        serialize_json(&result, "timeout result")
    })
    .map_err(py_sysprims_err)
}

// =============================================================================
// Session Spawn, Group Spawn & Containment
// =============================================================================

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRunSetsidConfig {
    schema_id: String,
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Option<BTreeMap<String, String>>,
    #[serde(default)]
    wait: bool,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(not(unix), allow(dead_code))]
#[serde(deny_unknown_fields)]
struct WireRunNohupConfig {
    schema_id: String,
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Option<BTreeMap<String, String>>,
    #[serde(default)]
    wait: bool,
    #[serde(default)]
    output_file: Option<String>,
}

#[derive(Serialize)]
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

#[pyfunction]
fn run_setsid(py: Python<'_>, config_json: &str) -> PyResult<String> {
    if config_json.is_empty() {
        return Err(py_sysprims_err(SysprimsError::invalid_argument(
            "config_json cannot be empty",
        )));
    }
    let wire: WireRunSetsidConfig = serde_json::from_str(config_json).map_err(|err| {
        py_sysprims_err(SysprimsError::invalid_argument(format!(
            "invalid config JSON: {err}"
        )))
    })?;
    if wire.schema_id != RUN_SETSID_CONFIG_V1 {
        return Err(py_sysprims_err(SysprimsError::invalid_argument(format!(
            "invalid schema_id (expected {RUN_SETSID_CONFIG_V1})"
        ))));
    }
    let _ = split_argv(&wire.argv).map_err(py_sysprims_err)?;
    py.detach(|| {
        let (command, args) = split_argv(&wire.argv)?;
        let config = sysprims_session::SetsidConfig {
            wait: wire.wait,
            ctty: false,
            cwd: wire.cwd.map(PathBuf::from),
            env: wire.env,
        };
        let outcome = sysprims_session::run_setsid(command, &args, config)?;
        let result = session_setsid_result_from_outcome(outcome)?;
        serialize_json(&result, "session spawn result")
    })
    .map_err(py_sysprims_err)
}

#[pyfunction]
fn run_nohup(py: Python<'_>, config_json: &str) -> PyResult<String> {
    if config_json.is_empty() {
        return Err(py_sysprims_err(SysprimsError::invalid_argument(
            "config_json cannot be empty",
        )));
    }
    let wire: WireRunNohupConfig = serde_json::from_str(config_json).map_err(|err| {
        py_sysprims_err(SysprimsError::invalid_argument(format!(
            "invalid config JSON: {err}"
        )))
    })?;
    if wire.schema_id != RUN_NOHUP_CONFIG_V1 {
        return Err(py_sysprims_err(SysprimsError::invalid_argument(format!(
            "invalid schema_id (expected {RUN_NOHUP_CONFIG_V1})"
        ))));
    }
    let _ = split_argv(&wire.argv).map_err(py_sysprims_err)?;

    #[cfg(not(unix))]
    {
        let _ = py;
        Err(py_sysprims_err(SysprimsError::not_supported(
            "nohup",
            std::env::consts::OS,
        )))
    }

    #[cfg(unix)]
    {
        py.detach(|| {
            let (command, args) = split_argv(&wire.argv)?;
            let caller_sid = sysprims_session::getsid(0)?;
            let caller_pgid = sysprims_session::getpgid(0)?;
            let config = sysprims_session::NohupConfig {
                wait: wire.wait,
                output_file: wire.output_file,
                cwd: wire.cwd.map(PathBuf::from),
                env: wire.env,
            };
            let outcome = sysprims_session::run_nohup(command, &args, config)?;
            let result = session_nohup_result_from_outcome(outcome, caller_sid, caller_pgid)?;
            serialize_json(&result, "session spawn result")
        })
        .map_err(py_sysprims_err)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSpawnInGroupConfig {
    schema_id: String,
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Option<BTreeMap<String, String>>,
}

#[pyfunction]
fn spawn_in_group(py: Python<'_>, config_json: &str) -> PyResult<String> {
    if config_json.is_empty() {
        return Err(py_sysprims_err(SysprimsError::invalid_argument(
            "config_json cannot be empty",
        )));
    }
    let wire: WireSpawnInGroupConfig = serde_json::from_str(config_json).map_err(|err| {
        py_sysprims_err(SysprimsError::invalid_argument(format!(
            "invalid config JSON: {err}"
        )))
    })?;
    if wire.schema_id != SPAWN_IN_GROUP_CONFIG_V1 {
        return Err(py_sysprims_err(SysprimsError::invalid_argument(format!(
            "invalid schema_id (expected {SPAWN_IN_GROUP_CONFIG_V1})"
        ))));
    }
    let cfg = SpawnInGroupConfig {
        argv: wire.argv,
        cwd: wire.cwd,
        env: wire.env,
    };
    py.detach(|| {
        let result = timeout_spawn_in_group(cfg)?;
        serialize_json(&result, "spawn-in-group result")
    })
    .map_err(py_sysprims_err)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireContainmentSpawnConfig {
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

#[pyfunction]
fn containment_spawn(py: Python<'_>, config_json: &str) -> PyResult<u64> {
    if config_json.is_empty() {
        return Err(py_sysprims_err(SysprimsError::invalid_argument(
            "config_json cannot be empty",
        )));
    }
    let wire: WireContainmentSpawnConfig = serde_json::from_str(config_json).map_err(|err| {
        py_sysprims_err(SysprimsError::invalid_argument(format!(
            "invalid config JSON: {err}"
        )))
    })?;
    if wire.schema_id != CONTAINMENT_SPAWN_CONFIG_V1 {
        return Err(py_sysprims_err(SysprimsError::invalid_argument(format!(
            "invalid schema_id (expected {CONTAINMENT_SPAWN_CONFIG_V1})"
        ))));
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
    py.detach(|| timeout_containment_spawn(request))
        .map_err(py_sysprims_err)
}

#[pyfunction]
fn containment_identity(py: Python<'_>, token: i128) -> PyResult<String> {
    let token = validate_u64(token, "containment handle").map_err(py_sysprims_err)?;
    py.detach(|| {
        let snapshot = timeout_containment_identity(token)?;
        serialize_json(&snapshot, "containment snapshot")
    })
    .map_err(py_sysprims_err)
}

#[pyfunction]
fn containment_poll(py: Python<'_>, token: i128) -> PyResult<String> {
    let token = validate_u64(token, "containment handle").map_err(py_sysprims_err)?;
    py.detach(|| {
        let snapshot = timeout_containment_poll(token)?;
        serialize_json(&snapshot, "containment snapshot")
    })
    .map_err(py_sysprims_err)
}

#[pyfunction]
#[pyo3(signature = (token, wait_timeout_ms = None))]
fn containment_wait(
    py: Python<'_>,
    token: i128,
    wait_timeout_ms: Option<i128>,
) -> PyResult<String> {
    let token = validate_u64(token, "containment handle").map_err(py_sysprims_err)?;
    let timeout = match wait_timeout_ms {
        Some(ms) => validate_u64(ms, "wait_timeout_ms").map_err(py_sysprims_err)?,
        None => 0,
    };
    py.detach(|| {
        let snapshot = timeout_containment_wait(token, timeout)?;
        serialize_json(&snapshot, "containment snapshot")
    })
    .map_err(py_sysprims_err)
}

#[pyfunction]
fn containment_terminate(py: Python<'_>, token: i128) -> PyResult<String> {
    let token = validate_u64(token, "containment handle").map_err(py_sysprims_err)?;
    py.detach(|| {
        let snapshot = timeout_containment_terminate(token)?;
        serialize_json(&snapshot, "containment snapshot")
    })
    .map_err(py_sysprims_err)
}

#[pyfunction]
fn containment_close(py: Python<'_>, token: i128) -> PyResult<()> {
    let token = validate_u64(token, "containment handle").map_err(py_sysprims_err)?;
    py.detach(|| timeout_containment_close(token))
        .map_err(py_sysprims_err)
}

// =============================================================================
// RFC 5545 Recurrence (`contract: recurrence/v0`)
// =============================================================================

fn parse_recurrence_instant(value: &str, name: &str) -> PyResult<RecurrenceInstant> {
    RecurrenceInstant::parse(value).ok_or_else(|| {
        PyValueError::new_err(format!(
            "{name} must be a canonical UTC timestamp YYYY-MM-DDTHH:MM:SSZ in years 0001..=9999"
        ))
    })
}

#[pyclass(frozen, module = "sysprims._sysprims", name = "RawRecurrenceRule")]
struct RawRecurrenceRule {
    inner: RecurrenceRule,
}

#[pymethods]
impl RawRecurrenceRule {
    #[staticmethod]
    fn from_json(data: &[u8]) -> PyResult<Self> {
        RecurrenceRule::from_json(data)
            .map(|inner| Self { inner })
            .map_err(py_recurrence_refusal)
    }

    #[staticmethod]
    fn from_document_json(data: &[u8]) -> PyResult<Self> {
        RecurrenceRule::from_document_json(data)
            .map(|inner| Self { inner })
            .map_err(py_recurrence_refusal)
    }

    #[staticmethod]
    fn from_text(lines: Vec<String>) -> PyResult<Self> {
        RecurrenceRule::from_text(&lines)
            .map(|inner| Self { inner })
            .map_err(py_recurrence_refusal)
    }

    fn dtstart(&self) -> String {
        self.inner.dtstart().to_string()
    }

    fn tzid(&self) -> &str {
        self.inner.tzid()
    }

    fn freq(&self) -> &'static str {
        self.inner.freq().as_str()
    }

    fn to_canonical_json<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.to_canonical_json())
    }

    fn digest(&self) -> String {
        self.inner.digest()
    }

    fn to_text(&self) -> Vec<String> {
        self.inner.to_text()
    }

    fn expand(
        &self,
        py: Python<'_>,
        from_utc: &str,
        to_utc: &str,
        limit: i128,
    ) -> PyResult<Vec<String>> {
        let from_inst = parse_recurrence_instant(from_utc, "from_utc")?;
        let to_inst = parse_recurrence_instant(to_utc, "to_utc")?;
        if limit < 1 || limit > i128::from(u32::MAX) {
            return Err(PyValueError::new_err(format!(
                "limit must be between 1 and {}",
                u32::MAX
            )));
        }
        let non_zero =
            NonZeroU32::new(limit as u32).ok_or_else(|| PyValueError::new_err("limit >= 1"))?;
        py.detach(|| {
            self.inner
                .expand(from_inst, to_inst, non_zero)
                .map(|items| items.into_iter().map(|inst| inst.to_string()).collect())
        })
        .map_err(py_recurrence_query_err)
    }

    fn next_after(&self, py: Python<'_>, after_utc: &str) -> PyResult<Option<String>> {
        let after_inst = parse_recurrence_instant(after_utc, "after_utc")?;
        py.detach(|| {
            self.inner
                .next_after(after_inst)
                .map(|opt| opt.map(|inst| inst.to_string()))
        })
        .map_err(py_recurrence_query_err)
    }
}

#[pyfunction]
fn tzdb_version() -> &'static str {
    sysprims_recurrence::tzdb_version()
}

#[pymodule]
fn _sysprims(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("RawSysprimsError", m.py().get_type::<RawSysprimsError>())?;
    m.add(
        "RawRecurrenceRefusal",
        m.py().get_type::<RawRecurrenceRefusal>(),
    )?;
    m.add(
        "RawRecurrenceQueryError",
        m.py().get_type::<RawRecurrenceQueryError>(),
    )?;
    m.add_class::<RawRecurrenceRule>()?;

    m.add_function(wrap_pyfunction!(abi_version, m)?)?;
    m.add_function(wrap_pyfunction!(platform, m)?)?;
    m.add_function(wrap_pyfunction!(proc_get, m)?)?;
    m.add_function(wrap_pyfunction!(proc_list, m)?)?;
    m.add_function(wrap_pyfunction!(proc_listening_ports, m)?)?;
    m.add_function(wrap_pyfunction!(proc_list_fds, m)?)?;
    m.add_function(wrap_pyfunction!(proc_wait_pid, m)?)?;
    m.add_function(wrap_pyfunction!(proc_descendants, m)?)?;
    m.add_function(wrap_pyfunction!(proc_kill_descendants, m)?)?;
    m.add_function(wrap_pyfunction!(proc_guard_step, m)?)?;
    m.add_function(wrap_pyfunction!(proc_ancestors, m)?)?;
    m.add_function(wrap_pyfunction!(self_getpgid, m)?)?;
    m.add_function(wrap_pyfunction!(self_getsid, m)?)?;
    m.add_function(wrap_pyfunction!(signal_send, m)?)?;
    m.add_function(wrap_pyfunction!(signal_send_group, m)?)?;
    m.add_function(wrap_pyfunction!(terminate, m)?)?;
    m.add_function(wrap_pyfunction!(force_kill, m)?)?;
    m.add_function(wrap_pyfunction!(signal_kill_many, m)?)?;
    m.add_function(wrap_pyfunction!(terminate_tree, m)?)?;
    m.add_function(wrap_pyfunction!(run_with_timeout, m)?)?;
    m.add_function(wrap_pyfunction!(run_setsid, m)?)?;
    m.add_function(wrap_pyfunction!(run_nohup, m)?)?;
    m.add_function(wrap_pyfunction!(spawn_in_group, m)?)?;
    m.add_function(wrap_pyfunction!(containment_spawn, m)?)?;
    m.add_function(wrap_pyfunction!(containment_identity, m)?)?;
    m.add_function(wrap_pyfunction!(containment_poll, m)?)?;
    m.add_function(wrap_pyfunction!(containment_wait, m)?)?;
    m.add_function(wrap_pyfunction!(containment_terminate, m)?)?;
    m.add_function(wrap_pyfunction!(containment_close, m)?)?;
    m.add_function(wrap_pyfunction!(tzdb_version, m)?)?;

    m.add("VERSION", env!("CARGO_PKG_VERSION"))?;
    m.add("RECURRENCE_CONTRACT", RECURRENCE_CONTRACT_STR)?;
    m.add("RECURRENCE_CORPUS_DIGEST", RECURRENCE_CORPUS_DIGEST_STR)?;
    m.add("SIGHUP", sysprims_signal::SIGHUP)?;
    m.add("SIGINT", sysprims_signal::SIGINT)?;
    m.add("SIGKILL", sysprims_signal::SIGKILL)?;
    m.add("SIGTERM", sysprims_signal::SIGTERM)?;
    m.add("SIGUSR1", sysprims_signal::SIGUSR1)?;
    m.add("SIGUSR2", sysprims_signal::SIGUSR2)?;
    Ok(())
}
