//! Integration tests for sysprims-signal.
//!
//! # PID Safety (ADR-0011)
//!
//! These tests ONLY send signals to processes we spawn ourselves.
//! We NEVER use hardcoded PIDs or attempt to signal system processes.
//!
//! Safe patterns used:
//! - Spawn a child process via `std::process::Command`
//! - Capture the child's PID from the `Child` handle
//! - Signal only that specific PID
//! - Wait for the child to exit and verify the outcome
//!
//! This ensures we cannot accidentally signal Finder, init, or other
//! critical processes even if tests are run with elevated privileges.

use std::process::{Child, Command, Stdio};
use std::time::Duration;
use sysprims_signal::{kill, terminate, force_kill, SIGTERM, SIGKILL};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

/// Helper to spawn a sleep process that we control.
///
/// Returns the Child handle. The caller is responsible for cleanup.
fn spawn_sleep(seconds: u32) -> Child {
    Command::new("sleep")
        .arg(seconds.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("Failed to spawn sleep process")
}

/// Helper to verify a child process is still running.
fn is_running(child: &mut Child) -> bool {
    match child.try_wait() {
        Ok(None) => true,      // Still running
        Ok(Some(_)) => false,  // Exited
        Err(_) => false,       // Error checking, assume not running
    }
}

// ============================================================================
// kill() Integration Tests
// ============================================================================

#[test]
#[cfg(unix)]
fn kill_terminates_spawned_child_with_sigterm() {
    // SAFETY: We spawn this process ourselves and control its PID.
    let mut child = spawn_sleep(60);
    let pid = child.id();

    // Verify it's running
    assert!(is_running(&mut child), "Child should be running");

    // Send SIGTERM via kill()
    kill(pid, SIGTERM).expect("kill() should succeed on our child");

    // Wait for exit
    let status = child.wait().expect("Failed to wait for child");

    // Verify it was killed by SIGTERM
    assert!(!status.success(), "Child should not exit successfully");
    assert_eq!(status.signal(), Some(SIGTERM), "Child should be killed by SIGTERM");
}

#[test]
#[cfg(unix)]
fn kill_terminates_spawned_child_with_sigkill() {
    // SAFETY: We spawn this process ourselves and control its PID.
    let mut child = spawn_sleep(60);
    let pid = child.id();

    // Verify it's running
    assert!(is_running(&mut child), "Child should be running");

    // Send SIGKILL via kill()
    kill(pid, SIGKILL).expect("kill() should succeed on our child");

    // Wait for exit
    let status = child.wait().expect("Failed to wait for child");

    // Verify it was killed by SIGKILL
    assert!(!status.success(), "Child should not exit successfully");
    assert_eq!(status.signal(), Some(SIGKILL), "Child should be killed by SIGKILL");
}

#[test]
#[cfg(unix)]
fn terminate_wrapper_kills_spawned_child() {
    // SAFETY: We spawn this process ourselves and control its PID.
    let mut child = spawn_sleep(60);
    let pid = child.id();

    // Use the convenience wrapper
    terminate(pid).expect("terminate() should succeed on our child");

    // Wait for exit
    let status = child.wait().expect("Failed to wait for child");

    // Verify it was killed by SIGTERM
    assert_eq!(status.signal(), Some(SIGTERM), "terminate() should send SIGTERM");
}

#[test]
#[cfg(unix)]
fn force_kill_wrapper_kills_spawned_child() {
    // SAFETY: We spawn this process ourselves and control its PID.
    let mut child = spawn_sleep(60);
    let pid = child.id();

    // Use the convenience wrapper
    force_kill(pid).expect("force_kill() should succeed on our child");

    // Wait for exit
    let status = child.wait().expect("Failed to wait for child");

    // Verify it was killed by SIGKILL
    assert_eq!(status.signal(), Some(SIGKILL), "force_kill() should send SIGKILL");
}

#[test]
fn kill_returns_not_found_for_exited_process() {
    // SAFETY: We spawn this process ourselves, let it exit, then try to signal.
    let mut child = spawn_sleep(0); // Exits immediately
    let pid = child.id();

    // Wait for it to exit naturally
    let _ = child.wait();

    // Small delay to ensure OS has cleaned up
    std::thread::sleep(Duration::from_millis(100));

    // Now try to kill the exited process
    let result = kill(pid, SIGTERM);

    // Should return NotFound (ESRCH)
    assert!(
        matches!(result, Err(sysprims_core::SysprimsError::NotFound { .. })),
        "Expected NotFound for exited process, got: {:?}",
        result
    );
}

// ============================================================================
// kill_by_name() Integration Tests
// ============================================================================

#[test]
#[cfg(unix)]
fn kill_by_name_with_term_terminates_child() {
    use sysprims_signal::kill_by_name;

    // SAFETY: We spawn this process ourselves and control its PID.
    let mut child = spawn_sleep(60);
    let pid = child.id();

    // Kill using signal name
    kill_by_name(pid, "TERM").expect("kill_by_name(TERM) should succeed");

    // Wait and verify
    let status = child.wait().expect("Failed to wait for child");
    assert_eq!(status.signal(), Some(SIGTERM));
}

#[test]
#[cfg(unix)]
fn kill_by_name_case_insensitive() {
    use sysprims_signal::kill_by_name;

    // SAFETY: We spawn this process ourselves and control its PID.
    let mut child = spawn_sleep(60);
    let pid = child.id();

    // Kill using lowercase signal name
    kill_by_name(pid, "sigterm").expect("kill_by_name(sigterm) should succeed");

    // Wait and verify
    let status = child.wait().expect("Failed to wait for child");
    assert_eq!(status.signal(), Some(SIGTERM));
}

// ============================================================================
// killpg() Integration Tests (Unix only)
// ============================================================================

#[test]
#[cfg(unix)]
fn killpg_terminates_process_group() {
    use sysprims_signal::killpg;
    use std::os::unix::process::CommandExt;

    // SAFETY: We spawn this process in a new process group we control.
    // The child becomes the leader of its own process group.
    let mut child = unsafe {
        Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .pre_exec(|| {
                // Create new process group with child as leader
                // pgid == 0 means use child's own PID as PGID
                libc::setpgid(0, 0);
                Ok(())
            })
            .spawn()
            .expect("Failed to spawn sleep in new process group")
    };

    let pid = child.id();

    // Small delay to ensure process group is established
    std::thread::sleep(Duration::from_millis(50));

    // The child's PID is also its PGID since it's the group leader
    killpg(pid, SIGTERM).expect("killpg() should succeed on our process group");

    // Wait for exit
    let status = child.wait().expect("Failed to wait for child");

    // Verify it was killed by SIGTERM
    assert_eq!(status.signal(), Some(SIGTERM), "Process group should be killed by SIGTERM");
}

// ============================================================================
// Process Group Convenience Wrappers (Unix only)
// ============================================================================

#[test]
#[cfg(unix)]
fn terminate_group_wrapper_kills_process_group() {
    use sysprims_signal::terminate_group;
    use std::os::unix::process::CommandExt;

    // SAFETY: We spawn this process in a new process group we control.
    let mut child = unsafe {
        Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            })
            .spawn()
            .expect("Failed to spawn sleep in new process group")
    };

    let pid = child.id();
    std::thread::sleep(Duration::from_millis(50));

    // Use the convenience wrapper
    terminate_group(pid).expect("terminate_group() should succeed");

    let status = child.wait().expect("Failed to wait for child");
    assert_eq!(status.signal(), Some(SIGTERM), "terminate_group() should send SIGTERM");
}

