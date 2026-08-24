//! Windows implementation of timeout with Job Objects.
//!
//! Uses Job Objects with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` to ensure
//! all processes in the job are terminated when the job handle is closed.

use std::os::windows::io::AsRawHandle;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::process::{Child, Command};
use std::ptr;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::WaitForSingleObject;

use sysprims_core::{SysprimsError, SysprimsResult};

use crate::{
    capture_containment_identity, ContainmentAdoptionError, ContainmentChild, ContainmentGuard,
    ContainmentOutcome, ContainmentSpawnError, GroupingMode, SpawnInGroupConfig,
    SpawnInGroupResult, TerminateTreeConfig, TimeoutConfig, TimeoutOutcome, TreeKillReliability,
};

/// Polling interval for checking if child has exited.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub fn spawn_contained_impl(
    command: Command,
) -> Result<ContainmentGuard<Child>, ContainmentSpawnError> {
    let _ = command;
    Err(ContainmentSpawnError::Spawn(SysprimsError::not_supported(
        "spawn_contained without create-suspended Job assignment",
        "windows; use adopt_contained for explicitly unproven post-spawn adoption",
    )))
}

pub fn adopt_contained_impl<C: ContainmentChild>(
    child: C,
) -> Result<ContainmentGuard<C>, ContainmentAdoptionError<C>> {
    let pid = match child.process_id() {
        Some(pid) => pid,
        None => {
            return Err(ContainmentAdoptionError {
                error: SysprimsError::invalid_argument("child process id is unavailable"),
                child,
            });
        }
    };
    let identity = match capture_containment_identity(pid) {
        Ok(identity) => identity,
        Err(error) => return Err(ContainmentAdoptionError { error, child }),
    };
    let process = match child.raw_process_handle() {
        Some(process) => process as HANDLE,
        None => {
            return Err(ContainmentAdoptionError {
                error: SysprimsError::invalid_argument("child process handle is unavailable"),
                child,
            });
        }
    };
    let job = match create_job_object() {
        Ok(job) => unsafe { OwnedHandle::from_raw_handle(job) },
        Err(error) => return Err(ContainmentAdoptionError { error, child }),
    };
    if unsafe { AssignProcessToJobObject(job.as_raw_handle() as HANDLE, process) } == 0 {
        return Err(ContainmentAdoptionError {
            error: SysprimsError::group_creation_failed("AssignProcessToJobObject failed"),
            child,
        });
    }

    Ok(ContainmentGuard {
        child: Some(child),
        identity,
        reliability: TreeKillReliability::Unproven,
        finalized: false,
        job,
    })
}

pub fn terminate_contained_impl<C: ContainmentChild>(
    guard: &mut ContainmentGuard<C>,
    config: TerminateTreeConfig,
) -> SysprimsResult<ContainmentOutcome> {
    let child = guard
        .child
        .as_mut()
        .expect("active containment guard retains its child");
    if child.process_id() != Some(guard.identity.pid) {
        return Err(SysprimsError::invalid_argument(
            "owned child identity changed; refusing containment operation",
        ));
    }
    let job = guard.job.as_raw_handle() as HANDLE;
    if child.raw_process_handle().is_none() {
        return Err(SysprimsError::invalid_argument(
            "owned process handle is unavailable; refusing containment operation",
        ));
    }
    if unsafe { TerminateJobObject(job, 1) } == 0 {
        return Err(SysprimsError::system(
            "TerminateJobObject failed",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
        ));
    }
    let exited = wait_for_contained_child(child, Duration::from_millis(config.kill_timeout_ms))?;

    let mut warnings =
        vec!["Post-spawn Job Object adoption has an escape window and is unproven".to_string()];
    if !exited {
        warnings.push("Timed out waiting to reap contained child".to_string());
    } else {
        guard.finalized = true;
    }

    Ok(ContainmentOutcome {
        identity: guard.identity.clone(),
        pgid: None,
        signal_sent: None,
        kill_signal: None,
        escalated: false,
        exited,
        timed_out: !exited,
        tree_kill_reliability: guard.reliability,
        warnings,
    })
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
    let process = child.raw_process_handle().ok_or_else(|| {
        SysprimsError::invalid_argument(
            "owned process handle is unavailable; refusing containment operation",
        )
    })? as HANDLE;

    match unsafe { WaitForSingleObject(process, 0) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        _ => Err(SysprimsError::system(
            "failed to observe contained child state",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
        )),
    }
}

