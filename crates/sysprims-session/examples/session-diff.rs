// SPDX-License-Identifier: MIT OR Apache-2.0

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

use canonprims_core::Limits;
use canonprims_json::canonicalize;
use serde_json::{json, Value};
use sysprims_core::schema::{
    RUN_NOHUP_CONFIG_V1, RUN_SETSID_CONFIG_V1, SESSION_SPAWN_RESULT_V1, SPAWN_IN_GROUP_CONFIG_V1,
    SPAWN_IN_GROUP_RESULT_V1,
};
use sysprims_core::SysprimsError;
use sysprims_session::{
    run_nohup, run_setsid, NohupConfig, NohupOutcome, SetsidConfig, SetsidOutcome,
};

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

fn validate_config(verb: &str, schema_id: &str, argv: &[String]) -> Result<(), SysprimsError> {
    let expected_schema = match verb {
        "setsid" => RUN_SETSID_CONFIG_V1,
        "nohup" => RUN_NOHUP_CONFIG_V1,
        "spawn_in_group" => SPAWN_IN_GROUP_CONFIG_V1,
        _ => return Err(SysprimsError::invalid_argument("unknown verb")),
    };
    if schema_id != expected_schema {
        return Err(SysprimsError::invalid_argument(format!(
            "invalid schema_id (expected {expected_schema})"
        )));
    }
    let command = argv
        .first()
        .ok_or_else(|| SysprimsError::invalid_argument("argv must not be empty"))?;
    if command.is_empty() {
        return Err(SysprimsError::invalid_argument("argv[0] must not be empty"));
    }
    Ok(())
}

