//! Native-owned managed containment lifecycle for language bindings.
//!
//! One registry owns every [`ContainmentGuard<Child>`] used by C/Go and N-API
//! projections. Callers receive a generation-checked `u64` capability token,
//! never a PID, Rust pointer, or reusable proof of spawn.

use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use sysprims_core::schema::CONTAINMENT_SNAPSHOT_V1;
use sysprims_core::time::now_rfc3339;
use sysprims_core::{get_platform, SysprimsError, SysprimsResult};

use crate::{
    spawn_contained, ContainmentBoundaryStrength, ContainmentCompletionEvidence, ContainmentGuard,
    ContainmentIdentity, ContainmentOutcome, ContainmentSpawnError, TerminateTreeConfig,
    TreeKillReliability, SIGKILL, SIGTERM,
};

pub const MAX_REGISTRY_SLOTS: usize = 1024;
pub const MAX_ARGV_ENTRIES: usize = 256;
pub const MAX_ARGV_ENTRY_BYTES: usize = 4096;
pub const MAX_ARGV_TOTAL_BYTES: usize = 65_536;
pub const MAX_ENV_ENTRIES: usize = 256;
pub const MAX_ENV_KEY_BYTES: usize = 1024;
pub const MAX_ENV_VALUE_BYTES: usize = 4096;
pub const MAX_ENV_TOTAL_BYTES: usize = 65_536;
pub const MAX_CWD_BYTES: usize = 4096;
pub const MAX_DURATION_MS: u64 = 86_400_000;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone)]
pub struct ManagedSpawnRequest {
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub env: Option<BTreeMap<String, String>>,
    pub execution_timeout_ms: Option<u64>,
    pub grace_timeout_ms: Option<u64>,
    pub kill_timeout_ms: Option<u64>,
    pub signal: Option<i32>,
    pub kill_signal: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotState {
    Empty,
    Active,
    Finalizing,
    Inert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeaderStatus {
    Running,
    Completed,
    TimedOut,
    Terminated,
}

impl LeaderStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::TimedOut => "timed_out",
            Self::Terminated => "terminated",
        }
    }
}

struct SlotData {
    generation: u32,
    retired: bool,
    state: SlotState,
    guard: Option<ContainmentGuard<ChildAdapter>>,
    identity: Option<ContainmentIdentity>,
    reliability: Option<TreeKillReliability>,
    boundary: Option<ContainmentBoundaryStrength>,
    terminate_config: TerminateTreeConfig,
    outcome: Option<ContainmentOutcome>,
    leader_status: LeaderStatus,
    deadline_cancel: Arc<AtomicBool>,
}

struct Slot {
    data: Mutex<SlotData>,
    cond: Condvar,
}

struct Registry {
    slots: Vec<Slot>,
    free: Mutex<Vec<u32>>,
}

type ChildAdapter = std::process::Child;

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut slots = Vec::with_capacity(MAX_REGISTRY_SLOTS);
        let mut free = Vec::with_capacity(MAX_REGISTRY_SLOTS);
        for index in 0..MAX_REGISTRY_SLOTS {
            slots.push(Slot {
                data: Mutex::new(SlotData {
                    generation: 1,
                    retired: false,
                    state: SlotState::Empty,
                    guard: None,
                    identity: None,
                    reliability: None,
                    boundary: None,
                    terminate_config: TerminateTreeConfig::default(),
                    outcome: None,
                    leader_status: LeaderStatus::Running,
                    deadline_cancel: Arc::new(AtomicBool::new(false)),
                }),
                cond: Condvar::new(),
            });
            free.push(index as u32);
        }
        Registry {
            slots,
            free: Mutex::new(free),
        }
    })
}

fn lock_poisoned() -> SysprimsError {
    SysprimsError::internal("containment registry lock poisoned")
}

fn lock_data(slot: &Slot) -> SysprimsResult<MutexGuard<'_, SlotData>> {
    slot.data.lock().map_err(|_| lock_poisoned())
}

fn lock_free(reg: &Registry) -> SysprimsResult<MutexGuard<'_, Vec<u32>>> {
    reg.free.lock().map_err(|_| lock_poisoned())
}

