//! Behavioral comparison tests for sysprims-session.
//!
//! These tests compare our implementation against system tools to validate
//! behavioral equivalence. This is a cleanroom verification technique:
//!
//! - We shell out to system tools (which may be GPL-licensed) to observe their BEHAVIOR
//! - We compare our output against their output
//! - This is NOT code copying - it's black-box behavioral testing
//! - The tests themselves are MIT/Apache-2.0 licensed
//!
//! This approach is explicitly permitted per `.plans/provenance/sysprims-session.md`:
//! > Behavioral comparison against GPL tools (via shell-out) is permitted for testing
//!
//! # Test Categories
//!
//! 1. **setsid comparison** - Verify session leadership properties
//! 2. **nohup comparison** - Verify SIGHUP handling
//! 3. **Low-level API tests** - Verify getsid/getpgid against system queries

#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::time::{SystemTime, UNIX_EPOCH};

// ============================================================================
// Helper Functions (Unix only)
// ============================================================================

/// Check if a system tool exists at the expected path.
#[cfg(unix)]
fn tool_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

/// Get current process ID.
#[cfg(unix)]
fn getpid() -> u32 {
    std::process::id()
}

#[cfg(unix)]
fn temp_path(tag: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let filename = format!("sysprims_session_{tag}_{}_{}.txt", getpid(), nanos);
    std::env::temp_dir().join(filename)
}

// ============================================================================
// setsid Behavioral Comparison Tests
// ============================================================================

/// Test that our setsid creates a new session (session ID differs from parent).
///
/// This test verifies the core POSIX property: after setsid(), the process
/// should be in a new session (SID != parent's SID).
#[test]
#[cfg(target_os = "linux")]
fn setsid_creates_new_session() {
    use sysprims_session::{run_setsid, SetsidConfig, SetsidOutcome};

    let parent_sid = sysprims_session::getsid(0).expect("Should get parent SID");
    let output_path = temp_path("setsid_new_session");
    let output_path_str = output_path.to_string_lossy();

    // Run a command that outputs its session ID from /proc (Linux only).
    let script = format!(
        "sid=$(cat /proc/self/stat | cut -d' ' -f6); printf \"%s\" \"$sid\" > \"{}\"",
        output_path_str
    );

    let result = run_setsid(
        "sh",
        &["-c", &script],
        SetsidConfig {
            wait: true,
            ..Default::default()
        },
    );

    assert!(result.is_ok(), "run_setsid should succeed");

    if let Ok(SetsidOutcome::Completed { exit_status }) = result {
        // The command should have succeeded
        assert!(
            exit_status.success(),
            "Child command should exit successfully"
        );
    }

    let child_sid_raw = std::fs::read_to_string(&output_path).expect("Should read child SID");
    let child_sid: u32 = child_sid_raw
        .trim()
        .parse()
        .expect("Child SID should parse");
    assert_ne!(
        child_sid, parent_sid,
        "Child SID should differ from parent SID"
    );
}

/// Test that setsid makes the process its own session leader.
///
/// POSIX property: After setsid(), the process becomes:
/// 1. Session leader (SID == PID)
/// 2. Process group leader (PGID == PID)
#[test]
#[cfg(target_os = "linux")]
fn setsid_process_becomes_session_leader() {
    use sysprims_session::{run_setsid, SetsidConfig};

    let output_path = temp_path("setsid_leader");
    let output_path_str = output_path.to_string_lossy();

    // This script outputs: "PID SID PGID" to a file
    // After setsid, all three should be equal (session leader property)
    let script = format!(
        "pid=$$; stat=$(cat /proc/self/stat); sid=$(echo \"$stat\" | cut -d' ' -f6); pgid=$(echo \"$stat\" | cut -d' ' -f5); echo \"$pid $sid $pgid\" > \"{}\"",
        output_path_str
    );

    let result = run_setsid(
        "sh",
        &["-c", &script],
        SetsidConfig {
            wait: true,
            ..Default::default()
        },
    );

    assert!(result.is_ok(), "run_setsid should succeed");

    let content = std::fs::read_to_string(&output_path).expect("Should read leader output");
    let fields: Vec<&str> = content.split_whitespace().collect();
    assert_eq!(fields.len(), 3, "Expected PID SID PGID");
    let pid: u32 = fields[0].parse().expect("PID should parse");
    let sid: u32 = fields[1].parse().expect("SID should parse");
    let pgid: u32 = fields[2].parse().expect("PGID should parse");
    assert_eq!(pid, sid, "PID should equal SID after setsid");
    assert_eq!(pid, pgid, "PID should equal PGID after setsid");
}

