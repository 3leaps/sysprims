// SPDX-License-Identifier: MIT OR Apache-2.0

//! Deterministic NDJSON evaluation runner over the `identity-evidence/v0`
//! conformance corpus and `parity-cases.json` for cross-language parity checks
//! against `github.com/3leaps/sysprims/evidence`.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use canonprims_core::Limits;
use serde_json::{json, Value};
use sysprims_evidence::{
    admit_and_validate_report, admit_live_observation, admit_report_bytes,
    bind_native_observation_report, canonical_sha256_value, check_import_collisions,
    compare_native_observations_json, derive_arguments_digest, derive_executable_digest,
    derive_linux_namespace_context_digest, derive_linux_security_context_digest, json_equal,
    linux_id_is_unambiguously_mapped, necessary_conditions, process_session_fit,
    project_host_scope_urn, project_to_local_process_session_process_json,
    validate_observation_shape, validate_provenance_dag, validate_report_shape,
    validate_session_bundle_shape, ProfileRef, LOCAL_PROCESS_SESSION_PROFILE_ID,
    LOCAL_PROCESS_SESSION_PROFILE_REVISION, NATIVE_PROCESS_OBSERVATION_PROFILE_ID,
    NATIVE_PROCESS_OBSERVATION_PROFILE_REVISION,
};

fn emit_line(record: &Value) {
    let raw = serde_json::to_vec(record).expect("serialize NDJSON record");
    let canonical =
        canonprims_json::canonicalize(&raw, &Limits::default()).expect("canonicalize NDJSON");
    println!("{}", std::str::from_utf8(&canonical).unwrap());
}

fn decode_hex(hex: &str) -> Vec<u8> {
    assert_eq!(hex.len() % 2, 0);
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex byte"))
        .collect()
}

fn materialize_wire_case(case: &Value) -> Vec<u8> {
    match case["kind"].as_str().expect("wire case kind") {
        "raw" => case["raw"].as_str().unwrap().as_bytes().to_vec(),
        "hex" => decode_hex(case["hex"].as_str().unwrap()),
        "padded-object" => {
            let total = case["bytes"].as_u64().unwrap() as usize;
            let mut out = Vec::with_capacity(total);
            out.push(b'{');
            out.resize(total - 1, b' ');
            out.push(b'}');
            out
        }
        "nested-array" => {
            let depth = case["depth"].as_u64().unwrap() as usize;
            let mut s = "[".repeat(depth);
            s.push('0');
            s.push_str(&"]".repeat(depth));
            s.into_bytes()
        }
        "number-array" => {
            let count = case["count"].as_u64().unwrap() as usize;
            let elems: Vec<&str> = std::iter::repeat_n("1e20", count).collect();
            format!("[{}]", elems.join(",")).into_bytes()
        }
        other => panic!("unknown wire case kind: {other}"),
    }
}

fn apply_json_changes(doc: &mut Value, changes: &[Value]) {
    for change in changes {
        let path = change["path"].as_array().expect("change path");
        let val = change["value"].clone();
        let mut cur = &mut *doc;
        for part in &path[..path.len() - 1] {
            if let Some(k) = part.as_str() {
                cur = cur.get_mut(k).expect("object key in change path");
            } else if let Some(idx) = part.as_u64() {
                cur = cur
                    .get_mut(idx as usize)
                    .expect("array index in change path");
            } else {
                panic!("unsupported path segment: {part:?}");
            }
        }
        let leaf = &path[path.len() - 1];
        if leaf.as_str() == Some("-") {
            cur.as_array_mut().expect("append target array").push(val);
        } else if let Some(k) = leaf.as_str() {
            cur.as_object_mut()
                .expect("leaf target object")
                .insert(k.to_string(), val);
        } else if let Some(idx) = leaf.as_u64() {
            cur.as_array_mut().expect("leaf target array")[idx as usize] = val;
        }
    }
}