fn pack_token(slot: u32, generation: u32) -> u64 {
    ((generation as u64) << 32) | (u64::from(slot) + 1)
}

fn unpack_token(token: u64) -> SysprimsResult<(u32, u32)> {
    if token == 0 {
        return Err(invalid_handle("containment handle is invalid"));
    }
    let slot_plus_one = token & 0xffff_ffff;
    if slot_plus_one == 0 {
        return Err(invalid_handle("containment handle is invalid"));
    }
    let slot = slot_plus_one - 1;
    if slot >= MAX_REGISTRY_SLOTS as u64 {
        return Err(invalid_handle("containment handle is invalid"));
    }
    let generation = (token >> 32) as u32;
    if generation == 0 {
        return Err(invalid_handle("containment handle is invalid"));
    }
    Ok((slot as u32, generation))
}

fn invalid_handle(message: &str) -> SysprimsError {
    SysprimsError::invalid_argument(message)
}

fn stale_handle() -> SysprimsError {
    invalid_handle("containment handle is stale or closed")
}

fn spawn_error(error: ContainmentSpawnError) -> SysprimsError {
    match error {
        ContainmentSpawnError::Spawn(error) | ContainmentSpawnError::Adoption(error) => error,
    }
}

pub fn validate_spawn_request(
    request: &ManagedSpawnRequest,
) -> SysprimsResult<TerminateTreeConfig> {
    if request.argv.is_empty() {
        return Err(SysprimsError::invalid_argument("argv must not be empty"));
    }
    if request.argv.len() > MAX_ARGV_ENTRIES {
        return Err(SysprimsError::invalid_argument(format!(
            "argv exceeds maximum of {MAX_ARGV_ENTRIES} entries"
        )));
    }
    let mut argv_bytes = 0usize;
    for (index, arg) in request.argv.iter().enumerate() {
        if arg.is_empty() {
            return Err(SysprimsError::invalid_argument(format!(
                "argv[{index}] must not be empty"
            )));
        }
        if arg.len() > MAX_ARGV_ENTRY_BYTES {
            return Err(SysprimsError::invalid_argument(format!(
                "argv[{index}] exceeds maximum of {MAX_ARGV_ENTRY_BYTES} bytes"
            )));
        }
        argv_bytes = argv_bytes.saturating_add(arg.len());
        if argv_bytes > MAX_ARGV_TOTAL_BYTES {
            return Err(SysprimsError::invalid_argument(format!(
                "argv exceeds maximum of {MAX_ARGV_TOTAL_BYTES} bytes"
            )));
        }
    }

    if let Some(cwd) = request.cwd.as_deref() {
        if cwd.is_empty() {
            return Err(SysprimsError::invalid_argument("cwd must not be empty"));
        }
        if cwd.len() > MAX_CWD_BYTES {
            return Err(SysprimsError::invalid_argument(format!(
                "cwd exceeds maximum of {MAX_CWD_BYTES} bytes"
            )));
        }
    }

    if let Some(env) = request.env.as_ref() {
        if env.len() > MAX_ENV_ENTRIES {
            return Err(SysprimsError::invalid_argument(format!(
                "env exceeds maximum of {MAX_ENV_ENTRIES} entries"
            )));
        }
        let mut env_bytes = 0usize;
        for (key, value) in env {
            if key.is_empty() {
                return Err(SysprimsError::invalid_argument(
                    "env keys must not be empty",
                ));
            }
            if key.len() > MAX_ENV_KEY_BYTES {
                return Err(SysprimsError::invalid_argument(format!(
                    "env key exceeds maximum of {MAX_ENV_KEY_BYTES} bytes"
                )));
            }
            if value.len() > MAX_ENV_VALUE_BYTES {
                return Err(SysprimsError::invalid_argument(format!(
                    "env value exceeds maximum of {MAX_ENV_VALUE_BYTES} bytes"
                )));
            }
            env_bytes = env_bytes
                .saturating_add(key.len())
                .saturating_add(value.len());
            if env_bytes > MAX_ENV_TOTAL_BYTES {
                return Err(SysprimsError::invalid_argument(format!(
                    "env exceeds maximum of {MAX_ENV_TOTAL_BYTES} bytes"
                )));
            }
        }
    }

    if let Some(timeout) = request.execution_timeout_ms {
        if timeout == 0 || timeout > MAX_DURATION_MS {
            return Err(SysprimsError::invalid_argument(format!(
                "execution_timeout_ms must be between 1 and {MAX_DURATION_MS}"
            )));
        }
    }

    let mut config = TerminateTreeConfig::default();
    if let Some(grace) = request.grace_timeout_ms {
        if grace > MAX_DURATION_MS {
            return Err(SysprimsError::invalid_argument(format!(
                "grace_timeout_ms exceeds maximum of {MAX_DURATION_MS}"
            )));
        }
        config.grace_timeout_ms = grace;
    }
    if let Some(kill) = request.kill_timeout_ms {
        if kill > MAX_DURATION_MS {
            return Err(SysprimsError::invalid_argument(format!(
                "kill_timeout_ms exceeds maximum of {MAX_DURATION_MS}"
            )));
        }
        config.kill_timeout_ms = kill;
    }
    if let Some(signal) = request.signal {
        if signal == 0 {
            return Err(SysprimsError::invalid_argument(
                "signal 0 is not a valid termination policy",
            ));
        }
        if signal < 0 {
            return Err(SysprimsError::invalid_argument(
                "signal must be a positive termination signal",
            ));
        }
        config.signal = signal;
    }
    if let Some(kill_signal) = request.kill_signal {
        if kill_signal == 0 {
            return Err(SysprimsError::invalid_argument(
                "kill_signal 0 is not a valid termination policy",
            ));
        }
        if kill_signal < 0 {
            return Err(SysprimsError::invalid_argument(
                "kill_signal must be a positive termination signal",
            ));
        }
        config.kill_signal = kill_signal;
    }
    if config.signal == 0 {
        config.signal = SIGTERM;
    }
    if config.kill_signal == 0 {
        config.kill_signal = SIGKILL;
    }
    Ok(config)
}