/// Compare our setsid behavior with system setsid (Linux only).
///
/// This test shells out to /usr/bin/setsid to compare behavioral equivalence.
/// We're comparing:
/// - Exit code propagation
/// - Session ID behavior
#[test]
#[cfg(target_os = "linux")]
fn setsid_matches_system_behavior_linux() {
    const SYSTEM_SETSID: &str = "/usr/bin/setsid";

    if !tool_exists(SYSTEM_SETSID) {
        eprintln!("Skipping test: {} not found", SYSTEM_SETSID);
        return;
    }

    use sysprims_session::{run_setsid, SetsidConfig, SetsidOutcome};

    // Test 1: Exit code propagation (exit 42)
    let our_result = run_setsid(
        "sh",
        &["-c", "exit 42"],
        SetsidConfig {
            wait: true,
            ..Default::default()
        },
    );

    let sys_result = Command::new(SYSTEM_SETSID)
        .args(["sh", "-c", "exit 42"])
        .status();

    if let (Ok(SetsidOutcome::Completed { exit_status: ours }), Ok(theirs)) =
        (our_result, sys_result)
    {
        assert_eq!(
            ours.code(),
            theirs.code(),
            "Exit codes should match: ours={:?}, system={:?}",
            ours.code(),
            theirs.code()
        );
    }

    // Test 2: Exit code 0 (success)
    let our_result = run_setsid(
        "true",
        &[],
        SetsidConfig {
            wait: true,
            ..Default::default()
        },
    );

    let sys_result = Command::new(SYSTEM_SETSID).args(["true"]).status();

    if let (Ok(SetsidOutcome::Completed { exit_status: ours }), Ok(theirs)) =
        (our_result, sys_result)
    {
        assert_eq!(
            ours.code(),
            theirs.code(),
            "Exit codes should match for 'true'"
        );
    }

    // Test 3: Command not found behavior
    let our_result = run_setsid(
        "nonexistent_command_xyz_12345",
        &[],
        SetsidConfig {
            wait: true,
            ..Default::default()
        },
    );

    let sys_result = Command::new(SYSTEM_SETSID)
        .args(["nonexistent_command_xyz_12345"])
        .status();

    // Both should fail (though error codes may differ)
    assert!(
        our_result.is_err(),
        "Our setsid should fail for nonexistent command"
    );
    // System setsid may return 127 or similar
    if let Ok(status) = sys_result {
        assert!(!status.success(), "System setsid should fail too");
    }
}

/// Verify session ID differs from parent session.
///
/// After setsid(), the child should be in a different session than the parent.
#[test]
#[cfg(target_os = "linux")]
fn setsid_session_differs_from_parent() {
    use sysprims_session::{getsid, run_setsid, SetsidConfig};

    // Get parent's session ID
    let parent_sid = getsid(0).expect("Should get current session ID");
    let output_path = temp_path("setsid_parent_diff");
    let output_path_str = output_path.to_string_lossy();

    // Run a command that outputs its session ID from /proc (Linux only).
    let script = format!(
        "sid=$(cat /proc/self/stat | cut -d' ' -f6); printf \"%s\" \"$sid\" > \"{}\"",
        output_path_str
    );

    let result = run_setsid(
        "sh",
        &["-c", &script],
        SetsidConfig {
            wait: true,
            ..Default::default()
        },
    );

    // The important thing is that we can successfully create a new session
    assert!(result.is_ok(), "Should be able to create new session");

    let child_sid_raw = std::fs::read_to_string(&output_path).expect("Should read child SID");
    let child_sid: u32 = child_sid_raw
        .trim()
        .parse()
        .expect("Child SID should parse");
    assert_ne!(
        child_sid, parent_sid,
        "Child SID should differ from parent SID"
    );

    // Verify parent's SID is still valid
    let parent_sid_after = getsid(0).expect("Should still get current session ID");
    assert_eq!(
        parent_sid, parent_sid_after,
        "Parent's session ID should not change"
    );
}

