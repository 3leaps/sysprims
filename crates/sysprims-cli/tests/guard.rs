use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct GuardPidfileCleanup {
    root_pid: u32,
    path: PathBuf,
}

impl Drop for GuardPidfileCleanup {
    fn drop(&mut self) {
        if !self.path.exists() {
            return;
        }

        let mut cmd = cargo_bin_cmd!("sysprims");
        let _ = cmd
            .arg("--log-level")
            .arg("error")
            .arg("guard")
            .arg(self.root_pid.to_string())
            .arg("--stop")
            .arg("--pidfile")
            .arg(&self.path)
            .output();

        let deadline = Instant::now() + Duration::from_secs(3);
        while self.path.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }

        let _ = fs::remove_file(&self.path);
    }
}

fn unique_pidfile() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sysprims-guard-test-{}-{nanos}.pid",
        std::process::id()
    ))
}

fn guard_cmd(root_pid: u32, pidfile: &Path) -> Command {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level")
        .arg("error")
        .arg("guard")
        .arg(root_pid.to_string())
        .arg("--pidfile")
        .arg(pidfile);
    cmd
}

#[test]
#[cfg(unix)]
fn guard_daemon_status_stop_round_trip() {
    let root_pid = std::process::id();
    let pidfile = unique_pidfile();
    let cleanup = GuardPidfileCleanup {
        root_pid,
        path: pidfile.clone(),
    };

    let output = guard_cmd(root_pid, &pidfile)
        .arg("--daemon")
        .arg("--interval")
        .arg("200ms")
        .arg("--dry-run")
        .output()
        .expect("daemon guard should start");
    assert!(
        output.status.success(),
        "daemon start failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut running_json = None;
    while Instant::now() < deadline {
        let output = guard_cmd(root_pid, &pidfile)
            .arg("--status")
            .arg("--json")
            .output()
            .expect("status command should run");

        if output.status.code() == Some(0) {
            let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
            let parsed: Value = serde_json::from_str(&stdout).expect("status JSON should parse");
            if parsed.get("running").and_then(Value::as_bool) == Some(true) {
                running_json = Some(parsed);
                break;
            }
        }

        thread::sleep(Duration::from_millis(50));
    }

    let running_json = running_json.expect("daemonized guard should become visible via --status");
    assert_eq!(
        running_json.get("root_pid").and_then(Value::as_u64),
        Some(root_pid as u64)
    );
    assert_eq!(
        running_json.get("pidfile").and_then(Value::as_str),
        Some(pidfile.to_string_lossy().as_ref())
    );
    assert_eq!(
        running_json.get("interval").and_then(Value::as_str),
        Some("200ms")
    );

    let stop = guard_cmd(root_pid, &pidfile)
        .arg("--stop")
        .output()
        .expect("stop command should run");
    assert!(
        stop.status.success(),
        "stop failed: {}",
        String::from_utf8_lossy(&stop.stderr)
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let output = guard_cmd(root_pid, &pidfile)
            .arg("--status")
            .arg("--json")
            .output()
            .expect("status command should run");
        if output.status.code() == Some(1) {
            let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
            let parsed: Value = serde_json::from_str(&stdout).expect("status JSON should parse");
            if parsed.get("running").and_then(Value::as_bool) == Some(false) {
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }

    assert!(
        !pidfile.exists(),
        "pidfile should be removed after clean guard shutdown"
    );

    drop(cleanup);
}

#[test]
#[cfg(unix)]
fn guard_daemon_fails_when_pidfile_cannot_be_written() {
    let root_pid = std::process::id();
    let pidfile = std::env::temp_dir()
        .join(format!(
            "sysprims-guard-test-missing-dir-{}",
            std::process::id()
        ))
        .join("guard.pid");

    let output = guard_cmd(root_pid, &pidfile)
        .arg("--daemon")
        .arg("--dry-run")
        .output()
        .expect("daemon guard should run");

    assert!(
        !output.status.success(),
        "daemon startup should fail when pidfile path is invalid"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("detached guard") || stderr.contains("pidfile"),
        "unexpected stderr: {stderr}"
    );
}
