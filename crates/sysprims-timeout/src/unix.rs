//! Unix implementation of timeout with process groups.
//!
//! Uses `setpgid(0, 0)` to create a new process group with the child as leader,
//! then `killpg()` to signal the entire group on timeout.

use std::os::unix::process::CommandExt;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use libc::{killpg, SIGKILL};
use sysprims_core::{SysprimsError, SysprimsResult};

use crate::{
    capture_containment_identity, completion_from_pids, unknown_completion,
    verify_containment_identity, ContainmentAdoptionError, ContainmentChild,
    ContainmentCompletionEvidence, ContainmentGuard, ContainmentObservation, ContainmentOutcome,
    ContainmentSpawnError, GroupingMode, TerminateTreeConfig, TimeoutConfig, TimeoutOutcome,
    TreeKillReliability, MAX_COMPLETION_OBSERVATION_RETRIES, MAX_COMPLETION_OBSERVED_PIDS,
};
use crate::{SpawnInGroupConfig, SpawnInGroupResult};
use sysprims_core::get_platform;
use sysprims_core::schema::SPAWN_IN_GROUP_RESULT_V1;

/// Polling interval for checking if child has exited.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub fn spawn_contained_impl(
    mut command: Command,
) -> Result<ContainmentGuard<Child>, ContainmentSpawnError> {
    let program = command.get_program().to_string_lossy().into_owned();
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = command.spawn().map_err(|error| {
        ContainmentSpawnError::Spawn(if error.kind() == std::io::ErrorKind::NotFound {
            SysprimsError::not_found_command(&program)
        } else if error.kind() == std::io::ErrorKind::PermissionDenied {
            SysprimsError::permission_denied_command(&program)
        } else {
            SysprimsError::spawn_failed(&program, error.to_string())
        })
    })?;
    let mut guard = match adopt_contained_impl(child) {
        Ok(guard) => guard,
        Err(mut adoption) => {
            if let Some(pid) = adoption.child.process_id() {
                let _ = sysprims_signal::killpg(pid, libc::SIGKILL);
            }
            let _ = adoption.child.kill();
            let _ = adoption.child.wait();
            return Err(ContainmentSpawnError::Adoption(adoption.error));
        }
    };
    guard.reliability = TreeKillReliability::Guaranteed;
    Ok(guard)
}

pub fn adopt_contained_impl<C: ContainmentChild>(
    child: C,
) -> Result<ContainmentGuard<C>, ContainmentAdoptionError<C>> {
    let evidence = match acquire_unix_evidence(&child) {
        Ok(evidence) => evidence,
        Err(error) => return Err(ContainmentAdoptionError { error, child }),
    };

    Ok(ContainmentGuard {
        child: Some(child),
        identity: evidence.0,
        reliability: TreeKillReliability::Unproven,
        finalized: false,
        pgid: evidence.1,
        session_id: evidence.2,
    })
}

fn acquire_unix_evidence<C: ContainmentChild>(
    child: &C,
) -> SysprimsResult<(crate::ContainmentIdentity, u32, u32)> {
    let pid = child
        .process_id()
        .ok_or_else(|| SysprimsError::invalid_argument("child process id is unavailable"))?;
    let identity = capture_containment_identity(pid)?;
    let pid_i32 = pid as i32;
    let pgid = unsafe { libc::getpgid(pid_i32) };
    let session_id = unsafe { libc::getsid(pid_i32) };
    if pgid <= 0 || session_id <= 0 {
        return Err(SysprimsError::system(
            "process group or session unavailable during containment acquisition",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
        ));
    }
    if pgid != pid_i32 {
        return Err(SysprimsError::invalid_argument(
            "child is not a process-group leader; refusing group adoption",
        ));
    }

    let caller_pgid = unsafe { libc::getpgid(0) };
    if caller_pgid == pgid {
        return Err(SysprimsError::invalid_argument(
            "child shares the caller's process group; refusing group adoption",
        ));
    }

    Ok((identity, pgid as u32, session_id as u32))
}