fn parse_inputs_map(v: Option<&Value>) -> BTreeMap<String, Value> {
    let mut map = BTreeMap::new();
    if let Some(obj) = v.and_then(Value::as_object) {
        for (k, val) in obj {
            map.insert(k.clone(), val.clone());
        }
    }
    map
}

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let corpus_dir = env::var("EVIDENCE_CORPUS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("tests/identity-evidence-v0"));
    let parity_path = env::var("EVIDENCE_PARITY_CASES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("tests/parity-cases.json"));

    run_all(&corpus_dir, &parity_path);
}

fn run_all(corpus_dir: &Path, parity_path: &Path) {
    let parity: Value =
        serde_json::from_slice(&fs::read(parity_path).expect("read parity-cases.json"))
            .expect("parse parity-cases.json");

    // 1. Wire admission cases
    let wire_doc: Value = serde_json::from_slice(
        &fs::read(corpus_dir.join("vectors/wire-admission.json")).expect("read wire vectors"),
    )
    .unwrap();
    for case in wire_doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(parity["extra_wire_cases"].as_array().unwrap().iter())
    {
        let id = case["id"].as_str().unwrap();
        let raw = materialize_wire_case(case);
        let expected = case["expected"].as_str().unwrap();
        match admit_report_bytes(&raw) {
            Ok(admitted) => {
                assert_eq!(expected, "accept", "wire case {id} expected refusal");
                if let Some(exp_can) = case.get("canonical").and_then(Value::as_str) {
                    assert_eq!(
                        std::str::from_utf8(&admitted.canonical_bytes).unwrap(),
                        exp_can,
                        "wire canonical mismatch on {id}"
                    );
                }
                emit_line(&json!({
                    "suite": "wire",
                    "id": id,
                    "outcome": "accept",
                    "digest": admitted.digest,
                    "canonical_len": admitted.canonical_bytes.len(),
                }));
            }
            Err(err) => {
                assert_eq!(err.code(), expected, "wire refusal mismatch on {id}");
                emit_line(&json!({
                    "suite": "wire",
                    "id": id,
                    "outcome": err.code(),
                }));
            }
        }
    }

    // 2. Example reports
    let mut examples_by_stem: BTreeMap<String, Value> = BTreeMap::new();
    let mut report_inputs: BTreeMap<String, Value> = BTreeMap::new();
    let mut example_paths: Vec<PathBuf> = fs::read_dir(corpus_dir.join("examples"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    example_paths.sort();
    for p in &example_paths {
        let raw = fs::read(p).unwrap();
        let admitted = admit_report_bytes(&raw).expect("admit example report");
        let stem = p.file_stem().unwrap().to_str().unwrap().to_string();
        let rep_id = admitted.value["report_id"].as_str().unwrap().to_string();
        report_inputs.insert(rep_id, admitted.value.clone());
        examples_by_stem.insert(stem, admitted.value);
    }
    for p in &example_paths {
        let stem = p.file_stem().unwrap().to_str().unwrap();
        let raw = fs::read(p).unwrap();
        let admitted =
            admit_and_validate_report(&raw, Some(&report_inputs)).expect("validate example report");
        emit_line(&json!({
            "suite": "example-report",
            "id": stem,
            "outcome": "accept",
            "digest": admitted.digest,
        }));
    }

    // 3. Two-producer import collision
    let collision: Value = serde_json::from_slice(
        &fs::read(corpus_dir.join("fixtures/two-producer-collision.json")).unwrap(),
    )
    .unwrap();
    let col_inputs: Vec<Value> = collision["inputs"].as_array().unwrap().clone();
    let col_err = check_import_collisions(&col_inputs).expect_err("collision must refuse");
    assert_eq!(col_err.code(), collision["expected"].as_str().unwrap());
    emit_line(&json!({
        "suite": "import-collision",
        "id": "two-producer-collision",
        "outcome": col_err.code(),
    }));
    check_import_collisions(&[col_inputs[0].clone(), col_inputs[0].clone()])
        .expect("identical duplicate report import allowed");
    emit_line(&json!({
        "suite": "import-collision",
        "id": "identical-self-duplicate",
        "outcome": "accept",
    }));

    // 4. Report shape & semantic & provenance DAG cases
    for case in parity["report_shape_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        let got = match validate_report_shape(&case["report"]) {
            Ok(()) => "accept".to_string(),
            Err(e) => e.code,
        };
        assert_eq!(got, expected, "report shape case {id}");
        emit_line(&json!({ "suite": "report-shape", "id": id, "outcome": got }));
    }

    for case in parity["report_semantic_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        validate_report_shape(&case["report"]).expect("semantic case must pass report shape");
        let inputs = parse_inputs_map(case.get("inputs"));
        let got = match necessary_conditions(&case["report"], Some(&inputs)) {
            Ok(()) => "accept".to_string(),
            Err(e) => e.code,
        };
        assert_eq!(got, expected, "report semantic case {id}");
        emit_line(&json!({ "suite": "report-semantic", "id": id, "outcome": got }));
    }

    for case in parity["provenance_dag_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        let max_depth = case["max_depth"].as_u64().unwrap() as usize;
        let inputs = parse_inputs_map(case.get("inputs"));
        match validate_provenance_dag(&case["report"], &inputs, max_depth) {
            Ok(()) => {
                assert_eq!(expected, "accept", "dag case {id}");
                emit_line(&json!({
                    "suite": "provenance-dag",
                    "id": id,
                    "outcome": "accept",
                }));
            }
            Err(e) => {
                assert_eq!(e.code(), expected, "dag case {id}");
                emit_line(&json!({
                    "suite": "provenance-dag",
                    "id": id,
                    "outcome": e.code(),
                }));
            }
        }
    }

    // 5. Local-process-session fit cases
    let sess_dir = corpus_dir.join("profiles/local-process-session");
    let sess_bundle: Value =
        serde_json::from_slice(&fs::read(sess_dir.join("evidence.example.json")).unwrap()).unwrap();
    validate_session_bundle_shape(&sess_bundle).expect("valid session bundle example");
    let orig_ctx: Value = serde_json::from_slice(
        &fs::read(sess_dir.join("evaluation-context.example.json")).unwrap(),
    )
    .unwrap();
    let sess_prof = ProfileRef::new(
        LOCAL_PROCESS_SESSION_PROFILE_ID,
        LOCAL_PROCESS_SESSION_PROFILE_REVISION,
    );
    let sess_cases_a: Vec<Value> =
        serde_json::from_slice(&fs::read(sess_dir.join("cases.json")).unwrap()).unwrap();
    let sess_cases_b: Vec<Value> =
        serde_json::from_slice(&fs::read(sess_dir.join("report-cases.json")).unwrap()).unwrap();
    for case in sess_cases_a.iter().chain(sess_cases_b.iter()) {
        let id = case["id"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        let mut ctx = orig_ctx.clone();
        let mut rep = examples_by_stem["local-process-session-launch"].clone();
        let mut orig_rep = rep.clone();
        let mut ctx_changes: Vec<Value> = case
            .get("context_changes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(ch) = case.get("change") {
            if !ch.is_null() {
                ctx_changes.insert(0, ch.clone());
            }
        }
        apply_json_changes(&mut ctx, &ctx_changes);
        if let Some(rc) = case.get("report_changes").and_then(Value::as_array) {
            apply_json_changes(&mut rep, rc);
        }
        if let Some(irc) = case.get("input_report_changes").and_then(Value::as_array) {
            apply_json_changes(&mut orig_rep, irc);
        }
        validate_report_shape(&rep).unwrap();
        let mut in_map = BTreeMap::new();
        in_map.insert(
            orig_rep["report_id"].as_str().unwrap().to_string(),
            orig_rep,
        );
        necessary_conditions(&rep, Some(&in_map)).unwrap();
        let got = match process_session_fit(&rep, &sess_bundle, &ctx, &sess_prof) {
            Ok(()) => "accept".to_string(),
            Err(e) => e.code,
        };
        assert_eq!(got, expected, "session fit case {id}");
        emit_line(&json!({ "suite": "session-fit", "id": id, "outcome": got }));
    }

    for case in parity["extra_session_fit_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        validate_session_bundle_shape(&case["bundle"]).unwrap();
        let got = match process_session_fit(
            &case["report"],
            &case["bundle"],
            &case["context"],
            &sess_prof,
        ) {
            Ok(()) => "accept".to_string(),
            Err(e) => e.code,
        };
        assert_eq!(got, expected, "extra session fit case {id}");
        emit_line(&json!({ "suite": "session-fit-extra", "id": id, "outcome": got }));
    }

    // 6. Native-process-observation shape, admit, compare, project, bind & preimage suites
    let obs_prof = ProfileRef::new(
        NATIVE_PROCESS_OBSERVATION_PROFILE_ID,
        NATIVE_PROCESS_OBSERVATION_PROFILE_REVISION,
    );

    for case in parity["observation_shape_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        let got = match validate_observation_shape(&case["obs"]) {
            Ok(()) => "accept".to_string(),
            Err(e) => e.code,
        };
        assert_eq!(got, expected, "obs shape case {id}");
        emit_line(&json!({ "suite": "obs-shape", "id": id, "outcome": got }));
    }

    for case in parity["observation_admit_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        let now_opt = case.get("now_utc").and_then(Value::as_str);
        let rep_opt = case.get("report").filter(|v| !v.is_null());
        let inputs = parse_inputs_map(case.get("inputs"));
        let got = match admit_live_observation(
            &case["obs"],
            now_opt,
            &obs_prof,
            rep_opt,
            Some(&inputs),
        ) {
            Ok(()) => "accept".to_string(),
            Err(e) => e.code,
        };
        assert_eq!(got, expected, "obs admit case {id}");
        emit_line(&json!({ "suite": "obs-admit", "id": id, "outcome": got }));
    }

    for case in parity["observation_compare_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        let got = compare_native_observations_json(
            &case["baseline"],
            &case["candidate"],
            &case["policy"],
            &obs_prof,
        )
        .as_code();
        assert_eq!(got, expected, "obs compare case {id}");
        emit_line(&json!({ "suite": "obs-compare", "id": id, "outcome": got }));
    }

    for case in parity["observation_projection_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        let rep_opt = case.get("report").filter(|v| !v.is_null());
        let inputs = parse_inputs_map(case.get("inputs"));
        match project_to_local_process_session_process_json(
            &case["obs"],
            &case["host_scope_trusted"],
            &case["now_utc"],
            &obs_prof,
            rep_opt,
            Some(&inputs),
            case.get("allowed_clone_behaviors"),
        ) {
            Ok(proj) => {
                assert_eq!(expected, "accept", "obs projection case {id}");
                assert!(
                    json_equal(&proj, &case["projected"]),
                    "projected payload mismatch on {id}"
                );
                let digest = canonical_sha256_value(&proj).unwrap();
                emit_line(&json!({
                    "suite": "obs-projection",
                    "id": id,
                    "outcome": "accept",
                    "digest": digest,
                }));
            }
            Err(e) => {
                assert_eq!(e.code(), expected, "obs projection refusal on {id}");
                emit_line(&json!({
                    "suite": "obs-projection",
                    "id": id,
                    "outcome": e.code(),
                }));
            }
        }
    }

    for case in parity["observation_bind_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        let got = match bind_native_observation_report(&case["report"], &case["obs"], &obs_prof) {
            Ok(()) => "accept".to_string(),
            Err(e) => e.code,
        };
        assert_eq!(got, expected, "obs bind case {id}");
        emit_line(&json!({ "suite": "obs-bind", "id": id, "outcome": got }));
    }

    // Golden preimage & overflow-mapping parity lines
    let linux_obs: Value = serde_json::from_slice(
        &fs::read(
            corpus_dir.join("profiles/native-process-observation/evidence.linux.example.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let argv_dig = derive_arguments_digest(&[&b"/usr/bin/sleep"[..], &b"300"[..]]);
    let exe_dig = derive_executable_digest(b"\x7fELF-synthetic-sleep-image-v0\n");
    let mut ns_map = BTreeMap::new();
    for (k, v) in [
        ("cgroup", "ns:0:4:4026531835"),
        ("ipc", "ns:0:4:4026531839"),
        ("mnt", "ns:0:4:4026531840"),
        ("net", "ns:0:4:4026531992"),
        ("pid", "ns:0:4:4026531836"),
        ("time", "ns:0:4:4026531834"),
        ("time_for_children", "ns:0:4:4026531834"),
        ("user", "ns:0:4:4026531837"),
        ("uts", "ns:0:4:4026531838"),
    ] {
        ns_map.insert(k.to_string(), v.to_string());
    }
    let ns_dig = derive_linux_namespace_context_digest(&ns_map).unwrap();
    let sec_dig = derive_linux_security_context_digest(
        b"observer_user_ns=userns:0:4:4026531837;groups=27,1000;no_new_privs=1;seccomp=2;cap_inh=0000000000000000;cap_prm=0000000000000000;cap_eff=0000000000000000;cap_bnd=000001ffffffffff;cap_amb=0000000000000000",
    )
    .unwrap();
    let hs_urn = project_host_scope_urn(&linux_obs["process"]["host_scope"]).unwrap();
    let id_checks = [
        linux_id_is_unambiguously_mapped(1000, &[(0, u32::MAX)], 65534),
        linux_id_is_unambiguously_mapped(65534, &[(0, u32::MAX)], 65534),
        linux_id_is_unambiguously_mapped(u32::MAX, &[(0, u32::MAX)], 65534),
        linux_id_is_unambiguously_mapped(1000, &[(0, 65536)], 65534),
        linux_id_is_unambiguously_mapped(65534, &[(0, 65536)], 65534),
        linux_id_is_unambiguously_mapped(70000, &[(0, 65536)], 65534),
    ];
    emit_line(&json!({
        "suite": "preimage-goldens",
        "id": "linux-goldens",
        "arguments_digest": argv_dig,
        "executable_digest": exe_dig,
        "namespace_context_digest": ns_dig,
        "security_context_digest": sec_dig,
        "host_scope_urn": hs_urn,
        "id_map_checks": id_checks,
    }));

    if let Ok(extra_path) = env::var("EVIDENCE_EXTRA_OBSERVATION_CASES") {
        if !extra_path.is_empty() {
            run_extra_observation_pack(Path::new(&extra_path), &obs_prof);
        }
    }
}

fn run_extra_observation_pack(path: &Path, obs_prof: &ProfileRef) {
    let doc: Value = serde_json::from_slice(&fs::read(path).expect("read extra observation cases"))
        .expect("parse extra observation cases");
    let cases = if let Some(arr) = doc.as_array() {
        arr.clone()
    } else if let Some(arr) = doc.get("cases").and_then(Value::as_array) {
        arr.clone()
    } else {
        Vec::new()
    };
    for case in &cases {
        let id = case["id"].as_str().unwrap_or("unnamed");
        let kind = case
            .get("kind")
            .or_else(|| case.get("operation"))
            .and_then(Value::as_str)
            .unwrap_or("compare");
        match kind {
            "compare" => {
                let expected = case["expected"].as_str().unwrap();
                let got = compare_native_observations_json(
                    &case["baseline"],
                    &case["candidate"],
                    &case["policy"],
                    obs_prof,
                )
                .as_code();
                assert_eq!(got, expected, "extra compare case {id}");
                emit_line(&json!({ "suite": "extra-obs", "id": id, "outcome": got }));
            }
            "admit" => {
                let expected = case["expected"].as_str().unwrap();
                let now_opt = case
                    .get("now_utc")
                    .or_else(|| case.get("now"))
                    .and_then(Value::as_str);
                let rep_opt = case.get("report").filter(|v| !v.is_null());
                let inputs =
                    parse_inputs_map(case.get("inputs").or_else(|| case.get("input_reports")));
                let got = match admit_live_observation(
                    &case["obs"],
                    now_opt,
                    obs_prof,
                    rep_opt,
                    Some(&inputs),
                ) {
                    Ok(()) => "accept".to_string(),
                    Err(e) => e.code,
                };
                assert_eq!(got, expected, "extra admit case {id}");
                emit_line(&json!({ "suite": "extra-obs", "id": id, "outcome": got }));
            }
            "project" | "projection" => {
                let expected = case["expected"].as_str().unwrap();
                let now_val = case
                    .get("now_utc")
                    .or_else(|| case.get("now"))
                    .unwrap_or(&Value::Null);
                let rep_opt = case.get("report").filter(|v| !v.is_null());
                let inputs =
                    parse_inputs_map(case.get("inputs").or_else(|| case.get("input_reports")));
                match project_to_local_process_session_process_json(
                    &case["obs"],
                    case.get("host_scope_trusted").unwrap_or(&Value::Null),
                    now_val,
                    obs_prof,
                    rep_opt,
                    Some(&inputs),
                    case.get("allowed_clone_behaviors"),
                ) {
                    Ok(proj) => {
                        assert_eq!(expected, "accept", "extra projection case {id}");
                        if let Some(exp_proj) = case.get("projected") {
                            assert!(json_equal(&proj, exp_proj), "extra projected mismatch {id}");
                        }
                        let digest = canonical_sha256_value(&proj).unwrap();
                        emit_line(&json!({
                            "suite": "extra-obs",
                            "id": id,
                            "outcome": "accept",
                            "digest": digest,
                        }));
                    }
                    Err(e) => {
                        assert_eq!(e.code(), expected, "extra projection refusal {id}");
                        emit_line(&json!({
                            "suite": "extra-obs",
                            "id": id,
                            "outcome": e.code(),
                        }));
                    }
                }
            }
            "shape" => {
                let expected = case["expected"].as_str().unwrap();
                let got = match validate_observation_shape(&case["obs"]) {
                    Ok(()) => "accept".to_string(),
                    Err(e) => e.code,
                };
                assert_eq!(got, expected, "extra shape case {id}");
                emit_line(&json!({ "suite": "extra-obs", "id": id, "outcome": got }));
            }
            other => panic!("unsupported extra case kind {other} on {id}"),
        }
    }
}
