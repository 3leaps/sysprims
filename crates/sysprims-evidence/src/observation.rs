// SPDX-License-Identifier: MIT OR Apache-2.0

//! `native-process-observation:v0` shape validation, deterministic snapshot
//! comparison, live freshness admission, context-digest preimages, and
//! injective projection into `local-process-session:v0`.

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use canonprims_core::DigestAlg;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::report::{necessary_conditions, validate_report_shape};
use crate::wire::canonical_sha256_value;
use crate::{
    is_local_id, is_nonempty_512, is_principal_obj, is_profile_ref_obj, is_sha256_digest,
    parse_strict_utc_unix, EvidenceError, ProfileRef,
};

/// `native-process-observation:v0` profile identifier.
pub const NATIVE_PROCESS_OBSERVATION_PROFILE_ID: &str =
    "urn:3leaps:identity-evidence:native-process-observation:v0";

/// Pinned SHA-256 revision of `profiles/native-process-observation/profile.json`.
pub const NATIVE_PROCESS_OBSERVATION_PROFILE_REVISION: &str =
    "sha256:0c116af8e558012f654be4c8ca772583b2d3b58c43cc157ce606e4f74f6b4a60";

/// Pinned SHA-256 revision of `profiles/native-process-observation/README.md`
/// (shared by the profile's sub-profiles and context derivations).
pub const NATIVE_PROCESS_OBSERVATION_README_REVISION: &str =
    "sha256:8a4347fcec0ad51874f28c6c1dfc6ca0888e9ccf2e25f239ba46b9be9d3749cb";

/// Pinned SHA-256 revision of `profiles/native-process-observation/evidence.schema.json`.
pub const NATIVE_PROCESS_OBSERVATION_SCHEMA_REVISION: &str =
    "sha256:670e254ca9cdeab8071757f91cff2226089c1d96630ce0e8c70185ddbaec8e45";

/// Scoped Linux start-identity profile emitted by [`project_to_local_process_session_process`].
pub const LINUX_PROCFS_STAT_TIMENS_SCOPED_PROFILE_ID: &str =
    "urn:3leaps:identity-evidence:start-identity:linux-procfs-stat-timens-scoped:v0";

/// Canonical `execution_context` fact order (`profile.json` `canonical_context_fact_order`).
pub const CANONICAL_CONTEXT_FACTS: &[&str] = &[
    "security_principal",
    "namespace_context_digest",
    "executable_digest",
    "arguments_digest",
    "security_context_digest",
];

/// Fixed 9-entry Linux namespace kind order (`linux-ns-inum-set:v0`).
pub const LINUX_NAMESPACE_KINDS: &[&str] = &[
    "cgroup",
    "ipc",
    "mnt",
    "net",
    "pid",
    "time",
    "time_for_children",
    "user",
    "uts",
];

const FACT_STATUSES: &[&str] = &[
    "observed",
    "not-present",
    "inaccessible",
    "unsupported",
    "not-probed",
    "error",
    "unknown",
];

const PLATFORM_FAMILIES: &[&str] = &["linux", "macos", "windows"];

const STABILITIES: &[&str] = &[
    "unknown",
    "process",
    "boot",
    "installation",
    "enrollment",
    "device",
];

const ESTABLISHED_STABILITIES: &[&str] = &["boot", "installation", "enrollment", "device"];

const CLONE_BEHAVIORS: &[&str] = &["unknown", "copyable", "regenerated", "device-bound"];

const MAX_OBSERVATION_AGE_SECONDS: i64 = 5;
const MAX_FUTURE_SKEW_SECONDS: i64 = 0;
const MIN_PID: u32 = 1;
const MAX_PID: u32 = 2_147_483_647;

/// Three-valued outcome returned by [`compare_native_observations`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComparisonOutcome {
    /// Both observations refer to the same process incarnation and satisfy policy.
    Match,
    /// Observations positively disagree on incarnation, state, or required/watched context.
    Mismatch(String),
    /// Comparison cannot conclude safely due to invalid policy, unestablished scope, or missing facts.
    Insufficient(String),
}

impl ComparisonOutcome {
    /// Format as the exact normative result code (`"match"`, `"mismatch:..."`, or `"insufficient:..."`).
    pub fn as_code(&self) -> String {
        match self {
            Self::Match => "match".to_string(),
            Self::Mismatch(r) => format!("mismatch:{r}"),
            Self::Insufficient(r) => format!("insufficient:{r}"),
        }
    }
}

impl fmt::Display for ComparisonOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_code())
    }
}

/// Caller-supplied evaluation policy for [`compare_native_observations`] (no defaults).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComparisonPolicy {
    /// Caller attests `baseline.process.host_scope` was authenticated/established out-of-band.
    pub baseline_host_scope_trusted: bool,
    /// Caller attests `candidate.process.host_scope` was authenticated/established out-of-band.
    pub candidate_host_scope_trusted: bool,
    /// Require `candidate.process.process_state.value == "live"`.
    pub require_live: bool,
    /// Context fact names that must be `observed` on both sides and equal.
    pub required_context_facts: Vec<String>,
    /// Compare any context fact observed on both sides even when omitted from `required_context_facts`.
    pub detect_observed_context_drift: bool,
    /// Optional allow-list of `host_scope.clone_behavior` values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_clone_behaviors: Option<Vec<String>>,
}

impl ComparisonPolicy {
    /// Parse and validate a raw JSON policy object with strict boolean/array types and no defaults.
    pub fn from_json_value(policy: &Value) -> Result<Self, EvidenceError> {
        let Some(obj) = policy.as_object() else {
            return Err(EvidenceError::new("invalid-evaluation-policy"));
        };
        let required_keys = [
            "baseline_host_scope_trusted",
            "candidate_host_scope_trusted",
            "require_live",
            "required_context_facts",
            "detect_observed_context_drift",
        ];
        for key in required_keys {
            if !obj.contains_key(key) {
                return Err(EvidenceError::new("invalid-evaluation-policy"));
            }
        }
        for k in obj.keys() {
            if !required_keys.contains(&k.as_str()) && k != "allowed_clone_behaviors" {
                return Err(EvidenceError::new("invalid-evaluation-policy"));
            }
        }
        let Some(baseline_host_scope_trusted) = obj
            .get("baseline_host_scope_trusted")
            .and_then(Value::as_bool)
        else {
            return Err(EvidenceError::new("invalid-evaluation-policy"));
        };
        let Some(candidate_host_scope_trusted) = obj
            .get("candidate_host_scope_trusted")
            .and_then(Value::as_bool)
        else {
            return Err(EvidenceError::new("invalid-evaluation-policy"));
        };
        let Some(require_live) = obj.get("require_live").and_then(Value::as_bool) else {
            return Err(EvidenceError::new("invalid-evaluation-policy"));
        };
        let Some(detect_observed_context_drift) = obj
            .get("detect_observed_context_drift")
            .and_then(Value::as_bool)
        else {
            return Err(EvidenceError::new("invalid-evaluation-policy"));
        };
        let Some(req_arr) = obj.get("required_context_facts").and_then(Value::as_array) else {
            return Err(EvidenceError::new("invalid-evaluation-policy"));
        };
        let mut required_context_facts = Vec::with_capacity(req_arr.len());
        for item in req_arr {
            let Some(s) = item.as_str() else {
                return Err(EvidenceError::new("invalid-evaluation-policy"));
            };
            if !CANONICAL_CONTEXT_FACTS.contains(&s) {
                return Err(EvidenceError::new("invalid-evaluation-policy"));
            }
            required_context_facts.push(s.to_string());
        }
        let allowed_clone_behaviors = if let Some(ac_val) = obj.get("allowed_clone_behaviors") {
            let Some(ac_arr) = ac_val.as_array() else {
                return Err(EvidenceError::new("invalid-evaluation-policy"));
            };
            if ac_arr.is_empty() {
                return Err(EvidenceError::new("invalid-evaluation-policy"));
            }
            let mut out = Vec::with_capacity(ac_arr.len());
            for item in ac_arr {
                let Some(s) = item.as_str() else {
                    return Err(EvidenceError::new("invalid-evaluation-policy"));
                };
                if !CLONE_BEHAVIORS.contains(&s) {
                    return Err(EvidenceError::new("invalid-evaluation-policy"));
                }
                out.push(s.to_string());
            }
            Some(out)
        } else {
            None
        };

        Ok(Self {
            baseline_host_scope_trusted,
            candidate_host_scope_trusted,
            require_live,
            required_context_facts,
            detect_observed_context_drift,
            allowed_clone_behaviors,
        })
    }

