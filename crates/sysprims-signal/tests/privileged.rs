//! Privileged and Cross-User Tests for sysprims-signal.
//!
//! These tests require a container environment to run safely:
//! - `privileged-tests`: Requires root and container isolation
//! - `cross-user-tests`: Requires multiple user accounts
//!
//! # Running These Tests
//!
//! ```bash
//! # Build and run the test container
//! docker build -t sysprims-test-fixture -f Dockerfile.container .
//! docker run --rm -v $(pwd):/workspace:ro \
//!     -v $(pwd)/target:/workspace/target \
//!     sysprims-test-fixture
//! ```
//!
//! # Why Container Isolation?
//!
//! See ADR-0011 for the incident that motivated this approach. Testing
//! broadcast signal semantics on a host system is catastrophic - it can
//! kill Finder, Terminal, and all user processes.
//!
//! # Test Categories
//!
//! ## privileged-tests
//! - Verify that API correctly rejects dangerous PIDs (u32::MAX, 0)
//! - Demonstrate what WOULD happen if rejection wasn't in place
//! - Verify tree-kill terminates all descendants
//!
//! ## cross-user-tests
//! - Verify EPERM when signaling processes owned by other users
//! - Verify signaling own processes succeeds

#[cfg(all(unix, feature = "privileged-tests"))]
mod privileged {
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Duration;

    use sysprims_signal::{kill, SIGTERM};

    /// Helper to spawn a sleep process.
    fn spawn_sleep(seconds: u32) -> std::process::Child {
        Command::new("sleep")
            .arg(seconds.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to spawn sleep process")
    }

    /// Verify that our API rejects u32::MAX (which would become -1).
    ///
    /// In a container, we can safely demonstrate the danger by probing
    /// with signal 0 (which tests permissions without sending a real signal).
    #[test]
    fn broadcast_signal_rejection_is_enforced() {
        // First verify our API correctly rejects u32::MAX
        let result = kill(u32::MAX, SIGTERM);
        assert!(result.is_err(), "u32::MAX must be rejected per ADR-0011");

        // Now demonstrate what would happen without our safety check:
        // Spawn several sleep processes to create targets
        let mut children: Vec<_> = (0..5).map(|_| spawn_sleep(60)).collect();

        // Allow processes to start
        thread::sleep(Duration::from_millis(50));

        // Probe with signal 0 to PID -1 to demonstrate the danger
        // Signal 0 doesn't kill anything, just tests if we CAN signal
        let probe_result = unsafe { libc::kill(-1, 0) };

        // As root in a container, this should succeed (we have permission to signal everything)
        if unsafe { libc::getuid() } == 0 {
            assert_eq!(
                probe_result, 0,
                "kill(-1, 0) probe should succeed as root - demonstrating the danger"
            );
        }

        // Clean up our spawned processes
        for mut child in children.drain(..) {
            let _ = child.kill();
            let _ = child.wait();
        }

        // The key assertion: our API prevents the dangerous call
        eprintln!("INFO: Broadcast signal rejection verified - API blocks kill(-1, sig)");
    }

    /// Verify that PID 0 is rejected (would signal our own process group).
    #[test]
    fn process_group_zero_rejection_is_enforced() {
        let result = kill(0, SIGTERM);
        assert!(result.is_err(), "PID 0 must be rejected per ADR-0011");

        // Demonstrate what would happen: signal 0 to PID 0
        // This would signal our own process group
        let our_pgid = unsafe { libc::getpgrp() };
        eprintln!(
            "INFO: Our PGID is {}. PID 0 would signal this group.",
            our_pgid
        );
    }

    /// Verify the boundary: i32::MAX is valid, i32::MAX + 1 is not.
    #[test]
    fn max_safe_pid_boundary_is_correct() {
        // i32::MAX should be accepted (though will get ESRCH)
        let max_safe = i32::MAX as u32;
        let result = kill(max_safe, SIGTERM);
        assert!(
            matches!(result, Err(sysprims_core::SysprimsError::NotFound { .. })),
            "i32::MAX should be accepted but return NotFound"
        );

        // i32::MAX + 1 should be rejected (would overflow to negative)
        let first_unsafe = max_safe + 1;
        let result = kill(first_unsafe, SIGTERM);
        assert!(
            matches!(
                result,
                Err(sysprims_core::SysprimsError::InvalidArgument { .. })
            ),
            "i32::MAX + 1 must be rejected"
        );
    }

    /// Count processes in a process group (container-only utility).
    #[allow(dead_code)]
    fn count_processes_in_group(pgid: u32) -> usize {
        let output = Command::new("ps")
            .args(["-o", "pid=", "-g", &pgid.to_string()])
            .output()
            .expect("Failed to run ps");

        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
    }
}

#[cfg(all(unix, feature = "cross-user-tests"))]
mod cross_user {
    use std::process::{Command, Stdio};

