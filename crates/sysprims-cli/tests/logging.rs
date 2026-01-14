use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use serde_json::Value;

#[test]
fn test_log_format_text() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-format")
        .arg("text")
        .arg("--log-level")
        .arg("info");

    cmd.assert()
        .success()
        .stderr(predicate::str::contains("INFO"))
        .stderr(predicate::str::contains("Initialization complete"))
        .stderr(predicate::str::contains("Main logic finished"));
}

#[test]
fn test_log_format_json() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-format")
        .arg("json")
        .arg("--log-level")
        .arg("info");

    let output = cmd.output().unwrap();
    assert!(output.status.success());

    // Verify each line of stderr is valid JSON and has the correct structure
    let stderr = String::from_utf8(output.stderr).unwrap();
    let log_lines: Vec<Value> = stderr
        .lines()
        .map(|line| serde_json::from_str(line).expect("stderr line should be valid JSON"))
        .collect();

    assert_eq!(log_lines.len(), 2);

    // Check the first log line
    assert_eq!(log_lines[0]["level"].as_str().unwrap(), "INFO");
    assert_eq!(
        log_lines[0]["fields"]["message"].as_str().unwrap(),
        "Initialization complete. Starting main logic."
    );

    // Check the second log line
    assert_eq!(log_lines[1]["level"].as_str().unwrap(), "INFO");
    assert_eq!(
        log_lines[1]["fields"]["message"].as_str().unwrap(),
        "Main logic finished."
    );
}

#[test]
fn test_log_level_debug() {
    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level").arg("debug");

    cmd.assert()
        .success()
        .stderr(predicate::str::is_empty().not());
}