fn allocate_slot() -> SysprimsResult<(u32, u32)> {
    let reg = registry();
    let mut free = lock_free(reg)?;
    while let Some(slot_index) = free.pop() {
        let slot = &reg.slots[slot_index as usize];
        let data = lock_data(slot)?;
        if data.retired || data.state != SlotState::Empty {
            continue;
        }
        let generation = data.generation;
        drop(data);
        return Ok((slot_index, generation));
    }
    Err(SysprimsError::invalid_argument(
        "containment registry has no free slots",
    ))
}

fn free_slot_locked(reg: &Registry, slot_index: u32, data: &mut SlotData) -> SysprimsResult<()> {
    data.guard = None;
    data.identity = None;
    data.reliability = None;
    data.boundary = None;
    data.outcome = None;
    data.leader_status = LeaderStatus::Running;
    data.deadline_cancel.store(true, Ordering::SeqCst);
    data.deadline_cancel = Arc::new(AtomicBool::new(false));
    data.state = SlotState::Empty;
    if data.generation == u32::MAX {
        data.retired = true;
        return Ok(());
    }
    let next = data.generation.saturating_add(1);
    if next == 0 || next == u32::MAX {
        data.generation = u32::MAX;
        data.retired = true;
        return Ok(());
    }
    data.generation = next;
    let mut free = lock_free(reg)?;
    free.push(slot_index);
    Ok(())
}

fn lookup<'a>(token: u64) -> SysprimsResult<(u32, u32, &'a Slot)> {
    let (slot_index, generation) = unpack_token(token)?;
    let slot = &registry().slots[slot_index as usize];
    Ok((slot_index, generation, slot))
}

