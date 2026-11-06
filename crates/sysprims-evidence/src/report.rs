// SPDX-License-Identifier: MIT OR Apache-2.0

//! `identity-evidence/v0` report shape validation, necessary reference/time
//! conditions, multi-report import collision detection, and provenance depth
//! bounds.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde_json::Value;

use crate::wire::{admit_report_bytes, canonical_sha256_value, AdmittedWire};
use crate::{
    is_local_id, is_nonempty_512, is_principal_obj, is_profile_ref_obj, is_sha256_digest,
    json_equal, parse_strict_utc_unix, EvidenceError,
};

/// Closed contract identifier for `identity-evidence/v0` reports.
pub const IDENTITY_EVIDENCE_CONTRACT: &str = "identity-evidence/v0";

/// Maximum transitive `input_reports` provenance depth (`local-process-session` & `native-process-observation`).
pub const MAX_PROVENANCE_DEPTH: usize = 4;

const SUBJECT_KINDS: &[&str] = &[
    "physical-device",
    "os-installation",
    "virtual-machine",
    "container",
    "process",
    "session",
    "key-holder",
];

const STABILITIES: &[&str] = &[
    "unknown",
    "process",
    "boot",
    "installation",
    "enrollment",
    "device",
];

const CLONE_BEHAVIORS: &[&str] = &["unknown", "copyable", "regenerated", "device-bound"];

const EVIDENCE_METHODS: &[&str] = &[
    "declaration",
    "os-observation",
    "enrollment",
    "possession-proof",
    "device-attestation",
    "workload-attestation",
    "authority-statement",
    "custody-observation",
    "isolation-observation",
];

const DIMENSIONS: &[&str] = &[
    "identity-binding",
    "key-custody",
    "execution-isolation",
    "platform-evidence",
];

const IDENTITY_BINDING_VALUES: &[&str] = &[
    "unknown",
    "declared",
    "observed",
    "enrolled-key",
    "attested-key",
];

const KEY_CUSTODY_VALUES: &[&str] = &[
    "unknown",
    "not-applicable",
    "software",
    "os-managed",
    "hardware-backed",
    "virtual-hardware-backed",
];

const EXECUTION_ISOLATION_VALUES: &[&str] = &[
    "unknown",
    "shared-account",
    "separate-account",
    "os-confined",
    "virtual-machine",
    "confidential-workload",
];

const PLATFORM_EVIDENCE_VALUES: &[&str] = &[
    "unknown",
    "declared",
    "locally-observed",
    "authority-verified",
    "hardware-attested",
];

const ASSESSMENT_STATES: &[&str] = &["current", "stale", "revoked", "conflicting", "unknown"];

const CAPABILITY_STATUSES: &[&str] = &[
    "available",
    "not-present",
    "inaccessible",
    "unsupported",
    "not-probed",
    "error",
    "unknown",
];

fn err_shape() -> EvidenceError {
    EvidenceError::new("invalid-report-shape")
}

fn check_allowed_keys(
    obj: &serde_json::Map<String, Value>,
    required: &[&str],
    optional: &[&str],
) -> bool {
    for &req in required {
        if !obj.contains_key(req) {
            return false;
        }
    }
    obj.keys()
        .all(|k| required.contains(&k.as_str()) || optional.contains(&k.as_str()))
}

fn validate_unique_local_id_array(v: &Value, min_items: usize, max_items: usize) -> bool {
    let Some(arr) = v.as_array() else {
        return false;
    };
    if arr.len() < min_items || arr.len() > max_items {
        return false;
    }
    let mut seen = HashSet::with_capacity(arr.len());
    for item in arr {
        let Some(s) = item.as_str() else {
            return false;
        };
        if !is_local_id(s) || !seen.insert(s) {
            return false;
        }
    }
    true
}

fn validate_unique_string_array(v: &Value, min_items: usize, max_items: usize) -> bool {
    let Some(arr) = v.as_array() else {
        return false;
    };
    if arr.len() < min_items || arr.len() > max_items {
        return false;
    }
    let mut seen = HashSet::with_capacity(arr.len());
    for item in arr {
        let Some(s) = item.as_str() else {
            return false;
        };
        if !is_nonempty_512(s) || !seen.insert(s) {
            return false;
        }
    }
    true
}

fn validate_software_obj(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    if !check_allowed_keys(obj, &["name", "revision"], &[]) {
        return false;
    }
    obj.get("name")
        .and_then(Value::as_str)
        .is_some_and(is_nonempty_512)
        && obj
            .get("revision")
            .and_then(Value::as_str)
            .is_some_and(is_sha256_digest)
}

fn validate_origin_obj(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    if !check_allowed_keys(obj, &["input_ref", "record_ref"], &[]) {
        return false;
    }
    obj.get("input_ref")
        .and_then(Value::as_str)
        .is_some_and(is_local_id)
        && obj
            .get("record_ref")
            .and_then(Value::as_str)
            .is_some_and(is_local_id)
}