    fn validate(&self) -> Result<(HashSet<&str>, Option<HashSet<&str>>), ()> {
        let mut req_set = HashSet::with_capacity(self.required_context_facts.len());
        for f in &self.required_context_facts {
            if !CANONICAL_CONTEXT_FACTS.contains(&f.as_str()) {
                return Err(());
            }
            req_set.insert(f.as_str());
        }
        let allowed = match &self.allowed_clone_behaviors {
            Some(list) => {
                if list.is_empty() {
                    return Err(());
                }
                let mut set = HashSet::with_capacity(list.len());
                for c in list {
                    if !CLONE_BEHAVIORS.contains(&c.as_str()) {
                        return Err(());
                    }
                    set.insert(c.as_str());
                }
                Some(set)
            }
            None => None,
        };
        Ok((req_set, allowed))
    }
}

fn err_obs_shape() -> EvidenceError {
    EvidenceError::new("invalid-observation-shape")
}

fn check_allowed_keys(
    obj: &serde_json::Map<String, Value>,
    required: &[&str],
    optional: &[&str],
) -> bool {
    for &r in required {
        if !obj.contains_key(r) {
            return false;
        }
    }
    obj.keys()
        .all(|k| required.contains(&k.as_str()) || optional.contains(&k.as_str()))
}

fn is_arch_string(s: &str) -> bool {
    let b = s.as_bytes();
    (2..=32).contains(&b.len())
        && b.iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

fn is_canonical_u32(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 10 {
        return None;
    }
    let b = s.as_bytes();
    if b.len() > 1 && b[0] == b'0' {
        return None;
    }
    if !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    s.parse::<u32>().ok()
}

fn is_canonical_u64(s: &str) -> Option<u64> {
    if s.is_empty() || s.len() > 20 {
        return None;
    }
    let b = s.as_bytes();
    if b.len() > 1 && b[0] == b'0' {
        return None;
    }
    if !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    s.parse::<u64>().ok()
}

fn is_canonical_nonneg_i64(s: &str) -> Option<i64> {
    if s.is_empty() || s.len() > 19 {
        return None;
    }
    let b = s.as_bytes();
    if b.len() > 1 && b[0] == b'0' {
        return None;
    }
    if !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    s.parse::<i64>().ok()
}

fn is_valid_pid_string(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > 10 || !(b'1'..=b'9').contains(&b[0]) {
        return false;
    }
    if !b[1..].iter().all(u8::is_ascii_digit) {
        return false;
    }
    s.parse::<u32>()
        .is_ok_and(|n| (MIN_PID..=MAX_PID).contains(&n))
}

fn is_schema_namespace_value(s: &str) -> bool {
    let Some((prefix, rest)) = s.split_once(':') else {
        return false;
    };
    let pb = prefix.as_bytes();
    let rb = rest.as_bytes();
    if !(2..=32).contains(&pb.len()) || !pb[0].is_ascii_lowercase() {
        return false;
    }
    if !pb[1..]
        .iter()
        .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    {
        return false;
    }
    (1..=256).contains(&rb.len()) && rb.iter().all(|&c| (0x21..=0x7e).contains(&c))
}

fn is_lowercase_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 {
        return false;
    }
    for (idx, &c) in b.iter().enumerate() {
        if matches!(idx, 8 | 13 | 18 | 23) {
            if c != b'-' {
                return false;
            }
        } else if !(c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
            return false;
        }
    }
    true
}

fn is_linux_scoped_ns_id(s: &str, expected_prefix: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 4 || parts[0] != expected_prefix {
        return false;
    }
    is_canonical_u32(parts[1]).is_some()
        && is_canonical_u32(parts[2]).is_some()
        && is_canonical_u64(parts[3]).is_some()
}

fn validate_fact_reason_opt(obj: &serde_json::Map<String, Value>) -> bool {
    match obj.get("reason") {
        Some(r) => r.as_str().is_some_and(is_nonempty_512),
        None => true,
    }
}

fn validate_fact_reason_req(obj: &serde_json::Map<String, Value>) -> bool {
    obj.get("reason")
        .and_then(Value::as_str)
        .is_some_and(is_nonempty_512)
}

fn validate_namespace_fact(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    let Some(status) = obj.get("status").and_then(Value::as_str) else {
        return false;
    };
    if !FACT_STATUSES.contains(&status) {
        return false;
    }
    if status == "observed" {
        check_allowed_keys(obj, &["status", "value"], &["reason"])
            && obj
                .get("value")
                .and_then(Value::as_str)
                .is_some_and(is_schema_namespace_value)
            && validate_fact_reason_opt(obj)
    } else {
        check_allowed_keys(obj, &["status", "reason"], &[]) && validate_fact_reason_req(obj)
    }
}

fn validate_boot_id_fact(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    let Some(status) = obj.get("status").and_then(Value::as_str) else {
        return false;
    };
    if !FACT_STATUSES.contains(&status) {
        return false;
    }
    if status == "observed" {
        check_allowed_keys(obj, &["status", "value"], &["reason"])
            && obj
                .get("value")
                .and_then(Value::as_str)
                .is_some_and(is_nonempty_512)
            && validate_fact_reason_opt(obj)
    } else {
        check_allowed_keys(obj, &["status", "reason"], &[]) && validate_fact_reason_req(obj)
    }
}

fn validate_state_fact(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    let Some(status) = obj.get("status").and_then(Value::as_str) else {
        return false;
    };
    if !FACT_STATUSES.contains(&status) {
        return false;
    }
    if status == "observed" {
        check_allowed_keys(obj, &["status", "value"], &["reason"])
            && obj
                .get("value")
                .and_then(Value::as_str)
                .is_some_and(|s| s == "live" || s == "zombie")
            && validate_fact_reason_opt(obj)
    } else {
        check_allowed_keys(obj, &["status", "reason"], &[]) && validate_fact_reason_req(obj)
    }
}

fn validate_start_identity_fact(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    let Some(status) = obj.get("status").and_then(Value::as_str) else {
        return false;
    };
    if !FACT_STATUSES.contains(&status) || !obj.get("profile").is_some_and(is_profile_ref_obj) {
        return false;
    }
    if status == "observed" {
        check_allowed_keys(obj, &["status", "profile", "value"], &["reason"])
            && obj.get("value").and_then(Value::as_str).is_some_and(|s| {
                let c = s.chars().count();
                (1..=128).contains(&c)
            })
            && validate_fact_reason_opt(obj)
    } else {
        check_allowed_keys(obj, &["status", "profile", "reason"], &[])
            && validate_fact_reason_req(obj)
    }
}

fn validate_principal_fact(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    let Some(status) = obj.get("status").and_then(Value::as_str) else {
        return false;
    };
    if !FACT_STATUSES.contains(&status) {
        return false;
    }
    if status == "observed" {
        check_allowed_keys(obj, &["status", "derivation", "principal"], &["reason"])
            && obj.get("derivation").is_some_and(is_profile_ref_obj)
            && obj.get("principal").is_some_and(is_principal_obj)
            && validate_fact_reason_opt(obj)
    } else {
        check_allowed_keys(obj, &["status", "reason"], &[]) && validate_fact_reason_req(obj)
    }
}

fn validate_digest_fact(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    let Some(status) = obj.get("status").and_then(Value::as_str) else {
        return false;
    };
    if !FACT_STATUSES.contains(&status) {
        return false;
    }
    if status == "observed" {
        check_allowed_keys(obj, &["status", "derivation", "digest"], &["reason"])
            && obj.get("derivation").is_some_and(is_profile_ref_obj)
            && obj
                .get("digest")
                .and_then(Value::as_str)
                .is_some_and(is_sha256_digest)
            && validate_fact_reason_opt(obj)
    } else {
        check_allowed_keys(obj, &["status", "reason"], &[]) && validate_fact_reason_req(obj)
    }
}

