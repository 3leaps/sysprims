//! Unix implementation of timeout with process groups.
//!
//! Uses `setpgid(0, 0)` to create a new process group with the child as leader,
//! then `killpg()` to signal the entire group on timeout.

use std::os::unix::process::CommandExt;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use libc::{killpg, SIGKILL};
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