pub fn terminate_contained_impl<C: ContainmentChild>(
    guard: &mut ContainmentGuard<C>,
    config: TerminateTreeConfig,
) -> SysprimsResult<ContainmentOutcome> {
    validate_signal(config.signal, "signal")?;
    validate_signal(config.kill_signal, "kill_signal")?;

    let child = guard
        .child
        .as_mut()
        .expect("active containment guard retains its child");
    if child.process_id() != Some(guard.identity.pid) {
        return Err(SysprimsError::invalid_argument(
            "owned child identity changed; refusing containment operation",
        ));
    }
    let identity_metadata_verified = verify_containment_identity(&guard.identity)?;

    if identity_metadata_verified {
        let pid_i32 = guard.identity.pid as i32;
        let current_pgid = unsafe { libc::getpgid(pid_i32) };
        let current_session_id = unsafe { libc::getsid(pid_i32) };
        if current_pgid != guard.pgid as i32 || current_session_id != guard.session_id as i32 {
            return Err(SysprimsError::invalid_argument(
                "process group identity changed; refusing containment operation",
            ));
        }
    }

    if unsafe { libc::getpgid(0) } == guard.pgid as i32 {
        return Err(SysprimsError::invalid_argument(
            "contained group matches the caller's group; refusing group signal",
        ));
    }

    let mut warnings = Vec::new();
    if !identity_metadata_verified {
        warnings.push(
            "Live identity metadata unavailable; proceeding with owned child and bound group"
                .to_string(),
        );
    }

    let graceful_sent = match sysprims_signal::killpg(guard.pgid, config.signal) {
        Ok(()) => true,
        Err(SysprimsError::NotFound { .. }) if !identity_metadata_verified => {
            warnings.push("Process group exited before cleanup signal".to_string());
            false
        }
        #[cfg(target_os = "macos")]
        Err(error @ SysprimsError::PermissionDenied { .. }) if !identity_metadata_verified => {
            let completion = observe_containment_completion(guard.pgid, guard.session_id);
            if !matches!(completion, ContainmentCompletionEvidence::Empty { .. }) {
                return Err(error);
            }
            warnings.push(
                "macOS process-group enumeration confirmed no live descendants; proceeding to reap"
                    .to_string(),
            );
            false
        }
        Err(error) => return Err(error),
    };

    if graceful_sent {
        std::thread::sleep(Duration::from_millis(config.grace_timeout_ms));
    }

    let escalated = if graceful_sent {
        match sysprims_signal::killpg(guard.pgid, config.kill_signal) {
            Ok(()) => true,
            Err(SysprimsError::NotFound { .. }) => {
                warnings.push("Process group exited before escalation".to_string());
                false
            }
            Err(error) => return Err(error),
        }
    } else {
        false
    };

    let _ = wait_for_contained_child_exit(
        guard.identity.pid,
        Duration::from_millis(config.kill_timeout_ms),
    );
    let completion = observe_containment_completion(guard.pgid, guard.session_id);
    let exited = reap_contained_child(child)?;
    if !exited {
        warnings.push("Timed out waiting to reap contained child".to_string());
    } else {
        guard.finalized = true;
    }

    Ok(ContainmentOutcome {
        identity: guard.identity.clone(),
        pgid: Some(guard.pgid),
        signal_sent: graceful_sent.then_some(config.signal),
        kill_signal: escalated.then_some(config.kill_signal),
        escalated,
        exited,
        timed_out: !exited,
        tree_kill_reliability: guard.reliability,
        completion,
        warnings,
    })
}

fn stabilize_completion<F>(
    observation: ContainmentObservation,
    mut snapshot: F,
) -> ContainmentCompletionEvidence
where
    F: FnMut() -> Result<Vec<u32>, ()>,
{
    let mut previous = None;
    for _ in 0..=MAX_COMPLETION_OBSERVATION_RETRIES {
        let current = match snapshot() {
            Ok(mut pids) => {
                pids.sort_unstable();
                pids
            }
            Err(()) => return unknown_completion(observation),
        };
        if previous.as_ref() == Some(&current) {
            return completion_from_pids(observation, current);
        }
        previous = Some(current);
        std::thread::sleep(POLL_INTERVAL);
    }
    unknown_completion(observation)
}

#[cfg(target_os = "linux")]
fn observe_containment_completion(pgid: u32, session_id: u32) -> ContainmentCompletionEvidence {
    stabilize_completion(ContainmentObservation::LinuxProcfsProcessGroup, || {
        linux_group_snapshot(pgid, session_id)
    })
}

