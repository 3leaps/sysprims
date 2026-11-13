// SPDX-License-Identifier: MIT OR Apache-2.0

//! Deterministic RFC 8785 NDJSON evaluation runner over
//! `testdata/conformance/signal/vectors.json` for cross-language parity checks
//! across Rust (`sysprims-signal`), pure Go (`signal`), Python, and TypeScript.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use canonprims_core::Limits;
use serde_json::{json, Value};
use sysprims_core::SysprimsError;
use sysprims_signal::{
    kill_many, match_signal_names, open_process_handle, open_process_handle_verified,
    resolve_signal_name, validate_pgid, validate_pid, MAX_SAFE_PID, SIGALRM, SIGHUP, SIGINT,
    SIGKILL, SIGPIPE, SIGQUIT, SIGTERM, SIGUSR1, SIGUSR2,
};

fn emit_line(record: &Value) {
    let raw = serde_json::to_vec(record).expect("serialize NDJSON record");
    let canonical =
        canonprims_json::canonicalize(&raw, &Limits::default()).expect("canonicalize NDJSON");
    println!("{}", std::str::from_utf8(&canonical).unwrap());
}

fn error_code_slug(err: &SysprimsError) -> &'static str {
    match err {
        SysprimsError::InvalidArgument { .. } => "invalid_argument",
        SysprimsError::SpawnFailed { .. } => "spawn_failed",
        SysprimsError::Timeout => "timeout",
        SysprimsError::PermissionDenied { .. }
        | SysprimsError::PermissionDeniedCommand { .. }
        | SysprimsError::PermissionDeniedPath { .. } => "permission_denied",
        SysprimsError::NotFound { .. } | SysprimsError::NotFoundCommand { .. } => "not_found",
        SysprimsError::NotSupported { .. } => "not_supported",
        SysprimsError::GroupCreationFailed { .. } => "group_creation_failed",
        SysprimsError::System { .. } => "system",
        SysprimsError::Internal { .. } => "internal",
    }
}

