use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

#[test]
fn kill_rejects_pid_zero() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level")
        .arg("error")
        .arg("kill")
        .arg("0")
        .arg("--signal")
        .arg("TERM");

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("must be > 0"));
}

#[test]
fn kill_signal_glob_with_multiple_matches_errors() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level")
        .arg("error")
        .arg("kill")
        .arg("0")
        .arg("--signal")
        .arg("SIG*");

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("matched multiple signals"));
}