fn validate_boundary_obj(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    if !check_allowed_keys(
        obj,
        &["profile", "comparison_subject_refs", "threats"],
        &["external_boundary"],
    ) {
        return false;
    }
    if !obj.get("profile").is_some_and(is_profile_ref_obj) {
        return false;
    }
    if !obj
        .get("comparison_subject_refs")
        .is_some_and(|x| validate_unique_local_id_array(x, 0, 256))
    {
        return false;
    }
    if !obj
        .get("threats")
        .is_some_and(|x| validate_unique_string_array(x, 1, 256))
    {
        return false;
    }
    if let Some(ext) = obj.get("external_boundary") {
        if !ext.as_str().is_some_and(is_nonempty_512) {
            return false;
        }
    }
    let comp_nonempty = obj["comparison_subject_refs"]
        .as_array()
        .is_some_and(|a| !a.is_empty());
    comp_nonempty || obj.contains_key("external_boundary")
}

fn validate_time_attribution_obj(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    if !check_allowed_keys(obj, &["instant", "source"], &["evidence"]) {
        return false;
    }
    if !obj
        .get("instant")
        .and_then(Value::as_str)
        .is_some_and(|s| parse_strict_utc_unix(s).is_some())
    {
        return false;
    }
    let Some(source) = obj.get("source").and_then(Value::as_str) else {
        return false;
    };
    match source {
        "producer_clock" => !obj.contains_key("evidence"),
        "trusted_time" => {
            let Some(ev) = obj.get("evidence").and_then(Value::as_object) else {
                return false;
            };
            if !check_allowed_keys(ev, &["format_profile", "digest"], &["ref"]) {
                return false;
            }
            if !ev.get("format_profile").is_some_and(is_profile_ref_obj)
                || !ev
                    .get("digest")
                    .and_then(Value::as_str)
                    .is_some_and(is_sha256_digest)
            {
                return false;
            }
            if let Some(r) = ev.get("ref") {
                if !r.as_str().is_some_and(is_nonempty_512) {
                    return false;
                }
            }
            true
        }
        _ => false,
    }
}

