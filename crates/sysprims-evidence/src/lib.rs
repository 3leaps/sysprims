// SPDX-License-Identifier: MIT OR Apache-2.0

//! Portable `identity-evidence/v0` wire admission, report validation,
//! `local-process-session:v0` contract-fit evaluation, and
//! `native-process-observation:v0` deterministic comparison over RFC 8785
//! canonical JSON (`canonprims-json`).
//!
//! This crate forbids `unsafe` code and performs no OS process syscalls. Live
//! process collection and private OS continuity handles reside in
//! `sysprims-proc`.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub mod observation;
pub mod report;
pub mod session;
pub mod wire;

pub use observation::{
    admit_live_observation, bind_native_observation_report, compare_native_observations,
    compare_native_observations_json, derive_arguments_digest, derive_executable_digest,
    derive_linux_namespace_context_digest, derive_linux_security_context_digest,
    linux_id_is_unambiguously_mapped, project_host_scope_urn,
    project_to_local_process_session_process, project_to_local_process_session_process_json,
    validate_observation_shape, ComparisonOutcome, ComparisonPolicy, CANONICAL_CONTEXT_FACTS,
    LINUX_NAMESPACE_KINDS, LINUX_PROCFS_STAT_TIMENS_SCOPED_PROFILE_ID,
    NATIVE_PROCESS_OBSERVATION_PROFILE_ID, NATIVE_PROCESS_OBSERVATION_PROFILE_REVISION,
    NATIVE_PROCESS_OBSERVATION_README_REVISION, NATIVE_PROCESS_OBSERVATION_SCHEMA_REVISION,
};
pub use report::{
    admit_and_validate_report, check_import_collisions, necessary_conditions,
    validate_provenance_dag, validate_report_shape, IDENTITY_EVIDENCE_CONTRACT,
    MAX_PROVENANCE_DEPTH,
};
pub use session::{
    process_session_fit, validate_session_bundle_shape, LOCAL_PROCESS_SESSION_PROFILE_ID,
    LOCAL_PROCESS_SESSION_PROFILE_REVISION,
};
pub use wire::{
    admit_report_bytes, admit_report_reader, canonical_sha256_value, AdmittedWire, WireRefusal,
    MAX_CANONICAL_BYTES, MAX_JSON_DEPTH, MAX_REPORT_BYTES,
};

/// `sha256:` digest of the `canonical-json/v0` corpus `MANIFEST.json` pinned by `canonprims-json`.
pub const CORPUS_DIGEST: &str = canonprims_json::CORPUS_DIGEST;

/// Pinned `{id, revision}` profile reference shared across `identity-evidence/v0` contracts.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileRef {
    /// Closed URN or profile identifier (`1..=512` UTF-8 bytes).
    pub id: String,
    /// Content digest (`sha256:<64 lowercase hex>`).
    pub revision: String,
}

impl ProfileRef {
    /// Construct a new [`ProfileRef`].
    pub fn new(id: impl Into<String>, revision: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            revision: revision.into(),
        }
    }
}

/// Deterministic contract refusal carrying the exact normative refusal code.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{code}")]
pub struct EvidenceError {
    /// Normative contract refusal code (identical across Rust and Go twins).
    pub code: String,
}

impl EvidenceError {
    /// Construct a refusal with `code`.
    pub fn new(code: impl Into<String>) -> Self {
        Self { code: code.into() }
    }

    /// Borrow the normative refusal code string.
    pub fn code(&self) -> &str {
        &self.code
    }
}

/// Parse a strict `YYYY-MM-DDTHH:MM:SSZ` UTC instant into Unix whole seconds.
///
/// Sub-second fractions, non-`Z` offsets, lowercase `t`/`z`, leap-second `60`,
/// and invalid calendar dates are rejected.
pub fn parse_strict_utc_unix(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() != 20 {
        return None;
    }
    for &idx in &[0usize, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18] {
        if !b[idx].is_ascii_digit() {
            return None;
        }
    }
    if b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return None;
    }
    let sec = (b[17] - b'0') * 10 + (b[18] - b'0');
    if sec > 59 {
        return None;
    }
    let dt = OffsetDateTime::parse(text, &Rfc3339).ok()?;
    if dt.nanosecond() != 0 || dt.offset().whole_seconds() != 0 {
        return None;
    }
    Some(dt.unix_timestamp())
}

/// Compare two validated JSON values without conflating booleans and numbers,
/// treating object member order as immaterial and numerically equal numbers
/// (such as `1` and `1.0`) as equal.
pub fn json_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(fa), Some(fb)) => fa == fb,
            _ => a == b,
        },
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| json_equal(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, lv)| b.get(k).is_some_and(|rv| json_equal(lv, rv)))
        }
        _ => false,
    }
}

pub(crate) fn is_local_id(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > 64 || !b[0].is_ascii_lowercase() {
        return false;
    }
    b[1..]
        .iter()
        .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

pub(crate) fn is_sha256_digest(s: &str) -> bool {
    let Some(hex) = s.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

pub(crate) fn is_nonempty_512(s: &str) -> bool {
    let chars = s.chars().count();
    (1..=512).contains(&chars)
}

pub(crate) fn is_profile_ref_obj(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    if obj.len() != 2 {
        return false;
    }
    let Some(id) = obj.get("id").and_then(Value::as_str) else {
        return false;
    };
    let Some(rev) = obj.get("revision").and_then(Value::as_str) else {
        return false;
    };
    is_nonempty_512(id) && is_sha256_digest(rev)
}

pub(crate) fn is_principal_obj(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    if obj.len() != 2 {
        return false;
    }
    let Some(ns) = obj.get("namespace").and_then(Value::as_str) else {
        return false;
    };
    let Some(val) = obj.get("value").and_then(Value::as_str) else {
        return false;
    };
    is_nonempty_512(ns) && is_nonempty_512(val)
}