fn canonical_signal_number(name: &str) -> i32 {
    match name {
        "SIGHUP" => SIGHUP,
        "SIGINT" => SIGINT,
        "SIGQUIT" => SIGQUIT,
        "SIGKILL" => SIGKILL,
        "SIGPIPE" => SIGPIPE,
        "SIGALRM" => SIGALRM,
        "SIGTERM" => SIGTERM,
        "SIGUSR1" => SIGUSR1,
        "SIGUSR2" => SIGUSR2,
        other => panic!("unknown canonical signal name: {other}"),
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() == 3 && args[1] == "--live-pid" {
        let pid: u32 = args[2].parse().expect("valid u32 pid for --live-pid");
        let handle = open_process_handle(pid).expect("open live ProcessHandle");
        let exited = handle.exited().expect("poll live ProcessHandle exited");
        emit_line(&json!({
            "pid": handle.pid(),
            "start_identity": handle.start_identity(),
            "binding": handle.binding().as_str(),
            "exited": exited,
        }));
        return;
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vectors_path = env::var("SIGNAL_CONFORMANCE_VECTORS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            manifest_dir
                .parent()
                .and_then(Path::parent)
                .expect("workspace root")
                .join("testdata/conformance/signal/vectors.json")
        });

    run_vectors(&vectors_path);
}

fn run_vectors(vectors_path: &Path) {
    let doc: Value = serde_json::from_slice(&fs::read(vectors_path).expect("read vectors.json"))
        .expect("parse vectors.json");

    assert_eq!(
        doc["schema"].as_str(),
        Some("sysprims-signal-conformance/v1"),
        "unexpected conformance schema"
    );

    // 1. Constants
    let constants = &doc["constants"];
    assert_eq!(
        constants["max_safe_pid"].as_u64().unwrap(),
        u64::from(MAX_SAFE_PID)
    );
    let common = &constants["signals_common"];
    assert_eq!(common["SIGHUP"].as_i64().unwrap(), i64::from(SIGHUP));
    assert_eq!(common["SIGINT"].as_i64().unwrap(), i64::from(SIGINT));
    assert_eq!(common["SIGQUIT"].as_i64().unwrap(), i64::from(SIGQUIT));
    assert_eq!(common["SIGKILL"].as_i64().unwrap(), i64::from(SIGKILL));
    assert_eq!(common["SIGPIPE"].as_i64().unwrap(), i64::from(SIGPIPE));
    assert_eq!(common["SIGALRM"].as_i64().unwrap(), i64::from(SIGALRM));
    assert_eq!(common["SIGTERM"].as_i64().unwrap(), i64::from(SIGTERM));

    let plat_key = if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "other"
    };
    let plat = &constants["signals_platform"][plat_key];
    assert_eq!(plat["SIGUSR1"].as_i64().unwrap(), i64::from(SIGUSR1));
    assert_eq!(plat["SIGUSR2"].as_i64().unwrap(), i64::from(SIGUSR2));

    emit_line(&json!({
        "suite": "constants",
        "id": "signal-constants",
        "max_safe_pid": MAX_SAFE_PID,
        "sighup": SIGHUP,
        "sigint": SIGINT,
        "sigquit": SIGQUIT,
        "sigkill": SIGKILL,
        "sigpipe": SIGPIPE,
        "sigalrm": SIGALRM,
        "sigterm": SIGTERM,
        "sigusr1": SIGUSR1,
        "sigusr2": SIGUSR2,
    }));

    // 2. PID / PGID validation cases
    for case in doc["pid_validation_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let kind = case["kind"].as_str().unwrap();
        let val = case["value"].as_u64().unwrap() as u32;
        let expected = case["expected"].as_str().unwrap();
        let res = match kind {
            "pid" => validate_pid(val),
            "pgid" => validate_pgid(val),
            other => panic!("unknown pid_validation_cases kind: {other}"),
        };
        let got = match res {
            Ok(()) => "ok",
            Err(ref e) => error_code_slug(e),
        };
        assert_eq!(got, expected, "pid validation mismatch on {id}");
        emit_line(&json!({
            "suite": "pid-validation",
            "id": id,
            "kind": kind,
            "outcome": got,
        }));
    }

    // 3. Batch validation cases
    for case in doc["batch_validation_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let pids: Vec<u32> = case["pids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u32)
            .collect();
        let expected = case["expected"].as_str().unwrap();
        let got = match kill_many(&pids, SIGTERM) {
            Ok(_) => "ok",
            Err(ref e) => error_code_slug(e),
        };
        assert_eq!(got, expected, "batch validation mismatch on {id}");
        emit_line(&json!({
            "suite": "batch-validation",
            "id": id,
            "outcome": got,
        }));
    }

    // 4. Resolve signal cases
    for case in doc["resolve_signal_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let input = case["input"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        match resolve_signal_name(input) {
            Ok(signum) => {
                assert_eq!(expected, "ok", "resolve signal expected error on {id}");
                let exp_sig_name = case["signal"].as_str().unwrap();
                let exp_signum = canonical_signal_number(exp_sig_name);
                assert_eq!(signum, exp_signum, "resolved signum mismatch on {id}");
                emit_line(&json!({
                    "suite": "resolve-signal",
                    "id": id,
                    "outcome": "ok",
                    "signum": signum,
                }));
            }
            Err(ref e) => {
                let got = error_code_slug(e);
                assert_eq!(got, expected, "resolve signal error mismatch on {id}");
                emit_line(&json!({
                    "suite": "resolve-signal",
                    "id": id,
                    "outcome": got,
                }));
            }
        }
    }

    // 5. Match signal cases
    for case in doc["match_signal_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let pattern = case["pattern"].as_str().unwrap();
        let expected: Vec<String> = case["expected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let got = match_signal_names(pattern);
        assert_eq!(got, expected, "match signal names mismatch on {id}");
        emit_line(&json!({
            "suite": "match-signal",
            "id": id,
            "matches": got,
        }));
    }

    // 6. ProcessHandle validation cases
    for case in doc["process_handle_validation_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let pid = if case["pid"].as_str() == Some("self") {
            std::process::id()
        } else {
            case["pid"].as_u64().unwrap() as u32
        };
        let expected = case["expected"].as_str().unwrap();
        let res = if let Some(exp_id) = case.get("expected_start_identity").and_then(Value::as_str)
        {
            open_process_handle_verified(pid, exp_id)
        } else {
            open_process_handle(pid)
        };
        let got = match res {
            Ok(_) => "ok",
            Err(ref e) => error_code_slug(e),
        };
        assert_eq!(got, expected, "process handle validation mismatch on {id}");
        emit_line(&json!({
            "suite": "process-handle-validation",
            "id": id,
            "outcome": got,
        }));
    }
}