fn unique_temp_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    env::temp_dir().join(format!(
        "sysprims-session-diff-{label}-{}-{nanos}",
        process::id()
    ))
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let vectors_path = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        PathBuf::from("testdata/conformance/session/vectors.json")
    };

    let raw = fs::read_to_string(&vectors_path).unwrap_or_else(|e| {
        eprintln!("failed to read {}: {e}", vectors_path.display());
        process::exit(2);
    });
    let doc: Value = serde_json::from_str(&raw).expect("parse vectors.json");

    // 1. Schema constants
    emit(json!({
        "suite": "schemas",
        "run_setsid_config_v1": RUN_SETSID_CONFIG_V1,
        "run_nohup_config_v1": RUN_NOHUP_CONFIG_V1,
        "session_spawn_result_v1": SESSION_SPAWN_RESULT_V1,
        "spawn_in_group_config_v1": SPAWN_IN_GROUP_CONFIG_V1,
        "spawn_in_group_result_v1": SPAWN_IN_GROUP_RESULT_V1,
    }));

    // 2. Config validation cases
    if let Some(cases) = doc["config_validation_cases"].as_array() {
        for c in cases {
            let id = c["id"].as_str().unwrap();
            let verb = c["verb"].as_str().unwrap();
            let schema_id = c["schema_id"].as_str().unwrap();
            let argv: Vec<String> = c["argv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect();
            let expected = c["expected"].as_str().unwrap();
            let outcome = match validate_config(verb, schema_id, &argv) {
                Ok(()) => "ok",
                Err(ref e) => error_code_slug(e),
            };
            assert_eq!(outcome, expected, "case {id} mismatch");
            emit(json!({
                "suite": "config-validation",
                "id": id,
                "verb": verb,
                "outcome": outcome,
            }));
        }
    }

    // 3. Self introspection cases
    if let Some(cases) = doc["self_introspection_cases"].as_array() {
        for c in cases {
            let id = c["id"].as_str().unwrap();
            let op = c["op"].as_str().unwrap();
            #[cfg(unix)]
            let expected = c["expected_unix"].as_str().unwrap();
            #[cfg(windows)]
            let expected = c["expected_windows"].as_str().unwrap();

            let outcome = match op {
                "self_getpgid" => {
                    #[cfg(unix)]
                    {
                        match sysprims_session::getpgid(0) {
                            Ok(pgid) if pgid > 0 => "ok",
                            Ok(_) => "invalid_value",
                            Err(ref e) => error_code_slug(e),
                        }
                    }
                    #[cfg(windows)]
                    {
                        "not_supported"
                    }
                }
                "self_getsid" => {
                    #[cfg(unix)]
                    {
                        match sysprims_session::getsid(0) {
                            Ok(sid) if sid > 0 => "ok",
                            Ok(_) => "invalid_value",
                            Err(ref e) => error_code_slug(e),
                        }
                    }
                    #[cfg(windows)]
                    {
                        "not_supported"
                    }
                }
                _ => panic!("unknown op {op}"),
            };
            assert_eq!(outcome, expected, "case {id} mismatch");
            emit(json!({
                "suite": "self-introspection",
                "id": id,
                "op": op,
                "outcome": outcome,
            }));
        }
    }

    // 4. Spawn cases
    if let Some(cases) = doc["spawn_cases"].as_array() {
        for c in cases {
            let id = c["id"].as_str().unwrap();
            let verb = c["verb"].as_str().unwrap();
            let argv: Vec<String> = c["argv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect();
            let wait = c["wait"].as_bool().unwrap_or(false);
            let use_temp_output = c["use_temp_output_file"].as_bool().unwrap_or(false);
            let use_symlink_output = c["use_symlink_output_file"].as_bool().unwrap_or(false);
            #[cfg(unix)]
            let expected_obj = c["expected_unix"].as_object().expect("expected_unix");
            #[cfg(windows)]
            let expected_obj = c["expected_windows"].as_object().expect("expected_windows");

            let cmd = &argv[0];
            let cmd_args: Vec<&str> = argv.iter().skip(1).map(String::as_str).collect();

            let row = match verb {
                "setsid" => {
                    let res = run_setsid(
                        cmd,
                        &cmd_args,
                        SetsidConfig {
                            wait,
                            ..Default::default()
                        },
                    );
                    match res {
                        Ok(SetsidOutcome::Completed {
                            child_pid,
                            exit_status,
                        }) => {
                            let structural_ids_match = child_pid > 0;
                            json!({
                                "suite": "spawn",
                                "id": id,
                                "outcome": "ok",
                                "verb": "setsid",
                                "status": "completed",
                                "session_kind": "new_session",
                                "identifier_provenance": "setsid_structural_child_pid",
                                "structural_ids_match": structural_ids_match,
                                "exit_code": exit_status.code(),
                            })
                        }
                        Ok(SetsidOutcome::Spawned { child_pid }) => {
                            json!({
                                "suite": "spawn",
                                "id": id,
                                "outcome": "ok",
                                "verb": "setsid",
                                "status": "spawned",
                                "session_kind": "new_session",
                                "identifier_provenance": "setsid_structural_child_pid",
                                "structural_ids_match": child_pid > 0,
                                "exit_code": Value::Null,
                            })
                        }
                        Err(ref e) => {
                            json!({
                                "suite": "spawn",
                                "id": id,
                                "outcome": error_code_slug(e),
                            })
                        }
                    }
                }
                "nohup" => {
                    let temp_dir = unique_temp_dir(id);
                    let _ = fs::create_dir_all(&temp_dir);
                    let output_file = if use_symlink_output {
                        #[cfg(unix)]
                        {
                            let target = temp_dir.join("target.log");
                            let link = temp_dir.join("link.log");
                            let _ = fs::write(&target, b"existing");
                            let _ = std::os::unix::fs::symlink(&target, &link);
                            Some(link.display().to_string())
                        }
                        #[cfg(not(unix))]
                        {
                            Some(temp_dir.join("link.log").display().to_string())
                        }
                    } else if use_temp_output {
                        Some(temp_dir.join("nohup.out").display().to_string())
                    } else {
                        None
                    };

                    #[cfg(unix)]
                    let caller_ok =
                        sysprims_session::getsid(0).is_ok() && sysprims_session::getpgid(0).is_ok();
                    #[cfg(not(unix))]
                    let caller_ok = false;

                    let res = run_nohup(
                        cmd,
                        &cmd_args,
                        NohupConfig {
                            wait,
                            output_file,
                            ..Default::default()
                        },
                    );
                    let _ = fs::remove_dir_all(&temp_dir);

                    match res {
                        Ok(NohupOutcome::Completed {
                            child_pid,
                            exit_status,
                            ..
                        }) => {
                            json!({
                                "suite": "spawn",
                                "id": id,
                                "outcome": "ok",
                                "verb": "nohup",
                                "status": "completed",
                                "session_kind": "inherited_session",
                                "identifier_provenance": "caller_context_before_spawn",
                                "inherited_context_matches": caller_ok && child_pid > 0,
                                "exit_code": exit_status.code(),
                            })
                        }
                        Ok(NohupOutcome::Spawned { child_pid, .. }) => {
                            json!({
                                "suite": "spawn",
                                "id": id,
                                "outcome": "ok",
                                "verb": "nohup",
                                "status": "spawned",
                                "session_kind": "inherited_session",
                                "identifier_provenance": "caller_context_before_spawn",
                                "inherited_context_matches": caller_ok && child_pid > 0,
                                "exit_code": Value::Null,
                            })
                        }
                        Err(ref e) => {
                            json!({
                                "suite": "spawn",
                                "id": id,
                                "outcome": error_code_slug(e),
                            })
                        }
                    }
                }
                "spawn_in_group" => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::CommandExt;
                        let mut command = process::Command::new(cmd);
                        command.args(&cmd_args);
                        unsafe {
                            command.pre_exec(|| {
                                if sysprims_session::setpgid(0, 0).is_err() {
                                    return Err(std::io::Error::last_os_error());
                                }
                                Ok(())
                            });
                        }
                        match command.spawn() {
                            Ok(mut child) => {
                                let pid = child.id();
                                let _ = child.wait();
                                json!({
                                    "suite": "spawn",
                                    "id": id,
                                    "outcome": "ok",
                                    "structural_ids_match": pid > 0,
                                    "tree_kill_reliability": "best_effort",
                                })
                            }
                            Err(_) => {
                                json!({
                                    "suite": "spawn",
                                    "id": id,
                                    "outcome": "spawn_failed",
                                })
                            }
                        }
                    }
                    #[cfg(windows)]
                    {
                        json!({
                            "suite": "spawn",
                            "id": id,
                            "outcome": "not_supported",
                        })
                    }
                }
                _ => panic!("unknown verb {verb}"),
            };

            for (k, v) in expected_obj {
                assert_eq!(&row[k], v, "spawn case {id}.{k} mismatch");
            }
            emit(row);
        }
    }
}
