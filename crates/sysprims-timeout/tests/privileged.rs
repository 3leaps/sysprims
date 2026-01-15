//! Privileged Tests for sysprims-timeout.
//!
//! These tests require container isolation to run safely.
//! They verify tree-kill behavior that would be dangerous on a host system.
//!
//! # Running These Tests
//!
//! ```bash
//! docker build -t sysprims-test-fixture -f Dockerfile.container .
//! docker run --rm -v $(pwd):/workspace:ro \
//!     -v $(pwd)/target:/workspace/target \
//!     sysprims-test-fixture
//! ```
//!
//! # Test Categories
//!
//! ## privileged-tests
//! - Tree-kill verification with actual process counting
//! - Process group edge cases
//! - Session/orphan behavior

#[cfg(all(unix, feature = "privileged-tests"))]
mod privileged {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Duration;

    use sysprims_signal::SIGKILL;
    use sysprims_timeout::{run_with_timeout, TimeoutConfig, TimeoutOutcome, TreeKillReliability};

    /// Count processes in a process group.
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

    /// Verify that tree-kill actually terminates ALL descendants.
    ///
    /// This test is safe in a container because:
    /// 1. Container processes are isolated from host
    /// 2. Even if something goes wrong, only container processes are affected
    /// 3. Container will be destroyed after tests complete
    #[test]
    fn tree_kill_terminates_all_descendants() {
        // Spawn a process that spawns children in the same process group
        let parent = unsafe {
            Command::new("sh")
                .args([
                    "-c",
                    "
                    sleep 60 &
                    sleep 60 &
                    sleep 60 &
                    wait
                    ",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .pre_exec(|| {
                    // Create new process group with this process as leader
                    libc::setpgid(0, 0);
                    Ok(())
                })
                .spawn()
                .expect("Failed to spawn parent process")
        };

        let parent_pid = parent.id();

        // Wait for children to spawn
        thread::sleep(Duration::from_millis(200));

        // Count processes in the group before kill
        let before = count_processes_in_group(parent_pid);
        eprintln!(
            "INFO: Process group {} has {} processes before kill",
            parent_pid, before
        );

        // Should have at least parent + 3 children
        assert!(
            before >= 4,
            "Expected at least 4 processes in group, found {}",
            before
        );

        // Kill the entire process group with SIGKILL
        sysprims_signal::killpg(parent_pid, SIGKILL).expect("killpg should succeed");

        // Wait for OS to clean up
        thread::sleep(Duration::from_millis(200));

        // Verify all processes are dead
        let after = count_processes_in_group(parent_pid);
        eprintln!(
            "INFO: Process group {} has {} processes after kill",
            parent_pid, after
        );

        assert_eq!(
            after, 0,
            "All processes in group should be dead, but {} remain",
            after
        );
    }

    /// Verify that timeout with group-by-default actually kills the tree.
    #[test]
    fn timeout_group_by_default_kills_tree() {
        let marker = format!(
            "sysprims_priv_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        // Script that spawns background processes
        let script = format!(
            r#"
            for i in 1 2 3; do
                (echo {marker}; sleep 300) &
            done
            sleep 300
            "#,
            marker = marker
        );

        // Count matching processes before
        let before = count_matching_processes(&marker);
        assert_eq!(before, 0, "Marker processes should not exist yet");

        // Run with timeout using group-by-default
        let result = run_with_timeout(
            "bash",
            &["-c", &script],
            Duration::from_millis(500),
            TimeoutConfig {
                kill_after: Duration::from_millis(200),
                ..Default::default()
            },
        )
        .expect("run_with_timeout should not error");

        // Verify timeout occurred with guaranteed tree-kill
        match result {
            TimeoutOutcome::TimedOut {
                tree_kill_reliability,
                ..
            } => {
                assert_eq!(
                    tree_kill_reliability,
                    TreeKillReliability::Guaranteed,
                    "Should have guaranteed tree-kill reliability"
                );
            }
            TimeoutOutcome::Completed { .. } => {
                panic!("Expected timeout, but command completed");
            }
        }

        // Wait for cleanup
        thread::sleep(Duration::from_millis(300));

        // Verify no orphans remain
        let after = count_matching_processes(&marker);
        assert_eq!(
            after, 0,
            "All marker processes should be dead, {} remain",
            after
        );
    }

    /// Count processes matching a pattern.
    fn count_matching_processes(pattern: &str) -> usize {
        let output = Command::new("pgrep")
            .arg("-f")
            .arg(pattern)
            .output()
            .expect("Failed to run pgrep");

        if output.status.success() {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| !line.is_empty())
                .count()
        } else {
            0
        }
    }

    /// Test that setsid escape is documented (processes that call setsid DO escape).
    ///
    /// This is a limitation test - we verify that setsid creates a new session
    /// that escapes our process group. This is expected behavior, not a bug.
    #[test]
    fn setsid_escape_is_documented_limitation() {
        let marker = format!(
            "sysprims_setsid_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        // Script where child calls setsid before sleeping
        let script = format!(
            r#"
            setsid sh -c 'echo {marker}; sleep 300' &
            sleep 300
            "#,
            marker = marker
        );

        let _ = run_with_timeout(
            "bash",
            &["-c", &script],
            Duration::from_millis(300),
            TimeoutConfig {
                kill_after: Duration::from_millis(100),
                ..Default::default()
            },
        );

        // Wait a moment
        thread::sleep(Duration::from_millis(200));

        // The setsid process MAY have escaped - this is documented behavior
        let escaped = count_matching_processes(&marker);

        // Clean up any escaped processes
        let _ = Command::new("pkill").args(["-9", "-f", &marker]).output();

        if escaped > 0 {
            eprintln!(
                "INFO: setsid escape confirmed - {} process(es) escaped to new session",
                escaped
            );
            eprintln!("INFO: This is documented behavior, not a bug. See ADR-0003.");
        }

        // This test passes regardless - we're documenting, not asserting
    }
}

#[cfg(all(unix, feature = "cross-user-tests"))]
mod cross_user {
    // Cross-user timeout tests would go here
    // For now, the signal-level tests in sysprims-signal cover this adequately
}

// Placeholder when features are disabled
#[cfg(not(any(feature = "privileged-tests", feature = "cross-user-tests")))]
mod placeholder {
    #[test]
    fn privileged_tests_require_feature_flag() {
        // Real tests require --features privileged-tests
        // and should only run inside the test container.
    }
}