#[cfg(target_os = "linux")]
fn linux_group_snapshot(pgid: u32, session_id: u32) -> Result<Vec<u32>, ()> {
    let entries = std::fs::read_dir("/proc").map_err(|_| ())?;
    let mut members = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| ())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if pid == 0 || pid > sysprims_signal::MAX_SAFE_PID {
            return Err(());
        }
        let stat = match std::fs::read(entry.path().join("stat")) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(()),
        };
        let (state, process_group, session) = parse_linux_stat_membership(&stat)?;
        if process_group == pgid && session == session_id && !matches!(state, 'Z' | 'X' | 'x') {
            if members.len() == MAX_COMPLETION_OBSERVED_PIDS {
                return Err(());
            }
            members.push(pid);
        }
    }
    Ok(members)
}

#[cfg(target_os = "linux")]
fn parse_linux_stat_membership(stat: &[u8]) -> Result<(char, u32, u32), ()> {
    let end_paren = stat.iter().rposition(|byte| *byte == b')').ok_or(())?;
    let suffix = std::str::from_utf8(&stat[end_paren + 1..]).map_err(|_| ())?;
    let mut fields = suffix.split_whitespace();
    let state = fields
        .next()
        .and_then(|value| value.chars().next())
        .ok_or(())?;
    let _parent_pid = fields.next().ok_or(())?;
    let process_group = fields.next().ok_or(())?.parse().map_err(|_| ())?;
    let session = fields.next().ok_or(())?.parse().map_err(|_| ())?;
    Ok((state, process_group, session))
}

#[cfg(target_os = "macos")]
fn observe_containment_completion(pgid: u32, _session_id: u32) -> ContainmentCompletionEvidence {
    stabilize_completion(ContainmentObservation::MacosLibprocProcessGroup, || {
        macos_group_snapshot(pgid)
    })
}

#[cfg(target_os = "macos")]
fn macos_group_snapshot(pgid: u32) -> Result<Vec<u32>, ()> {
    use std::ffi::c_void;

    const PROC_PGRP_ONLY: u32 = 2;

    unsafe extern "C" {
        fn proc_listpids(
            type_: u32,
            typeinfo: u32,
            buffer: *mut c_void,
            buffersize: libc::c_int,
        ) -> libc::c_int;
    }

    macos_group_snapshot_with(
        |buffer| {
            let written = match buffer {
                None => unsafe { proc_listpids(PROC_PGRP_ONLY, pgid, std::ptr::null_mut(), 0) },
                Some(buffer) => unsafe {
                    proc_listpids(
                        PROC_PGRP_ONLY,
                        pgid,
                        buffer.as_mut_ptr().cast(),
                        std::mem::size_of_val(buffer) as libc::c_int,
                    )
                },
            };
            if written < 0 || !(written as usize).is_multiple_of(std::mem::size_of::<i32>()) {
                return Err(());
            }
            Ok(written as usize / std::mem::size_of::<i32>())
        },
        |pid| sysprims_proc::is_live(pid).map_err(|_| ()),
    )
}