pub fn drop_contained_impl<C: ContainmentChild>(guard: &mut ContainmentGuard<C>) {
    let child = guard
        .child
        .as_mut()
        .expect("active containment guard retains its child");
    if child.process_id() != Some(guard.identity.pid) || child.raw_process_handle().is_none() {
        return;
    }

    let _ = unsafe { TerminateJobObject(guard.job.as_raw_handle() as HANDLE, 1) };
    let _ = wait_for_contained_child(child, Duration::from_secs(2));
    guard.finalized = true;
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

pub fn run_with_timeout_impl(
    command: &str,
    args: &[&str],
    timeout: Duration,
    config: &TimeoutConfig,
) -> SysprimsResult<TimeoutOutcome> {
    let use_job_object = config.grouping == GroupingMode::GroupByDefault;
    let mut reliability = TreeKillReliability::Unproven;

    // Create Job Object if GroupByDefault
    let mut job_handle: Option<HANDLE> = if use_job_object {
        match create_job_object() {
            Ok(handle) => Some(handle),
            Err(_) => {
                // Fallback: proceed without Job Object
                reliability = TreeKillReliability::BestEffort;
                None
            }
        }
    } else {
        reliability = TreeKillReliability::BestEffort;
        None
    };

    // Spawn the child process
    let mut child = Command::new(command).args(args).spawn().map_err(|e| {
        // Clean up job handle on error
        if let Some(job) = job_handle {
            unsafe { CloseHandle(job) };
        }
        if e.kind() == std::io::ErrorKind::NotFound {
            SysprimsError::not_found_command(command)
        } else if e.kind() == std::io::ErrorKind::PermissionDenied {
            SysprimsError::permission_denied_command(command)
        } else {
            SysprimsError::spawn_failed(command, e.to_string())
        }
    })?;

    // Assign process to Job Object if available
    if let Some(job) = job_handle {
        let process_handle = child.as_raw_handle() as HANDLE;
        let assigned = unsafe { AssignProcessToJobObject(job, process_handle) };
        if assigned == 0 {
            // Failed to assign - fall back to best-effort
            reliability = TreeKillReliability::BestEffort;
            unsafe { CloseHandle(job) };
            job_handle = None;
        }
    }

    let start = Instant::now();

    // Wait loop with timeout
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // Child exited within timeout
                if let Some(job) = job_handle {
                    unsafe { CloseHandle(job) };
                }
                return Ok(TimeoutOutcome::Completed {
                    exit_status: status,
                });
            }
            Ok(None) => {
                // Still running - check timeout
                if start.elapsed() >= timeout {
                    // Timeout! Kill the tree
                    return kill_tree(&mut child, job_handle, config, reliability);
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(e) => {
                if let Some(job) = job_handle {
                    unsafe { CloseHandle(job) };
                }
                return Err(SysprimsError::system(
                    format!("wait failed: {}", e),
                    e.raw_os_error().unwrap_or(0),
                ));
            }
        }
    }
}

/// Create a Job Object configured to kill all processes on close.
fn create_job_object() -> SysprimsResult<HANDLE> {
    unsafe {
        let job = CreateJobObjectW(ptr::null(), ptr::null());
        if job.is_null() || job == INVALID_HANDLE_VALUE {
            return Err(SysprimsError::group_creation_failed(
                "CreateJobObjectW failed",
            ));
        }

        // Configure job to kill all processes when handle is closed
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        let result = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );

        if result == 0 {
            CloseHandle(job);
            return Err(SysprimsError::group_creation_failed(
                "SetInformationJobObject failed",
            ));
        }

        Ok(job)
    }
}