fn snapshot_from(
    data: &SlotData,
    handle_state: &'static str,
    leader_status: LeaderStatus,
) -> SysprimsResult<ManagedSnapshot> {
    let identity = data
        .identity
        .as_ref()
        .ok_or_else(|| SysprimsError::internal("containment identity missing"))?;
    let reliability = data
        .reliability
        .ok_or_else(|| SysprimsError::internal("containment reliability missing"))?;
    let boundary = data
        .boundary
        .ok_or_else(|| SysprimsError::internal("containment boundary missing"))?;
    let mut snapshot = ManagedSnapshot {
        schema_id: CONTAINMENT_SNAPSHOT_V1,
        timestamp: now_rfc3339(),
        platform: get_platform(),
        handle_state,
        leader_status: leader_status.as_str(),
        identity: ManagedIdentity {
            pid: identity.pid,
            start_time_unix_ms: identity.start_time_unix_ms,
            exe_path: identity.exe_path.clone(),
        },
        tree_kill_reliability: reliability.as_str(),
        boundary_strength: boundary.as_str(),
        pgid: None,
        signal_sent: None,
        kill_signal: None,
        escalated: None,
        exited: None,
        timed_out: None,
        completion: None,
        warnings: Vec::new(),
    };
    if let Some(outcome) = data.outcome.as_ref() {
        snapshot.pgid = outcome.pgid;
        snapshot.signal_sent = outcome.signal_sent;
        snapshot.kill_signal = outcome.kill_signal;
        snapshot.escalated = Some(outcome.escalated);
        snapshot.exited = Some(outcome.exited);
        snapshot.timed_out = Some(outcome.timed_out);
        snapshot.completion = Some(outcome.completion.clone());
        snapshot.warnings = outcome.warnings.clone();
    }
    Ok(snapshot)
}

fn commit_inert(
    slot: &Slot,
    data: &mut SlotData,
    outcome: ContainmentOutcome,
    leader_status: LeaderStatus,
) -> SysprimsResult<ManagedSnapshot> {
    data.outcome = Some(outcome);
    data.leader_status = leader_status;
    data.state = SlotState::Inert;
    slot.cond.notify_all();
    snapshot_from(data, "inert", leader_status)
}

/// Spawn a sysprims-owned contained child and return a capability token.
pub fn spawn(request: ManagedSpawnRequest) -> SysprimsResult<u64> {
    let terminate_config = validate_spawn_request(&request)?;

    #[cfg(windows)]
    {
        let _ = terminate_config;
        return Err(SysprimsError::not_supported(
            "managed containment spawn without create-suspended Job assignment",
            "windows",
        ));
    }

    #[cfg(unix)]
    {
        let (slot_index, generation) = allocate_slot()?;
        let token = pack_token(slot_index, generation);
        let command = match build_command(&request) {
            Ok(command) => command,
            Err(error) => {
                let _ = recycle_unused_slot(slot_index, generation);
                return Err(error);
            }
        };

        let guard = match spawn_contained(command) {
            Ok(guard) => guard,
            Err(error) => {
                let _ = recycle_unused_slot(slot_index, generation);
                return Err(spawn_error(error));
            }
        };

        let reliability = guard.tree_kill_reliability();
        let boundary = guard.boundary_strength();
        if reliability != TreeKillReliability::Guaranteed
            || boundary != ContainmentBoundaryStrength::CooperativeGroup
        {
            drop(guard);
            let _ = recycle_unused_slot(slot_index, generation);
            return Err(SysprimsError::internal(
                "managed spawn produced a non-guaranteed cooperative-group guard",
            ));
        }

        let identity = guard.identity().clone();
        let slot = &registry().slots[slot_index as usize];
        {
            let mut data = lock_data(slot)?;
            if data.generation != generation || data.state != SlotState::Empty {
                drop(guard);
                return Err(SysprimsError::internal(
                    "containment slot changed before spawn insert",
                ));
            }
            data.identity = Some(identity);
            data.reliability = Some(reliability);
            data.boundary = Some(boundary);
            data.terminate_config = terminate_config;
            data.leader_status = LeaderStatus::Running;
            data.outcome = None;
            data.guard = Some(guard);
            data.state = SlotState::Active;
            if let Some(timeout_ms) = request.execution_timeout_ms {
                let cancel = Arc::clone(&data.deadline_cancel);
                drop(data);
                start_deadline_thread(token, timeout_ms, cancel);
            }
        }
        Ok(token)
    }
}