// ============================================================================
// nohup Behavioral Comparison Tests
// ============================================================================

/// Test that our nohup ignores SIGHUP.
///
/// This is the core nohup functionality: processes should survive SIGHUP.
#[test]
#[cfg(unix)]
fn nohup_ignores_sighup() {
    use sysprims_session::{run_nohup, NohupConfig, NohupOutcome};

    let output_path = temp_path("nohup_hup");
    let output_path_str = output_path.to_string_lossy();

    // Send SIGHUP to self. If SIGHUP is ignored, we continue and write output.
    // If not ignored, the process terminates before writing.
    let script = format!("kill -HUP $$; echo survived > \"{}\"", output_path_str);

    let result = run_nohup(
        "sh",
        &["-c", &script],
        NohupConfig {
            wait: true,
            output_file: Some("/dev/null".to_string()),
        },
    );

    assert!(result.is_ok(), "run_nohup should succeed");

    if let Ok(NohupOutcome::Completed { exit_status }) = result {
        assert!(exit_status.success(), "nohup command should succeed");
    }

    let survived = std::fs::read_to_string(&output_path).unwrap_or_default();
    assert!(
        survived.contains("survived"),
        "Expected nohup child to survive SIGHUP"
    );
}

/// Compare our nohup exit code behavior with system nohup.
#[test]
#[cfg(unix)]
fn nohup_exit_code_propagation() {
    const SYSTEM_NOHUP: &str = "/usr/bin/nohup";

    if !tool_exists(SYSTEM_NOHUP) {
        eprintln!("Skipping test: {} not found", SYSTEM_NOHUP);
        return;
    }

    use sysprims_session::{run_nohup, NohupConfig, NohupOutcome};

    // Test exit code propagation
    let our_result = run_nohup(
        "sh",
        &["-c", "exit 42"],
        NohupConfig {
            wait: true,
            output_file: Some("/dev/null".to_string()),
        },
    );

    let sys_result = Command::new(SYSTEM_NOHUP)
        .args(["sh", "-c", "exit 42"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    if let (Ok(NohupOutcome::Completed { exit_status: ours }), Ok(theirs)) =
        (our_result, sys_result)
    {
        assert_eq!(
            ours.code(),
            theirs.code(),
            "Exit codes should match: ours={:?}, system={:?}",
            ours.code(),
            theirs.code()
        );
    }
}

/// Test nohup with successful command.
#[test]
#[cfg(unix)]
fn nohup_success_case() {
    use sysprims_session::{run_nohup, NohupConfig, NohupOutcome};

    let result = run_nohup(
        "true",
        &[],
        NohupConfig {
            wait: true,
            output_file: Some("/dev/null".to_string()),
        },
    );

    assert!(result.is_ok(), "run_nohup should succeed");

    if let Ok(NohupOutcome::Completed { exit_status }) = result {
        assert!(exit_status.success(), "Exit status should be success");
        assert_eq!(exit_status.code(), Some(0), "Exit code should be 0");
    }
}

// ============================================================================
// Low-Level API Tests
// ============================================================================

/// Test getsid(0) returns current session ID.
#[test]
#[cfg(unix)]
fn getsid_returns_current_session() {
    use sysprims_session::getsid;

    let sid = getsid(0);
    assert!(sid.is_ok(), "getsid(0) should succeed");

    let sid_value = sid.unwrap();
    assert!(sid_value > 0, "Session ID should be positive");

    // Session ID should be consistent on repeated calls
    let sid2 = getsid(0).unwrap();
    assert_eq!(sid_value, sid2, "Session ID should be consistent");
}

/// Test getpgid(0) returns current process group.
#[test]
#[cfg(unix)]
fn getpgid_returns_current_process_group() {
    use sysprims_session::getpgid;

    let pgid = getpgid(0);
    assert!(pgid.is_ok(), "getpgid(0) should succeed");

    let pgid_value = pgid.unwrap();
    assert!(pgid_value > 0, "Process group ID should be positive");

    // Process group ID should be consistent on repeated calls
    let pgid2 = getpgid(0).unwrap();
    assert_eq!(pgid_value, pgid2, "Process group ID should be consistent");
}

/// Compare getsid with system 'ps' command output (Linux).
#[test]
#[cfg(target_os = "linux")]
fn getsid_matches_proc_filesystem() {
    use sysprims_session::getsid;

    let our_sid = getsid(0).expect("getsid should succeed");

    // Read from /proc/self/stat - field 6 is session ID
    if let Ok(stat) = std::fs::read_to_string("/proc/self/stat") {
        let fields: Vec<&str> = stat.split_whitespace().collect();
        if fields.len() > 5 {
            if let Ok(proc_sid) = fields[5].parse::<u32>() {
                assert_eq!(
                    our_sid, proc_sid,
                    "getsid should match /proc/self/stat: ours={}, /proc={}",
                    our_sid, proc_sid
                );
            }
        }
    }
}

/// Compare getpgid with system 'ps' command output (Linux).
#[test]
#[cfg(target_os = "linux")]
fn getpgid_matches_proc_filesystem() {
    use sysprims_session::getpgid;

    let our_pgid = getpgid(0).expect("getpgid should succeed");

    // Read from /proc/self/stat - field 5 is process group ID
    if let Ok(stat) = std::fs::read_to_string("/proc/self/stat") {
        let fields: Vec<&str> = stat.split_whitespace().collect();
        if fields.len() > 4 {
            if let Ok(proc_pgid) = fields[4].parse::<u32>() {
                assert_eq!(
                    our_pgid, proc_pgid,
                    "getpgid should match /proc/self/stat: ours={}, /proc={}",
                    our_pgid, proc_pgid
                );
            }
        }
    }
}

/// Compare getsid with ps command output (Linux).
#[test]
#[cfg(target_os = "linux")]
fn getsid_matches_ps_command() {
    use sysprims_session::getsid;

    let our_sid = getsid(0).expect("getsid should succeed");
    let pid = getpid();

    // Use ps to get session ID (Linux uses "sid")
    let output = Command::new("ps")
        .args(["-o", "sid=", "-p", &pid.to_string()])
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let ps_output = String::from_utf8_lossy(&output.stdout);
            if let Ok(ps_sid) = ps_output.trim().parse::<u32>() {
                assert_eq!(
                    our_sid, ps_sid,
                    "getsid should match ps output: ours={}, ps={}",
                    our_sid, ps_sid
                );
            }
        }
    }
}

