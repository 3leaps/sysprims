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
    use std::{cell::Cell, rc::Rc};

    use sysprims_signal::SIGKILL;
    use sysprims_timeout::{
        adopt_contained, run_with_timeout, spawn_contained, ContainmentChild,
        ContainmentCompletionEvidence, ContainmentObservation, TerminateTreeConfig, TimeoutConfig,
        TimeoutOutcome, TreeKillReliability,
    };

    #[derive(Debug)]
    struct MutableIdentityChild {
        child: std::process::Child,
        visible_pid: Rc<Cell<Option<u32>>>,
    }

    impl ContainmentChild for MutableIdentityChild {
        fn process_id(&self) -> Option<u32> {
            self.visible_pid.get()
        }

        fn try_wait(&mut self) -> std::io::Result<bool> {
            self.child.try_wait().map(|status| status.is_some())
        }
    }

    /// Check if we're running in the container test environment.
    /// Container tests mount workspace at /workspace and set SYSPRIMS_CONTAINER_TEST=1.
    fn in_container_environment() -> bool {
        std::env::var("SYSPRIMS_CONTAINER_TEST").is_ok()
            || std::path::Path::new("/workspace/Cargo.toml").exists()
    }

    /// Count processes in a process group.
    /// Note: `ps -g` on Alpine/procps-ng selects by real group ID (GID), not process group.
    /// Use `pgrep -g` which correctly selects by PGID.
    fn count_processes_in_group(pgid: u32) -> usize {
        let output = Command::new("pgrep")
            .args(["-g", &pgid.to_string()])
            .output()
            .expect("Failed to run pgrep");

        // pgrep returns exit code 1 if no processes found, which is not an error
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
        if !in_container_environment() {
            eprintln!("SKIP: tree_kill test requires container environment");
            return;
        }

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

        // Reap the parent process to prevent zombie
        // (we hold the Child handle, so the parent becomes a zombie until we wait)
        let mut parent = parent;
        let _ = parent.wait();

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

    /// Verify adopted group cleanup continues after the leader exits on SIGTERM.
    #[test]
    fn adopted_guard_cleans_descendants_after_leader_exit() {
        if !in_container_environment() {
            eprintln!("SKIP: containment guard test requires container environment");
            return;
        }

        let mut command = Command::new("sh");
        command
            .args(["-c", "(trap '' TERM; sleep 60) & echo ready; wait"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = command.spawn().expect("failed to spawn contained process");
        let pgid = child.id();
        thread::sleep(Duration::from_millis(200));
        assert!(count_processes_in_group(pgid) >= 2);

        let mut guard = adopt_contained(child).expect("failed to adopt process group");
        assert_eq!(guard.tree_kill_reliability(), TreeKillReliability::Unproven);
        let outcome = guard
            .terminate(TerminateTreeConfig {
                grace_timeout_ms: 100,
                kill_timeout_ms: 500,
                ..TerminateTreeConfig::default()
            })
            .expect("contained termination failed");

        assert!(outcome.escalated, "trapped descendant requires escalation");
        assert!(matches!(
            outcome.completion,
            ContainmentCompletionEvidence::Empty {
                observation: ContainmentObservation::LinuxProcfsProcessGroup,
            }
        ));
        thread::sleep(Duration::from_millis(200));
        assert_eq!(count_processes_in_group(pgid), 0);
    }

    /// Verify a lost child capability fails closed before any group signal.
    #[test]
    fn adopted_guard_fails_closed_on_identity_loss() {
        if !in_container_environment() {
            eprintln!("SKIP: containment guard test requires container environment");
            return;
        }

        let mut command = Command::new("sleep");
        command.arg("60").stdin(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = command.spawn().expect("failed to spawn contained process");
        let pid = child.id();
        let visible_pid = Rc::new(Cell::new(Some(pid)));
        let mut guard = adopt_contained(MutableIdentityChild {
            child,
            visible_pid: Rc::clone(&visible_pid),
        })
        .expect("failed to adopt process group");

        visible_pid.set(None);
        let error = guard
            .terminate(TerminateTreeConfig::default())
            .expect_err("identity loss must fail closed");
        assert!(matches!(
            error,
            sysprims_core::SysprimsError::InvalidArgument { .. }
        ));
        assert!(sysprims_proc::get_process(pid).is_ok());

        visible_pid.set(Some(pid));
        let invalid_signal = guard
            .terminate(TerminateTreeConfig {
                kill_signal: i32::MAX,
                ..TerminateTreeConfig::default()
            })
            .expect_err("invalid escalation signal must fail before termination starts");
        assert!(matches!(
            invalid_signal,
            sysprims_core::SysprimsError::InvalidArgument { .. }
        ));
        assert!(sysprims_proc::get_process(pid).is_ok());

        guard
            .terminate(TerminateTreeConfig {
                grace_timeout_ms: 10,
                ..TerminateTreeConfig::default()
            })
            .expect("guard should remain usable after fail-closed validation");
    }

    /// Verify normal completion cleans descendants before reaping the leader.
    #[test]
    fn adopted_guard_completes_after_leader_exit() {
        if !in_container_environment() {
            eprintln!("SKIP: containment guard test requires container environment");
            return;
        }

        let mut command = Command::new("sh");
        command
            .args(["-c", "(trap '' TERM; sleep 60) & sleep 0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = command.spawn().expect("failed to spawn contained process");
        let pgid = child.id();
        let mut guard = adopt_contained(child).expect("failed to adopt process group");
        thread::sleep(Duration::from_millis(250));
        assert!(count_processes_in_group(pgid) >= 1);

        let outcome = guard
            .try_complete(TerminateTreeConfig {
                grace_timeout_ms: 10,
                kill_timeout_ms: 500,
                ..TerminateTreeConfig::default()
            })
            .expect("contained completion failed after leader exit")
            .expect("leader exit should be observed without an early reap");
        assert!(outcome.exited);
        assert!(
            matches!(
                &outcome.completion,
                ContainmentCompletionEvidence::Empty {
                    observation: ContainmentObservation::LinuxProcfsProcessGroup,
                }
            ),
            "unexpected completion evidence: {:?}",
            outcome.completion
        );
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("Live identity metadata unavailable")));
        assert_eq!(count_processes_in_group(pgid), 0);
        let mut child = match guard.into_child() {
            Ok(child) => child,
            Err(_) => panic!("finalized guard should release its reaped child"),
        };
        assert!(child
            .try_wait()
            .expect("child status should remain available")
            .is_some());
    }

    /// Verify live owned members are reported as evidence and cleaned on guard drop.
    #[test]
    fn adopted_guard_reports_survivors_without_resignaling_them() {
        if !in_container_environment() {
            eprintln!("SKIP: containment guard test requires container environment");
            return;
        }

        let mut command = Command::new("sleep");
        command.arg("60").stdin(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = command.spawn().expect("failed to spawn contained process");
        let pgid = child.id();
        let mut guard = adopt_contained(child).expect("failed to adopt process group");
        let outcome = guard
            .terminate(TerminateTreeConfig {
                grace_timeout_ms: 10,
                kill_timeout_ms: 25,
                signal: libc::SIGSTOP,
                kill_signal: libc::SIGSTOP,
            })
            .expect("contained observation failed");

        match outcome.completion {
            ContainmentCompletionEvidence::Survivors {
                observation,
                observed_count,
                survivor_pids,
            } => {
                assert_eq!(observation, ContainmentObservation::LinuxProcfsProcessGroup);
                assert_eq!(observed_count as usize, survivor_pids.len());
                assert!(survivor_pids.contains(&pgid));
            }
            other => panic!("expected survivor evidence, got {other:?}"),
        }
        assert!(!outcome.exited);
        assert!(outcome.timed_out);

        drop(guard);
        thread::sleep(Duration::from_millis(200));
        assert_eq!(count_processes_in_group(pgid), 0);
    }

    /// Verify dropping an active guard kills its group and reaps its child.
    #[test]
    fn active_guard_drop_cleans_contained_group() {
        if !in_container_environment() {
            eprintln!("SKIP: containment guard test requires container environment");
            return;
        }

        let mut command = Command::new("sh");
        command
            .args(["-c", "(trap '' TERM; sleep 60) & wait"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = command.spawn().expect("failed to spawn contained process");
        let pgid = child.id();
        let guard = adopt_contained(child).expect("failed to adopt process group");
        thread::sleep(Duration::from_millis(200));
        assert!(count_processes_in_group(pgid) >= 2);

        drop(guard);
        thread::sleep(Duration::from_millis(200));
        assert_eq!(count_processes_in_group(pgid), 0);
    }

    /// Verify the owned spawn path establishes the Unix group before exec.
    #[test]
    fn spawn_contained_returns_guaranteed_unix_guard() {
        if !in_container_environment() {
            eprintln!("SKIP: containment guard test requires container environment");
            return;
        }

        let mut command = Command::new("sleep");
        command.arg("60").stdin(Stdio::null());
        let mut guard = spawn_contained(command).expect("contained spawn failed");
        assert_eq!(
            guard.tree_kill_reliability(),
            TreeKillReliability::Guaranteed
        );
        let outcome = guard
            .terminate(TerminateTreeConfig {
                grace_timeout_ms: 10,
                ..TerminateTreeConfig::default()
            })
            .expect("contained termination failed");
        assert!(matches!(
            outcome.completion,
            ContainmentCompletionEvidence::Empty { .. }
        ));
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
