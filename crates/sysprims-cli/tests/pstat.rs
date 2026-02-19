use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;

#[test]
fn pstat_pid_json_uses_snapshot_envelope() {
    let pid = std::process::id();

    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level")
        .arg("error")
        .arg("pstat")
        .arg("--json")
        .arg("--pid")
        .arg(pid.to_string());

    let output = cmd.output().expect("pstat should run");
    assert!(
        output.status.success(),
        "expected success, stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
    let parsed: Value = serde_json::from_str(&stdout).expect("stdout should be valid json");

    let schema_id = parsed
        .get("schema_id")
        .and_then(Value::as_str)
        .expect("schema_id should be present");
    assert!(
        schema_id.contains("process-info.schema.json"),
        "unexpected schema_id: {schema_id}"
    );

    let processes = parsed
        .get("processes")
        .and_then(Value::as_array)
        .expect("processes should be an array");
    assert_eq!(processes.len(), 1, "expected exactly one process for --pid");
    assert_eq!(
        processes[0].get("pid").and_then(Value::as_u64),
        Some(pid as u64),
        "returned pid should match requested pid"
    );
}

#[test]
fn pstat_pid_json_not_found_emits_compliant_empty_snapshot() {
    let missing_pid: u32 = 1_000_000_000;

    let mut cmd = cargo_bin_cmd!("sysprims");
    cmd.arg("--log-level")
        .arg("error")
        .arg("pstat")
        .arg("--json")
        .arg("--pid")
        .arg(missing_pid.to_string());

    let output = cmd.output().expect("pstat should run");
    assert_eq!(
        output.status.code(),
        Some(1),
        "missing pid should return non-zero"
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
    let parsed: Value = serde_json::from_str(&stdout).expect("stdout should be valid json");

    assert!(
        parsed.get("schema_id").and_then(Value::as_str).is_some(),
        "schema_id should be present on not-found output"
    );
    let processes = parsed
        .get("processes")
        .and_then(Value::as_array)
        .expect("processes should be an array");
    assert!(
        processes.is_empty(),
        "not-found output should use empty processes array"
    );
}