/// Verify getsid consistency on macOS.
///
/// Note: macOS ps "sess" field shows login session, NOT POSIX session ID.
/// The getsid() syscall returns the correct POSIX session ID, but we cannot
/// easily compare it against external tools on macOS. Instead, we verify
/// internal consistency.
#[test]
#[cfg(target_os = "macos")]
fn getsid_matches_ps_command() {
    use sysprims_session::getsid;

    let our_sid = getsid(0).expect("getsid should succeed");

    // On macOS, we cannot easily compare with ps output because macOS ps
    // "sess" field shows something different than POSIX session ID.
    // Instead, verify the session ID is reasonable:
    // 1. It should be a positive number
    // 2. It should be consistent across calls
    // 3. It should be <= our PID (session leaders have SID == PID)

    assert!(our_sid > 0, "Session ID should be positive");

    let sid2 = getsid(0).expect("getsid should succeed again");
    assert_eq!(our_sid, sid2, "Session ID should be consistent");

    // Note: We cannot assert our_sid <= getpid() because the session leader
    // could have exited and the session may persist with a lower SID.
    // This is valid POSIX behavior.
}

/// Compare getpgid with ps command output (cross-platform).
#[test]
#[cfg(unix)]
fn getpgid_matches_ps_command() {
    use sysprims_session::getpgid;

    let our_pgid = getpgid(0).expect("getpgid should succeed");
    let pid = getpid();

    // Use ps to get process group ID
    let output = Command::new("ps")
        .args(["-o", "pgid=", "-p", &pid.to_string()])
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let ps_output = String::from_utf8_lossy(&output.stdout);
            if let Ok(ps_pgid) = ps_output.trim().parse::<u32>() {
                assert_eq!(
                    our_pgid, ps_pgid,
                    "getpgid should match ps output: ours={}, ps={}",
                    our_pgid, ps_pgid
                );
            }
        }
    }
}

/// Test getsid with specific PID (current process).
#[test]
#[cfg(unix)]
fn getsid_with_own_pid() {
    use sysprims_session::getsid;

    let pid = getpid();

    let sid_zero = getsid(0).expect("getsid(0) should succeed");
    let sid_self = getsid(pid).expect("getsid(pid) should succeed");

    assert_eq!(
        sid_zero, sid_self,
        "getsid(0) and getsid(self) should return same value"
    );
}