/// Validate strict `identity-evidence/v0` `report.schema.json` structural and
/// conditional rules.
pub fn validate_report_shape(report: &Value) -> Result<(), EvidenceError> {
    let Some(obj) = report.as_object() else {
        return Err(err_shape());
    };
    let required = &[
        "contract",
        "report_id",
        "created_at",
        "producer",
        "subjects",
        "identifiers",
        "evidence",
        "assessments",
        "capabilities",
        "keys",
        "claims",
        "input_reports",
    ];
    if !check_allowed_keys(obj, required, &["evaluator", "time_attribution"]) {
        return Err(err_shape());
    }
    if obj.get("contract").and_then(Value::as_str) != Some(IDENTITY_EVIDENCE_CONTRACT) {
        return Err(err_shape());
    }
    if !obj
        .get("report_id")
        .and_then(Value::as_str)
        .is_some_and(is_local_id)
    {
        return Err(err_shape());
    }
    if !obj
        .get("created_at")
        .and_then(Value::as_str)
        .is_some_and(|s| parse_strict_utc_unix(s).is_some())
    {
        return Err(err_shape());
    }
    if !obj.get("producer").is_some_and(validate_software_obj) {
        return Err(err_shape());
    }
    if let Some(ta) = obj.get("time_attribution") {
        if !validate_time_attribution_obj(ta) {
            return Err(err_shape());
        }
    }

    let Some(assessments) = obj.get("assessments").and_then(Value::as_array) else {
        return Err(err_shape());
    };
    if assessments.len() > 256 {
        return Err(err_shape());
    }
    if !assessments.is_empty() {
        let Some(eval) = obj.get("evaluator").and_then(Value::as_object) else {
            return Err(err_shape());
        };
        if !check_allowed_keys(
            eval,
            &["software", "principal", "verification_profile"],
            &[],
        ) || !eval.get("software").is_some_and(validate_software_obj)
            || !eval.get("principal").is_some_and(is_principal_obj)
            || !eval
                .get("verification_profile")
                .is_some_and(is_profile_ref_obj)
        {
            return Err(err_shape());
        }
    } else if obj.contains_key("evaluator") {
        return Err(err_shape());
    }

    let Some(subjects) = obj.get("subjects").and_then(Value::as_array) else {
        return Err(err_shape());
    };
    if subjects.is_empty() || subjects.len() > 256 {
        return Err(err_shape());
    }
    for s in subjects {
        let Some(so) = s.as_object() else {
            return Err(err_shape());
        };
        if !check_allowed_keys(so, &["id", "kind"], &["label"])
            || !so
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(is_local_id)
            || !so
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|k| SUBJECT_KINDS.contains(&k))
        {
            return Err(err_shape());
        }
        if let Some(lbl) = so.get("label") {
            if !lbl.as_str().is_some_and(is_nonempty_512) {
                return Err(err_shape());
            }
        }
    }

    let Some(identifiers) = obj.get("identifiers").and_then(Value::as_array) else {
        return Err(err_shape());
    };
    if identifiers.len() > 256 {
        return Err(err_shape());
    }
    for ident in identifiers {
        let Some(io) = ident.as_object() else {
            return Err(err_shape());
        };
        if !check_allowed_keys(
            io,
            &[
                "id",
                "subject_ref",
                "namespace",
                "value",
                "derivation",
                "stability",
                "clone_behavior",
            ],
            &[],
        ) {
            return Err(err_shape());
        }
        if !io
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(is_local_id)
            || !io
                .get("subject_ref")
                .and_then(Value::as_str)
                .is_some_and(is_local_id)
            || !io
                .get("namespace")
                .and_then(Value::as_str)
                .is_some_and(is_nonempty_512)
            || !io
                .get("value")
                .and_then(Value::as_str)
                .is_some_and(is_nonempty_512)
            || !io.get("derivation").is_some_and(is_profile_ref_obj)
            || !io
                .get("stability")
                .and_then(Value::as_str)
                .is_some_and(|s| STABILITIES.contains(&s))
            || !io
                .get("clone_behavior")
                .and_then(Value::as_str)
                .is_some_and(|c| CLONE_BEHAVIORS.contains(&c))
        {
            return Err(err_shape());
        }
    }

    let Some(evidence) = obj.get("evidence").and_then(Value::as_array) else {
        return Err(err_shape());
    };
    if evidence.len() > 256 {
        return Err(err_shape());
    }
    for ev in evidence {
        let Some(eo) = ev.as_object() else {
            return Err(err_shape());
        };
        if !check_allowed_keys(
            eo,
            &[
                "id",
                "subject_ref",
                "method",
                "source",
                "collector",
                "collected_at",
                "digest",
                "format_profile",
            ],
            &[
                "identifier_ref",
                "key_ref",
                "claim_refs",
                "boundary",
                "origin",
                "time_attribution",
            ],
        ) {
            return Err(err_shape());
        }
        if !eo
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(is_local_id)
            || !eo
                .get("subject_ref")
                .and_then(Value::as_str)
                .is_some_and(is_local_id)
            || !eo
                .get("method")
                .and_then(Value::as_str)
                .is_some_and(|m| EVIDENCE_METHODS.contains(&m))
            || !eo
                .get("source")
                .and_then(Value::as_str)
                .is_some_and(is_nonempty_512)
            || !eo
                .get("collector")
                .and_then(Value::as_str)
                .is_some_and(is_nonempty_512)
            || !eo
                .get("collected_at")
                .and_then(Value::as_str)
                .is_some_and(|s| parse_strict_utc_unix(s).is_some())
            || !eo
                .get("digest")
                .and_then(Value::as_str)
                .is_some_and(is_sha256_digest)
            || !eo.get("format_profile").is_some_and(is_profile_ref_obj)
        {
            return Err(err_shape());
        }
        if let Some(ir) = eo.get("identifier_ref") {
            if !ir.as_str().is_some_and(is_local_id) {
                return Err(err_shape());
            }
        }
        if let Some(kr) = eo.get("key_ref") {
            if !kr.as_str().is_some_and(is_local_id) {
                return Err(err_shape());
            }
        }
        if let Some(cr) = eo.get("claim_refs") {
            if !validate_unique_local_id_array(cr, 1, 256) {
                return Err(err_shape());
            }
        }
        if let Some(b) = eo.get("boundary") {
            if !validate_boundary_obj(b) {
                return Err(err_shape());
            }
        }
        if let Some(o) = eo.get("origin") {
            if !validate_origin_obj(o) {
                return Err(err_shape());
            }
        }
        if let Some(ta) = eo.get("time_attribution") {
            if !validate_time_attribution_obj(ta) {
                return Err(err_shape());
            }
        }
    }

    for a in assessments {
        let Some(ao) = a.as_object() else {
            return Err(err_shape());
        };
        if !check_allowed_keys(
            ao,
            &[
                "subject_ref",
                "scope",
                "dimension",
                "value",
                "state",
                "rule",
                "assessed_at",
                "valid_until",
                "evidence_refs",
                "residuals",
            ],
            &[
                "identifier_ref",
                "key_ref",
                "claim_refs",
                "boundary",
                "time_attribution",
            ],
        ) {
            return Err(err_shape());
        }
        let Some(dim) = ao.get("dimension").and_then(Value::as_str) else {
            return Err(err_shape());
        };
        let Some(val) = ao.get("value").and_then(Value::as_str) else {
            return Err(err_shape());
        };
        let Some(state) = ao.get("state").and_then(Value::as_str) else {
            return Err(err_shape());
        };
        if !DIMENSIONS.contains(&dim)
            || !ASSESSMENT_STATES.contains(&state)
            || !ao
                .get("subject_ref")
                .and_then(Value::as_str)
                .is_some_and(is_local_id)
            || !ao
                .get("scope")
                .and_then(Value::as_str)
                .is_some_and(is_nonempty_512)
            || !ao.get("rule").is_some_and(is_profile_ref_obj)
            || !ao
                .get("assessed_at")
                .and_then(Value::as_str)
                .is_some_and(|s| parse_strict_utc_unix(s).is_some())
            || !ao
                .get("valid_until")
                .and_then(Value::as_str)
                .is_some_and(|s| parse_strict_utc_unix(s).is_some())
            || !ao
                .get("evidence_refs")
                .is_some_and(|x| validate_unique_local_id_array(x, 0, 256))
        {
            return Err(err_shape());
        }
        let Some(residuals) = ao.get("residuals").and_then(Value::as_array) else {
            return Err(err_shape());
        };
        if residuals.len() > 256
            || !residuals
                .iter()
                .all(|r| r.as_str().is_some_and(is_nonempty_512))
        {
            return Err(err_shape());
        }
        if let Some(ir) = ao.get("identifier_ref") {
            if !ir.as_str().is_some_and(is_local_id) {
                return Err(err_shape());
            }
        }
        if let Some(kr) = ao.get("key_ref") {
            if !kr.as_str().is_some_and(is_local_id) {
                return Err(err_shape());
            }
        }
        if let Some(cr) = ao.get("claim_refs") {
            if !validate_unique_local_id_array(cr, 1, 256) {
                return Err(err_shape());
            }
        }
        if let Some(b) = ao.get("boundary") {
            if !validate_boundary_obj(b) {
                return Err(err_shape());
            }
        }
        if let Some(ta) = ao.get("time_attribution") {
            if !validate_time_attribution_obj(ta) {
                return Err(err_shape());
            }
        }

        let dim_ok = match dim {
            "identity-binding" => IDENTITY_BINDING_VALUES.contains(&val),
            "key-custody" => KEY_CUSTODY_VALUES.contains(&val),
            "execution-isolation" => EXECUTION_ISOLATION_VALUES.contains(&val),
            "platform-evidence" => PLATFORM_EVIDENCE_VALUES.contains(&val),
            _ => false,
        };
        if !dim_ok {
            return Err(err_shape());
        }
        if val == "unknown" && state != "unknown" {
            return Err(err_shape());
        }
        if val != "unknown"
            && val != "not-applicable"
            && ao["evidence_refs"].as_array().is_none_or(Vec::is_empty)
        {
            return Err(err_shape());
        }
        if dim == "identity-binding" && val != "unknown" && !ao.contains_key("identifier_ref") {
            return Err(err_shape());
        }
        if dim == "key-custody"
            && val != "unknown"
            && val != "not-applicable"
            && !ao.contains_key("key_ref")
        {
            return Err(err_shape());
        }
        if dim == "identity-binding"
            && (val == "enrolled-key" || val == "attested-key")
            && !ao.contains_key("key_ref")
        {
            return Err(err_shape());
        }
        if (dim == "execution-isolation") != ao.contains_key("boundary") {
            return Err(err_shape());
        }
        if (dim == "platform-evidence") != ao.contains_key("claim_refs") {
            return Err(err_shape());
        }
        if dim == "key-custody" && val == "not-applicable" && ao.contains_key("key_ref") {
            return Err(err_shape());
        }
    }

    let Some(capabilities) = obj.get("capabilities").and_then(Value::as_array) else {
        return Err(err_shape());
    };
    if capabilities.len() > 256 {
        return Err(err_shape());
    }
    for c in capabilities {
        let Some(co) = c.as_object() else {
            return Err(err_shape());
        };
        if !check_allowed_keys(
            co,
            &[
                "id",
                "subject_ref",
                "provider",
                "capability",
                "status",
                "reason",
                "observed_at",
            ],
            &["origin", "time_attribution"],
        ) {
            return Err(err_shape());
        }
        if !co
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(is_local_id)
            || !co
                .get("subject_ref")
                .and_then(Value::as_str)
                .is_some_and(is_local_id)
            || !co
                .get("provider")
                .and_then(Value::as_str)
                .is_some_and(is_nonempty_512)
            || !co
                .get("capability")
                .and_then(Value::as_str)
                .is_some_and(is_nonempty_512)
            || !co
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|s| CAPABILITY_STATUSES.contains(&s))
            || !co
                .get("reason")
                .and_then(Value::as_str)
                .is_some_and(is_nonempty_512)
            || !co
                .get("observed_at")
                .and_then(Value::as_str)
                .is_some_and(|s| parse_strict_utc_unix(s).is_some())
        {
            return Err(err_shape());
        }
        if let Some(o) = co.get("origin") {
            if !validate_origin_obj(o) {
                return Err(err_shape());
            }
        }
        if let Some(ta) = co.get("time_attribution") {
            if !validate_time_attribution_obj(ta) {
                return Err(err_shape());
            }
        }
    }

    let Some(keys) = obj.get("keys").and_then(Value::as_array) else {
        return Err(err_shape());
    };
    if keys.len() > 256 {
        return Err(err_shape());
    }
    for k in keys {
        let Some(ko) = k.as_object() else {
            return Err(err_shape());
        };
        if !check_allowed_keys(
            ko,
            &[
                "id",
                "subject_ref",
                "representation_profile",
                "public_key_digest",
            ],
            &[],
        ) || !ko
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(is_local_id)
            || !ko
                .get("subject_ref")
                .and_then(Value::as_str)
                .is_some_and(is_local_id)
            || !ko
                .get("representation_profile")
                .is_some_and(is_profile_ref_obj)
            || !ko
                .get("public_key_digest")
                .and_then(Value::as_str)
                .is_some_and(is_sha256_digest)
        {
            return Err(err_shape());
        }
    }

    let Some(claims) = obj.get("claims").and_then(Value::as_array) else {
        return Err(err_shape());
    };
    if claims.len() > 256 {
        return Err(err_shape());
    }
    for cl in claims {
        let Some(co) = cl.as_object() else {
            return Err(err_shape());
        };
        if !check_allowed_keys(
            co,
            &["id", "subject_ref", "property_profile", "property", "value"],
            &["key_ref"],
        ) || !co
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(is_local_id)
            || !co
                .get("subject_ref")
                .and_then(Value::as_str)
                .is_some_and(is_local_id)
            || !co.get("property_profile").is_some_and(is_profile_ref_obj)
            || !co
                .get("property")
                .and_then(Value::as_str)
                .is_some_and(is_nonempty_512)
        {
            return Err(err_shape());
        }
        match co.get("value") {
            Some(Value::String(s)) => {
                if s.chars().count() > 512 {
                    return Err(err_shape());
                }
            }
            Some(Value::Bool(_) | Value::Null) => {}
            Some(Value::Number(n)) => {
                if n.as_f64().is_none_or(|f| !f.is_finite()) {
                    return Err(err_shape());
                }
            }
            _ => return Err(err_shape()),
        }
        if let Some(kr) = co.get("key_ref") {
            if !kr.as_str().is_some_and(is_local_id) {
                return Err(err_shape());
            }
        }
    }

    let Some(input_reports) = obj.get("input_reports").and_then(Value::as_array) else {
        return Err(err_shape());
    };
    if input_reports.len() > 256 {
        return Err(err_shape());
    }
    for ir in input_reports {
        let Some(io) = ir.as_object() else {
            return Err(err_shape());
        };
        if !check_allowed_keys(io, &["id", "report_id", "digest"], &[])
            || !io
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(is_local_id)
            || !io
                .get("report_id")
                .and_then(Value::as_str)
                .is_some_and(is_local_id)
            || !io
                .get("digest")
                .and_then(Value::as_str)
                .is_some_and(is_sha256_digest)
        {
            return Err(err_shape());
        }
    }
    Ok(())
}