fn recycle_unused_slot(slot_index: u32, generation: u32) -> SysprimsResult<()> {
    let reg = registry();
    let slot = &reg.slots[slot_index as usize];
    let data = lock_data(slot)?;
    if data.generation == generation && data.state == SlotState::Empty && !data.retired {
        let mut free = lock_free(reg)?;
        free.push(slot_index);
    }
    Ok(())
}

fn build_command(request: &ManagedSpawnRequest) -> SysprimsResult<Command> {
    let mut command = Command::new(&request.argv[0]);
    if request.argv.len() > 1 {
        command.args(&request.argv[1..]);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(cwd) = request.cwd.as_deref() {
        command.current_dir(cwd);
    }
    if let Some(env) = request.env.as_ref() {
        for (key, value) in env {
            command.env(key, value);
        }
    }
    Ok(command)
}

fn start_deadline_thread(token: u64, timeout_ms: u64, cancel: Arc<AtomicBool>) {
    thread::Builder::new()
        .name("sysprims-containment-deadline".into())
        .spawn(move || {
            let deadline = Instant::now() + Duration::from_millis(timeout_ms);
            while !cancel.load(Ordering::SeqCst) {
                let now = Instant::now();
                if now >= deadline {
                    let _ = terminate_with_status(token, LeaderStatus::TimedOut);
                    return;
                }
                thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(now)));
            }
        })
        .ok();
}

fn wait_for_not_finalizing<'a>(
    slot: &'a Slot,
    mut data: MutexGuard<'a, SlotData>,
    wait_deadline: Option<Instant>,
) -> SysprimsResult<MutexGuard<'a, SlotData>> {
    while data.state == SlotState::Finalizing {
        if let Some(deadline) = wait_deadline {
            let now = Instant::now();
            if now >= deadline {
                return Ok(data);
            }
            let remaining = deadline.saturating_duration_since(now);
            let (guard, _) = slot
                .cond
                .wait_timeout(data, remaining)
                .map_err(|_| lock_poisoned())?;
            data = guard;
        } else {
            data = slot.cond.wait(data).map_err(|_| lock_poisoned())?;
        }
    }
    Ok(data)
}

pub fn identity(token: u64) -> SysprimsResult<ManagedSnapshot> {
    let (_slot_index, generation, slot) = lookup(token)?;
    let data = lock_data(slot)?;
    if data.generation != generation || data.retired {
        return Err(stale_handle());
    }
    match data.state {
        SlotState::Empty => Err(stale_handle()),
        SlotState::Active | SlotState::Finalizing => {
            snapshot_from(&data, "active", LeaderStatus::Running)
        }
        SlotState::Inert => snapshot_from(&data, "inert", data.leader_status),
    }
}

pub fn poll(token: u64) -> SysprimsResult<ManagedSnapshot> {
    operate_non_blocking(token, LeaderStatus::Completed, |guard, config| {
        guard.try_complete(config)
    })
}

pub fn wait(token: u64, wait_timeout_ms: u64) -> SysprimsResult<ManagedSnapshot> {
    if wait_timeout_ms > MAX_DURATION_MS {
        return Err(SysprimsError::invalid_argument(format!(
            "wait timeout exceeds maximum of {MAX_DURATION_MS}"
        )));
    }
    let wait_deadline = if wait_timeout_ms == 0 {
        None
    } else {
        Some(Instant::now() + Duration::from_millis(wait_timeout_ms))
    };

    loop {
        match operate_non_blocking(token, LeaderStatus::Completed, |guard, config| {
            guard.try_complete(config)
        }) {
            Ok(snapshot) if snapshot.leader_status != "running" => return Ok(snapshot),
            Ok(snapshot) => {
                if let Some(deadline) = wait_deadline {
                    if Instant::now() >= deadline {
                        return Ok(snapshot);
                    }
                }
                thread::sleep(POLL_INTERVAL);
            }
            Err(error) => return Err(error),
        }
    }
}

pub fn terminate(token: u64) -> SysprimsResult<ManagedSnapshot> {
    terminate_with_status(token, LeaderStatus::Terminated)
}

