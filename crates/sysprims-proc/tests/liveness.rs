//! Integration tests for the liveness predicates `is_live` / `is_fully_gone`.
//!
//! Safety: every PID used here is either our own process, a child we spawned
//! ourselves, or a high PID (`99_999`) that is almost certainly absent. No test
//! signals PID 0, 1, or a PID it does not own.

use std::time::Duration;

use sysprims_core::SysprimsError;
use sysprims_proc::{is_fully_gone, is_live};

/// A PID that is almost certainly not in use (mirrors the convention used
/// elsewhere in the proc test suite).
const ABSENT_PID: u32 = 99_999;

#[test]
fn self_is_live_and_not_gone() {
    let pid = std::process::id();
    assert!(is_live(pid).expect("querying our own PID must succeed"));
    assert!(!is_fully_gone(pid).expect("querying our own PID must succeed"));
}

#[test]
fn pid_zero_is_invalid() {
    assert!(is_live(0).is_err(), "PID 0 must be rejected");
    assert!(is_fully_gone(0).is_err(), "PID 0 must be rejected");
}

/// PIDs above `i32::MAX` would become negative `pid_t` values with POSIX
/// broadcast/process-group semantics once cast. Both predicates must reject them
/// with `InvalidArgument` *before* any platform probe runs (ADR-0011). These
/// values are safe to test precisely because they never reach a syscall.
#[test]
fn pid_above_i32_max_is_invalid() {
    let over_max = (i32::MAX as u32) + 1;
    for &pid in &[over_max, u32::MAX] {
        // Assert the exact InvalidArgument variant, not merely is_err(): a weaker
        // check would still pass if the forbidden PID reached the pid_t cast and
        // kill(-1, 0) returned some *other* error (e.g. PermissionDenied), which
        // is precisely the safety regression this test must catch.
        assert!(
            matches!(is_live(pid), Err(SysprimsError::InvalidArgument { .. })),
            "is_live must reject PID {pid} (> i32::MAX) with InvalidArgument before any probe"
        );
        assert!(
            matches!(is_fully_gone(pid), Err(SysprimsError::InvalidArgument { .. })),
            "is_fully_gone must reject PID {pid} (> i32::MAX) with InvalidArgument before any probe"
        );
    }
}

#[test]
fn absent_pid_is_fully_gone_and_not_live() {
    // If this flakes, ABSENT_PID happened to be in use; it is chosen to be
    // improbable, matching the rest of the suite.
    assert!(!is_live(ABSENT_PID).expect("absent PID probe must succeed"));
    assert!(is_fully_gone(ABSENT_PID).expect("absent PID probe must succeed"));
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::process::{Command, Stdio};

    fn spawn_sleep(seconds: u32) -> std::process::Child {
        Command::new("sleep")
            .arg(seconds.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn sleep")
    }

    #[test]
    fn running_child_is_live() {
        let mut child = spawn_sleep(60);
        let pid = child.id();

        assert!(is_live(pid).expect("probe must succeed"));
        assert!(!is_fully_gone(pid).expect("probe must succeed"));

        // Cleanup: kill and reap so we don't leak the child.
        let _ = child.kill();
        let _ = child.wait();
    }

    /// The motivating case: a killed-but-unreaped child is a zombie on Linux
    /// (still in the process table) and is unreadable on macOS. `is_live` must
    /// report `false` on both, and while the zombie is unreaped it is not yet
    /// `is_fully_gone`.
    #[test]
    fn killed_unreaped_child_is_not_live() {
        let mut child = spawn_sleep(60);
        let pid = child.id();
        assert!(is_live(pid).expect("child should start live"));

        child.kill().expect("SIGKILL to our own child must succeed");
        // Give the kernel a moment to transition the child to its exited state.
        std::thread::sleep(Duration::from_millis(200));

        // Not reaped yet: the record (zombie) still exists, so it is neither
        // live nor fully gone on either platform.
        assert!(
            !is_live(pid).expect("probe must succeed"),
            "a killed-but-unreaped child must not report as live"
        );
        assert!(
            !is_fully_gone(pid).expect("probe must succeed"),
            "an unreaped zombie still holds a record and is not fully gone"
        );

        // Reap it; afterward the PID record should be released.
        child.wait().expect("reaping our own child must succeed");
        assert!(
            is_fully_gone(pid).expect("probe must succeed"),
            "after reaping, the PID should be fully gone"
        );
        assert!(!is_live(pid).expect("probe must succeed"));
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn running_child_is_live_then_gone() {
        // A long-lived child that owns its PID; ping loops without user input.
        let mut child = Command::new("cmd")
            .args(["/C", "ping", "-n", "60", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn ping");
        let pid = child.id();

        assert!(is_live(pid).expect("probe must succeed"));
        assert!(!is_fully_gone(pid).expect("probe must succeed"));

        child
            .kill()
            .expect("terminating our own child must succeed");
        let _ = child.wait();
        std::thread::sleep(Duration::from_millis(200));

        // Windows has no zombie state: once terminated the PID is gone.
        assert!(!is_live(pid).expect("probe must succeed"));
        assert!(is_fully_gone(pid).expect("probe must succeed"));
    }
}
