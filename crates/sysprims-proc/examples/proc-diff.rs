// SPDX-License-Identifier: MIT OR Apache-2.0

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

use canonprims_core::Limits;
use canonprims_json::canonicalize;
use serde_json::{json, Value};
use sysprims_core::schema::{
    ANCESTORS_RESULT_V1, DESCENDANTS_RESULT_SAMPLED_V1, DESCENDANTS_RESULT_V1, FD_SNAPSHOT_V1,
    GUARD_EVENT_V1, PORT_BINDINGS_V1, PORT_FILTER_V1, PROCESS_INFO_SAMPLED_V1, PROCESS_INFO_V1,
    WAIT_PID_RESULT_V1,
};
use sysprims_core::SysprimsError;
use sysprims_proc::{
    ancestors, descendants, get_process, guard_step, listening_ports, select_descendant_targets,
    CpuMode, DescendantsLevel, DescendantsResult, GuardAction, GuardConfig, GuardPreset, GuardRule,
    PortFilter, ProcessFilter, ProcessInfo, ProcessOptions, ProcessState, Protocol,
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

fn parse_state(s: &str) -> ProcessState {
    match s {
        "running" => ProcessState::Running,
        "sleeping" => ProcessState::Sleeping,
        "stopped" => ProcessState::Stopped,
        "zombie" => ProcessState::Zombie,
        _ => ProcessState::Unknown,
    }
}

fn build_synthetic_tree(tree_val: &Value) -> DescendantsResult {
    let root_pid = tree_val["root_pid"].as_u64().unwrap() as u32;
    let max_levels = tree_val["max_levels"].as_u64().unwrap() as u32;
    let mut levels = Vec::new();
    let mut total_found = 0usize;

    for lvl in tree_val["levels"].as_array().unwrap() {
        let level_num = lvl["level"].as_u64().unwrap() as u32;
        let mut procs = Vec::new();
        for p in lvl["processes"].as_array().unwrap() {
            total_found += 1;
            let cmdline: Vec<String> = p["cmdline"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect();
            procs.push(ProcessInfo {
                pid: p["pid"].as_u64().unwrap() as u32,
                ppid: p["ppid"].as_u64().unwrap() as u32,
                name: p["name"].as_str().unwrap().to_string(),
                user: p["user"].as_str().map(|s| s.to_string()),
                cpu_percent: p["cpu_percent"].as_f64().unwrap(),
                memory_kb: p["memory_kb"].as_u64().unwrap(),
                elapsed_seconds: p["elapsed_seconds"].as_u64().unwrap(),
                start_time_unix_ms: None,
                exe_path: None,
                state: parse_state(p["state"].as_str().unwrap()),
                cmdline,
                env: None,
                thread_count: None,
            });
        }
        levels.push(DescendantsLevel {
            level: level_num,
            processes: procs,
        });
    }

    DescendantsResult {
        schema_id: DESCENDANTS_RESULT_V1,
        root_pid,
        max_levels,
        levels,
        total_found,
        matched_by_filter: total_found,
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        platform: "macos",
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let vectors_path = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        PathBuf::from("testdata/conformance/proc/vectors.json")
    };

    let raw = fs::read_to_string(&vectors_path).unwrap_or_else(|e| {
        eprintln!("failed to read {}: {e}", vectors_path.display());
        process::exit(2);
    });
    let doc: Value = serde_json::from_str(&raw).expect("parse vectors.json");

    // 1. Schema constants
    emit(json!({
        "suite": "schemas",
        "process_info_v1": PROCESS_INFO_V1,
        "process_info_sampled_v1": PROCESS_INFO_SAMPLED_V1,
        "descendants_result_v1": DESCENDANTS_RESULT_V1,
        "descendants_result_sampled_v1": DESCENDANTS_RESULT_SAMPLED_V1,
        "ancestors_result_v1": ANCESTORS_RESULT_V1,
        "wait_pid_result_v1": WAIT_PID_RESULT_V1,
        "fd_snapshot_v1": FD_SNAPSHOT_V1,
        "port_bindings_v1": PORT_BINDINGS_V1,
        "port_filter_v1": PORT_FILTER_V1,
        "guard_event_v1": GUARD_EVENT_V1,
    }));

    let synthetic_tree = build_synthetic_tree(&doc["synthetic_tree"]);

    for vec_entry in doc["vectors"].as_array().unwrap() {
        let id = vec_entry["id"].as_str().unwrap();
        let kind = vec_entry["kind"].as_str().unwrap();

        match kind {
            "validate_pid" => {
                let pid_u64 = vec_entry["pid"].as_u64().unwrap();
                let pid = if pid_u64 > u32::MAX as u64 {
                    u32::MAX
                } else {
                    pid_u64 as u32
                };
                let expected = vec_entry["expected_error_category"].as_str().unwrap();
                let err = get_process(pid).expect_err("expected invalid pid error");
                let slug = error_code_slug(&err);
                assert_eq!(slug, expected, "vector {id} mismatch");
                emit(json!({
                    "id": id,
                    "kind": kind,
                    "outcome": slug,
                }));
            }
            "validate_filter" => {
                let expected = vec_entry["expected_error_category"].as_str().unwrap();
                let filter: ProcessFilter =
                    serde_json::from_value(vec_entry["filter"].clone()).expect("parse filter");
                let err = filter.validate().expect_err("expected filter error");
                let slug = error_code_slug(&err);
                assert_eq!(slug, expected, "vector {id} mismatch");
                emit(json!({
                    "id": id,
                    "kind": kind,
                    "outcome": slug,
                }));
            }
            "validate_port_filter" => {
                let expected = vec_entry["expected_error_category"].as_str().unwrap();
                let local_port = vec_entry["local_port"].as_u64().unwrap() as u16;
                let pf = PortFilter {
                    protocol: Some(Protocol::Tcp),
                    local_port: Some(local_port),
                };
                let err = listening_ports(Some(&pf)).expect_err("expected port filter error");
                let slug = error_code_slug(&err);
                assert_eq!(slug, expected, "vector {id} mismatch");
                emit(json!({
                    "id": id,
                    "kind": kind,
                    "outcome": slug,
                }));
            }
            "guard_preset" => {
                let preset_str = vec_entry["preset"].as_str().unwrap();
                let preset = match preset_str {
                    "interactive" => GuardPreset::Interactive,
                    "background" => GuardPreset::Background,
                    "watchdog" => GuardPreset::Watchdog,
                    _ => panic!("unknown preset {preset_str}"),
                };
                let interval_ms = preset.interval().as_millis() as u64;
                let sample_ms = preset.sample_duration().as_millis() as u64;
                let expected = &vec_entry["expected"];
                assert_eq!(interval_ms, expected["interval_ms"].as_u64().unwrap());
                assert_eq!(sample_ms, expected["sample_duration_ms"].as_u64().unwrap());
                emit(json!({
                    "id": id,
                    "kind": kind,
                    "preset": preset_str,
                    "interval_ms": interval_ms,
                    "sample_duration_ms": sample_ms,
                }));
            }
            "select_targets" => {
                let filter: ProcessFilter =
                    serde_json::from_value(vec_entry["filter"].clone()).expect("parse filter");
                let cascade = vec_entry["cascade"].as_bool().unwrap();
                let expected_pids: Vec<u32> = vec_entry["expected_pids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_u64().unwrap() as u32)
                    .collect();
                let got_pids = select_descendant_targets(&synthetic_tree, Some(&filter), cascade);
                assert_eq!(got_pids, expected_pids, "vector {id} mismatch");
                emit(json!({
                    "id": id,
                    "kind": kind,
                    "cascade": cascade,
                    "matched_pids": got_pids,
                }));
            }
            "live_probe_self" => {
                let self_pid = process::id();
                let self_info = get_process(self_pid).expect("get_process(self)");
                let anc = ancestors(self_pid, 3, ProcessOptions::default()).expect("ancestors");
                let desc = descendants(self_pid, 1, None).expect("descendants");
                let guard = guard_step(GuardConfig {
                    rule: GuardRule {
                        root_pid: self_pid,
                        max_levels: 1,
                        filter: None,
                        cpu_mode: CpuMode::Lifetime,
                        sample_duration: None,
                    },
                    action: GuardAction::KillDescendants {
                        signal: 15,
                        cascade: false,
                    },
                    action_enabled: false,
                    max_targets: 8,
                })
                .expect("guard_step");

                emit(json!({
                    "id": id,
                    "kind": kind,
                    "ancestors_chain_non_empty": !anc.chain.is_empty() && anc.chain[0].pid == self_pid,
                    "ancestors_schema_id": anc.schema_id,
                    "descendants_schema_id": desc.schema_id,
                    "fd_snapshot_schema_id": FD_SNAPSHOT_V1,
                    "guard_event_schema_id": guard.schema_id,
                    "port_bindings_schema_id": PORT_BINDINGS_V1,
                    "process_info_schema_id": PROCESS_INFO_V1,
                    "self_found": self_info.pid == self_pid && !self_info.name.is_empty(),
                }));
            }
            _ => panic!("unknown vector kind: {kind}"),
        }
    }
}