fn boundary_identity(boundary: Option<&Value>) -> Option<String> {
    let b = boundary?.as_object()?;
    let mut normalized = b.clone();
    for field in ["comparison_subject_refs", "threats"] {
        if let Some(arr) = normalized.get(field).and_then(Value::as_array) {
            let mut strs: Vec<String> = arr
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            strs.sort();
            normalized.insert(
                field.to_owned(),
                Value::Array(strs.into_iter().map(Value::String).collect()),
            );
        }
    }
    canonical_sha256_value(&Value::Object(normalized)).ok()
}

fn strip_origin(record: &Value) -> Value {
    let Some(obj) = record.as_object() else {
        return record.clone();
    };
    let mut copy = obj.clone();
    copy.remove("origin");
    Value::Object(copy)
}

/// Enforce `identity-evidence/v0` necessary reference, time, assessment-method,
/// and `input_reports` carry-forward rules.
pub fn necessary_conditions(
    report: &Value,
    input_reports: Option<&BTreeMap<String, Value>>,
) -> Result<(), EvidenceError> {
    validate_report_shape(report)?;
    let req = |ok: bool, code: &str| -> Result<(), EvidenceError> {
        if ok {
            Ok(())
        } else {
            Err(EvidenceError::new(code))
        }
    };
    let obj = report
        .as_object()
        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;

    let collections = [
        "subjects",
        "identifiers",
        "keys",
        "claims",
        "evidence",
        "capabilities",
        "input_reports",
    ];
    let mut indexes: HashMap<&str, HashMap<&str, &Value>> = HashMap::new();
    for coll in collections {
        let arr = obj
            .get(coll)
            .and_then(Value::as_array)
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        let mut map = HashMap::with_capacity(arr.len());
        for rec in arr {
            let id = rec
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
            req(map.insert(id, rec).is_none(), "duplicate-reference")?;
        }
        indexes.insert(coll, map);
    }

    let input_arr = obj
        .get("input_reports")
        .and_then(Value::as_array)
        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
    let mut seen_report_ids = HashSet::new();
    for r in input_arr {
        let rid = r
            .get("report_id")
            .and_then(Value::as_str)
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        req(seen_report_ids.insert(rid), "duplicate-reference")?;
    }

    if let Some(ta) = obj.get("time_attribution") {
        req(
            ta.get("instant") == obj.get("created_at"),
            "time-attribution-instant-mismatch",
        )?;
    }
    for (coll, field) in [
        ("evidence", "collected_at"),
        ("assessments", "assessed_at"),
        ("capabilities", "observed_at"),
    ] {
        let arr = obj
            .get(coll)
            .and_then(Value::as_array)
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        for rec in arr {
            if let Some(ta) = rec.get("time_attribution") {
                req(
                    ta.get("instant") == rec.get(field),
                    "time-attribution-instant-mismatch",
                )?;
            }
        }
    }

    let created = obj
        .get("created_at")
        .and_then(Value::as_str)
        .and_then(parse_strict_utc_unix)
        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;

    let assessments_arr = obj
        .get("assessments")
        .and_then(Value::as_array)
        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
    for a in assessments_arr {
        if a.get("dimension").and_then(Value::as_str) == Some("key-custody")
            && a.get("value").and_then(Value::as_str) == Some("not-applicable")
        {
            req(a.get("key_ref").is_none(), "not-applicable-custody-has-key")?;
        }
    }

    let subjects = &indexes["subjects"];
    let identifiers = &indexes["identifiers"];
    let keys = &indexes["keys"];
    let claims = &indexes["claims"];
    let evidence = &indexes["evidence"];

    for coll in [
        "identifiers",
        "keys",
        "claims",
        "evidence",
        "assessments",
        "capabilities",
    ] {
        let arr = obj
            .get(coll)
            .and_then(Value::as_array)
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        for r in arr {
            let subj = r
                .get("subject_ref")
                .and_then(Value::as_str)
                .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
            req(subjects.contains_key(subj), "unknown-subject")?;
            if let Some(ir) = r.get("identifier_ref").and_then(Value::as_str) {
                let item = identifiers.get(ir);
                req(
                    item.is_some_and(|x| {
                        x.get("subject_ref").and_then(Value::as_str) == Some(subj)
                    }),
                    "identifier_ref-subject-mismatch",
                )?;
            }
            if let Some(kr) = r.get("key_ref").and_then(Value::as_str) {
                let item = keys.get(kr);
                req(
                    item.is_some_and(|x| {
                        x.get("subject_ref").and_then(Value::as_str) == Some(subj)
                    }),
                    "key_ref-subject-mismatch",
                )?;
            }
            if let Some(crs) = r.get("claim_refs").and_then(Value::as_array) {
                for cr_v in crs {
                    let cr = cr_v
                        .as_str()
                        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
                    let Some(c) = claims.get(cr).copied() else {
                        return Err(EvidenceError::new("claim-subject-mismatch"));
                    };
                    req(
                        c.get("subject_ref").and_then(Value::as_str) == Some(subj),
                        "claim-subject-mismatch",
                    )?;
                    if let Some(ck) = c.get("key_ref") {
                        if coll == "evidence" || r.get("key_ref").is_some() {
                            req(Some(ck) == r.get("key_ref"), "claim-key-mismatch")?;
                        }
                    }
                }
            }
            if let Some(b) = r.get("boundary") {
                let comp = b
                    .get("comparison_subject_refs")
                    .and_then(Value::as_array)
                    .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
                req(
                    comp.iter().all(|x| {
                        x.as_str()
                            .is_some_and(|s| subjects.contains_key(s) && s != subj)
                    }),
                    "invalid-comparison-subject",
                )?;
            }
        }
    }

    let evidence_arr = obj
        .get("evidence")
        .and_then(Value::as_array)
        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
    for e in evidence_arr {
        let ts = e["collected_at"]
            .as_str()
            .and_then(parse_strict_utc_unix)
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        req(ts <= created, "future-evidence")?;
    }
    let caps_arr = obj
        .get("capabilities")
        .and_then(Value::as_array)
        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
    for c in caps_arr {
        let ts = c["observed_at"]
            .as_str()
            .and_then(parse_strict_utc_unix)
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        req(ts <= created, "future-capability")?;
    }

    let report_id = obj["report_id"]
        .as_str()
        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
    let mut resolved: HashMap<&str, &Value> = HashMap::new();
    for source in input_arr {
        let src_rid = source["report_id"]
            .as_str()
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        let original = input_reports
            .and_then(|m| m.get(src_rid))
            .ok_or_else(|| EvidenceError::new("input-report-unavailable"))?;
        req(
            original.get("report_id").and_then(Value::as_str) == Some(src_rid),
            "input-report-id-mismatch",
        )?;
        let actual_digest =
            canonical_sha256_value(original).map_err(|w| EvidenceError::new(w.code))?;
        req(
            Some(actual_digest.as_str()) == source.get("digest").and_then(Value::as_str),
            "input-report-digest-mismatch",
        )?;
        validate_report_shape(original).map_err(|_| EvidenceError::new("invalid-input-report"))?;
        let orig_created = original
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(parse_strict_utc_unix)
            .ok_or_else(|| EvidenceError::new("invalid-input-report"))?;
        req(
            src_rid != report_id && orig_created <= created,
            "invalid-input-report",
        )?;
        let local_id = source["id"]
            .as_str()
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        resolved.insert(local_id, original);
    }

    for coll in ["evidence", "capabilities"] {
        let arr = obj
            .get(coll)
            .and_then(Value::as_array)
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        for r in arr {
            let Some(origin) = r.get("origin") else {
                continue;
            };
            let input_ref = origin["input_ref"]
                .as_str()
                .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
            let record_ref = origin["record_ref"]
                .as_str()
                .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
            let source = resolved
                .get(input_ref)
                .copied()
                .ok_or_else(|| EvidenceError::new("unknown-input-report"))?;
            let old = source.get(coll).and_then(Value::as_array).and_then(|arr| {
                arr.iter()
                    .find(|x| x.get("id").and_then(Value::as_str) == Some(record_ref))
            });
            req(
                old.is_some_and(|o| json_equal(&strip_origin(r), &strip_origin(o))),
                "input-observation-changed",
            )?;
            let r_subj = r["subject_ref"]
                .as_str()
                .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
            let mut refs: Vec<(&str, &str)> = vec![("subjects", r_subj)];
            if let Some(ir) = r.get("identifier_ref").and_then(Value::as_str) {
                refs.push(("identifiers", ir));
            }
            if let Some(kr) = r.get("key_ref").and_then(Value::as_str) {
                refs.push(("keys", kr));
            }
            if let Some(crs) = r.get("claim_refs").and_then(Value::as_array) {
                for x in crs {
                    let cid = x
                        .as_str()
                        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
                    refs.push(("claims", cid));
                }
            }
            if let Some(comp) = r
                .get("boundary")
                .and_then(|b| b.get("comparison_subject_refs"))
                .and_then(Value::as_array)
            {
                for x in comp {
                    let sid = x
                        .as_str()
                        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
                    refs.push(("subjects", sid));
                }
            }
            for (rc, rid) in refs {
                let old_item = source.get(rc).and_then(Value::as_array).and_then(|arr| {
                    arr.iter()
                        .find(|x| x.get("id").and_then(Value::as_str) == Some(rid))
                });
                let cur_item = indexes[rc].get(rid).copied();
                req(
                    matches!((old_item, cur_item), (Some(oi), Some(ci)) if json_equal(oi, ci)),
                    "input-reference-changed",
                )?;
            }
        }
    }

    type AssessmentScopeKey = (
        String,
        Option<String>,
        Option<String>,
        String,
        String,
        Option<String>,
        Vec<String>,
    );
    let mut seen_scopes: HashSet<AssessmentScopeKey> = HashSet::new();

    for a in assessments_arr {
        let mut sorted_claims: Vec<String> = a
            .get("claim_refs")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        sorted_claims.sort();
        let a_subj = a["subject_ref"]
            .as_str()
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        let dim = a["dimension"]
            .as_str()
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        let scope = a["scope"]
            .as_str()
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        let value = a["value"]
            .as_str()
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        let scope_key = (
            a_subj.to_owned(),
            a.get("identifier_ref")
                .and_then(Value::as_str)
                .map(str::to_owned),
            a.get("key_ref").and_then(Value::as_str).map(str::to_owned),
            dim.to_owned(),
            scope.to_owned(),
            boundary_identity(a.get("boundary")),
            sorted_claims,
        );
        req(seen_scopes.insert(scope_key), "duplicate-assessment-scope")?;

        let start = a["assessed_at"]
            .as_str()
            .and_then(parse_strict_utc_unix)
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        let end = a["valid_until"]
            .as_str()
            .and_then(parse_strict_utc_unix)
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        req(start <= created && start < end, "invalid-assessment-window")?;
        if a["state"].as_str() == Some("current") {
            req(created < end, "expired-current-assessment")?;
        }
        let refs = a["evidence_refs"]
            .as_array()
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        let mut es: Vec<&Value> = Vec::with_capacity(refs.len());
        for er in refs {
            let er_id = er
                .as_str()
                .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
            let ev = evidence
                .get(er_id)
                .copied()
                .ok_or_else(|| EvidenceError::new("unknown-evidence"))?;
            es.push(ev);
        }
        req(
            es.iter().all(|e| e["subject_ref"].as_str() == Some(a_subj)),
            "evidence-subject-mismatch",
        )?;
        req(
            es.iter().all(|e| {
                e["collected_at"]
                    .as_str()
                    .and_then(parse_strict_utc_unix)
                    .is_some_and(|t| t <= start)
            }),
            "evidence-after-assessment",
        )?;
        if let Some(ak) = a.get("key_ref") {
            req(
                es.iter().all(|e| e.get("key_ref") == Some(ak)),
                "evidence-key-mismatch",
            )?;
        }
        let keyed: BTreeSet<&str> = es
            .iter()
            .filter(|e| a.get("key_ref").is_some() && e.get("key_ref") == a.get("key_ref"))
            .filter_map(|e| e["method"].as_str())
            .collect();
        let named: BTreeSet<&str> = es
            .iter()
            .filter(|e| {
                a.get("identifier_ref").is_some()
                    && e.get("identifier_ref") == a.get("identifier_ref")
            })
            .filter_map(|e| e["method"].as_str())
            .collect();

        match dim {
            "identity-binding" => match value {
                "declared" => req(named.contains("declaration"), "declaration-required")?,
                "observed" => req(named.contains("os-observation"), "observation-required")?,
                "enrolled-key" => req(
                    named.contains("enrollment")
                        && keyed.contains("enrollment")
                        && keyed.contains("possession-proof"),
                    "enrollment-and-proof-required",
                )?,
                "attested-key" => req(
                    keyed.contains("possession-proof")
                        && (keyed.contains("device-attestation")
                            || keyed.contains("workload-attestation"))
                        && (named.contains("device-attestation")
                            || named.contains("workload-attestation")),
                    "attestation-and-proof-required",
                )?,
                _ => {}
            },
            "key-custody" => {
                if value == "software" || value == "os-managed" {
                    req(
                        keyed.contains("custody-observation"),
                        "custody-observation-required",
                    )?;
                }
                if value == "hardware-backed" || value == "virtual-hardware-backed" {
                    req(
                        keyed.contains("device-attestation")
                            || keyed.contains("workload-attestation")
                            || keyed.contains("custody-observation"),
                        "key-custody-evidence-required",
                    )?;
                }
            }
            "execution-isolation" if value != "unknown" => {
                let target_b = boundary_identity(a.get("boundary"));
                let matching: BTreeSet<&str> = es
                    .iter()
                    .filter(|e| boundary_identity(e.get("boundary")) == target_b)
                    .filter_map(|e| e["method"].as_str())
                    .collect();
                req(
                    matching.contains("isolation-observation")
                        || matching.contains("workload-attestation")
                        || matching.contains("authority-statement"),
                    "isolation-boundary-evidence-required",
                )?;
                if value == "confidential-workload" {
                    req(
                        matching.contains("workload-attestation"),
                        "workload-attestation-required",
                    )?;
                }
            }
            "platform-evidence" if value != "unknown" => {
                let allowed_methods: &[&str] = match value {
                    "declared" => &["declaration"],
                    "locally-observed" => &["os-observation"],
                    "authority-verified" => &["authority-statement"],
                    "hardware-attested" => &["device-attestation", "workload-attestation"],
                    _ => &[],
                };
                let claim_refs = a["claim_refs"]
                    .as_array()
                    .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
                for cr_v in claim_refs {
                    let cr = cr_v
                        .as_str()
                        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
                    let covered = es.iter().any(|e| {
                        let m = e["method"].as_str().unwrap_or("");
                        allowed_methods.contains(&m)
                            && e.get("claim_refs")
                                .and_then(Value::as_array)
                                .is_some_and(|arr| arr.iter().any(|x| x.as_str() == Some(cr)))
                    });
                    req(covered, "platform-claim-evidence-required")?;
                }
            }
            _ => {}
        }
    }

    Ok(())
}