#[cfg(target_os = "macos")]
fn macos_group_snapshot_with<F, L>(mut list: F, mut is_live: L) -> Result<Vec<u32>, ()>
where
    F: FnMut(Option<&mut [i32]>) -> Result<usize, ()>,
    L: FnMut(u32) -> Result<bool, ()>,
{
    for _ in 0..=MAX_COMPLETION_OBSERVATION_RETRIES {
        let reported = list(None)?;
        if reported >= MAX_COMPLETION_OBSERVED_PIDS {
            return Err(());
        }
        let capacity = reported + 1;
        let mut pids = Vec::new();
        pids.try_reserve_exact(capacity).map_err(|_| ())?;
        pids.resize(capacity, 0_i32);
        let count = list(Some(&mut pids))?;
        if count >= capacity {
            continue;
        }
        let mut members = Vec::new();
        for pid in pids.into_iter().take(count).filter(|pid| *pid > 0) {
            let pid = pid as u32;
            if pid > sysprims_signal::MAX_SAFE_PID {
                return Err(());
            }
            if is_live(pid)? {
                members.push(pid);
            }
        }
        return Ok(members);
    }
    Err(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn observe_containment_completion(_pgid: u32, _session_id: u32) -> ContainmentCompletionEvidence {
    unknown_completion(ContainmentObservation::UnsupportedPlatform)
}

pub fn contained_child_has_exited<C: ContainmentChild>(
    guard: &ContainmentGuard<C>,
) -> SysprimsResult<bool> {
    let child = guard
        .child
        .as_ref()
        .expect("active containment guard retains its child");
    if child.process_id() != Some(guard.identity.pid) {
        return Err(SysprimsError::invalid_argument(
            "owned child identity changed; refusing containment operation",
        ));
    }

    sysprims_proc::is_live(guard.identity.pid).map(|live| !live)
}

pub fn drop_contained_impl<C: ContainmentChild>(guard: &mut ContainmentGuard<C>) {
    let child = guard
        .child
        .as_mut()
        .expect("active containment guard retains its child");
    if child.process_id() != Some(guard.identity.pid)
        || unsafe { libc::getpgid(0) } == guard.pgid as i32
    {
        return;
    }

    let _ = sysprims_signal::killpg(guard.pgid, SIGKILL);
    let _ = wait_for_contained_child(child, Duration::from_secs(2));
    guard.finalized = true;
}

fn validate_signal(signal: i32, name: &str) -> SysprimsResult<()> {
    #[cfg(target_os = "linux")]
    let max_signal = libc::SIGRTMAX();
    #[cfg(not(target_os = "linux"))]
    let max_signal = 31;

    if signal <= 0 || signal > max_signal {
        return Err(SysprimsError::invalid_argument(format!(
            "{name} must be between 1 and {max_signal}"
        )));
    }
    Ok(())
}

fn wait_for_contained_child<C: ContainmentChild>(
    child: &mut C,
    timeout: Duration,
) -> SysprimsResult<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(true) => return Ok(true),
            Ok(false) if Instant::now() < deadline => std::thread::sleep(POLL_INTERVAL),
            Ok(false) => return Ok(false),
            Err(error) => {
                return Err(SysprimsError::system(
                    format!("failed to reap contained child: {error}"),
                    error.raw_os_error().unwrap_or(0),
                ));
            }
        }
    }
}

fn wait_for_contained_child_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match sysprims_proc::is_live(pid) {
            Ok(false) | Err(SysprimsError::NotFound { .. }) => return true,
            Ok(true) | Err(_) if Instant::now() < deadline => {
                std::thread::sleep(POLL_INTERVAL);
            }
            Ok(true) | Err(_) => return false,
        }
    }
}

fn reap_contained_child<C: ContainmentChild>(child: &mut C) -> SysprimsResult<bool> {
    child.try_wait().map_err(|error| {
        SysprimsError::system(
            format!("failed to reap contained child: {error}"),
            error.raw_os_error().unwrap_or(0),
        )
    })
}

pub fn spawn_in_group_impl(config: SpawnInGroupConfig) -> SysprimsResult<SpawnInGroupResult> {
    let command = config.argv[0].as_str();
    if command.is_empty() {
        return Err(SysprimsError::invalid_argument(
            "argv[0] (command) must not be empty",
        ));
    }

    let args: Vec<&str> = config.argv.iter().skip(1).map(|s| s.as_str()).collect();

    let mut cmd = Command::new(command);
    cmd.args(args);

    if let Some(cwd) = config.cwd.as_deref() {
        if !cwd.is_empty() {
            cmd.current_dir(cwd);
        }
    }

    if let Some(env) = config.env {
        for (k, v) in env {
            cmd.env(k, v);
        }
    }

    // New process group: child becomes leader (pid == pgid).
    unsafe {
        cmd.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SysprimsError::not_found_command(command)
        } else if e.kind() == std::io::ErrorKind::PermissionDenied {
            SysprimsError::permission_denied_command(command)
        } else {
            SysprimsError::spawn_failed(command, e.to_string())
        }
    })?;

    let pid = child.id();

    Ok(SpawnInGroupResult {
        schema_id: SPAWN_IN_GROUP_RESULT_V1,
        timestamp: sysprims_core::time::now_rfc3339(),
        platform: get_platform(),
        pid,
        pgid: Some(pid),
        tree_kill_reliability: "guaranteed".to_string(),
        warnings: vec![],
    })
}