fn terminate_with_status(
    token: u64,
    leader_status: LeaderStatus,
) -> SysprimsResult<ManagedSnapshot> {
    operate_blocking(token, leader_status, |guard, config| {
        guard.terminate(config).map(Some)
    })
}

pub fn close(token: u64) -> SysprimsResult<()> {
    let (slot_index, generation, slot) = lookup(token)?;
    let mut data = lock_data(slot)?;
    if data.generation != generation || data.retired {
        return Err(stale_handle());
    }
    data = wait_for_not_finalizing(slot, data, Some(Instant::now() + Duration::from_secs(120)))?;
    if data.generation != generation {
        return Err(stale_handle());
    }
    match data.state {
        SlotState::Empty => Err(stale_handle()),
        SlotState::Finalizing => Err(SysprimsError::invalid_argument(
            "containment handle is busy; retry close",
        )),
        SlotState::Active | SlotState::Inert => {
            data.state = SlotState::Finalizing;
            data.deadline_cancel.store(true, Ordering::SeqCst);
            data.guard = None;
            let reg = registry();
            free_slot_locked(reg, slot_index, &mut data)?;
            slot.cond.notify_all();
            Ok(())
        }
    }
}

fn operate_non_blocking(
    token: u64,
    complete_status: LeaderStatus,
    op: impl FnOnce(
        &mut ContainmentGuard<ChildAdapter>,
        TerminateTreeConfig,
    ) -> SysprimsResult<Option<ContainmentOutcome>>,
) -> SysprimsResult<ManagedSnapshot> {
    operate_inner(token, complete_status, false, op)
}

fn operate_blocking(
    token: u64,
    complete_status: LeaderStatus,
    op: impl FnOnce(
        &mut ContainmentGuard<ChildAdapter>,
        TerminateTreeConfig,
    ) -> SysprimsResult<Option<ContainmentOutcome>>,
) -> SysprimsResult<ManagedSnapshot> {
    operate_inner(token, complete_status, true, op)
}

