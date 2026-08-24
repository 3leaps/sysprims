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
    capture_containment_identity, verify_containment_identity, ContainmentAdoptionError,
    ContainmentChild, ContainmentGuard, ContainmentOutcome, ContainmentSpawnError, GroupingMode,
    TerminateTreeConfig, TimeoutConfig, TimeoutOutcome, TreeKillReliability,
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
            if macos_group_has_live_descendants(guard.pgid, guard.identity.pid)? {
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

    let exited = wait_for_contained_child(child, Duration::from_millis(config.kill_timeout_ms))?;
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
        warnings,
    })
}

#[cfg(target_os = "macos")]
fn macos_group_has_live_descendants(pgid: u32, leader_pid: u32) -> SysprimsResult<bool> {
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

    let byte_count = unsafe { proc_listpids(PROC_PGRP_ONLY, pgid, std::ptr::null_mut(), 0) };
    if byte_count < 0 {
        return Err(SysprimsError::system(
            "failed to size macOS process-group enumeration",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
        ));
    }

    let mut pids = vec![0_i32; byte_count as usize / std::mem::size_of::<i32>() + 1];
    let written = unsafe {
        proc_listpids(
            PROC_PGRP_ONLY,
            pgid,
            pids.as_mut_ptr().cast(),
            (pids.len() * std::mem::size_of::<i32>()) as libc::c_int,
        )
    };
    if written < 0 {
        return Err(SysprimsError::system(
            "failed to enumerate macOS process group",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
        ));
    }

    let count = written as usize / std::mem::size_of::<i32>();
    for pid in pids.into_iter().take(count).filter(|pid| *pid > 0) {
        let pid = pid as u32;
        if pid != leader_pid && sysprims_proc::is_live(pid)? {
            return Ok(true);
        }
    }
    Ok(false)
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
                break;
            }
            assert!(Instant::now() < deadline, "contained child did not exit");
            std::thread::sleep(POLL_INTERVAL);
        }

        assert!(guard.into_child().is_ok());
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