pub fn run_with_timeout_impl(
    command: &str,
    args: &[&str],
    timeout: Duration,
    config: &TimeoutConfig,
) -> SysprimsResult<TimeoutOutcome> {
    let mut cmd = Command::new(command);
    cmd.args(args);

    // Set up process group if GroupByDefault
    let use_process_group = config.grouping == GroupingMode::GroupByDefault;

    if use_process_group {
        // SAFETY: setpgid(0, 0) creates a new process group with the child's
        // PID as the PGID. This is safe and standard practice for job control.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    // Spawn the child process
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SysprimsError::not_found_command(command)
        } else if e.kind() == std::io::ErrorKind::PermissionDenied {
            SysprimsError::permission_denied_command(command)
        } else {
            SysprimsError::spawn_failed(command, e.to_string())
        }
    })?;

    let child_pid = child.id() as i32;
    let start = Instant::now();

    // Wait loop with timeout
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // Child exited within timeout
                return Ok(TimeoutOutcome::Completed {
                    exit_status: status,
                });
            }
            Ok(None) => {
                // Still running - check timeout
                if start.elapsed() >= timeout {
                    // Timeout! Kill the tree
                    return kill_tree(child_pid, &mut child, config, use_process_group);
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(e) => {
                return Err(SysprimsError::system(
                    format!("wait failed: {}", e),
                    e.raw_os_error().unwrap_or(0),
                ));
            }
        }
    }
}

