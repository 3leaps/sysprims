use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

/// Test that timeout completes successfully for fast commands.
#[test]
fn timeout_fast_command_succeeds() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level")
        .arg("error")
        .arg("timeout")
        .arg("5s")
        .arg("echo")
        .arg("hello");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("hello"));
}

/// Test that timeout returns exit code 124 when command times out.
#[test]
#[cfg(unix)]
fn timeout_slow_command_returns_124() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level")
        .arg("error")
        .arg("timeout")
        .arg("100ms")
        .arg("--kill-after")
        .arg("100ms")
        .arg("sleep")
        .arg("60");

    cmd.assert().code(124);
}

/// Test that timeout returns exit code 127 for command not found.
#[test]
fn timeout_command_not_found_returns_127() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level")
        .arg("error")
        .arg("timeout")
        .arg("5s")
        .arg("nonexistent_command_xyz_12345");

    cmd.assert().code(127);
}

/// Test duration parsing with various formats.
#[test]
fn timeout_duration_parsing_seconds() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level")
        .arg("error")
        .arg("timeout")
        .arg("1") // Plain number = seconds
        .arg("echo")
        .arg("test");

    cmd.assert().success();
}

#[test]
fn timeout_duration_parsing_milliseconds() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level")
        .arg("error")
        .arg("timeout")
        .arg("500ms")
        .arg("echo")
        .arg("test");

    cmd.assert().success();
}

#[test]
fn timeout_duration_parsing_minutes() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level")
        .arg("error")
        .arg("timeout")
        .arg("1m")
        .arg("echo")
        .arg("test");

    cmd.assert().success();
}

/// Test invalid duration is rejected.
#[test]
fn timeout_invalid_duration_rejected() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level")
        .arg("error")
        .arg("timeout")
        .arg("invalid")
        .arg("echo")
        .arg("test");

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("invalid duration"));
}

/// Test that --signal option works.
#[test]
#[cfg(unix)]
fn timeout_custom_signal() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level")
        .arg("error")
        .arg("timeout")
        .arg("--signal")
        .arg("KILL")
        .arg("100ms")
        .arg("--kill-after")
        .arg("100ms")
        .arg("sleep")
        .arg("60");

    cmd.assert().code(124);
}

/// Test that --foreground option is accepted.
#[test]
#[cfg(unix)]
fn timeout_foreground_mode() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level")
        .arg("error")
        .arg("timeout")
        .arg("--foreground")
        .arg("100ms")
        .arg("--kill-after")
        .arg("100ms")
        .arg("sleep")
        .arg("60");

    cmd.assert().code(124);
}

/// Test that command arguments are passed through correctly.
#[test]
#[cfg(unix)]
fn timeout_passes_args_to_command() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    // Use -- to separate CLI options from command and its arguments
    cmd.arg("--log-level")
        .arg("error")
        .arg("timeout")
        .arg("5s")
        .arg("--") // End of CLI options
        .arg("sh")
        .arg("-c")
        .arg("echo hello world");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("hello world"));
}

/// Test --preserve-status returns signal-based exit code.
#[test]
#[cfg(unix)]
fn timeout_preserve_status() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level")
        .arg("error")
        .arg("timeout")
        .arg("--preserve-status")
        .arg("100ms")
        .arg("--kill-after")
        .arg("100ms")
        .arg("sleep")
        .arg("60");

    // When escalation happens, we return 128 + SIGKILL (9 -> 137).
    cmd.assert().code(137);
}
