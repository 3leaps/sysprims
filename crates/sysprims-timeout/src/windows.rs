//! Windows implementation of timeout with Job Objects.
//!
//! Uses Job Objects with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` to ensure
//! all processes in the job are terminated when the job handle is closed.

use std::os::windows::io::AsRawHandle;
use std::process::{Child, Command};
use std::ptr;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

use sysprims_core::{SysprimsError, SysprimsResult};

use crate::{GroupingMode, TimeoutConfig, TimeoutOutcome, TreeKillReliability};

/// Polling interval for checking if child has exited.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub fn run_with_timeout_impl(
    command: &str,
    args: &[&str],
    timeout: Duration,
    config: &TimeoutConfig,
) -> SysprimsResult<TimeoutOutcome> {
    let use_job_object = config.grouping == GroupingMode::GroupByDefault;
    let mut reliability = TreeKillReliability::Guaranteed;

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
    let mut child = Command::new(command)
        .args(args)
        .spawn()
        .map_err(|e| {
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
        if job == 0 || job == INVALID_HANDLE_VALUE {
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
    // Windows tests would go here, but we can't run them on macOS
    // They'll be tested in CI on Windows runners
}