/// Test getpgid with specific PID (current process).
#[test]
#[cfg(unix)]
fn getpgid_with_own_pid() {
    use sysprims_session::getpgid;

    let pid = getpid();

    let pgid_zero = getpgid(0).expect("getpgid(0) should succeed");
    let pgid_self = getpgid(pid).expect("getpgid(pid) should succeed");

    assert_eq!(
        pgid_zero, pgid_self,
        "getpgid(0) and getpgid(self) should return same value"
    );
}

/// Test that getsid fails for non-existent PID.
#[test]
#[cfg(unix)]
fn getsid_nonexistent_pid_fails() {
    use sysprims_session::getsid;

    // Use a very high PID that's unlikely to exist
    let fake_pid = 99_999_u32;
    let result = getsid(fake_pid);

    assert!(result.is_err(), "getsid should fail for non-existent PID");
}

/// Test that getpgid fails for non-existent PID.
#[test]
#[cfg(unix)]
fn getpgid_nonexistent_pid_fails() {
    use sysprims_session::getpgid;

    // Use a very high PID that's unlikely to exist
    let fake_pid = 99_999_u32;
    let result = getpgid(fake_pid);

    assert!(result.is_err(), "getpgid should fail for non-existent PID");
}

// ============================================================================
// Session Leader Property Tests
// ============================================================================

/// Verify that processes spawned with setsid become session leaders (Linux only).
///
/// Session leader property: PID == SID == PGID
///
/// Note: This test uses /proc/self/stat which is Linux-specific.
/// macOS does not have an easy shell-based way to query session ID.
#[test]
#[cfg(target_os = "linux")]
fn setsid_child_is_session_leader() {
    use sysprims_session::{run_setsid, SetsidConfig};

    // Script that verifies session leader properties and exits with status
    // indicating whether properties hold
    let script = r#"
        pid=$$
        # Get SID and PGID from /proc/self/stat
        if [ -f /proc/self/stat ]; then
            stat=$(cat /proc/self/stat)
            sid=$(echo "$stat" | cut -d' ' -f6)
            pgid=$(echo "$stat" | cut -d' ' -f5)
        else
            echo "SKIP: /proc/self/stat not available"
            exit 0
        fi

        # Session leader: SID == PID
        if [ "$sid" != "$pid" ]; then
            echo "FAIL: SID ($sid) != PID ($pid)"
            exit 1
        fi

        # Process group leader: PGID == PID
        if [ "$pgid" != "$pid" ]; then
            echo "FAIL: PGID ($pgid) != PID ($pid)"
            exit 2
        fi

        echo "OK: Session leader properties verified"
        exit 0
    "#;

    let result = run_setsid(
        "sh",
        &["-c", script],
        SetsidConfig {
            wait: true,
            ..Default::default()
        },
    );

    match result {
        Ok(sysprims_session::SetsidOutcome::Completed { exit_status }) => {
            assert!(
                exit_status.success(),
                "Session leader verification should pass (exit code: {:?})",
                exit_status.code()
            );
        }
        Ok(sysprims_session::SetsidOutcome::Spawned { .. }) => {
            panic!("Expected Completed outcome with wait=true");
        }
        Err(e) => {
            panic!("run_setsid failed: {:?}", e);
        }
    }
}

/// Verify that processes spawned with setsid become session leaders (macOS).
///
/// On macOS, we verify using getpgid which should equal the PID for a session leader.
#[test]
#[cfg(target_os = "macos")]
fn setsid_child_is_session_leader() {
    use sysprims_session::{run_setsid, SetsidConfig};

    // On macOS, after setsid the process becomes process group leader (PGID == PID).
    // Use ps for PGID; if ps is blocked by permissions, skip the assertion.
    let script = r#"
        pid=$$
        # Get PGID using ps (macOS compatible)
        pgid=$(ps -o pgid= -p $pid 2>/dev/null | tr -d ' ')

        if [ -z "$pgid" ]; then
            echo "SKIP: unable to read PGID"
            exit 0
        fi

        # Process group leader: PGID == PID
        if [ "$pgid" != "$pid" ]; then
            echo "FAIL: PGID ($pgid) != PID ($pid)"
            exit 2
        fi

        echo "OK: Process group leader property verified"
        exit 0
    "#;

    let result = run_setsid(
        "sh",
        &["-c", script],
        SetsidConfig {
            wait: true,
            ..Default::default()
        },
    );

    match result {
        Ok(sysprims_session::SetsidOutcome::Completed { exit_status }) => {
            assert!(
                exit_status.success(),
                "Session leader verification should pass (exit code: {:?})",
                exit_status.code()
            );
        }
        Ok(sysprims_session::SetsidOutcome::Spawned { .. }) => {
            panic!("Expected Completed outcome with wait=true");
        }
        Err(e) => {
            panic!("run_setsid failed: {:?}", e);
        }
    }
}

