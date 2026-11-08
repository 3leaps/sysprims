// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

use canonprims_core::DigestAlg;
use serde_json::{json, Value};
use sysprims_evidence::{
    admit_live_observation, admit_report_reader, derive_arguments_digest, derive_executable_digest,
    derive_linux_namespace_context_digest, derive_linux_security_context_digest, json_equal,
    linux_id_is_unambiguously_mapped, parse_strict_utc_unix, project_host_scope_urn,
    validate_observation_shape, ProfileRef, CORPUS_DIGEST, MAX_REPORT_BYTES,
    NATIVE_PROCESS_OBSERVATION_PROFILE_ID, NATIVE_PROCESS_OBSERVATION_PROFILE_REVISION,
    NATIVE_PROCESS_OBSERVATION_README_REVISION, NATIVE_PROCESS_OBSERVATION_SCHEMA_REVISION,
};

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/identity-evidence-v0")
}

#[test]
fn corpus_manifest_and_profile_pins_match() {
    assert!(CORPUS_DIGEST.starts_with("sha256:"));
    let root = corpus_root();
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("MANIFEST.json")).unwrap()).unwrap();
    let files = manifest["files"].as_object().unwrap();
    assert_eq!(
        files.len(),
        manifest["file_count"].as_u64().unwrap() as usize
    );
    for (rel, expected_sha) in files {
        let bytes = fs::read(root.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"));
        let actual_sha = DigestAlg::Sha256.format(&bytes);
        assert_eq!(
            &actual_sha,
            expected_sha.as_str().unwrap(),
            "vendored corpus SHA-256 drift on {rel}"
        );
    }

    let obs_dir = root.join("profiles/native-process-observation");
    assert_eq!(
        DigestAlg::Sha256.format(&fs::read(obs_dir.join("profile.json")).unwrap()),
        NATIVE_PROCESS_OBSERVATION_PROFILE_REVISION
    );
    assert_eq!(
        DigestAlg::Sha256.format(&fs::read(obs_dir.join("README.md")).unwrap()),
        NATIVE_PROCESS_OBSERVATION_README_REVISION
    );
    assert_eq!(
        DigestAlg::Sha256.format(&fs::read(obs_dir.join("evidence.schema.json")).unwrap()),
        NATIVE_PROCESS_OBSERVATION_SCHEMA_REVISION
    );
}

#[test]
fn streaming_reader_caps_oversized_input() {
    let mut exact = vec![b' '; MAX_REPORT_BYTES];
    exact[0] = b'{';
    exact[MAX_REPORT_BYTES - 1] = b'}';
    let ok = admit_report_reader(Cursor::new(exact)).expect("exact 256 KiB reader");
    assert_eq!(ok.canonical_bytes, b"{}");

    let over = vec![b' '; MAX_REPORT_BYTES + 4096];
    let err = admit_report_reader(Cursor::new(over)).expect_err("oversized stream must refuse");
    assert_eq!(err.code(), "report-too-large");
}

#[test]
fn strict_utc_and_json_equal_invariants() {
    assert_eq!(
        parse_strict_utc_unix("2026-10-29T12:00:00Z"),
        Some(1_793_275_200)
    );
    assert_eq!(parse_strict_utc_unix("2026-10-29T12:00:00.000Z"), None);
    assert_eq!(parse_strict_utc_unix("2026-10-29T12:00:00+00:00"), None);
    assert_eq!(parse_strict_utc_unix("2026-10-29t12:00:00z"), None);
    assert_eq!(parse_strict_utc_unix("2026-02-29T12:00:00Z"), None);
    assert_eq!(parse_strict_utc_unix("2026-10-29T12:00:60Z"), None);

    assert!(json_equal(
        &json!({"a": [1, -0.0]}),
        &json!({"a": [1.0, 0]})
    ));
    assert!(!json_equal(&json!({"a": [true]}), &json!({"a": [1]})));
    assert!(!json_equal(&json!([false]), &json!([0])));
    assert!(!json_equal(&json!([1, 2]), &json!([2, 1])));
    assert!(json_equal(
        &json!({"a": 1, "b": 2}),
        &json!({"b": 2.0, "a": 1.0})
    ));
}