    use sysprims_signal::{kill, SIGTERM};

    /// As non-root user, attempt to signal a root-owned process.
    /// Should get EPERM (PermissionDenied).
    #[test]
    fn signaling_root_process_returns_permission_denied() {
        // This test must NOT run as root
        let uid = unsafe { libc::getuid() };
        if uid == 0 {
            eprintln!("SKIP: This test must run as non-root user");
            return;
        }

        // The container runner creates a root-owned sleep process for this test.
        // This avoids using PID 1 (forbidden by repository safety protocols).
        let pid_str = match std::fs::read_to_string("/tmp/sysprims_root_sleep.pid") {
            Ok(s) => s,
            Err(_) => {
                eprintln!("SKIP: missing root PID fixture file; run in container via scripts/container-test-runner.sh");
                return;
            }
        };
        let pid: u32 = pid_str
            .trim()
            .parse()
            .expect("invalid root PID in fixture file");

        let result = kill(pid, SIGTERM);

        assert!(
            matches!(
                result,
                Err(sysprims_core::SysprimsError::PermissionDenied { .. })
            ),
            "Expected PermissionDenied when signaling root-owned process as non-root, got: {:?}",
            result
        );
    }

    /// As non-root user, signal our own child process (should succeed).
    #[test]
    fn signaling_own_child_succeeds() {
        // Spawn a child process
        let mut child = Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to spawn sleep");

        let pid = child.id();

        // Signal our own child - should succeed
        let result = kill(pid, SIGTERM);
        assert!(
            result.is_ok(),
            "Should be able to signal own child: {:?}",
            result
        );

        // Wait for cleanup
        let _ = child.wait();
    }

    /// Verify that we cannot signal processes owned by a different non-root user.
    ///
    /// This test requires the container to have multiple users (testuser, testuser2).
    /// It should be run as testuser, and testuser2 should have a running process.
    #[test]
    fn signaling_other_user_process_fails() {
        // This test requires multiple users, check if we're in the right environment
        let uid = unsafe { libc::getuid() };
        if uid == 0 {
            eprintln!("SKIP: This test must run as non-root user");
            return;
        }

        // The container runner creates a testuser2-owned sleep process for this test.
        let pid_str = match std::fs::read_to_string("/tmp/sysprims_testuser2_sleep.pid") {
            Ok(s) => s,
            Err(_) => {
                eprintln!("SKIP: missing testuser2 PID fixture file; run in container via scripts/container-test-runner.sh");
                return;
            }
        };
        let pid: u32 = pid_str
            .trim()
            .parse()
            .expect("invalid testuser2 PID in fixture file");

        let result = kill(pid, SIGTERM);
        assert!(
            matches!(
                result,
                Err(sysprims_core::SysprimsError::PermissionDenied { .. })
            ),
            "Expected PermissionDenied when signaling testuser2 PID {} as non-root, got: {:?}",
            pid,
            result
        );
    }
}

// Placeholder module that compiles when features are disabled
#[cfg(not(any(feature = "privileged-tests", feature = "cross-user-tests")))]
mod placeholder {
    #[test]
    fn privileged_tests_require_feature_flag() {
        // This test exists to prevent "no tests" warnings when features are disabled.
        // The real privileged tests require:
        //   cargo test --features privileged-tests
        //   cargo test --features cross-user-tests
        // And should only be run inside the test container.
    }
}
