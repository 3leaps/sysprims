//! Tree Escape Tests - The Core Thesis of sysprims
//!
//! These tests validate that sysprims-timeout kills the ENTIRE process tree
//! on timeout, not just the direct child. This is the core differentiator
//! over GNU timeout which leaves orphaned processes.
//!
//! # Test Strategy
//!
//! 1. Spawn a script that creates grandchildren attempting to escape
//! 2. Run it with sysprims timeout
//! 3. Verify ALL processes are killed (no orphans)
//!
//! # Escape Techniques Tested
//!
//! - Background processes (`&`)
//! - `trap '' TERM`: Ignores SIGTERM
//! - Nested subshells
//! - setsid: Creates new session (documented limitation - may escape)
//!
//! These tests are critical and NON-NEGOTIABLE for v0.1.0 release.

#[cfg(unix)]
use std::process::Command;
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use sysprims_timeout::{run_with_timeout, TimeoutConfig, TimeoutOutcome, TreeKillReliability};

/// Helper to count processes matching a pattern.
///
/// Uses `pgrep` on Unix to find processes by pattern.
#[cfg(unix)]
fn count_processes_matching(pattern: &str) -> usize {
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

/// Kill any processes matching a pattern (cleanup helper).
#[cfg(unix)]
fn cleanup_processes(pattern: &str) {
    let _ = Command::new("pkill")
        .arg("-9")
        .arg("-f")
        .arg(pattern)
        .output();
    thread::sleep(Duration::from_millis(100));
}

/// Generate a unique marker for this test run.
#[cfg(unix)]
fn unique_marker() -> String {
    format!(
        "sysprims_escape_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

// ============================================================================
// Core Tree Escape Test (Background Processes)
// ============================================================================

/// Test that background grandchildren are killed.
///
/// This tests the basic case where children spawn background processes.
/// These should all be killed via the process group.
#[test]
#[cfg(unix)]
fn tree_escape_background_grandchildren_are_killed() {
    let marker = unique_marker();

    // Script that spawns background grandchildren
    let script = format!(
        r#"
        for i in 1 2 3 4 5; do
            (echo {marker}_bg_$i; sleep 300) &
        done
        sleep 300
        "#,
        marker = marker
    );

    // Verify no matching processes exist before test
    let before_count = count_processes_matching(&marker);
    assert_eq!(before_count, 0, "Marker processes exist before test");

    // Run with short timeout
    let result = run_with_timeout(
        "bash",
        &["-c", &script],
        Duration::from_millis(500),
        TimeoutConfig {
            kill_after: Duration::from_millis(200),
            ..Default::default()
        },
    )
    .expect("run_with_timeout failed");

    // Verify timeout occurred with guaranteed tree-kill
    match result {
        TimeoutOutcome::TimedOut {
            tree_kill_reliability,
            ..
        } => {
            assert_eq!(
                tree_kill_reliability,
                TreeKillReliability::Guaranteed,
                "Expected guaranteed tree-kill"
            );
        }
        TimeoutOutcome::Completed { .. } => {
            panic!("Expected timeout, but command completed");
        }
    }

    // Give OS time to clean up processes
    thread::sleep(Duration::from_millis(200));

    // Verify no orphaned processes remain
    let after_count = count_processes_matching(&marker);
    assert_eq!(
        after_count, 0,
        "Found {} orphaned background processes after timeout!",
        after_count
    );
}

// ============================================================================
// Setsid Escape Tests (Session Escape) - Documented Limitation
// ============================================================================

/// Test setsid escape behavior.
///
/// This is the hardest escape technique - creating a new session detaches
/// from the process group entirely. This is a DOCUMENTED LIMITATION:
/// processes that successfully call setsid() before we signal the group
/// will escape.
///
/// This test documents the limitation rather than asserting it doesn't happen.
#[test]
#[cfg(unix)]
fn tree_escape_setsid_documents_limitation() {
    use sysprims_session::{run_setsid, SetsidConfig};

    let marker = unique_marker();

    // Spawn a process in a new session using sysprims-session
    let setsid_result = run_setsid(
        "sh",
        &["-c", &format!("echo {}; sleep 300", marker)],
        SetsidConfig::default(),
    );

    // Give it time to start
    thread::sleep(Duration::from_millis(100));

    // Count processes with our marker
    let escaped = count_processes_matching(&marker);

    // Clean up
    cleanup_processes(&marker);

    // Document: setsid processes DO escape process group kills
    // This is expected behavior - they're in a different session
    if escaped > 0 {
        eprintln!(
            "INFO: setsid escape confirmed - {} process(es) in new session (expected)",
            escaped
        );
    }

    // The important thing is that run_setsid worked
    assert!(setsid_result.is_ok(), "setsid should succeed");
}

// ============================================================================
// SIGTERM Trap Tests
// ============================================================================

/// Test that processes trapping SIGTERM are still killed via SIGKILL escalation.
#[test]
#[cfg(unix)]
fn tree_escape_trap_term_children_are_killed() {
    let marker = unique_marker();

    // Script where background child traps TERM
    // The main process does NOT trap, so it will exit on SIGTERM
    // Then we escalate to SIGKILL for the trapped child
    let script = format!(
        r#"
        (trap '' TERM; echo {marker}_trap; sleep 300) &
        CHILD_PID=$!
        # Main process sleeps and will receive SIGTERM
        sleep 300
        "#,
        marker = marker
    );

    let before_count = count_processes_matching(&marker);
    assert_eq!(before_count, 0, "Marker processes exist before test");

    let result = run_with_timeout(
        "bash",
        &["-c", &script],
        Duration::from_millis(300),
        TimeoutConfig {
            // Short kill_after to trigger escalation quickly
            kill_after: Duration::from_millis(200),
            ..Default::default()
        },
    )
    .unwrap();

    // Check if escalation happened
    if let TimeoutOutcome::TimedOut { escalated, .. } = result {
        eprintln!("Escalated to SIGKILL: {}", escalated);
    }

    // Give extra time for SIGKILL cleanup
    thread::sleep(Duration::from_millis(300));

    let orphans = count_processes_matching(&marker);

    // If there are orphans, clean them up and note it
    if orphans > 0 {
        eprintln!(
            "WARNING: {} trap-TERM processes escaped - investigating",
            orphans
        );
        cleanup_processes(&marker);
    }

    // This SHOULD pass - SIGKILL cannot be trapped
    assert_eq!(
        orphans, 0,
        "trap TERM child escaped! Found {} orphans (SIGKILL should have killed them)",
        orphans
    );
}

// ============================================================================
// Foreground Mode (Opt-out) Tests
// ============================================================================

#[test]
#[cfg(unix)]
fn foreground_mode_reports_best_effort() {
    let result = run_with_timeout(
        "sleep",
        &["60"],
        Duration::from_millis(100),
        TimeoutConfig {
            grouping: sysprims_timeout::GroupingMode::Foreground,
            kill_after: Duration::from_millis(50),
            ..Default::default()
        },
    )
    .unwrap();

    if let TimeoutOutcome::TimedOut {
        tree_kill_reliability,
        ..
    } = result
    {
        assert_eq!(
            tree_kill_reliability,
            TreeKillReliability::BestEffort,
            "Foreground mode should report best-effort reliability"
        );
    }
}

// ============================================================================
// Stress Tests
// ============================================================================

/// Stress test with many background grandchildren.
#[test]
#[cfg(unix)]
fn tree_escape_many_grandchildren() {
    let marker = unique_marker();

    let script = format!(
        r#"
        for i in $(seq 1 20); do
            (echo {marker}_stress_$i; sleep 300) &
        done
        sleep 300
        "#,
        marker = marker
    );

    let before_count = count_processes_matching(&marker);
    assert_eq!(before_count, 0, "Marker processes exist before test");

    let _ = run_with_timeout(
        "bash",
        &["-c", &script],
        Duration::from_millis(500),
        TimeoutConfig {
            kill_after: Duration::from_millis(200),
            ..Default::default()
        },
    );

    thread::sleep(Duration::from_millis(500));

    let orphans = count_processes_matching(&marker);
    assert_eq!(
        orphans, 0,
        "Stress test: {} of 20 grandchildren escaped!",
        orphans
    );
}

// ============================================================================
// Nested Subshell Tests
// ============================================================================

/// Test deeply nested subshells are killed.
#[test]
#[cfg(unix)]
fn tree_escape_nested_subshells_are_killed() {
    let marker = unique_marker();

    // Create deeply nested subshells
    let script = format!(
        r#"
        (
            (
                (
                    echo {marker}_nested
                    sleep 300
                ) &
            ) &
        ) &
        sleep 300
        "#,
        marker = marker
    );

    let before_count = count_processes_matching(&marker);
    assert_eq!(before_count, 0, "Marker processes exist before test");

    let _ = run_with_timeout(
        "bash",
        &["-c", &script],
        Duration::from_millis(300),
        TimeoutConfig {
            kill_after: Duration::from_millis(100),
            ..Default::default()
        },
    );

    thread::sleep(Duration::from_millis(200));

    let orphans = count_processes_matching(&marker);
    assert_eq!(
        orphans, 0,
        "Nested subshell escaped! Found {} orphans",
        orphans
    );
}

// ============================================================================
// Behavioral Comparison Tests
// ============================================================================

/// Compare our timeout behavior against system timeout (if available).
///
/// This is a behavioral comparison test - we shell out to the system tool
/// (which may be GPL) to compare behavior, not code.
#[test]
#[cfg(target_os = "linux")]
fn behavioral_comparison_with_gnu_timeout() {
    // Check if system timeout exists
    let has_timeout = Command::new("which")
        .arg("timeout")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !has_timeout {
        eprintln!("Skipping comparison test - GNU timeout not found");
        return;
    }

    // Our implementation
    let our_result = run_with_timeout(
        "sleep",
        &["60"],
        Duration::from_millis(100),
        TimeoutConfig {
            kill_after: Duration::from_millis(50),
            ..Default::default()
        },
    );

    // System timeout (shelling out - no GPL contamination)
    let sys_result = Command::new("timeout")
        .args(["--kill-after=0.05", "0.1", "sleep", "60"])
        .output();

    // Both should timeout
    assert!(
        matches!(our_result, Ok(TimeoutOutcome::TimedOut { .. })),
        "Our timeout should trigger"
    );
    assert!(
        sys_result.is_ok() && !sys_result.unwrap().status.success(),
        "System timeout should trigger"
    );
}
