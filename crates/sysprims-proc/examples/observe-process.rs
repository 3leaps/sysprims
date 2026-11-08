// SPDX-License-Identifier: MIT OR Apache-2.0

//! Emit a canonical JSON `native-process-observation:v0` payload for a target PID.

use std::io::Write;
use std::process::ExitCode;

use sysprims_evidence::{admit_report_bytes, ProfileRef};
use sysprims_proc::observation::{observe_process, HostScope, ObservationOptions};

const DEFAULT_DERIVATION_REV: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut pid: Option<u32> = None;
    let mut observed_at: Option<String> = None;
    let mut subject_ref = String::from("process-target");
    let mut all_context = false;
    let mut host_id_ref = Some(String::from("id-host-local"));
    let mut host_ns = String::from("urn:3leaps:identity-evidence:host-scope:test-fixture:v0");
    let mut host_val = String::from("local-qualification-host");
    let mut host_deriv_id =
        String::from("urn:3leaps:identity-evidence:derivation:caller-supplied-host-scope:v0");
    let mut host_deriv_rev = String::from(DEFAULT_DERIVATION_REV);
    let mut host_stability = String::from("boot");
    let mut host_clone = String::from("regenerated");

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pid" => {
                let Some(raw) = args.next() else {
                    eprintln!("error: --pid requires a numeric value");
                    return ExitCode::from(2);
                };
                match raw.parse::<u32>() {
                    Ok(v) => pid = Some(v),
                    Err(_) => {
                        eprintln!("error: invalid --pid value: {raw}");
                        return ExitCode::from(2);
                    }
                }
            }
            "--observed-at" => {
                let Some(v) = args.next() else {
                    eprintln!("error: --observed-at requires a value");
                    return ExitCode::from(2);
                };
                observed_at = Some(v);
            }
            "--subject-ref" => {
                let Some(v) = args.next() else {
                    eprintln!("error: --subject-ref requires a value");
                    return ExitCode::from(2);
                };
                subject_ref = v;
            }
            "--all-context" => {
                all_context = true;
            }
            "--no-host-id-ref" => {
                host_id_ref = None;
            }
            "--host-id-ref" => {
                let Some(v) = args.next() else {
                    eprintln!("error: --host-id-ref requires a value");
                    return ExitCode::from(2);
                };
                host_id_ref = Some(v);
            }
            "--host-namespace" => {
                let Some(v) = args.next() else {
                    eprintln!("error: --host-namespace requires a value");
                    return ExitCode::from(2);
                };
                host_ns = v;
            }
            "--host-value" => {
                let Some(v) = args.next() else {
                    eprintln!("error: --host-value requires a value");
                    return ExitCode::from(2);
                };
                host_val = v;
            }
            "--host-derivation-id" => {
                let Some(v) = args.next() else {
                    eprintln!("error: --host-derivation-id requires a value");
                    return ExitCode::from(2);
                };
                host_deriv_id = v;
            }
            "--host-derivation-rev" => {
                let Some(v) = args.next() else {
                    eprintln!("error: --host-derivation-rev requires a value");
                    return ExitCode::from(2);
                };
                host_deriv_rev = v;
            }
            "--host-stability" => {
                let Some(v) = args.next() else {
                    eprintln!("error: --host-stability requires a value");
                    return ExitCode::from(2);
                };
                host_stability = v;
            }
            "--host-clone-behavior" => {
                let Some(v) = args.next() else {
                    eprintln!("error: --host-clone-behavior requires a value");
                    return ExitCode::from(2);
                };
                host_clone = v;
            }
            other => {
                eprintln!("error: unrecognized flag {other}");
                return ExitCode::from(2);
            }
        }
    }

    let Some(target_pid) = pid else {
        eprintln!("error: --pid <1..=2147483647> is required");
        return ExitCode::from(2);
    };

    let host_scope = HostScope {
        identifier_ref: host_id_ref,
        namespace: host_ns,
        value: host_val,
        derivation: ProfileRef::new(host_deriv_id, host_deriv_rev),
        stability: host_stability,
        clone_behavior: host_clone,
    };

    let mut options = ObservationOptions {
        subject_ref,
        observed_at,
        ..ObservationOptions::default()
    };
    if all_context {
        options = options.with_all_context_facts();
    }

    let obs = match observe_process(target_pid, &host_scope, &options) {
        Ok(v) => v,
        Err(refusal) => {
            eprintln!("refused:{refusal}");
            return ExitCode::from(1);
        }
    };

    let raw = match serde_json::to_vec(&obs) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("serialization-error:{e}");
            return ExitCode::from(1);
        }
    };
    let admitted = match admit_report_bytes(&raw) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("canonicalization-error:{e}");
            return ExitCode::from(1);
        }
    };

    let mut out = std::io::stdout().lock();
    if out.write_all(&admitted.canonical_bytes).is_err() || out.write_all(b"\n").is_err() {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