/// Refuse multi-report imports that collide on `(collection, id)` with unequal
/// records (no local-ID remapping).
pub fn check_import_collisions<'a>(
    reports: impl IntoIterator<Item = &'a Value>,
) -> Result<(), EvidenceError> {
    let mut seen: HashMap<(&'static str, String), &'a Value> = HashMap::new();
    let collections: [&'static str; 7] = [
        "subjects",
        "identifiers",
        "keys",
        "claims",
        "evidence",
        "capabilities",
        "input_reports",
    ];
    for report in reports {
        validate_report_shape(report)?;
        for &coll in &collections {
            let arr = report
                .get(coll)
                .and_then(Value::as_array)
                .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
            for rec in arr {
                let id = rec
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
                let key = (coll, id.to_owned());
                if let Some(prev) = seen.get(&key) {
                    if !json_equal(prev, rec) {
                        return Err(EvidenceError::new("import-id-collision"));
                    }
                } else {
                    seen.insert(key, rec);
                }
            }
        }
    }
    Ok(())
}

/// Enforce `input_reports` DAG acyclicity and `dependency_depth <= max_depth`.
pub fn validate_provenance_dag(
    root_report: &Value,
    input_reports: &BTreeMap<String, Value>,
    max_depth: usize,
) -> Result<(), EvidenceError> {
    fn dfs<'a>(
        cur: &'a Value,
        inputs: &'a BTreeMap<String, Value>,
        depth: usize,
        max_depth: usize,
        visiting: &mut HashSet<&'a str>,
    ) -> Result<(), EvidenceError> {
        if depth > max_depth {
            return Err(EvidenceError::new("provenance-depth-exceeded"));
        }
        validate_report_shape(cur)?;
        let rid = cur
            .get("report_id")
            .and_then(Value::as_str)
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        if !visiting.insert(rid) {
            return Err(EvidenceError::new("provenance-cycle"));
        }
        let refs = cur
            .get("input_reports")
            .and_then(Value::as_array)
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        for r in refs {
            let child_id = r
                .get("report_id")
                .and_then(Value::as_str)
                .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
            let child = inputs
                .get(child_id)
                .ok_or_else(|| EvidenceError::new("input-report-unavailable"))?;
            dfs(child, inputs, depth + 1, max_depth, visiting)?;
        }
        visiting.remove(rid);
        Ok(())
    }

    let mut visiting = HashSet::new();
    dfs(root_report, input_reports, 0, max_depth, &mut visiting)
}

/// Admit raw report wire bytes (`report-admission:v0`), validate `report.schema.json`
/// shape, and enforce `necessary_conditions`.
pub fn admit_and_validate_report(
    raw: &[u8],
    input_reports: Option<&BTreeMap<String, Value>>,
) -> Result<AdmittedWire, EvidenceError> {
    let admitted = admit_report_bytes(raw).map_err(|w| EvidenceError::new(w.code))?;
    validate_report_shape(&admitted.value)?;
    necessary_conditions(&admitted.value, input_reports)?;
    if let Some(inputs) = input_reports {
        if !admitted.value["input_reports"]
            .as_array()
            .is_none_or(Vec::is_empty)
        {
            validate_provenance_dag(&admitted.value, inputs, MAX_PROVENANCE_DEPTH)?;
        }
    }
    Ok(admitted)
}
