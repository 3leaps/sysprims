// SPDX-License-Identifier: MIT OR Apache-2.0

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

use canonprims_core::Limits;
use canonprims_json::canonicalize;
use serde_json::{json, Value};
use sysprims_core::schema::{
    CONTAINMENT_SNAPSHOT_V1, CONTAINMENT_SPAWN_CONFIG_V1, TERMINATE_TREE_CONFIG_V1,
    TERMINATE_TREE_RESULT_V1, TIMEOUT_RESULT_V1_1,
};
use sysprims_core::SysprimsError;
use sysprims_timeout::managed::{self, ManagedSpawnRequest};
use sysprims_timeout::{terminate_tree, TerminateTreeConfig};

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

fn emit(v: Value) {
    let raw = serde_json::to_vec(&v).expect("serialize");
    let canonical = canonicalize(&raw, &Limits::default()).expect("JCS canonicalize");
    println!("{}", std::str::from_utf8(&canonical).unwrap());
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let vectors_path = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        PathBuf::from("testdata/conformance/timeout/vectors.json")
    };

    let raw = fs::read_to_string(&vectors_path).unwrap_or_else(|e| {
        eprintln!("failed to read {}: {e}", vectors_path.display());
        process::exit(2);
    });
    let doc: Value = serde_json::from_str(&raw).expect("parse vectors.json");

    // 1. Schema constants
    emit(json!({
        "suite": "schemas",
        "timeout_result_v1_1": TIMEOUT_RESULT_V1_1,
        "terminate_tree_config_v1": TERMINATE_TREE_CONFIG_V1,
        "terminate_tree_result_v1": TERMINATE_TREE_RESULT_V1,
        "containment_spawn_config_v1": CONTAINMENT_SPAWN_CONFIG_V1,
        "containment_snapshot_v1": CONTAINMENT_SNAPSHOT_V1,
    }));

    // 2. Terminate tree validation cases
    if let Some(cases) = doc["terminate_tree_validation_cases"].as_array() {
        for c in cases {
            let id = c["id"].as_str().unwrap();
            let pid_u64 = c["pid"].as_u64().unwrap();
            let expected = c["expected"].as_str().unwrap();

            let outcome = if pid_u64 > (u32::MAX as u64)
                || c["schema_id"]
                    .as_str()
                    .is_some_and(|s| s != TERMINATE_TREE_CONFIG_V1)
            {
                "invalid_argument"
            } else {
                let mut cfg = TerminateTreeConfig::default();
                if let Some(gt) = c["grace_timeout_ms"].as_u64() {
                    cfg.grace_timeout_ms = gt;
                }
                if let Some(kt) = c["kill_timeout_ms"].as_u64() {
                    cfg.kill_timeout_ms = kt;
                }
                if let Some(sig) = c["signal"].as_i64() {
                    cfg.signal = sig as i32;
                }
                if let Some(ksig) = c["kill_signal"].as_i64() {
                    cfg.kill_signal = ksig as i32;
                }
                match terminate_tree(pid_u64 as u32, cfg) {
                    Ok(_) => "ok",
                    Err(ref e) => error_code_slug(e),
                }
            };
            assert_eq!(outcome, expected, "case {id} mismatch");
            emit(json!({
                "suite": "terminate-tree-validation",
                "id": id,
                "outcome": outcome,
            }));
        }
    }

    // 3. Spawn contained validation cases
    if let Some(cases) = doc["spawn_contained_validation_cases"].as_array() {
        for c in cases {
            let id = c["id"].as_str().unwrap();
            let argv: Vec<String> = c["argv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect();
            let cwd = c["cwd"].as_str().map(String::from);
            let execution_timeout_ms = c["execution_timeout_ms"].as_u64();
            let grace_timeout_ms = c["grace_timeout_ms"].as_u64();
            let kill_timeout_ms = c["kill_timeout_ms"].as_u64();
            let signal = c["signal"].as_i64().map(|v| v as i32);
            let kill_signal = c["kill_signal"].as_i64().map(|v| v as i32);
            let expected = c["expected"].as_str().unwrap();

            let req = ManagedSpawnRequest {
                argv,
                cwd,
                env: None,
                execution_timeout_ms,
                grace_timeout_ms,
                kill_timeout_ms,
                signal,
                kill_signal,
            };
            let outcome = match managed::spawn(req) {
                Ok(token) => {
                    let _ = managed::close(token);
                    "ok"
                }
                Err(ref e) => error_code_slug(e),
            };
            assert_eq!(outcome, expected, "case {id} mismatch");
            emit(json!({
                "suite": "spawn-contained-validation",
                "id": id,
                "outcome": outcome,
            }));
        }
    }

    // 4. Containment lifecycle cases
    if let Some(cases) = doc["containment_lifecycle_cases"].as_array() {
        for c in cases {
            let id = c["id"].as_str().unwrap();
            let op = c["op"].as_str().unwrap();
            let argv: Vec<String> = c["argv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect();
            let execution_timeout_ms = c["execution_timeout_ms"].as_u64();
            let grace_timeout_ms = c["grace_timeout_ms"].as_u64();
            let kill_timeout_ms = c["kill_timeout_ms"].as_u64();
            let wait_timeout_ms = c["wait_timeout_ms"].as_u64().unwrap_or(5000);
            #[cfg(unix)]
            let expected_obj = c["expected_unix"].as_object().expect("expected_unix");
            #[cfg(windows)]
            let expected_obj = c["expected_windows"].as_object().expect("expected_windows");

            let req = ManagedSpawnRequest {
                argv,
                cwd: None,
                env: None,
                execution_timeout_ms,
                grace_timeout_ms,
                kill_timeout_ms,
                signal: None,
                kill_signal: None,
            };

            let row = match managed::spawn(req) {
                Ok(token) => {
                    let snap_res = match op {
                        "wait" | "deadline" => managed::wait(token, wait_timeout_ms),
                        "terminate" => managed::terminate(token),
                        _ => panic!("unknown op {op}"),
                    };
                    let _ = managed::close(token);
                    match snap_res {
                        Ok(snap) => {
                            let val = serde_json::to_value(&snap).expect("serialize snapshot");
                            json!({
                                "suite": "containment-lifecycle",
                                "id": id,
                                "outcome": "ok",
                                "handle_state": val["handle_state"],
                                "leader_status": val["leader_status"],
                                "tree_kill_reliability": val["tree_kill_reliability"],
                                "boundary_strength": val["boundary_strength"],
                                "exited": val["exited"],
                                "timed_out": val["timed_out"],
                            })
                        }
                        Err(ref e) => json!({
                            "suite": "containment-lifecycle",
                            "id": id,
                            "outcome": error_code_slug(e),
                        }),
                    }
                }
                Err(ref e) => json!({
                    "suite": "containment-lifecycle",
                    "id": id,
                    "outcome": error_code_slug(e),
                }),
            };

            for (k, want) in expected_obj {
                assert_eq!(&row[k], want, "case {id} field {k} mismatch");
            }
            emit(row);
        }
    }
}
