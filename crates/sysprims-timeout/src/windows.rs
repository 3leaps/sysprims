//! Windows implementation of timeout with Job Objects.
//!
//! Uses Job Objects with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` to ensure
//! all processes in the job are terminated when the job handle is closed.

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::process::{Child, Command};
use std::ptr;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_MORE_DATA, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JobObjectBasicProcessIdList,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{GetProcessId, WaitForSingleObject};

use sysprims_core::{SysprimsError, SysprimsResult};

use crate::{
    capture_containment_identity, completion_from_pids, unknown_completion,
    ContainmentAdoptionError, ContainmentBoundaryStrength, ContainmentChild,
    ContainmentCompletionEvidence, ContainmentGuard, ContainmentObservation, ContainmentOutcome,
    ContainmentSpawnError, GroupingMode, SpawnInGroupConfig, SpawnInGroupResult,
    TerminateTreeConfig, TimeoutConfig, TimeoutOutcome, TreeKillReliability,
    MAX_COMPLETION_OBSERVATION_RETRIES, MAX_COMPLETION_OBSERVED_PIDS,
};

/// Polling interval for checking if child has exited.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// A prepared non-breakaway Windows Job that has not yet acquired a process.
///
/// Preparation configures `KILL_ON_JOB_CLOSE` before process creation. This
/// value is single-use: assignment consumes it and either produces a sealed
/// receipt or closes the Job.
pub struct PreparedWindowsJob {
    job: OwnedHandle,
}

/// Sealed evidence that one exact process was assigned to a prepared Job.
///
/// The receipt is opaque and single-use. Dropping it closes the Job, which is a
/// termination action for an assigned process.
pub struct WindowsJobReceipt {
    job: OwnedHandle,
    process_handle: usize,
    process_id: u32,
}

impl PreparedWindowsJob {
    pub fn new() -> SysprimsResult<Self> {
        let job = create_job_object()?;
        Ok(Self {
            // SAFETY: create_job_object returned a fresh owned Job handle.
            job: unsafe { OwnedHandle::from_raw_handle(job) },
        })
    }

    /// Assign and verify the exact still-suspended process.
    ///
    /// This is the only assignment operation on a prepared Job. The Job is
    /// consumed even on failure, so callers cannot retry assignment or weaken
    /// policy after an error.
    ///
    /// # Safety
    ///
    /// `process` must be the live process handle returned by the adapter's one
    /// suspended `CreateProcessW` call. The primary thread must not have been
    /// resumed, and the adapter must retain ownership of both handles until the
    /// receipt is consumed with that same child.
    pub unsafe fn assign_process(self, process: RawHandle) -> SysprimsResult<WindowsJobReceipt> {
        let process = process as HANDLE;
        if process.is_null() || process == INVALID_HANDLE_VALUE {
            return Err(SysprimsError::invalid_argument(
                "suspended child process handle is invalid",
            ));
        }
        let process_id = GetProcessId(process);
        if process_id == 0 {
            return Err(last_system_error("GetProcessId failed"));
        }
        let job = self.job.as_raw_handle() as HANDLE;
        if AssignProcessToJobObject(job, process) == 0 {
            return Err(last_group_error("AssignProcessToJobObject failed"));
        }
        let mut member = 0;
        if IsProcessInJob(process, job, &mut member) == 0 {
            return Err(last_group_error("IsProcessInJob failed"));
        }
        if member == 0 {
            return Err(SysprimsError::group_creation_failed(
                "exact suspended child is not a member of the prepared Job",
            ));
        }

        Ok(WindowsJobReceipt {
            job: self.job,
            process_handle: process as usize,
            process_id,
        })
    }
}

fn last_system_error(operation: &str) -> SysprimsError {
    SysprimsError::system(
        operation,
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
    )
}

fn last_group_error(operation: &str) -> SysprimsError {
    let error = std::io::Error::last_os_error();
    SysprimsError::group_creation_failed(format!("{operation}: {error}"))
}

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
        boundary_strength: ContainmentBoundaryStrength::Unknown,
        finalized: false,
        job,
    })
}