fn operate_inner(
    token: u64,
    complete_status: LeaderStatus,
    block_for_finalizing: bool,
    op: impl FnOnce(
        &mut ContainmentGuard<ChildAdapter>,
        TerminateTreeConfig,
    ) -> SysprimsResult<Option<ContainmentOutcome>>,
) -> SysprimsResult<ManagedSnapshot> {
    let (_slot_index, generation, slot) = lookup(token)?;
    let mut data = lock_data(slot)?;
    if data.generation != generation || data.retired {
        return Err(stale_handle());
    }
    if block_for_finalizing {
        data = wait_for_not_finalizing(slot, data, None)?;
        if data.generation != generation {
            return Err(stale_handle());
        }
    } else if data.state == SlotState::Finalizing {
        data =
            wait_for_not_finalizing(slot, data, Some(Instant::now() + Duration::from_secs(120)))?;
        if data.generation != generation {
            return Err(stale_handle());
        }
    }
    match data.state {
        SlotState::Empty => Err(stale_handle()),
        SlotState::Finalizing => Err(SysprimsError::invalid_argument(
            "containment handle is busy",
        )),
        SlotState::Inert => snapshot_from(&data, "inert", data.leader_status),
        SlotState::Active => {
            data.state = SlotState::Finalizing;
            let config = data.terminate_config.clone();
            let result = {
                let guard = data.guard.as_mut().ok_or_else(|| {
                    SysprimsError::internal("containment guard missing during operation")
                })?;
                op(guard, config)
            };
            match result {
                Ok(Some(outcome)) => commit_inert(slot, &mut data, outcome, complete_status),
                Ok(None) => {
                    data.state = SlotState::Active;
                    slot.cond.notify_all();
                    snapshot_from(&data, "active", LeaderStatus::Running)
                }
                Err(error) => {
                    data.state = SlotState::Active;
                    slot.cond.notify_all();
                    Err(error)
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ManagedIdentity {
    pub pid: u32,
    pub start_time_unix_ms: u64,
    pub exe_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManagedSnapshot {
    pub schema_id: &'static str,
    pub timestamp: String,
    pub platform: &'static str,
    pub handle_state: &'static str,
    pub leader_status: &'static str,
    pub identity: ManagedIdentity,
    pub tree_kill_reliability: &'static str,
    pub boundary_strength: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pgid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_sent: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kill_signal: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escalated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exited: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timed_out: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion: Option<ContainmentCompletionEvidence>,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(argv: Vec<&str>) -> ManagedSpawnRequest {
        ManagedSpawnRequest {
            argv: argv.into_iter().map(str::to_string).collect(),
            cwd: None,
            env: None,
            execution_timeout_ms: None,
            grace_timeout_ms: Some(50),
            kill_timeout_ms: Some(200),
            signal: None,
            kill_signal: None,
        }
    }

    #[test]
    fn rejects_empty_argv_and_signal_zero() {
        assert!(validate_spawn_request(&request(vec![])).is_err());
        let mut bad = request(vec!["true"]);
        bad.signal = Some(0);
        assert!(validate_spawn_request(&bad).is_err());
        let mut overflow = request(vec!["true"]);
        overflow.execution_timeout_ms = Some(MAX_DURATION_MS + 1);
        assert!(validate_spawn_request(&overflow).is_err());
    }

    #[test]
    fn unpack_rejects_zero_and_malformed_tokens() {
        assert!(unpack_token(0).is_err());
        assert!(unpack_token(1u64 << 32).is_err());
        assert!(identity(0).is_err());
        assert!(identity(1).is_err());
        assert!(identity(u64::MAX).is_err());
        assert!(close(0).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn spawn_wait_complete_dispose_twice_and_stale_reuse() {
        let token = spawn(request(vec!["true"])).expect("spawn true");
        let snap = wait(token, 5_000).expect("wait true");
        assert_eq!(snap.leader_status, "completed");
        assert_eq!(snap.tree_kill_reliability, "guaranteed");
        assert_eq!(snap.boundary_strength, "cooperative_group");
        assert_eq!(snap.handle_state, "inert");
        match snap.completion {
            Some(ContainmentCompletionEvidence::Empty { .. })
            | Some(ContainmentCompletionEvidence::Unknown { .. }) => {}
            other => panic!("unexpected completion: {other:?}"),
        }
        close(token).expect("close");
        assert!(
            close(token).is_err(),
            "closed token must fail closed at FFI"
        );
        let token2 = spawn(request(vec!["true"])).expect("reuse slot");
        assert_ne!(token, token2);
        assert!(
            identity(token).is_err(),
            "stale token must not follow slot reuse"
        );
        close(token2).ok();
    }

    #[cfg(unix)]
    #[test]
    fn terminate_keeps_spawn_reliability_and_linearizes_waiters() {
        let token = spawn(request(vec!["sleep", "30"])).expect("spawn sleep");
        let identity_before = identity(token).expect("identity");
        let snap = terminate(token).expect("terminate");
        assert_eq!(snap.leader_status, "terminated");
        assert_eq!(
            snap.tree_kill_reliability,
            identity_before.tree_kill_reliability
        );
        let again = terminate(token).expect("inert terminate");
        assert_eq!(again.handle_state, "inert");
        assert_eq!(again.leader_status, "terminated");
        close(token).expect("close after terminate");
    }

    #[cfg(unix)]
    #[test]
    fn native_deadline_times_out_without_language_wait() {
        let mut req = request(vec!["sleep", "30"]);
        req.execution_timeout_ms = Some(200);
        req.grace_timeout_ms = Some(20);
        let token = spawn(req).expect("spawn with deadline");
        let snap = wait(token, 5_000).expect("wait for native deadline");
        assert_eq!(snap.leader_status, "timed_out");
        assert_eq!(snap.tree_kill_reliability, "guaranteed");
        close(token).ok();
    }

    #[cfg(windows)]
    #[test]
    fn windows_spawn_fails_before_creating_a_handle() {
        let marker = std::env::temp_dir().join(format!(
            "sysprims-containment-windows-{}.marker",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let result = spawn(request(vec![
            "cmd",
            "/C",
            &format!("echo spawned>{}", marker.display()),
        ]));
        assert!(result.is_err());
        assert!(!marker.exists(), "windows rejection must not start argv");
    }
}