#[test]
#[cfg(unix)]
fn force_kill_group_wrapper_kills_process_group() {
    use sysprims_signal::force_kill_group;
    use std::os::unix::process::CommandExt;

    // SAFETY: We spawn this process in a new process group we control.
    let mut child = unsafe {
        Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            })
            .spawn()
            .expect("Failed to spawn sleep in new process group")
    };

    let pid = child.id();
    std::thread::sleep(Duration::from_millis(50));

    // Use the convenience wrapper
    force_kill_group(pid).expect("force_kill_group() should succeed");

    let status = child.wait().expect("Failed to wait for child");
    assert_eq!(status.signal(), Some(SIGKILL), "force_kill_group() should send SIGKILL");
}

// ============================================================================
// PID Validation Integration Tests
// ============================================================================

#[test]
fn kill_rejects_zero_pid_at_api_boundary() {
    // This test verifies ADR-0011 is enforced.
    // PID 0 would signal our own process group - dangerous!
    let result = kill(0, SIGTERM);

    assert!(
        matches!(result, Err(sysprims_core::SysprimsError::InvalidArgument { .. })),
        "PID 0 must be rejected per ADR-0011"
    );
}

#[test]
fn kill_rejects_overflow_pid_at_api_boundary() {
    // This test verifies ADR-0011 is enforced.
    // u32::MAX would overflow to -1, signaling ALL processes - catastrophic!
    let result = kill(u32::MAX, SIGTERM);

    assert!(
        matches!(result, Err(sysprims_core::SysprimsError::InvalidArgument { .. })),
        "PID u32::MAX must be rejected per ADR-0011"
    );

    // Verify the error message mentions the safety concern
    if let Err(e) = result {
        assert!(
            e.to_string().contains("exceeds maximum safe value"),
            "Error should explain the overflow danger"
        );
    }
}

// ============================================================================
// Windows-specific tests
// ============================================================================

#[test]
#[cfg(windows)]
fn kill_terminates_spawned_child_on_windows() {
    // On Windows, spawn a process we can terminate
    let mut child = Command::new("cmd")
        .args(["/C", "ping", "-n", "60", "127.0.0.1"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("Failed to spawn ping process");

    let pid = child.id();

    // Terminate using SIGTERM (maps to TerminateProcess on Windows)
    kill(pid, SIGTERM).expect("kill() should succeed on Windows");

    // Wait for exit
    let status = child.wait().expect("Failed to wait for child");

    // On Windows, terminated processes have non-zero exit code
    assert!(!status.success(), "Child should not exit successfully");
}