pub fn contain_acquired_windows_job_impl<C: ContainmentChild>(
    child: C,
    receipt: WindowsJobReceipt,
) -> Result<ContainmentGuard<C>, ContainmentAdoptionError<C>> {
    let pid = match child.process_id() {
        Some(pid) if pid == receipt.process_id => pid,
        _ => {
            return Err(ContainmentAdoptionError {
                error: SysprimsError::invalid_argument(
                    "Windows Job receipt process id does not match the owned child",
                ),
                child,
            });
        }
    };
    let process = match child.raw_process_handle() {
        Some(process) if process as usize == receipt.process_handle => process as HANDLE,
        _ => {
            return Err(ContainmentAdoptionError {
                error: SysprimsError::invalid_argument(
                    "Windows Job receipt process handle does not match the owned child",
                ),
                child,
            });
        }
    };
    if unsafe { GetProcessId(process) } != pid {
        return Err(ContainmentAdoptionError {
            error: SysprimsError::invalid_argument(
                "owned Windows process handle no longer identifies the receipt child",
            ),
            child,
        });
    }
    let job = receipt.job;
    let mut member = 0;
    if unsafe { IsProcessInJob(process, job.as_raw_handle() as HANDLE, &mut member) } == 0 {
        return Err(ContainmentAdoptionError {
            error: last_group_error("IsProcessInJob receipt verification failed"),
            child,
        });
    }
    if member == 0 {
        return Err(ContainmentAdoptionError {
            error: SysprimsError::group_creation_failed(
                "owned child is no longer a member of the prepared Job",
            ),
            child,
        });
    }
    let identity = match capture_containment_identity(pid) {
        Ok(identity) => identity,
        Err(error) => return Err(ContainmentAdoptionError { error, child }),
    };

    Ok(ContainmentGuard {
        child: Some(child),
        identity,
        reliability: TreeKillReliability::Guaranteed,
        boundary_strength: ContainmentBoundaryStrength::KernelEnforcedJob,
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
    let _ = wait_for_contained_child_exit(child, Duration::from_millis(config.kill_timeout_ms))?;
    let completion = observe_job_completion(job);
    let exited = reap_contained_child(child)?;

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
        boundary_strength: guard.boundary_strength,
        completion,
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

fn wait_for_contained_child_exit<C: ContainmentChild>(
    child: &C,
    timeout: Duration,
) -> SysprimsResult<bool> {
    let process = child.raw_process_handle().ok_or_else(|| {
        SysprimsError::invalid_argument(
            "owned process handle is unavailable; refusing containment operation",
        )
    })? as HANDLE;
    let deadline = Instant::now() + timeout;
    loop {
        match unsafe { WaitForSingleObject(process, 0) } {
            WAIT_OBJECT_0 => return Ok(true),
            WAIT_TIMEOUT if Instant::now() < deadline => std::thread::sleep(POLL_INTERVAL),
            WAIT_TIMEOUT => return Ok(false),
            _ => {
                return Err(SysprimsError::system(
                    "failed to observe contained child state",
                    std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
                ));
            }
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

#[derive(Debug)]
struct JobProcessSnapshot {
    assigned: usize,
    pids: Vec<u32>,
}

#[derive(Debug, PartialEq, Eq)]
enum JobQueryError {
    RetryWithCapacity(usize),
    Unavailable,
}

const INITIAL_JOB_PROCESS_ID_CAPACITY: usize = 64;

fn query_job_process_snapshot(
    job: HANDLE,
    capacity: usize,
) -> Result<JobProcessSnapshot, JobQueryError> {
    if capacity == 0 || capacity >= MAX_COMPLETION_OBSERVED_PIDS {
        return Err(JobQueryError::Unavailable);
    }
    let pid_word_offset = std::mem::offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList)
        / std::mem::size_of::<usize>();
    let mut words = Vec::<usize>::new();
    words
        .try_reserve_exact(capacity + pid_word_offset)
        .map_err(|_| JobQueryError::Unavailable)?;
    words.resize(capacity + pid_word_offset, 0);
    let byte_len = words
        .len()
        .checked_mul(std::mem::size_of::<usize>())
        .and_then(|length| u32::try_from(length).ok())
        .ok_or(JobQueryError::Unavailable)?;
    let success = unsafe {
        QueryInformationJobObject(
            job,
            JobObjectBasicProcessIdList,
            words.as_mut_ptr().cast(),
            byte_len,
            ptr::null_mut(),
        )
    };
    let header = unsafe { &*words.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>() };
    let assigned = header.NumberOfAssignedProcesses as usize;
    let listed = header.NumberOfProcessIdsInList as usize;
    if success == 0 {
        let error = std::io::Error::last_os_error().raw_os_error();
        return if error == Some(ERROR_MORE_DATA as i32) {
            Err(JobQueryError::RetryWithCapacity(assigned.max(listed)))
        } else {
            Err(JobQueryError::Unavailable)
        };
    }
    if listed > capacity || listed >= MAX_COMPLETION_OBSERVED_PIDS {
        return Err(JobQueryError::Unavailable);
    }
    let mut pids = Vec::new();
    pids.try_reserve_exact(listed)
        .map_err(|_| JobQueryError::Unavailable)?;
    for pid in &words[pid_word_offset..pid_word_offset + listed] {
        pids.push(u32::try_from(*pid).map_err(|_| JobQueryError::Unavailable)?);
    }
    Ok(JobProcessSnapshot { assigned, pids })
}

fn observe_job_completion(job: HANDLE) -> ContainmentCompletionEvidence {
    observe_job_completion_with(|capacity| query_job_process_snapshot(job, capacity))
}

fn observe_job_completion_with<F>(mut query: F) -> ContainmentCompletionEvidence
where
    F: FnMut(usize) -> Result<JobProcessSnapshot, JobQueryError>,
{
    let observation = ContainmentObservation::WindowsJobProcessIdList;
    let mut capacity = INITIAL_JOB_PROCESS_ID_CAPACITY;
    for _ in 0..=MAX_COMPLETION_OBSERVATION_RETRIES {
        let snapshot = match query(capacity) {
            Ok(snapshot) => snapshot,
            Err(JobQueryError::RetryWithCapacity(required)) => {
                let next = required.max(capacity.saturating_mul(2));
                if next >= MAX_COMPLETION_OBSERVED_PIDS {
                    return unknown_completion(observation);
                }
                capacity = next;
                continue;
            }
            Err(JobQueryError::Unavailable) => return unknown_completion(observation),
        };
        if snapshot.assigned == snapshot.pids.len()
            && snapshot.assigned < MAX_COMPLETION_OBSERVED_PIDS
        {
            return completion_from_pids(observation, snapshot.pids);
        }
        let next = snapshot.assigned.max(capacity);
        if next >= MAX_COMPLETION_OBSERVED_PIDS {
            return unknown_completion(observation);
        }
        capacity = next;
    }
    unknown_completion(observation)
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
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    use windows_sys::Win32::System::JobObjects::{
        JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
    };
    use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

    #[test]
    fn prepared_job_disables_breakaway_modes() {
        let prepared = PreparedWindowsJob::new().expect("prepared Job creation failed");
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        let result = unsafe {
            QueryInformationJobObject(
                prepared.job.as_raw_handle() as HANDLE,
                JobObjectExtendedLimitInformation,
                (&mut info as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ptr::null_mut(),
            )
        };
        assert_ne!(result, 0, "prepared Job policy query failed");
        let flags = info.BasicLimitInformation.LimitFlags;
        assert_ne!(flags & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, 0);
        assert_eq!(flags & JOB_OBJECT_LIMIT_BREAKAWAY_OK, 0);
        assert_eq!(flags & JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK, 0);
    }

    #[test]
    fn prepared_job_seals_exact_suspended_child() {
        let prepared = PreparedWindowsJob::new().expect("prepared Job creation failed");
        let mut command = Command::new("ping");
        command
            .args(["-n", "60", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_SUSPENDED);
        let child = command.spawn().expect("suspended test child spawn failed");
        let process = child.as_raw_handle();
        let receipt = unsafe {
            prepared
                .assign_process(process)
                .expect("exact suspended child assignment failed")
        };
        let mut guard = unsafe {
            crate::contain_acquired_windows_job(child, receipt)
                .expect("receipt-bound Job containment failed")
        };

        assert_eq!(
            guard.tree_kill_reliability(),
            TreeKillReliability::Guaranteed
        );
        assert_eq!(
            guard.boundary_strength(),
            ContainmentBoundaryStrength::KernelEnforcedJob
        );
        assert!(
            !contained_child_has_exited(&guard).expect("suspended child observation failed"),
            "suspended child must not execute before an explicit resume"
        );

        let outcome = guard
            .terminate(TerminateTreeConfig::default())
            .expect("suspended child Job termination failed");
        assert!(outcome.exited);
        assert_eq!(
            outcome.boundary_strength,
            ContainmentBoundaryStrength::KernelEnforcedJob
        );
        assert!(matches!(
            outcome.completion,
            ContainmentCompletionEvidence::Empty { .. }
        ));
    }

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
        assert!(matches!(
            outcome.completion,
            ContainmentCompletionEvidence::Empty { .. }
        ));
    }

    #[test]
    fn contained_job_completes_normally_before_releasing_child() {
        let child = Command::new("cmd")
            .args(["/C", "ping -n 2 127.0.0.1 >NUL"])
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
        assert!(matches!(
            outcome.completion,
            ContainmentCompletionEvidence::Empty { .. }
        ));
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

    #[test]
    fn job_observation_retries_incomplete_snapshot() {
        let mut attempts = 0;
        let evidence = observe_job_completion_with(|_| {
            attempts += 1;
            if attempts == 1 {
                Ok(JobProcessSnapshot {
                    assigned: 2,
                    pids: vec![17],
                })
            } else {
                Ok(JobProcessSnapshot {
                    assigned: 2,
                    pids: vec![23, 17],
                })
            }
        });
        assert_eq!(attempts, 2);
        assert_eq!(
            evidence,
            ContainmentCompletionEvidence::Survivors {
                observation: ContainmentObservation::WindowsJobProcessIdList,
                observed_count: 2,
                survivor_pids: vec![17, 23],
            }
        );
    }

    #[test]
    fn job_observation_grows_incomplete_query_buffer() {
        let mut capacities = Vec::new();
        let evidence = observe_job_completion_with(|capacity| {
            capacities.push(capacity);
            if capacity < 200 {
                Err(JobQueryError::RetryWithCapacity(200))
            } else {
                Ok(JobProcessSnapshot {
                    assigned: 0,
                    pids: Vec::new(),
                })
            }
        });
        assert_eq!(capacities, vec![INITIAL_JOB_PROCESS_ID_CAPACITY, 200]);
        assert_eq!(
            evidence,
            ContainmentCompletionEvidence::Empty {
                observation: ContainmentObservation::WindowsJobProcessIdList,
            }
        );
    }

    #[test]
    fn job_observation_failure_is_unknown() {
        let mut attempts = 0;
        let evidence = observe_job_completion_with(|_| {
            attempts += 1;
            Err(JobQueryError::Unavailable)
        });
        assert_eq!(attempts, 1);
        assert_eq!(
            evidence,
            ContainmentCompletionEvidence::Unknown {
                observation: ContainmentObservation::WindowsJobProcessIdList,
            }
        );
    }

    #[test]
    fn job_observation_at_pid_limit_is_unknown() {
        let evidence = observe_job_completion_with(|_| {
            Err(JobQueryError::RetryWithCapacity(
                MAX_COMPLETION_OBSERVED_PIDS,
            ))
        });
        assert_eq!(
            evidence,
            ContainmentCompletionEvidence::Unknown {
                observation: ContainmentObservation::WindowsJobProcessIdList,
            }
        );
    }
}