/// Validate strict `native-process-observation/evidence.schema.json` structural
/// and 7-state conditional rules.
pub fn validate_observation_shape(obs: &Value) -> Result<(), EvidenceError> {
    let Some(obj) = obs.as_object() else {
        return Err(err_obs_shape());
    };
    if !check_allowed_keys(obj, &["profile", "observed_at", "observer", "process"], &[])
        || !obj.get("profile").is_some_and(is_profile_ref_obj)
        || !obj
            .get("observed_at")
            .and_then(Value::as_str)
            .is_some_and(|s| parse_strict_utc_unix(s).is_some())
    {
        return Err(err_obs_shape());
    }

    let Some(observer) = obj.get("observer").and_then(Value::as_object) else {
        return Err(err_obs_shape());
    };
    if !check_allowed_keys(
        observer,
        &[
            "platform_family",
            "architecture",
            "bracketed",
            "time_namespace",
            "user_namespace",
        ],
        &[],
    ) || !observer
        .get("platform_family")
        .and_then(Value::as_str)
        .is_some_and(|p| PLATFORM_FAMILIES.contains(&p))
        || !observer
            .get("architecture")
            .and_then(Value::as_str)
            .is_some_and(is_arch_string)
        || !observer.get("bracketed").is_some_and(Value::is_boolean)
        || !observer
            .get("time_namespace")
            .is_some_and(validate_namespace_fact)
        || !observer
            .get("user_namespace")
            .is_some_and(validate_namespace_fact)
    {
        return Err(err_obs_shape());
    }

    let Some(proc_obj) = obj.get("process").and_then(Value::as_object) else {
        return Err(err_obs_shape());
    };
    if !check_allowed_keys(
        proc_obj,
        &[
            "subject_ref",
            "host_scope",
            "boot_id",
            "pid_numbering_namespace",
            "target_pid_namespace",
            "pid",
            "process_state",
            "start_identity",
            "execution_context",
        ],
        &[],
    ) || !proc_obj
        .get("subject_ref")
        .and_then(Value::as_str)
        .is_some_and(is_local_id)
        || !proc_obj.get("boot_id").is_some_and(validate_boot_id_fact)
        || !proc_obj
            .get("pid_numbering_namespace")
            .is_some_and(validate_namespace_fact)
        || !proc_obj
            .get("target_pid_namespace")
            .is_some_and(validate_namespace_fact)
        || !proc_obj
            .get("pid")
            .and_then(Value::as_str)
            .is_some_and(is_valid_pid_string)
        || !proc_obj
            .get("process_state")
            .is_some_and(validate_state_fact)
        || !proc_obj
            .get("start_identity")
            .is_some_and(validate_start_identity_fact)
    {
        return Err(err_obs_shape());
    }

    let Some(hs) = proc_obj.get("host_scope").and_then(Value::as_object) else {
        return Err(err_obs_shape());
    };
    if !check_allowed_keys(
        hs,
        &[
            "namespace",
            "value",
            "derivation",
            "stability",
            "clone_behavior",
        ],
        &["identifier_ref"],
    ) || !hs
        .get("namespace")
        .and_then(Value::as_str)
        .is_some_and(is_nonempty_512)
        || !hs
            .get("value")
            .and_then(Value::as_str)
            .is_some_and(is_nonempty_512)
        || !hs.get("derivation").is_some_and(is_profile_ref_obj)
        || !hs
            .get("stability")
            .and_then(Value::as_str)
            .is_some_and(|s| STABILITIES.contains(&s))
        || !hs
            .get("clone_behavior")
            .and_then(Value::as_str)
            .is_some_and(|c| CLONE_BEHAVIORS.contains(&c))
    {
        return Err(err_obs_shape());
    }
    if let Some(ir) = hs.get("identifier_ref") {
        if !ir.as_str().is_some_and(is_local_id) {
            return Err(err_obs_shape());
        }
    }

    let Some(ec) = proc_obj.get("execution_context").and_then(Value::as_object) else {
        return Err(err_obs_shape());
    };
    if !check_allowed_keys(ec, CANONICAL_CONTEXT_FACTS, &[])
        || !ec
            .get("security_principal")
            .is_some_and(validate_principal_fact)
        || ![
            "namespace_context_digest",
            "executable_digest",
            "arguments_digest",
            "security_context_digest",
        ]
        .iter()
        .all(|k| ec.get(*k).is_some_and(validate_digest_fact))
    {
        return Err(err_obs_shape());
    }

    Ok(())
}

fn push_u32_be(out: &mut Vec<u8>, n: usize) {
    let v = u32::try_from(n).expect("framed length fits in u32");
    out.extend_from_slice(&v.to_be_bytes());
}

/// Derive `arguments_digest` (`argv-bytes:v0`) over raw NUL-free argument byte slices.
pub fn derive_arguments_digest<T: AsRef<[u8]>>(argv: &[T]) -> String {
    let mut pre = Vec::from(&b"3leaps.native-process-observation.arguments.v0\0"[..]);
    push_u32_be(&mut pre, argv.len());
    for arg in argv {
        let bytes = arg.as_ref();
        push_u32_be(&mut pre, bytes.len());
        pre.extend_from_slice(bytes);
    }
    DigestAlg::Sha256.format(&pre)
}

/// Derive `executable_digest` (`executable-file-bytes:v0`) over raw executable image bytes.
pub fn derive_executable_digest(image_bytes: &[u8]) -> String {
    let mut pre = Vec::with_capacity(55 + image_bytes.len());
    pre.extend_from_slice(b"3leaps.native-process-observation.executable-image.v0\0");
    pre.extend_from_slice(image_bytes);
    DigestAlg::Sha256.format(&pre)
}

/// Derive `namespace_context_digest` (`linux-ns-inum-set:v0`) over the complete
/// 9-entry Linux namespace map (`ns:<major_u32>:<minor_u32>:<ino_u64>`).
pub fn derive_linux_namespace_context_digest(
    ns_map: &BTreeMap<String, String>,
) -> Result<String, EvidenceError> {
    if ns_map.len() != LINUX_NAMESPACE_KINDS.len()
        || !LINUX_NAMESPACE_KINDS
            .iter()
            .all(|&k| ns_map.contains_key(k))
    {
        return Err(EvidenceError::new("incomplete-linux-namespace-set"));
    }
    let mut pre = Vec::from(&b"3leaps.native-process-observation.namespaces.v0\0"[..]);
    push_u32_be(&mut pre, LINUX_NAMESPACE_KINDS.len());
    for &kind in LINUX_NAMESPACE_KINDS {
        let val = &ns_map[kind];
        if !is_linux_scoped_ns_id(val, "ns") {
            return Err(EvidenceError::new("invalid-linux-ns-entry"));
        }
        push_u32_be(&mut pre, kind.len());
        pre.extend_from_slice(kind.as_bytes());
        push_u32_be(&mut pre, val.len());
        pre.extend_from_slice(val.as_bytes());
    }
    Ok(DigestAlg::Sha256.format(&pre))
}

