// SPDX-License-Identifier: MIT OR Apache-2.0

//! `local-process-session:v0` bundle shape validation and deterministic
//! `process_session_fit` contract-fit evaluation.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use crate::wire::canonical_sha256_value;
use crate::{
    is_local_id, is_nonempty_512, is_principal_obj, is_profile_ref_obj, is_sha256_digest,
    json_equal, parse_strict_utc_unix, validate_report_shape, EvidenceError, ProfileRef,
};

/// `local-process-session:v0` profile identifier.
pub const LOCAL_PROCESS_SESSION_PROFILE_ID: &str =
    "urn:3leaps:identity-evidence:local-process-session:v0";

/// Pinned SHA-256 revision of `schemas/identity-evidence/v0/profiles/local-process-session/profile.json`.
pub const LOCAL_PROCESS_SESSION_PROFILE_REVISION: &str =
    "sha256:5cf6300b565a7fca392003edd376f33c1c2b90b72e7436aa89a5c0533e96dc3d";

/// `local-process-session:v0` transcript domain.
pub const LOCAL_PROCESS_SESSION_TRANSCRIPT_DOMAIN: &str =
    "3leaps.identity-evidence.local-process-session.v0";

const MAX_RECEIPT_SECONDS: i64 = 300;
const MAX_CHALLENGE_SECONDS: i64 = 60;
const MAX_FUTURE_SKEW_SECONDS: i64 = 5;
const MAX_OBSERVATION_AGE_SECONDS: i64 = 5;

fn err_bundle_shape() -> EvidenceError {
    EvidenceError::new("invalid-session-bundle-shape")
}