pub fn spawn_in_group_impl(config: SpawnInGroupConfig) -> SysprimsResult<SpawnInGroupResult> {
    let _ = config;
    Err(SysprimsError::not_supported(
        "spawn_in_group without an owned containment guard",
        "windows; use spawn_contained",
    ))
}

/// Kill the process tree.
///
/// If Job Object is available, terminates the entire job.
/// Otherwise, kills only the direct child.
fn kill_tree(
    child: &mut Child,
    job_handle: Option<HANDLE>,
    config: &TimeoutConfig,
    reliability: TreeKillReliability,
) -> SysprimsResult<TimeoutOutcome> {
    if let Some(job) = job_handle {
        // Terminate all processes in the job
        // Exit code 1 is arbitrary; use sysprims-timeout CLI for nuanced codes
        unsafe {
            TerminateJobObject(job, 1);
            CloseHandle(job);
        }
    } else {
        // Fallback: terminate direct child only
        let _ = child.kill();
    }

    // Reap the child
    let _ = child.wait();

    Ok(TimeoutOutcome::TimedOut {
        signal_sent: config.signal,
        escalated: false, // Windows doesn't have signal escalation
        tree_kill_reliability: reliability,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    #[test]
    fn contained_job_terminates_owned_child() {
        let mut command = Command::new("ping");
        command
            .args(["-n", "60", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = command.spawn().expect("test child spawn failed");
        let mut guard = match adopt_contained_impl(child) {
            Ok(guard) => guard,
            Err(mut adoption) => {
                let _ = adoption.child.kill();
                let _ = adoption.child.wait();
                panic!("contained adoption failed: {}", adoption.error);
            }
        };
        assert_eq!(guard.reliability, TreeKillReliability::Unproven);
        let outcome = terminate_contained_impl(&mut guard, TerminateTreeConfig::default())
            .expect("Job termination failed");
        assert!(outcome.exited);
        assert_eq!(outcome.signal_sent, None);
    }

    #[test]
    fn contained_job_completes_normally_before_releasing_child() {
        let child = Command::new("cmd")
            .args(["/C", "exit 0"])
            .spawn()
            .expect("test child spawn failed");
        let mut guard = adopt_contained_impl(child).expect("contained adoption failed");
        let deadline = Instant::now() + Duration::from_secs(5);
        let outcome = loop {
            if let Some(outcome) = guard
                .try_complete(TerminateTreeConfig::default())
                .expect("contained completion failed")
            {
                break outcome;
            }
            assert!(Instant::now() < deadline, "contained child did not exit");
            std::thread::sleep(POLL_INTERVAL);
        };
        assert!(outcome.exited);
        let mut child = match guard.into_child() {
            Ok(child) => child,
            Err(_) => panic!("finalized guard should release its reaped child"),
        };
        assert!(child
            .try_wait()
            .expect("child status should remain available")
            .is_some());
    }

    #[test]
    fn active_guard_drop_terminates_owned_child() {
        let child = Command::new("ping")
            .args(["-n", "60", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("test child spawn failed");
        let pid = child.id();
        let guard = adopt_contained_impl(child).expect("contained adoption failed");

        drop(guard);

        assert!(
            sysprims_proc::is_fully_gone(pid).expect("failed to inspect dropped child"),
            "active guard drop must terminate and reap its child"
        );
    }

    #[test]
    fn pid_only_spawn_fails_closed() {
        let error = spawn_in_group_impl(SpawnInGroupConfig {
            argv: vec!["cmd".to_string(), "/C".to_string(), "exit 0".to_string()],
            cwd: None,
            env: None,
        })
        .expect_err("PID-only Windows spawn must fail closed");
        assert!(matches!(error, SysprimsError::NotSupported { .. }));
    }

    #[test]
    fn owned_spawn_fails_before_starting_on_windows() {
        let command = Command::new("cmd");
        let error = match spawn_contained_impl(command) {
            Ok(_) => panic!("Windows owned spawn requires a suspended-launch seam"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            ContainmentSpawnError::Spawn(SysprimsError::NotSupported { .. })
        ));
    }
}