#[test]
fn preimage_goldens_and_numeric_bounds() {
    assert_eq!(
        derive_arguments_digest(&[&b"/usr/bin/sleep"[..], &b"300"[..]]),
        "sha256:3846a65c6cdc97a8388e79fad49c2837d9e0abacec2bdeef7f4b54d361b970f1"
    );
    assert_eq!(
        derive_executable_digest(b"\x7fELF-synthetic-sleep-image-v0\n"),
        "sha256:58b9da5ab4aede636708b24b04ccfa6946746ab58058ec2e15ebda26b0a6d909"
    );

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
    assert_eq!(
        derive_linux_namespace_context_digest(&ns_map).unwrap(),
        "sha256:c74496ded4d8d5b0f0e11bfed4c3643e2496880b26502f885880355ec117bded"
    );

    let mut max_ns = ns_map.clone();
    max_ns.insert(
        "user".to_string(),
        "ns:4294967295:4294967295:18446744073709551615".to_string(),
    );
    derive_linux_namespace_context_digest(&max_ns).expect("u32/u64 max ns accepted");

    for bad_entry in [
        "ns:4294967296:0:1",
        "ns:0:4294967296:1",
        "ns:0:4:18446744073709551616",
    ] {
        let mut bad = ns_map.clone();
        bad.insert("user".to_string(), bad_entry.to_string());
        assert_eq!(
            derive_linux_namespace_context_digest(&bad)
                .unwrap_err()
                .code(),
            "invalid-linux-ns-entry"
        );
    }

    let mut partial = ns_map.clone();
    partial.remove("time");
    assert_eq!(
        derive_linux_namespace_context_digest(&partial)
            .unwrap_err()
            .code(),
        "incomplete-linux-namespace-set"
    );

    assert_eq!(
        derive_linux_security_context_digest(
            b"observer_user_ns=userns:0:4:4026531837;groups=27,1000;no_new_privs=1;seccomp=2;cap_inh=0000000000000000;cap_prm=0000000000000000;cap_eff=0000000000000000;cap_bnd=000001ffffffffff;cap_amb=0000000000000000"
        )
        .unwrap(),
        "sha256:954107c2e47b6b4406093ee4e6cf14a66ee9c7350d6550f606b9995104d44976"
    );
    derive_linux_security_context_digest(
        b"observer_user_ns=userns:4294967295:4294967295:18446744073709551615;groups=0,4294967295;no_new_privs=1;seccomp=4294967295;cap_inh=0000000000000000;cap_prm=0000000000000000;cap_eff=0000000000000000;cap_bnd=000001ffffffffff;cap_amb=0000000000000000"
    )
    .expect("u32/u64 max secctx accepted");

    for bad_sec in [
        &b"observer_user_ns=userns:0:4:4026531837;groups=4294967296;no_new_privs=1;seccomp=2;cap_inh=0000000000000000;cap_prm=0000000000000000;cap_eff=0000000000000000;cap_bnd=000001ffffffffff;cap_amb=0000000000000000"[..],
        &b"observer_user_ns=userns:0:4:4026531837;groups=1000,27;no_new_privs=1;seccomp=2;cap_inh=0000000000000000;cap_prm=0000000000000000;cap_eff=0000000000000000;cap_bnd=000001ffffffffff;cap_amb=0000000000000000"[..],
        &b"observer_user_ns=userns:0:4:4026531837;groups=27;no_new_privs=1;seccomp=4294967296;cap_inh=0000000000000000;cap_prm=0000000000000000;cap_eff=0000000000000000;cap_bnd=000001ffffffffff;cap_amb=0000000000000000"[..],
        &b"observer_user_ns=userns:4294967296:4:1;groups=27;no_new_privs=1;seccomp=2;cap_inh=0000000000000000;cap_prm=0000000000000000;cap_eff=0000000000000000;cap_bnd=000001ffffffffff;cap_amb=0000000000000000"[..],
    ] {
        assert_eq!(
            derive_linux_security_context_digest(bad_sec)
                .unwrap_err()
                .code(),
            "invalid-linux-security-context"
        );
    }

    assert!(linux_id_is_unambiguously_mapped(
        1000,
        &[(0, u32::MAX)],
        65534
    ));
    assert!(linux_id_is_unambiguously_mapped(
        65534,
        &[(0, u32::MAX)],
        65534
    ));
    assert!(!linux_id_is_unambiguously_mapped(
        u32::MAX,
        &[(0, u32::MAX)],
        65534
    ));
    assert!(linux_id_is_unambiguously_mapped(1000, &[(0, 65536)], 65534));
    assert!(!linux_id_is_unambiguously_mapped(
        65534,
        &[(0, 65536)],
        65534
    ));
    assert!(!linux_id_is_unambiguously_mapped(
        70000,
        &[(0, 65536)],
        65534
    ));

    let linux_obs: Value = serde_json::from_slice(
        &fs::read(
            corpus_root().join("profiles/native-process-observation/evidence.linux.example.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        project_host_scope_urn(&linux_obs["process"]["host_scope"]).unwrap(),
        "urn:3leaps:identity-evidence:host-scope:v0:sha256:8e3d1953b6494fc7c0399bf1487b90e57917b9a0651670dd83bc62a0af5d0a8d"
    );
}

#[test]
fn full_conformance_and_parity_suite_passes() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        .args(["run", "--quiet", "--example", "evidence-diff"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("execute evidence-diff example");
    assert!(
        output.status.success(),
        "evidence-diff failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let line_count = std::str::from_utf8(&output.stdout).unwrap().lines().count();
    assert!(
        line_count >= 280,
        "expected >= 280 NDJSON parity lines, got {line_count}"
    );
}

#[test]
fn strict_utc_timestamp_enforces_positive_four_digit_year_bounds() {
    assert_eq!(parse_strict_utc_unix("0000-01-01T00:00:00Z"), None);
    assert_eq!(parse_strict_utc_unix("0000-10-29T12:00:00Z"), None);
    assert!(parse_strict_utc_unix("0001-01-01T00:00:00Z").is_some());
    assert!(parse_strict_utc_unix("9999-12-31T23:59:59Z").is_some());

    let mut linux_obs: Value = serde_json::from_slice(
        &fs::read(
            corpus_root().join("profiles/native-process-observation/evidence.linux.example.json"),
        )
        .unwrap(),
    )
    .unwrap();
    linux_obs["observed_at"] = json!("0000-10-29T12:00:00Z");
    assert_eq!(
        validate_observation_shape(&linux_obs).unwrap_err().code(),
        "invalid-observation-shape"
    );
    let accepted = ProfileRef::new(
        NATIVE_PROCESS_OBSERVATION_PROFILE_ID,
        NATIVE_PROCESS_OBSERVATION_PROFILE_REVISION,
    );
    assert_eq!(
        admit_live_observation(
            &linux_obs,
            Some("2026-10-29T12:00:00Z"),
            &accepted,
            None,
            None
        )
        .unwrap_err()
        .code(),
        "invalid-observed-at"
    );
    assert_eq!(
        admit_live_observation(
            &linux_obs,
            Some("0000-10-29T12:00:00Z"),
            &accepted,
            None,
            None
        )
        .unwrap_err()
        .code(),
        "trusted-clock-unavailable"
    );
}