fn check_exact_keys(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> bool {
    obj.len() == keys.len() && keys.iter().all(|&k| obj.contains_key(k))
}

fn is_session_pid_string(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 20
        && (b'1'..=b'9').contains(&b[0])
        && b[1..].iter().all(u8::is_ascii_digit)
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// Validate `local-process-session/evidence.schema.json` shape rules.
pub fn validate_session_bundle_shape(bundle: &Value) -> Result<(), EvidenceError> {
    let Some(obj) = bundle.as_object() else {
        return Err(err_bundle_shape());
    };
    if !check_exact_keys(
        obj,
        &[
            "profile",
            "process",
            "session",
            "launch",
            "challenge",
            "proof",
        ],
    ) || !obj.get("profile").is_some_and(is_profile_ref_obj)
    {
        return Err(err_bundle_shape());
    }

    let Some(proc_obj) = obj.get("process").and_then(Value::as_object) else {
        return Err(err_bundle_shape());
    };
    if !check_exact_keys(
        proc_obj,
        &[
            "subject_ref",
            "host_scope",
            "boot_id",
            "pid_namespace",
            "pid",
            "start_identity",
            "execution_context",
        ],
    ) || !proc_obj
        .get("subject_ref")
        .and_then(Value::as_str)
        .is_some_and(is_local_id)
        || !proc_obj
            .get("host_scope")
            .and_then(Value::as_str)
            .is_some_and(is_nonempty_512)
        || !proc_obj
            .get("boot_id")
            .and_then(Value::as_str)
            .is_some_and(is_nonempty_512)
        || !proc_obj
            .get("pid_namespace")
            .and_then(Value::as_str)
            .is_some_and(is_nonempty_512)
        || !proc_obj
            .get("pid")
            .and_then(Value::as_str)
            .is_some_and(is_session_pid_string)
    {
        return Err(err_bundle_shape());
    }
    let Some(si) = proc_obj.get("start_identity").and_then(Value::as_object) else {
        return Err(err_bundle_shape());
    };
    if !check_exact_keys(si, &["profile", "value"])
        || !si.get("profile").is_some_and(is_profile_ref_obj)
        || !si
            .get("value")
            .and_then(Value::as_str)
            .is_some_and(is_nonempty_512)
    {
        return Err(err_bundle_shape());
    }
    let Some(ec) = proc_obj.get("execution_context").and_then(Value::as_object) else {
        return Err(err_bundle_shape());
    };
    if !check_exact_keys(
        ec,
        &[
            "security_principal",
            "namespace_context_digest",
            "executable_digest",
            "arguments_digest",
            "security_context_digest",
        ],
    ) || !ec.get("security_principal").is_some_and(is_principal_obj)
        || ![
            "namespace_context_digest",
            "executable_digest",
            "arguments_digest",
            "security_context_digest",
        ]
        .iter()
        .all(|k| {
            ec.get(*k)
                .and_then(Value::as_str)
                .is_some_and(is_sha256_digest)
        })
    {
        return Err(err_bundle_shape());
    }

    let Some(sess) = obj.get("session").and_then(Value::as_object) else {
        return Err(err_bundle_shape());
    };
    if !check_exact_keys(
        sess,
        &[
            "subject_ref",
            "identifier_ref",
            "identifier_digest",
            "key_ref",
            "public_key_digest",
            "key_representation_profile",
        ],
    ) || !sess
        .get("subject_ref")
        .and_then(Value::as_str)
        .is_some_and(is_local_id)
        || !sess
            .get("identifier_ref")
            .and_then(Value::as_str)
            .is_some_and(is_local_id)
        || !sess
            .get("identifier_digest")
            .and_then(Value::as_str)
            .is_some_and(is_sha256_digest)
        || !sess
            .get("key_ref")
            .and_then(Value::as_str)
            .is_some_and(is_local_id)
        || !sess
            .get("public_key_digest")
            .and_then(Value::as_str)
            .is_some_and(is_sha256_digest)
        || !sess
            .get("key_representation_profile")
            .is_some_and(is_profile_ref_obj)
    {
        return Err(err_bundle_shape());
    }

    let Some(launch) = obj.get("launch").and_then(Value::as_object) else {
        return Err(err_bundle_shape());
    };
    if !check_exact_keys(
        launch,
        &[
            "receipt_id",
            "issuer",
            "intent_digest",
            "process_digest",
            "session_digest",
            "issued_at",
            "expires_at",
        ],
    ) || !launch
        .get("receipt_id")
        .and_then(Value::as_str)
        .is_some_and(is_local_id)
        || !launch.get("issuer").is_some_and(is_principal_obj)
        || !["intent_digest", "process_digest", "session_digest"]
            .iter()
            .all(|k| {
                launch
                    .get(*k)
                    .and_then(Value::as_str)
                    .is_some_and(is_sha256_digest)
            })
        || !["issued_at", "expires_at"].iter().all(|k| {
            launch
                .get(*k)
                .and_then(Value::as_str)
                .is_some_and(|s| parse_strict_utc_unix(s).is_some())
        })
    {
        return Err(err_bundle_shape());
    }

    let Some(challenge) = obj.get("challenge").and_then(Value::as_object) else {
        return Err(err_bundle_shape());
    };
    if !check_exact_keys(
        challenge,
        &[
            "nonce",
            "audience",
            "receipt_digest",
            "issued_at",
            "expires_at",
        ],
    ) || !challenge
        .get("nonce")
        .and_then(Value::as_str)
        .is_some_and(is_hex64)
        || !challenge
            .get("audience")
            .and_then(Value::as_str)
            .is_some_and(is_nonempty_512)
        || !challenge
            .get("receipt_digest")
            .and_then(Value::as_str)
            .is_some_and(is_sha256_digest)
        || !["issued_at", "expires_at"].iter().all(|k| {
            challenge
                .get(*k)
                .and_then(Value::as_str)
                .is_some_and(|s| parse_strict_utc_unix(s).is_some())
        })
    {
        return Err(err_bundle_shape());
    }

    let Some(proof) = obj.get("proof").and_then(Value::as_object) else {
        return Err(err_bundle_shape());
    };
    if !check_exact_keys(proof, &["key_ref", "transcript_digest", "proof_digest"])
        || !proof
            .get("key_ref")
            .and_then(Value::as_str)
            .is_some_and(is_local_id)
        || !proof
            .get("transcript_digest")
            .and_then(Value::as_str)
            .is_some_and(is_sha256_digest)
        || !proof
            .get("proof_digest")
            .and_then(Value::as_str)
            .is_some_and(is_sha256_digest)
    {
        return Err(err_bundle_shape());
    }

    Ok(())
}

/// Evaluate `local-process-session:v0` contract fit over a validated report,
/// evidence bundle, and caller-supplied trusted evaluation context.
pub fn process_session_fit(
    report: &Value,
    bundle: &Value,
    context: &Value,
    accepted_profile: &ProfileRef,
) -> Result<(), EvidenceError> {
    let req = |ok: bool, code: &str| -> Result<(), EvidenceError> {
        if ok {
            Ok(())
        } else {
            Err(EvidenceError::new(code))
        }
    };
    let expected_prof = json!({
        "id": accepted_profile.id,
        "revision": accepted_profile.revision,
    });
    req(
        bundle.get("profile") == Some(&expected_prof),
        "unsupported-evidence-profile",
    )?;
    validate_report_shape(report)?;
    validate_session_bundle_shape(bundle)?;

    let process = &bundle["process"];
    let session = &bundle["session"];
    let receipt = &bundle["launch"];
    let challenge = &bundle["challenge"];
    let proof = &bundle["proof"];

    let now = context
        .get("now")
        .and_then(Value::as_str)
        .and_then(parse_strict_utc_unix)
        .ok_or_else(|| EvidenceError::new("trusted-clock-unavailable"))?;

    for (window, max_sec) in [
        (receipt, MAX_RECEIPT_SECONDS),
        (challenge, MAX_CHALLENGE_SECONDS),
    ] {
        let issued = window
            .get("issued_at")
            .and_then(Value::as_str)
            .and_then(parse_strict_utc_unix)
            .ok_or_else(|| EvidenceError::new("expired-or-invalid-window"))?;
        let expires = window
            .get("expires_at")
            .and_then(Value::as_str)
            .and_then(parse_strict_utc_unix)
            .ok_or_else(|| EvidenceError::new("expired-or-invalid-window"))?;
        req(
            issued < expires
                && (expires - issued) <= max_sec
                && (issued - now) <= MAX_FUTURE_SKEW_SECONDS
                && now < expires,
            "expired-or-invalid-window",
        )?;
    }

    req(
        context.get("process_live") == Some(&Value::Bool(true)),
        "process-not-live",
    )?;
    let observed_at = context
        .get("observed_at")
        .and_then(Value::as_str)
        .and_then(parse_strict_utc_unix)
        .ok_or_else(|| EvidenceError::new("stale-process-observation"))?;
    let obs_age = now - observed_at;
    req(
        (0..=MAX_OBSERVATION_AGE_SECONDS).contains(&obs_age),
        "stale-process-observation",
    )?;
    req(
        context
            .get("current_process")
            .is_some_and(|cp| json_equal(process, cp)),
        "process-incarnation-or-context-changed",
    )?;
    req(
        context
            .get("expected_session")
            .is_some_and(|es| json_equal(session, es)),
        "session-key-or-enrollment-changed",
    )?;

    let subjects: HashMap<&str, &Value> = report["subjects"]
        .as_array()
        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?
        .iter()
        .filter_map(|s| Some((s.get("id")?.as_str()?, s)))
        .collect();
    let proc_subj = process["subject_ref"].as_str().unwrap_or("");
    let sess_subj = session["subject_ref"].as_str().unwrap_or("");
    req(
        subjects
            .get(proc_subj)
            .and_then(|s| s.get("kind"))
            .and_then(Value::as_str)
            == Some("process")
            && subjects
                .get(sess_subj)
                .and_then(|s| s.get("kind"))
                .and_then(Value::as_str)
                == Some("session"),
        "process-session-subject-mismatch",
    )?;

    let sess_key_ref = session["key_ref"].as_str().unwrap_or("");
    let sess_ident_ref = session["identifier_ref"].as_str().unwrap_or("");
    let key = report["keys"].as_array().and_then(|arr| {
        arr.iter()
            .find(|x| x.get("id").and_then(Value::as_str) == Some(sess_key_ref))
    });
    let identifier = report["identifiers"].as_array().and_then(|arr| {
        arr.iter()
            .find(|x| x.get("id").and_then(Value::as_str) == Some(sess_ident_ref))
    });
    let ident_digest = match identifier {
        Some(ident) => Some(canonical_sha256_value(ident).map_err(|w| EvidenceError::new(w.code))?),
        None => None,
    };
    req(
        key.is_some_and(|k| {
            k.get("subject_ref") == session.get("subject_ref")
                && k.get("public_key_digest") == session.get("public_key_digest")
                && k.get("representation_profile") == session.get("key_representation_profile")
        }) && identifier.is_some_and(|i| i.get("subject_ref") == session.get("subject_ref"))
            && ident_digest.as_deref() == session.get("identifier_digest").and_then(Value::as_str),
        "report-session-key-mismatch",
    )?;
    let Some(key) = key else {
        return Err(EvidenceError::new("report-session-key-mismatch"));
    };

    let proc_digest = canonical_sha256_value(process).map_err(|w| EvidenceError::new(w.code))?;
    let sess_digest = canonical_sha256_value(session).map_err(|w| EvidenceError::new(w.code))?;
    req(
        receipt.get("process_digest").and_then(Value::as_str) == Some(proc_digest.as_str())
            && receipt.get("session_digest").and_then(Value::as_str) == Some(sess_digest.as_str()),
        "receipt-binding-mismatch",
    )?;
    req(
        context
            .get("accepted_launch_issuer")
            .is_some_and(|iss| json_equal(&receipt["issuer"], iss))
            && receipt.get("intent_digest") == context.get("expected_intent_digest"),
        "launch-intent-or-issuer-mismatch",
    )?;

    let consumed_receipts = context
        .get("consumed_receipts")
        .and_then(Value::as_array)
        .ok_or_else(|| EvidenceError::new("launch-receipt-replayed"))?;
    for item in consumed_receipts {
        let valid_entry = item.as_object().is_some_and(|m| {
            check_exact_keys(m, &["issuer", "receipt_id"])
                && is_principal_obj(&m["issuer"])
                && m["receipt_id"].as_str().is_some_and(is_local_id)
        });
        req(valid_entry, "launch-receipt-replayed")?;
        req(
            !(json_equal(&item["issuer"], &receipt["issuer"])
                && item.get("receipt_id") == receipt.get("receipt_id")),
            "launch-receipt-replayed",
        )?;
    }
    let consumed_challenges = context
        .get("consumed_challenges")
        .and_then(Value::as_array)
        .ok_or_else(|| EvidenceError::new("launch-challenge-replayed"))?;
    for n in consumed_challenges {
        req(
            n.as_str().is_some_and(is_hex64),
            "launch-challenge-replayed",
        )?;
        req(
            Some(n) != challenge.get("nonce"),
            "launch-challenge-replayed",
        )?;
    }

    let receipt_digest = canonical_sha256_value(receipt).map_err(|w| EvidenceError::new(w.code))?;
    req(
        context
            .get("verified_receipt_digest")
            .and_then(Value::as_str)
            == Some(receipt_digest.as_str()),
        "receipt-verification-binding-mismatch",
    )?;
    req(
        challenge.get("receipt_digest").and_then(Value::as_str) == Some(receipt_digest.as_str()),
        "challenge-receipt-mismatch",
    )?;
    req(
        challenge.get("audience") == context.get("audience")
            && challenge.get("nonce") == context.get("nonce"),
        "challenge-audience-or-nonce-mismatch",
    )?;

    let transcript = json!({
        "domain": LOCAL_PROCESS_SESSION_TRANSCRIPT_DOMAIN,
        "profile": expected_prof,
        "process_digest": proc_digest,
        "session_digest": sess_digest,
        "receipt_digest": receipt_digest,
        "challenge": challenge,
    });
    let transcript_digest =
        canonical_sha256_value(&transcript).map_err(|w| EvidenceError::new(w.code))?;
    req(
        proof.get("key_ref") == session.get("key_ref")
            && proof.get("transcript_digest").and_then(Value::as_str)
                == Some(transcript_digest.as_str())
            && context
                .get("verified_transcript_digest")
                .and_then(Value::as_str)
                == Some(transcript_digest.as_str())
            && context.get("verified_key_digest") == key.get("public_key_digest")
            && context.get("verified_proof_digest") == proof.get("proof_digest"),
        "possession-verification-binding-mismatch",
    )?;

    let claims: HashMap<&str, &Value> = report["claims"]
        .as_array()
        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?
        .iter()
        .filter_map(|c| Some((c.get("id")?.as_str()?, c)))
        .collect();
    let bundle_digest = canonical_sha256_value(bundle).map_err(|w| EvidenceError::new(w.code))?;

    let launch_claim_matches = |claim: &Value| -> bool {
        claim.get("subject_ref") == process.get("subject_ref")
            && claim.get("property_profile") == Some(&expected_prof)
            && claim.get("property").and_then(Value::as_str) == Some("launch-bound-session")
            && claim.get("value") == session.get("subject_ref")
            && claim.get("key_ref").is_none()
    };

    let mut bound: HashMap<&str, &Value> = HashMap::new();
    let evidence_arr = report["evidence"]
        .as_array()
        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
    for e in evidence_arr {
        let fp_matches = e.get("format_profile") == Some(&expected_prof);
        let dig_matches = e.get("digest").and_then(Value::as_str) == Some(bundle_digest.as_str());
        if !fp_matches && !dig_matches {
            continue;
        }
        req(
            fp_matches && dig_matches,
            "report-evidence-binding-mismatch",
        )?;
        match e.get("method").and_then(Value::as_str) {
            Some("possession-proof") => {
                req(
                    e.get("subject_ref") == session.get("subject_ref")
                        && e.get("identifier_ref") == session.get("identifier_ref")
                        && e.get("key_ref") == session.get("key_ref")
                        && e.get("claim_refs")
                            .and_then(Value::as_array)
                            .is_none_or(Vec::is_empty)
                        && e.get("boundary").is_none(),
                    "report-possession-descriptor-mismatch",
                )?;
            }
            Some("os-observation") => {
                req(
                    e.get("subject_ref") == process.get("subject_ref")
                        && e.get("identifier_ref").is_none()
                        && e.get("key_ref").is_none()
                        && e.get("boundary").is_none(),
                    "report-process-descriptor-mismatch",
                )?;
                let crs = e.get("claim_refs").and_then(Value::as_array);
                req(
                    crs.is_some_and(|arr| {
                        !arr.is_empty()
                            && arr.iter().all(|cr| {
                                cr.as_str()
                                    .and_then(|id| claims.get(id))
                                    .is_some_and(|c| launch_claim_matches(c))
                            })
                    }),
                    "process-session-claim-mismatch",
                )?;
            }
            _ => return Err(EvidenceError::new("report-evidence-method-mismatch")),
        }
        let eid = e["id"]
            .as_str()
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        bound.insert(eid, e);
    }

    let assessments = report["assessments"]
        .as_array()
        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
    let mut used_methods: HashSet<&str> = HashSet::new();
    for a in assessments {
        let selected: Vec<&Value> = a["evidence_refs"]
            .as_array()
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?
            .iter()
            .filter_map(|er| er.as_str().and_then(|id| bound.get(id).copied()))
            .collect();
        let selected_claims: Vec<&Value> = a
            .get("claim_refs")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|cr| cr.as_str().and_then(|id| claims.get(id).copied()))
                    .collect()
            })
            .unwrap_or_default();
        if selected.is_empty() {
            req(
                a.get("rule") != Some(&expected_prof)
                    && !selected_claims
                        .iter()
                        .any(|c| c.get("property_profile") == Some(&expected_prof)),
                "assessment-bundle-evidence-required",
            )?;
            continue;
        }
        for e in selected {
            let method = e["method"]
                .as_str()
                .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
            if method == "possession-proof" {
                req(
                    a.get("dimension").and_then(Value::as_str) == Some("identity-binding")
                        && a.get("value").and_then(Value::as_str) == Some("enrolled-key")
                        && a.get("subject_ref") == session.get("subject_ref")
                        && a.get("identifier_ref") == session.get("identifier_ref")
                        && a.get("key_ref") == session.get("key_ref"),
                    "assessment-possession-binding-mismatch",
                )?;
            } else {
                req(
                    a.get("dimension").and_then(Value::as_str) == Some("platform-evidence")
                        && a.get("value").and_then(Value::as_str) == Some("locally-observed")
                        && a.get("rule") == Some(&expected_prof)
                        && a.get("subject_ref") == process.get("subject_ref")
                        && a.get("identifier_ref").is_none()
                        && a.get("key_ref").is_none(),
                    "assessment-process-binding-mismatch",
                )?;
                let e_claims: HashSet<&str> = e["claim_refs"]
                    .as_array()
                    .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                let a_claims: HashSet<&str> = a
                    .get("claim_refs")
                    .and_then(Value::as_array)
                    .map(|arr| arr.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                req(
                    !selected_claims.is_empty()
                        && selected_claims.iter().all(|c| launch_claim_matches(c))
                        && a_claims.is_subset(&e_claims),
                    "assessment-process-claim-mismatch",
                )?;
            }
            used_methods.insert(method);
        }
    }
    req(
        used_methods.len() == 2
            && used_methods.contains("possession-proof")
            && used_methods.contains("os-observation"),
        "process-session-assessments-required",
    )?;

    let receipt_exp = receipt["expires_at"]
        .as_str()
        .and_then(parse_strict_utc_unix)
        .ok_or_else(|| EvidenceError::new("invalid-session-bundle-shape"))?;
    let challenge_exp = challenge["expires_at"]
        .as_str()
        .and_then(parse_strict_utc_unix)
        .ok_or_else(|| EvidenceError::new("invalid-session-bundle-shape"))?;
    let expiry = receipt_exp.min(challenge_exp);
    req(
        assessments.iter().all(|a| {
            a["valid_until"]
                .as_str()
                .and_then(parse_strict_utc_unix)
                .is_some_and(|vu| vu <= expiry)
        }),
        "assessment-outlives-binding",
    )?;
    req(
        assessments.iter().all(|a| {
            a["valid_until"]
                .as_str()
                .and_then(parse_strict_utc_unix)
                .is_some_and(|vu| now < vu)
        }),
        "assessment-expired",
    )?;
    req(
        assessments.iter().all(|a| {
            a.get("state").and_then(Value::as_str) == Some("current")
                && a["assessed_at"]
                    .as_str()
                    .and_then(parse_strict_utc_unix)
                    .is_some_and(|at| at <= now)
        }),
        "assessment-not-current",
    )?;

    Ok(())
}