// ============================================================================
// macOS-Specific Tests
// ============================================================================

/// Test setsid on macOS (no /usr/bin/setsid on macOS by default).
///
/// macOS doesn't ship with setsid command, but setsid(2) syscall works.
#[test]
#[cfg(target_os = "macos")]
fn setsid_works_on_macos() {
    use sysprims_session::{run_setsid, SetsidConfig, SetsidOutcome};

    let result = run_setsid(
        "echo",
        &["hello from macOS"],
        SetsidConfig {
            wait: true,
            ..Default::default()
        },
    );

    assert!(result.is_ok(), "setsid should work on macOS");

    if let Ok(SetsidOutcome::Completed { exit_status }) = result {
        assert!(exit_status.success(), "Command should succeed");
    }
}

/// Test nohup on macOS.
#[test]
#[cfg(target_os = "macos")]
fn nohup_works_on_macos() {
    use sysprims_session::{run_nohup, NohupConfig, NohupOutcome};

    let result = run_nohup(
        "echo",
        &["hello from macOS nohup"],
        NohupConfig {
            wait: true,
            output_file: Some("/dev/null".to_string()),
        },
    );

    assert!(result.is_ok(), "nohup should work on macOS");

    if let Ok(NohupOutcome::Completed { exit_status }) = result {
        assert!(exit_status.success(), "Command should succeed");
    }
}

// ============================================================================
// Edge Cases and Error Handling
// ============================================================================

/// Test setsid with command that doesn't exist.
#[test]
#[cfg(unix)]
fn setsid_nonexistent_command() {
    use sysprims_session::{run_setsid, SetsidConfig};

    let result = run_setsid(
        "this_command_definitely_does_not_exist_xyz_123",
        &[],
        SetsidConfig::default(),
    );

    assert!(
        result.is_err(),
        "setsid should fail for nonexistent command"
    );
}

/// Test nohup with command that doesn't exist.
#[test]
#[cfg(unix)]
fn nohup_nonexistent_command() {
    use sysprims_session::{run_nohup, NohupConfig};

    let result = run_nohup(
        "this_command_definitely_does_not_exist_xyz_123",
        &[],
        NohupConfig {
            output_file: Some("/dev/null".to_string()),
            ..Default::default()
        },
    );

    assert!(result.is_err(), "nohup should fail for nonexistent command");
}

/// Test setsid in background mode (non-waiting).
#[test]
#[cfg(unix)]
fn setsid_background_mode() {
    use sysprims_session::{run_setsid, SetsidConfig, SetsidOutcome};

    let result = run_setsid(
        "sleep",
        &["0.1"],
        SetsidConfig {
            wait: false,
            ..Default::default()
        },
    );

    assert!(result.is_ok(), "Background setsid should succeed");

    if let Ok(SetsidOutcome::Spawned { child_pid }) = result {
        assert!(child_pid > 0, "Child PID should be positive");
        // Give the process time to complete
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

/// Test nohup in background mode (non-waiting).
#[test]
#[cfg(unix)]
fn nohup_background_mode() {
    use sysprims_session::{run_nohup, NohupConfig, NohupOutcome};

    let result = run_nohup(
        "sleep",
        &["0.1"],
        NohupConfig {
            wait: false,
            output_file: Some("/dev/null".to_string()),
        },
    );

    assert!(result.is_ok(), "Background nohup should succeed");

    if let Ok(NohupOutcome::Spawned { child_pid, .. }) = result {
        assert!(child_pid > 0, "Child PID should be positive");
        // Give the process time to complete
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}