fn is_lower_hex16(s: &str) -> bool {
    s.len() == 16
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// Derive `security_context_digest` (`linux-procfs-status-secctx:v0`) over canonical
/// ASCII security-context bytes (validating `observer_user_ns`, sorted deduplicated
/// `u32` `groups`, `no_new_privs`, `u32` `seccomp`, and 16-hex capability masks).
pub fn derive_linux_security_context_digest(
    canonical_context_bytes: &[u8],
) -> Result<String, EvidenceError> {
    let err = || EvidenceError::new("invalid-linux-security-context");
    if !canonical_context_bytes.is_ascii() {
        return Err(err());
    }
    let text = std::str::from_utf8(canonical_context_bytes).map_err(|_| err())?;
    let parts: Vec<&str> = text.split(';').collect();
    if parts.len() != 9 {
        return Err(err());
    }
    let user_ns = parts[0].strip_prefix("observer_user_ns=").ok_or_else(err)?;
    if user_ns != "userns:initial" && !is_linux_scoped_ns_id(user_ns, "userns") {
        return Err(err());
    }
    let groups_s = parts[1].strip_prefix("groups=").ok_or_else(err)?;
    if !groups_s.is_empty() {
        let mut prev: Option<u32> = None;
        for tok in groups_s.split(',') {
            let gid = is_canonical_u32(tok).ok_or_else(err)?;
            if let Some(p) = prev {
                if gid <= p {
                    return Err(err());
                }
            }
            prev = Some(gid);
        }
    }
    let nnp = parts[2].strip_prefix("no_new_privs=").ok_or_else(err)?;
    if nnp != "0" && nnp != "1" {
        return Err(err());
    }
    let seccomp_s = parts[3].strip_prefix("seccomp=").ok_or_else(err)?;
    if is_canonical_u32(seccomp_s).is_none() {
        return Err(err());
    }
    for (idx, key) in [
        (4usize, "cap_inh="),
        (5, "cap_prm="),
        (6, "cap_eff="),
        (7, "cap_bnd="),
        (8, "cap_amb="),
    ] {
        let hex = parts[idx].strip_prefix(key).ok_or_else(err)?;
        if !is_lower_hex16(hex) {
            return Err(err());
        }
    }

    let scheme = b"urn:3leaps:identity-evidence:context-derivation:linux-procfs-status-secctx:v0";
    let mut pre = Vec::from(&b"3leaps.native-process-observation.security-context.v0\0"[..]);
    push_u32_be(&mut pre, scheme.len());
    pre.extend_from_slice(scheme);
    pre.push(0);
    push_u32_be(&mut pre, canonical_context_bytes.len());
    pre.extend_from_slice(canonical_context_bytes);
    Ok(DigestAlg::Sha256.format(&pre))
}

/// Fail-closed check against Linux `from_kuid_munged` / `from_kgid_munged`
/// overflow/unmapped ID conflation inside an observer user namespace.
///
/// Returns `true` iff `rendered_id < u32::MAX` (`4294967295` is `(uid_t)-1`
/// `INVALID_UID`) and either `id_map_extents` is the full identity extent
/// `[(0, 4294967295)]` or `rendered_id` falls inside a mapped `[first, first + count)`
/// extent and does not equal `overflow_id`.
pub fn linux_id_is_unambiguously_mapped(
    rendered_id: u32,
    id_map_extents: &[(u32, u32)],
    overflow_id: u32,
) -> bool {
    if rendered_id == u32::MAX {
        return false;
    }
    if id_map_extents.len() == 1 && id_map_extents[0] == (0, u32::MAX) {
        return true;
    }
    let id64 = u64::from(rendered_id);
    let in_extent = id_map_extents.iter().any(|&(first, count)| {
        let start = u64::from(first);
        let end = start + u64::from(count);
        (start..end).contains(&id64)
    });
    in_extent && rendered_id != overflow_id
}

/// Project `process.host_scope` into the fixed 114-character collision-free URN
/// `urn:3leaps:identity-evidence:host-scope:v0:sha256:<64 lowercase hex>`.
pub fn project_host_scope_urn(host_scope: &Value) -> Result<String, EvidenceError> {
    let err = || EvidenceError::new("invalid-observation-shape");
    let ns = host_scope
        .get("namespace")
        .and_then(Value::as_str)
        .ok_or_else(err)?;
    let val = host_scope
        .get("value")
        .and_then(Value::as_str)
        .ok_or_else(err)?;
    let drv = host_scope.get("derivation").ok_or_else(err)?;
    let drv_id = drv.get("id").and_then(Value::as_str).ok_or_else(err)?;
    let drv_rev = drv
        .get("revision")
        .and_then(Value::as_str)
        .ok_or_else(err)?;
    let stability = host_scope
        .get("stability")
        .and_then(Value::as_str)
        .ok_or_else(err)?;
    let clone_behavior = host_scope
        .get("clone_behavior")
        .and_then(Value::as_str)
        .ok_or_else(err)?;

    let mut pre = Vec::from(&b"3leaps.native-process-observation.host-scope.v0\0"[..]);
    for part in [ns, val, drv_id, drv_rev, stability, clone_behavior] {
        let pb = part.as_bytes();
        push_u32_be(&mut pre, pb.len());
        pre.extend_from_slice(pb);
    }
    let sha = DigestAlg::Sha256.format(&pre);
    let hex = sha.strip_prefix("sha256:").unwrap_or(&sha);
    Ok(format!(
        "urn:3leaps:identity-evidence:host-scope:v0:sha256:{hex}"
    ))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StartSubprofile {
    LinuxProcfsStatV0,
    MacosKinfoStarttimeV0,
}

impl StartSubprofile {
    fn lookup(obs: &Value) -> Option<Self> {
        let prof = obs.get("process")?.get("start_identity")?.get("profile")?;
        let id = prof.get("id")?.as_str()?;
        let rev = prof.get("revision")?.as_str()?;
        if rev != NATIVE_PROCESS_OBSERVATION_README_REVISION {
            return None;
        }
        match id {
            "urn:3leaps:identity-evidence:start-identity:linux-procfs-stat:v0" => {
                Some(Self::LinuxProcfsStatV0)
            }
            "urn:3leaps:identity-evidence:start-identity:macos-kinfo-starttime:v0" => {
                Some(Self::MacosKinfoStarttimeV0)
            }
            _ => None,
        }
    }

    fn platform_family(self) -> &'static str {
        match self {
            Self::LinuxProcfsStatV0 => "linux",
            Self::MacosKinfoStarttimeV0 => "macos",
        }
    }

    fn validate_start_value(self, value: &str) -> bool {
        match self {
            Self::LinuxProcfsStatV0 => is_canonical_u64(value).is_some(),
            Self::MacosKinfoStarttimeV0 => {
                let Some((sec_s, usec_s)) = value.split_once('.') else {
                    return false;
                };
                is_canonical_nonneg_i64(sec_s).is_some()
                    && usec_s.len() == 6
                    && usec_s.bytes().all(|c| c.is_ascii_digit())
            }
        }
    }
}

fn observer_userns_scope(obs: &Value) -> Option<String> {
    let uns = obs.get("observer")?.get("user_namespace")?;
    match uns.get("status")?.as_str()? {
        "observed" => Some(uns.get("value")?.as_str()?.to_string()),
        "not-present" => Some("userns:initial".to_string()),
        _ => None,
    }
}

fn validate_principal_tokens(value: &str, keys: &[&str]) -> bool {
    let parts: Vec<&str> = value.split(';').collect();
    if parts.len() != keys.len() {
        return false;
    }
    for (part, &key) in parts.iter().zip(keys.iter()) {
        let Some(rest) = part.strip_prefix(key) else {
            return false;
        };
        let Some(num_s) = rest.strip_prefix('=') else {
            return false;
        };
        if is_canonical_u32(num_s).is_none() {
            return false;
        }
    }
    true
}

fn validate_context_derivation(fact_name: &str, fact_obj: &Value, obs: &Value) -> bool {
    let Some(platform_family) = obs
        .get("observer")
        .and_then(|o| o.get("platform_family"))
        .and_then(Value::as_str)
    else {
        return false;
    };
    let Some(drv) = fact_obj.get("derivation") else {
        return false;
    };
    let Some(drv_id) = drv.get("id").and_then(Value::as_str) else {
        return false;
    };
    let Some(drv_rev) = drv.get("revision").and_then(Value::as_str) else {
        return false;
    };
    if drv_rev != NATIVE_PROCESS_OBSERVATION_README_REVISION {
        return false;
    }
    match fact_name {
        "security_principal" => {
            let Some(p) = fact_obj.get("principal") else {
                return false;
            };
            let Some(ns) = p.get("namespace").and_then(Value::as_str) else {
                return false;
            };
            let Some(val) = p.get("value").and_then(Value::as_str) else {
                return false;
            };
            match drv_id {
                "urn:3leaps:identity-evidence:principal-derivation:macos-kinfo-ucred:v0" => {
                    platform_family == "macos"
                        && ns == "urn:3leaps:posix-credentials:macos-kinfo:v0"
                        && validate_principal_tokens(val, &["ruid", "euid", "rgid", "egid"])
                }
                "urn:3leaps:identity-evidence:principal-derivation:linux-procfs-status-uidgid:v0" =>
                {
                    if platform_family != "linux" {
                        return false;
                    }
                    let Some(scope) = observer_userns_scope(obs) else {
                        return false;
                    };
                    let expected_ns =
                        format!("urn:3leaps:posix-credentials:linux-procfs-status:v0:{scope}");
                    ns == expected_ns
                        && validate_principal_tokens(
                            val,
                            &[
                                "ruid", "euid", "suid", "fsuid", "rgid", "egid", "sgid", "fsgid",
                            ],
                        )
                }
                _ => false,
            }
        }
        "arguments_digest" => {
            drv_id == "urn:3leaps:identity-evidence:context-derivation:argv-bytes:v0"
                && (platform_family == "linux" || platform_family == "macos")
        }
        "executable_digest" => {
            drv_id == "urn:3leaps:identity-evidence:context-derivation:executable-file-bytes:v0"
                && (platform_family == "linux" || platform_family == "macos")
        }
        "namespace_context_digest" => {
            drv_id == "urn:3leaps:identity-evidence:context-derivation:linux-ns-inum-set:v0"
                && platform_family == "linux"
        }
        "security_context_digest" => {
            drv_id
                == "urn:3leaps:identity-evidence:context-derivation:linux-procfs-status-secctx:v0"
                && platform_family == "linux"
        }
        _ => false,
    }
}

fn admit_live_observation_steps(
    obs: &Value,
    now_utc: Option<&str>,
    accepted_profile: &ProfileRef,
    report: Option<&Value>,
    input_reports: Option<&BTreeMap<String, Value>>,
) -> Result<(), EvidenceError> {
    let now_unix = now_utc
        .and_then(parse_strict_utc_unix)
        .ok_or_else(|| EvidenceError::new("trusted-clock-unavailable"))?;
    let expected_prof = json!({
        "id": accepted_profile.id,
        "revision": accepted_profile.revision,
    });
    if obs.get("profile") != Some(&expected_prof) {
        return Err(EvidenceError::new("unsupported-evidence-profile"));
    }
    let obs_unix = obs
        .get("observed_at")
        .and_then(Value::as_str)
        .and_then(parse_strict_utc_unix)
        .ok_or_else(|| EvidenceError::new("invalid-observed-at"))?;

    if let Some(rep) = report {
        necessary_conditions(rep, input_reports)?;
        let obs_digest = canonical_sha256_value(obs).map_err(|w| EvidenceError::new(w.code))?;
        let ev_arr = rep
            .get("evidence")
            .and_then(Value::as_array)
            .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?;
        let matched: Vec<&Value> = ev_arr
            .iter()
            .filter(|e| {
                e.get("format_profile") == Some(&expected_prof)
                    || e.get("digest").and_then(Value::as_str) == Some(obs_digest.as_str())
            })
            .collect();
        if matched.len() != 1 {
            return Err(EvidenceError::new(
                "observation-evidence-descriptor-missing",
            ));
        }
        let ev = matched[0];
        let proc_subj = obs.get("process").and_then(|p| p.get("subject_ref"));
        if ev.get("format_profile") != Some(&expected_prof)
            || ev.get("digest").and_then(Value::as_str) != Some(obs_digest.as_str())
            || ev.get("method").and_then(Value::as_str) != Some("os-observation")
            || ev.get("subject_ref") != proc_subj
            || ev.get("collected_at") != obs.get("observed_at")
        {
            return Err(EvidenceError::new(
                "observation-evidence-descriptor-mismatch",
            ));
        }
    }

    let age = now_unix - obs_unix;
    if age < -MAX_FUTURE_SKEW_SECONDS {
        return Err(EvidenceError::new("future-process-observation"));
    }
    if age > MAX_OBSERVATION_AGE_SECONDS {
        return Err(EvidenceError::new("stale-process-observation"));
    }
    Ok(())
}

/// Standalone freshness and optional report-time binding gate for live
/// `native-process-observation:v0` consumers (`admit_live_observation`).
pub fn admit_live_observation(
    obs: &Value,
    now_utc: Option<&str>,
    accepted_profile: &ProfileRef,
    report: Option<&Value>,
    input_reports: Option<&BTreeMap<String, Value>>,
) -> Result<(), EvidenceError> {
    admit_live_observation_steps(obs, now_utc, accepted_profile, report, input_reports)?;
    validate_observation_shape(obs)
}

/// Evaluate [`compare_native_observations`] when `policy` arrives as an untrusted JSON value.
pub fn compare_native_observations_json(
    baseline: &Value,
    candidate: &Value,
    policy: &Value,
    accepted_profile: &ProfileRef,
) -> ComparisonOutcome {
    let Ok(parsed_policy) = ComparisonPolicy::from_json_value(policy) else {
        return ComparisonOutcome::Insufficient("invalid-evaluation-policy".to_string());
    };
    compare_native_observations(baseline, candidate, &parsed_policy, accepted_profile)
}

/// Deterministically compare `baseline` and `candidate` `native-process-observation:v0`
/// payloads in strict top-to-bottom row order (`1a` through `9`).
pub fn compare_native_observations(
    baseline: &Value,
    candidate: &Value,
    policy: &ComparisonPolicy,
    accepted_profile: &ProfileRef,
) -> ComparisonOutcome {
    // Row 1a: policy check
    let Ok((req_facts_set, allowed_clones)) = policy.validate() else {
        return ComparisonOutcome::Insufficient("invalid-evaluation-policy".to_string());
    };

    // Row 1b: accepted profile
    let expected_prof = json!({
        "id": accepted_profile.id,
        "revision": accepted_profile.revision,
    });
    if baseline.get("profile") != Some(&expected_prof)
        || candidate.get("profile") != Some(&expected_prof)
    {
        return ComparisonOutcome::Insufficient("unsupported-evidence-profile".to_string());
    }

    // Row 1c: bracketed
    let b_bracketed = baseline
        .get("observer")
        .and_then(|o| o.get("bracketed"))
        .and_then(Value::as_bool);
    let c_bracketed = candidate
        .get("observer")
        .and_then(|o| o.get("bracketed"))
        .and_then(Value::as_bool);
    if b_bracketed != Some(true) || c_bracketed != Some(true) {
        return ComparisonOutcome::Insufficient("unbracketed-observation".to_string());
    }

    // Row 1d: start_identity subprofile lookup on BOTH sides
    let base_sub = StartSubprofile::lookup(baseline);
    let cand_sub = StartSubprofile::lookup(candidate);
    let (Some(base_sub), Some(cand_sub)) = (base_sub, cand_sub) else {
        return ComparisonOutcome::Insufficient("unsupported-start-identity-profile".to_string());
    };

    // Row 1e: platform_family vs subprofile on BOTH sides
    let b_plat = baseline["observer"]["platform_family"]
        .as_str()
        .unwrap_or("");
    let c_plat = candidate["observer"]["platform_family"]
        .as_str()
        .unwrap_or("");
    if b_plat != base_sub.platform_family() || c_plat != cand_sub.platform_family() {
        return ComparisonOutcome::Insufficient("inconsistent-observation".to_string());
    }

    let b_proc = &baseline["process"];
    let c_proc = &candidate["process"];
    let b_hs = &b_proc["host_scope"];
    let c_hs = &c_proc["host_scope"];

    // Row 2a: host_scope trust & stability
    let b_stab = b_hs["stability"].as_str().unwrap_or("");
    let c_stab = c_hs["stability"].as_str().unwrap_or("");
    if !policy.baseline_host_scope_trusted
        || !policy.candidate_host_scope_trusted
        || !ESTABLISHED_STABILITIES.contains(&b_stab)
        || !ESTABLISHED_STABILITIES.contains(&c_stab)
    {
        return ComparisonOutcome::Insufficient("unestablished-host-scope".to_string());
    }

    // Row 2b: host_scope (namespace, value, derivation)
    for hs in [b_hs, c_hs] {
        if !hs["namespace"].as_str().is_some_and(is_nonempty_512)
            || !hs["value"].as_str().is_some_and(is_nonempty_512)
            || !is_profile_ref_obj(&hs["derivation"])
        {
            return ComparisonOutcome::Insufficient("invalid-observation-shape".to_string());
        }
    }
    for key in ["namespace", "value", "derivation"] {
        if b_hs.get(key) != c_hs.get(key) {
            return ComparisonOutcome::Mismatch("host-scope-mismatch".to_string());
        }
    }

    // Row 2c: host_scope (stability, clone_behavior)
    let b_clone = b_hs["clone_behavior"].as_str().unwrap_or("");
    let c_clone = c_hs["clone_behavior"].as_str().unwrap_or("");
    if b_stab != c_stab || b_clone != c_clone {
        return ComparisonOutcome::Insufficient("inconsistent-host-scope-metadata".to_string());
    }

    // Row 2d: allowed_clone_behaviors
    if let Some(allowed) = &allowed_clones {
        if !allowed.contains(b_clone) || !allowed.contains(c_clone) {
            return ComparisonOutcome::Insufficient("disallowed-clone-behavior".to_string());
        }
    }

    // Row 3a-3c: boot_id
    if b_proc["boot_id"]["status"].as_str() != Some("observed")
        || c_proc["boot_id"]["status"].as_str() != Some("observed")
    {
        return ComparisonOutcome::Insufficient("missing-boot-id".to_string());
    }
    let b_boot = b_proc["boot_id"]["value"].as_str().unwrap_or("");
    let c_boot = c_proc["boot_id"]["value"].as_str().unwrap_or("");
    if !is_lowercase_uuid(b_boot) || !is_lowercase_uuid(c_boot) {
        return ComparisonOutcome::Insufficient("invalid-boot-id".to_string());
    }
    if b_boot != c_boot {
        return ComparisonOutcome::Mismatch("boot-id-mismatch".to_string());
    }

    // Row 4a: start_identity.profile equality
    if b_proc["start_identity"]["profile"] != c_proc["start_identity"]["profile"] {
        return ComparisonOutcome::Insufficient("incompatible-start-identity-profile".to_string());
    }

    // Rows 4b-4k: namespace rules
    match base_sub {
        StartSubprofile::LinuxProcfsStatV0 => {
            if b_proc["pid_numbering_namespace"]["status"].as_str() != Some("observed")
                || c_proc["pid_numbering_namespace"]["status"].as_str() != Some("observed")
            {
                return ComparisonOutcome::Insufficient(
                    "missing-pid-numbering-namespace".to_string(),
                );
            }
            let b_pns = b_proc["pid_numbering_namespace"]["value"]
                .as_str()
                .unwrap_or("");
            let c_pns = c_proc["pid_numbering_namespace"]["value"]
                .as_str()
                .unwrap_or("");
            if !is_linux_scoped_ns_id(b_pns, "pidns") || !is_linux_scoped_ns_id(c_pns, "pidns") {
                return ComparisonOutcome::Insufficient("invalid-pid-namespace".to_string());
            }
            if b_pns != c_pns {
                return ComparisonOutcome::Insufficient(
                    "pid-numbering-namespace-mismatch".to_string(),
                );
            }

            // Rows 4e-4g: observer.time_namespace
            let b_tns = &baseline["observer"]["time_namespace"];
            let c_tns = &candidate["observer"]["time_namespace"];
            let b_tns_st = b_tns["status"].as_str().unwrap_or("");
            let c_tns_st = c_tns["status"].as_str().unwrap_or("");
            if !matches!(b_tns_st, "observed" | "not-present")
                || !matches!(c_tns_st, "observed" | "not-present")
            {
                return ComparisonOutcome::Insufficient(
                    "missing-observer-time-namespace".to_string(),
                );
            }
            if (b_tns_st == "observed"
                && !is_linux_scoped_ns_id(b_tns["value"].as_str().unwrap_or(""), "timens"))
                || (c_tns_st == "observed"
                    && !is_linux_scoped_ns_id(c_tns["value"].as_str().unwrap_or(""), "timens"))
            {
                return ComparisonOutcome::Insufficient(
                    "invalid-observer-time-namespace".to_string(),
                );
            }
            if b_tns_st != c_tns_st || (b_tns_st == "observed" && b_tns["value"] != c_tns["value"])
            {
                return ComparisonOutcome::Insufficient(
                    "observer-time-namespace-mismatch".to_string(),
                );
            }

            // Rows 4h-4j: observer.user_namespace
            let b_uns = &baseline["observer"]["user_namespace"];
            let c_uns = &candidate["observer"]["user_namespace"];
            let b_uns_st = b_uns["status"].as_str().unwrap_or("");
            let c_uns_st = c_uns["status"].as_str().unwrap_or("");
            if !matches!(b_uns_st, "observed" | "not-present")
                || !matches!(c_uns_st, "observed" | "not-present")
            {
                return ComparisonOutcome::Insufficient(
                    "missing-observer-user-namespace".to_string(),
                );
            }
            if (b_uns_st == "observed"
                && !is_linux_scoped_ns_id(b_uns["value"].as_str().unwrap_or(""), "userns"))
                || (c_uns_st == "observed"
                    && !is_linux_scoped_ns_id(c_uns["value"].as_str().unwrap_or(""), "userns"))
            {
                return ComparisonOutcome::Insufficient(
                    "invalid-observer-user-namespace".to_string(),
                );
            }
            if b_uns_st != c_uns_st || (b_uns_st == "observed" && b_uns["value"] != c_uns["value"])
            {
                return ComparisonOutcome::Insufficient(
                    "observer-user-namespace-mismatch".to_string(),
                );
            }
        }
        StartSubprofile::MacosKinfoStarttimeV0 => {
            // Row 4k
            for item in [baseline, candidate] {
                let proc_item = &item["process"];
                if proc_item["pid_numbering_namespace"]["status"].as_str() != Some("unsupported")
                    || proc_item["target_pid_namespace"]["status"].as_str() != Some("unsupported")
                    || item["observer"]["time_namespace"]["status"].as_str() != Some("unsupported")
                    || item["observer"]["user_namespace"]["status"].as_str() != Some("unsupported")
                {
                    return ComparisonOutcome::Insufficient("inconsistent-observation".to_string());
                }
            }
        }
    }

    // Rows 5a-5b: pid range & equality
    let b_pid = b_proc["pid"].as_str().unwrap_or("");
    let c_pid = c_proc["pid"].as_str().unwrap_or("");
    if !is_valid_pid_string(b_pid) || !is_valid_pid_string(c_pid) {
        return ComparisonOutcome::Insufficient("invalid-pid-range".to_string());
    }
    if b_pid != c_pid {
        return ComparisonOutcome::Mismatch("pid-mismatch".to_string());
    }

    // Row 5c: candidate process exited
    if b_proc["process_state"]["status"].as_str() == Some("observed")
        && b_proc["start_identity"]["status"].as_str() == Some("observed")
        && (c_proc["process_state"]["status"].as_str() == Some("not-present")
            || c_proc["start_identity"]["status"].as_str() == Some("not-present"))
    {
        return ComparisonOutcome::Mismatch("process-exited".to_string());
    }

    // Rows 5d-5f: Linux target_pid_namespace
    if base_sub == StartSubprofile::LinuxProcfsStatV0 {
        if b_proc["target_pid_namespace"]["status"].as_str() != Some("observed")
            || c_proc["target_pid_namespace"]["status"].as_str() != Some("observed")
        {
            return ComparisonOutcome::Insufficient("missing-target-pid-namespace".to_string());
        }
        let b_tpns = b_proc["target_pid_namespace"]["value"]
            .as_str()
            .unwrap_or("");
        let c_tpns = c_proc["target_pid_namespace"]["value"]
            .as_str()
            .unwrap_or("");
        if !is_linux_scoped_ns_id(b_tpns, "pidns") || !is_linux_scoped_ns_id(c_tpns, "pidns") {
            return ComparisonOutcome::Insufficient("invalid-pid-namespace".to_string());
        }
        if b_tpns != c_tpns {
            return ComparisonOutcome::Mismatch("target-pid-namespace-mismatch".to_string());
        }
    }

    // Rows 6a-6c: start_identity
    if b_proc["start_identity"]["status"].as_str() != Some("observed")
        || c_proc["start_identity"]["status"].as_str() != Some("observed")
    {
        return ComparisonOutcome::Insufficient("missing-start-identity".to_string());
    }
    let b_start = b_proc["start_identity"]["value"].as_str().unwrap_or("");
    let c_start = c_proc["start_identity"]["value"].as_str().unwrap_or("");
    if !base_sub.validate_start_value(b_start) || !cand_sub.validate_start_value(c_start) {
        return ComparisonOutcome::Insufficient("invalid-start-identity".to_string());
    }
    if b_start != c_start {
        return ComparisonOutcome::Mismatch("start-identity-mismatch".to_string());
    }

    // Rows 7a-7b: process_state
    if b_proc["process_state"]["status"].as_str() != Some("observed")
        || c_proc["process_state"]["status"].as_str() != Some("observed")
    {
        return ComparisonOutcome::Insufficient("missing-process-state".to_string());
    }
    if policy.require_live && c_proc["process_state"]["value"].as_str() != Some("live") {
        return ComparisonOutcome::Mismatch("process-not-live".to_string());
    }

    // Rows 8a-8d: execution_context in canonical order
    let b_ctx = &b_proc["execution_context"];
    let c_ctx = &c_proc["execution_context"];

    // Step 8a: accepted derivation & principal namespace/grammar/u32 bounds on all observed facts
    for &fact in CANONICAL_CONTEXT_FACTS {
        for (side_obs, side_ctx) in [(baseline, b_ctx), (candidate, c_ctx)] {
            if side_ctx[fact]["status"].as_str() == Some("observed")
                && !validate_context_derivation(fact, &side_ctx[fact], side_obs)
            {
                return ComparisonOutcome::Insufficient(format!(
                    "unsupported-context-derivation-{fact}"
                ));
            }
        }
    }

    // Step 8b (Pass 1): missing required facts in canonical order
    for &fact in CANONICAL_CONTEXT_FACTS {
        if req_facts_set.contains(fact)
            && (b_ctx[fact]["status"].as_str() != Some("observed")
                || c_ctx[fact]["status"].as_str() != Some("observed"))
        {
            return ComparisonOutcome::Insufficient(format!("missing-context-{fact}"));
        }
    }

    // Step 8c (Pass 2): derivation compatibility in canonical order
    for &fact in CANONICAL_CONTEXT_FACTS {
        let both_obs = b_ctx[fact]["status"].as_str() == Some("observed")
            && c_ctx[fact]["status"].as_str() == Some("observed");
        if both_obs
            && (req_facts_set.contains(fact) || policy.detect_observed_context_drift)
            && b_ctx[fact]["derivation"] != c_ctx[fact]["derivation"]
        {
            return ComparisonOutcome::Insufficient(format!(
                "incompatible-context-derivation-{fact}"
            ));
        }
    }

    // Step 8d (Pass 3): value/digest equality in canonical order
    for &fact in CANONICAL_CONTEXT_FACTS {
        let both_obs = b_ctx[fact]["status"].as_str() == Some("observed")
            && c_ctx[fact]["status"].as_str() == Some("observed");
        if both_obs && (req_facts_set.contains(fact) || policy.detect_observed_context_drift) {
            if fact != "security_principal"
                && (!b_ctx[fact]["digest"].as_str().is_some_and(is_sha256_digest)
                    || !c_ctx[fact]["digest"].as_str().is_some_and(is_sha256_digest))
            {
                return ComparisonOutcome::Insufficient("invalid-observation-shape".to_string());
            }
            let payload_key = if fact == "security_principal" {
                "principal"
            } else {
                "digest"
            };
            if b_ctx[fact][payload_key] != c_ctx[fact][payload_key] {
                return ComparisonOutcome::Mismatch(format!("context-changed-{fact}"));
            }
        }
    }

    if validate_observation_shape(baseline).is_err()
        || validate_observation_shape(candidate).is_err()
    {
        return ComparisonOutcome::Insufficient("invalid-observation-shape".to_string());
    }

    // Row 9
    ComparisonOutcome::Match
}

/// JSON-value entrypoint for [`project_to_local_process_session_process`] so
/// untrusted/test-vector policy inputs (`null`, integer `1`, `"true"`, etc.)
/// enforce strict type checks (`invalid-evaluation-policy`, `trusted-clock-unavailable`).
pub fn project_to_local_process_session_process_json(
    obs: &Value,
    host_scope_trusted: &Value,
    now_utc: &Value,
    accepted_profile: &ProfileRef,
    report: Option<&Value>,
    input_reports: Option<&BTreeMap<String, Value>>,
    allowed_clone_behaviors: Option<&Value>,
) -> Result<Value, EvidenceError> {
    let Some(trusted) = host_scope_trusted.as_bool() else {
        return Err(EvidenceError::new("invalid-evaluation-policy"));
    };
    let parsed_clones: Option<Vec<String>> = match allowed_clone_behaviors {
        None | Some(Value::Null) => None,
        Some(v) => {
            let Some(arr) = v.as_array() else {
                return Err(EvidenceError::new("invalid-evaluation-policy"));
            };
            if arr.is_empty() {
                return Err(EvidenceError::new("invalid-evaluation-policy"));
            }
            let mut list = Vec::with_capacity(arr.len());
            for item in arr {
                let Some(s) = item.as_str() else {
                    return Err(EvidenceError::new("invalid-evaluation-policy"));
                };
                if !CLONE_BEHAVIORS.contains(&s) {
                    return Err(EvidenceError::new("invalid-evaluation-policy"));
                }
                list.push(s.to_string());
            }
            Some(list)
        }
    };
    project_to_local_process_session_process(
        obs,
        trusted,
        now_utc.as_str(),
        accepted_profile,
        report,
        input_reports,
        parsed_clones.as_deref(),
    )
}

/// Fail-closed adapter projecting a `native-process-observation:v0` payload into
/// a `local-process-session:v0` `process` object.
pub fn project_to_local_process_session_process(
    obs: &Value,
    host_scope_trusted: bool,
    now_utc: Option<&str>,
    accepted_profile: &ProfileRef,
    report: Option<&Value>,
    input_reports: Option<&BTreeMap<String, Value>>,
    allowed_clone_behaviors: Option<&[String]>,
) -> Result<Value, EvidenceError> {
    if let Some(clones) = allowed_clone_behaviors {
        if clones.is_empty()
            || clones
                .iter()
                .any(|c| !CLONE_BEHAVIORS.contains(&c.as_str()))
        {
            return Err(EvidenceError::new("invalid-evaluation-policy"));
        }
    }

    admit_live_observation_steps(obs, now_utc, accepted_profile, report, input_reports)?;

    if obs["observer"]["bracketed"].as_bool() != Some(true) {
        return Err(EvidenceError::new("unbracketed-observation"));
    }
    let Some(sub) = StartSubprofile::lookup(obs) else {
        return Err(EvidenceError::new(
            "insufficient:unsupported-start-identity-profile",
        ));
    };
    if obs["observer"]["platform_family"].as_str() != Some(sub.platform_family()) {
        return Err(EvidenceError::new("insufficient:inconsistent-observation"));
    }

    let proc_obj = &obs["process"];
    let stab = proc_obj["host_scope"]["stability"].as_str().unwrap_or("");
    if !host_scope_trusted || !ESTABLISHED_STABILITIES.contains(&stab) {
        return Err(EvidenceError::new("unestablished-host-scope"));
    }
    if let Some(clones) = allowed_clone_behaviors {
        let cb = proc_obj["host_scope"]["clone_behavior"]
            .as_str()
            .unwrap_or("");
        if !clones.iter().any(|c| c == cb) {
            return Err(EvidenceError::new("disallowed-clone-behavior"));
        }
    }

    if proc_obj["process_state"]["status"].as_str() != Some("observed")
        || proc_obj["process_state"]["value"].as_str() != Some("live")
    {
        return Err(EvidenceError::new("process-not-live"));
    }
    if proc_obj["boot_id"]["status"].as_str() != Some("observed")
        || !proc_obj["boot_id"]["value"]
            .as_str()
            .is_some_and(is_lowercase_uuid)
    {
        return Err(EvidenceError::new("missing-or-invalid-boot-id"));
    }
    if proc_obj["start_identity"]["status"].as_str() != Some("observed")
        || !proc_obj["start_identity"]["value"]
            .as_str()
            .is_some_and(|v| sub.validate_start_value(v))
    {
        return Err(EvidenceError::new("missing-or-invalid-start-identity"));
    }

    if proc_obj["pid_numbering_namespace"]["status"].as_str() != Some("observed")
        || proc_obj["target_pid_namespace"]["status"].as_str() != Some("observed")
    {
        return Err(EvidenceError::new(
            "missing-pid-namespace-for-session-bundle",
        ));
    }
    let pns = proc_obj["pid_numbering_namespace"]["value"]
        .as_str()
        .unwrap_or("");
    let tpns = proc_obj["target_pid_namespace"]["value"]
        .as_str()
        .unwrap_or("");
    if !is_linux_scoped_ns_id(pns, "pidns") || !is_linux_scoped_ns_id(tpns, "pidns") || pns != tpns
    {
        return Err(EvidenceError::new(
            "ambiguous-pid-namespace-for-session-bundle",
        ));
    }

    let tns = &obs["observer"]["time_namespace"];
    let tns_st = tns["status"].as_str().unwrap_or("");
    if !matches!(tns_st, "observed" | "not-present")
        || (tns_st == "observed"
            && !tns["value"]
                .as_str()
                .is_some_and(|v| is_linux_scoped_ns_id(v, "timens")))
    {
        return Err(EvidenceError::new(
            "invalid-observer-time-namespace-for-session-bundle",
        ));
    }

    let uns = &obs["observer"]["user_namespace"];
    let uns_st = uns["status"].as_str().unwrap_or("");
    if !matches!(uns_st, "observed" | "not-present")
        || (uns_st == "observed"
            && !uns["value"]
                .as_str()
                .is_some_and(|v| is_linux_scoped_ns_id(v, "userns")))
    {
        return Err(EvidenceError::new(
            "invalid-observer-user-namespace-for-session-bundle",
        ));
    }

    if !proc_obj["pid"].as_str().is_some_and(is_valid_pid_string) {
        return Err(EvidenceError::new("invalid-pid-range"));
    }

    let ctx = &proc_obj["execution_context"];
    for &fact in CANONICAL_CONTEXT_FACTS {
        if ctx[fact]["status"].as_str() != Some("observed") {
            return Err(EvidenceError::new(format!(
                "missing-context-for-session-bundle:{fact}"
            )));
        }
        if !validate_context_derivation(fact, &ctx[fact], obs) {
            return Err(EvidenceError::new(format!(
                "unsupported-context-derivation:{fact}"
            )));
        }
    }

    let projected_host_scope = project_host_scope_urn(&proc_obj["host_scope"])?;
    if projected_host_scope.len() > 512 {
        return Err(EvidenceError::new("projected-host-scope-too-long"));
    }

    let clock_domain = if tns_st == "observed" {
        tns["value"].as_str().unwrap_or("")
    } else {
        "host-boottime"
    };
    let start_ticks = proc_obj["start_identity"]["value"].as_str().unwrap_or("");
    let projected_start_value = format!("{clock_domain};ticks={start_ticks}");
    if projected_start_value.len() > 512 {
        return Err(EvidenceError::new("projected-start-identity-too-long"));
    }

    validate_observation_shape(obs)?;

    Ok(json!({
        "subject_ref": proc_obj["subject_ref"],
        "host_scope": projected_host_scope,
        "boot_id": proc_obj["boot_id"]["value"],
        "pid_namespace": tpns,
        "pid": proc_obj["pid"],
        "start_identity": {
            "profile": {
                "id": LINUX_PROCFS_STAT_TIMENS_SCOPED_PROFILE_ID,
                "revision": NATIVE_PROCESS_OBSERVATION_README_REVISION,
            },
            "value": projected_start_value,
        },
        "execution_context": {
            "security_principal": ctx["security_principal"]["principal"],
            "namespace_context_digest": ctx["namespace_context_digest"]["digest"],
            "executable_digest": ctx["executable_digest"]["digest"],
            "arguments_digest": ctx["arguments_digest"]["digest"],
            "security_context_digest": ctx["security_context_digest"]["digest"],
        },
    }))
}

/// Verify observation-only `identity-evidence/v0` report descriptor binding to a
/// `native-process-observation:v0` payload (`bind_native_observation_report`).
pub fn bind_native_observation_report(
    report: &Value,
    obs: &Value,
    accepted_profile: &ProfileRef,
) -> Result<(), EvidenceError> {
    let expected_prof = json!({
        "id": accepted_profile.id,
        "revision": accepted_profile.revision,
    });
    if obs.get("profile") != Some(&expected_prof) {
        return Err(EvidenceError::new("unsupported-evidence-profile"));
    }
    let Some(assessments) = report.get("assessments").and_then(Value::as_array) else {
        return Err(EvidenceError::new("invalid-report-shape"));
    };
    if !assessments.is_empty() || report.get("evaluator").is_some() {
        return Err(EvidenceError::new(
            "observation-report-not-observation-only",
        ));
    }

    let subjects: BTreeMap<&str, &Value> = report
        .get("subjects")
        .and_then(Value::as_array)
        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?
        .iter()
        .filter_map(|s| Some((s.get("id")?.as_str()?, s)))
        .collect();
    let proc_obj = &obs["process"];
    let proc_subj = proc_obj["subject_ref"].as_str().unwrap_or("");
    if subjects
        .get(proc_subj)
        .and_then(|s| s.get("kind"))
        .and_then(Value::as_str)
        != Some("process")
    {
        return Err(EvidenceError::new("observation-subject-not-process"));
    }

    if let Some(ident_ref) = proc_obj["host_scope"]
        .get("identifier_ref")
        .and_then(Value::as_str)
    {
        let ident = report
            .get("identifiers")
            .and_then(Value::as_array)
            .and_then(|arr| {
                arr.iter()
                    .find(|x| x.get("id").and_then(Value::as_str) == Some(ident_ref))
            });
        let ok = ident.is_some_and(|id_obj| {
            let subj_kind = id_obj
                .get("subject_ref")
                .and_then(Value::as_str)
                .and_then(|sr| subjects.get(sr))
                .and_then(|s| s.get("kind"))
                .and_then(Value::as_str);
            matches!(
                subj_kind,
                Some("os-installation" | "physical-device" | "virtual-machine" | "container")
            ) && [
                "namespace",
                "value",
                "derivation",
                "stability",
                "clone_behavior",
            ]
            .iter()
            .all(|k| id_obj.get(*k) == proc_obj["host_scope"].get(*k))
        });
        if !ok {
            return Err(EvidenceError::new("host-scope-identifier-mismatch"));
        }
    }

    let obs_digest = canonical_sha256_value(obs).map_err(|w| EvidenceError::new(w.code))?;
    let matched: Vec<&Value> = report
        .get("evidence")
        .and_then(Value::as_array)
        .ok_or_else(|| EvidenceError::new("invalid-report-shape"))?
        .iter()
        .filter(|e| {
            e.get("format_profile") == Some(&expected_prof)
                || e.get("digest").and_then(Value::as_str) == Some(obs_digest.as_str())
        })
        .collect();
    if matched.len() != 1 {
        return Err(EvidenceError::new(
            "observation-evidence-descriptor-missing",
        ));
    }
    let ev = matched[0];
    if ev.get("format_profile") != Some(&expected_prof)
        || ev.get("digest").and_then(Value::as_str) != Some(obs_digest.as_str())
        || ev.get("method").and_then(Value::as_str) != Some("os-observation")
        || ev.get("subject_ref") != proc_obj.get("subject_ref")
        || ev.get("collected_at") != obs.get("observed_at")
        || ev.get("key_ref").is_some()
        || ev.get("identifier_ref").is_some()
        || ev.get("boundary").is_some()
    {
        return Err(EvidenceError::new(
            "observation-evidence-descriptor-mismatch",
        ));
    }
    validate_report_shape(report)?;
    validate_observation_shape(obs)
}