/// Kill the process tree and wait for exit.
///
/// If using process group, sends signal to entire group via `killpg()`.
/// Otherwise, sends signal to direct child only.
///
/// IMPORTANT: When using process groups, we ALWAYS send SIGKILL after
/// `kill_after` duration, even if the group leader has exited. This is
/// because background children may have trapped SIGTERM and the leader
/// exiting doesn't mean all group members are dead.
fn kill_tree(
    pid: i32,
    child: &mut Child,
    config: &TimeoutConfig,
    use_process_group: bool,
) -> SysprimsResult<TimeoutOutcome> {
    let reliability = if use_process_group {
        TreeKillReliability::Guaranteed
    } else {
        TreeKillReliability::BestEffort
    };

    // Send initial signal
    if use_process_group {
        // Child is process group leader, so pid == pgid
        // SAFETY: killpg is safe with valid pgid and signal
        unsafe {
            killpg(pid, config.signal);
        }
    } else {
        // Foreground mode: signal direct child only
        // Use sysprims_signal for consistency
        let _ = sysprims_signal::kill(pid as u32, config.signal);
    }

    // Wait for kill_after duration for graceful exit
    let escalation_deadline = Instant::now() + config.kill_after;
    let mut leader_exited = false;

    while Instant::now() < escalation_deadline {
        if !leader_exited && child.try_wait().ok().flatten().is_some() {
            leader_exited = true;
            // For non-group mode, we can return early since we only care about the direct child
            if !use_process_group {
                return Ok(TimeoutOutcome::TimedOut {
                    signal_sent: config.signal,
                    escalated: false,
                    tree_kill_reliability: reliability,
                });
            }
            // For group mode, continue waiting - other group members may still be alive
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    // Escalate to SIGKILL
    // For process groups, ALWAYS send SIGKILL to ensure trapped processes are killed
    let escalated = if use_process_group {
        // SAFETY: killpg with SIGKILL to ensure termination of entire group
        // This may signal already-dead processes (ESRCH) which is harmless
        unsafe {
            killpg(pid, SIGKILL);
        }
        true
    } else {
        let _ = sysprims_signal::force_kill(pid as u32);
        true
    };

    // Reap the zombie (if not already reaped)
    let _ = child.wait();

    Ok(TimeoutOutcome::TimedOut {
        signal_sent: config.signal,
        escalated,
        tree_kill_reliability: reliability,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_completes_fast_command() {
        let result = run_with_timeout_impl(
            "echo",
            &["hello"],
            Duration::from_secs(10),
            &TimeoutConfig::default(),
        )
        .unwrap();

        assert!(matches!(result, TimeoutOutcome::Completed { .. }));
    }

    #[test]
    fn timeout_triggers_on_slow_command() {
        let result = run_with_timeout_impl(
            "sleep",
            &["60"],
            Duration::from_millis(100),
            &TimeoutConfig {
                kill_after: Duration::from_millis(100),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(matches!(result, TimeoutOutcome::TimedOut { .. }));
    }

    #[test]
    fn timeout_returns_not_found_for_missing_command() {
        let result = run_with_timeout_impl(
            "nonexistent_command_12345",
            &[],
            Duration::from_secs(10),
            &TimeoutConfig::default(),
        );

        assert!(matches!(result, Err(SysprimsError::NotFoundCommand { .. })));
    }

    #[test]
    fn normal_completion_is_observed_without_early_reap() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 0.1"]);
        let mut guard = spawn_contained_impl(command).expect("contained spawn failed");
        let deadline = Instant::now() + Duration::from_secs(5);

        loop {
            if let Some(outcome) = guard
                .try_complete(TerminateTreeConfig {
                    grace_timeout_ms: 0,
                    kill_timeout_ms: 500,
                    ..TerminateTreeConfig::default()
                })
                .expect("normal completion failed")
            {
                assert!(outcome.exited);
                assert!(matches!(
                    outcome.completion,
                    ContainmentCompletionEvidence::Empty { .. }
                ));
                break;
            }
            assert!(Instant::now() < deadline, "contained child did not exit");
            std::thread::sleep(POLL_INTERVAL);
        }

        assert!(guard.into_child().is_ok());
    }

    #[test]
    fn completion_observation_requires_a_stable_snapshot() {
        let mut attempts = 0;
        let completion =
            stabilize_completion(ContainmentObservation::LinuxProcfsProcessGroup, || {
                attempts += 1;
                Ok(if attempts == 1 {
                    vec![41]
                } else {
                    vec![29, 41]
                })
            });
        assert_eq!(attempts, 3);
        assert_eq!(
            completion,
            ContainmentCompletionEvidence::Survivors {
                observation: ContainmentObservation::LinuxProcfsProcessGroup,
                observed_count: 2,
                survivor_pids: vec![29, 41],
            }
        );
    }

    #[test]
    fn completion_observation_failure_is_unknown() {
        assert_eq!(
            stabilize_completion(ContainmentObservation::LinuxProcfsProcessGroup, || Err(())),
            ContainmentCompletionEvidence::Unknown {
                observation: ContainmentObservation::LinuxProcfsProcessGroup,
            }
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_stat_membership_parser_handles_complex_command_names() {
        assert_eq!(
            parse_linux_stat_membership(b"17 (worker ) pool) S 3 17 9 0 0"),
            Ok(('S', 17, 9))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_stat_membership_parser_ignores_non_utf8_command_names() {
        let stat = b"17 (worker \xff pool) S 3 17 9 0 0";
        assert_eq!(parse_linux_stat_membership(stat), Ok(('S', 17, 9)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_full_buffer_race_exhausts_bounded_retries() {
        let mut fetches = 0;
        let result = macos_group_snapshot_with(
            |buffer| match buffer {
                None => Ok(1),
                Some(buffer) => {
                    fetches += 1;
                    Ok(buffer.len())
                }
            },
            |_| Ok(true),
        );
        assert_eq!(fetches, MAX_COMPLETION_OBSERVATION_RETRIES + 1);
        assert_eq!(result, Err(()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_snapshot_returns_complete_live_members() {
        let result = macos_group_snapshot_with(
            |buffer| match buffer {
                None => Ok(2),
                Some(buffer) => {
                    buffer[0] = 23;
                    buffer[1] = 17;
                    Ok(2)
                }
            },
            |_| Ok(true),
        );
        assert_eq!(result, Ok(vec![23, 17]));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_snapshot_failure_is_unknown_completion() {
        let completion =
            stabilize_completion(ContainmentObservation::MacosLibprocProcessGroup, || {
                macos_group_snapshot_with(
                    |_| Err(()),
                    |_| panic!("liveness must not run after enumeration failure"),
                )
            });
        assert_eq!(
            completion,
            ContainmentCompletionEvidence::Unknown {
                observation: ContainmentObservation::MacosLibprocProcessGroup,
            }
        );
    }

    #[test]
    fn foreground_mode_does_not_create_process_group() {
        let config = TimeoutConfig {
            grouping: GroupingMode::Foreground,
            kill_after: Duration::from_millis(100),
            ..Default::default()
        };

        let result =
            run_with_timeout_impl("sleep", &["60"], Duration::from_millis(100), &config).unwrap();

        if let TimeoutOutcome::TimedOut {
            tree_kill_reliability,
            ..
        } = result
        {
            assert_eq!(tree_kill_reliability, TreeKillReliability::BestEffort);
        } else {
            panic!("Expected timeout");
        }
    }
}
