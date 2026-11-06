// SPDX-License-Identifier: MIT OR Apache-2.0

//! Read-only OS collectors and private continuity handles for
//! `urn:3leaps:identity-evidence:native-process-observation:v0`.
//!
//! Authority boundary:
//! - Collection and continuity handles use strictly non-destructive read-only
//!   OS facilities (`sysctl` + `kqueue` `EVFILT_PROC` `NOTE_EXIT` on macOS;
//!   descriptor-anchored `/proc` via `openat2` + `pidfd_open` on Linux).
//! - Live OS handles are private in-memory resources: they expose no raw file
//!   descriptor getter, expose no signal API, and are never serialized.
//! - `HostScope` is supplied across an explicit caller boundary and does not
//!   authenticate physical-machine uniqueness.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use sysprims_evidence::NATIVE_PROCESS_OBSERVATION_README_REVISION;
use sysprims_evidence::{
    admit_and_validate_report, admit_live_observation, bind_native_observation_report,
    canonical_sha256_value, compare_native_observations, parse_strict_utc_unix,
    validate_observation_shape, AdmittedWire, ComparisonOutcome, ComparisonPolicy, ProfileRef,
    IDENTITY_EVIDENCE_CONTRACT, NATIVE_PROCESS_OBSERVATION_PROFILE_ID,
    NATIVE_PROCESS_OBSERVATION_PROFILE_REVISION,
};
use thiserror::Error;
use time::OffsetDateTime;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod other;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use other as imp;

/// Minimum observable numeric PID (`1`).
pub const MIN_OBSERVABLE_PID: u32 = 1;
/// Maximum observable numeric PID (`2147483647`, `i32::MAX`).
pub const MAX_OBSERVABLE_PID: u32 = 2_147_483_647;
/// Maximum raw `argv` / `cmdline` / `KERN_PROCARGS2` buffer size (`262,144` bytes).
pub const MAX_ARGUMENTS_RAW_BYTES: usize = 262_144;
/// Maximum regular-file executable size hashed for `executable_digest` (`67,108,864` bytes, 64 MiB).
pub const MAX_EXECUTABLE_FILE_BYTES: u64 = 67_108_864;

/// Start-identity sub-profile ID for macOS `kinfo_proc.kp_proc.p_starttime`.
pub const MACOS_START_IDENTITY_PROFILE_ID: &str =
    "urn:3leaps:identity-evidence:start-identity:macos-kinfo-starttime:v0";
/// Start-identity sub-profile ID for Linux `/proc/<pid>/stat` field 22 ticks.
pub const LINUX_START_IDENTITY_PROFILE_ID: &str =
    "urn:3leaps:identity-evidence:start-identity:linux-procfs-stat:v0";
/// Security-principal derivation ID for macOS `kinfo_proc` credentials.
pub const MACOS_PRINCIPAL_DERIVATION_ID: &str =
    "urn:3leaps:identity-evidence:principal-derivation:macos-kinfo-ucred:v0";
/// Security-principal derivation ID for Linux `/proc/<pid>/status` UID/GID sets.
pub const LINUX_PRINCIPAL_DERIVATION_ID: &str =
    "urn:3leaps:identity-evidence:principal-derivation:linux-procfs-status-uidgid:v0";
/// Context derivation ID for `arguments_digest`.
pub const ARGV_DIGEST_DERIVATION_ID: &str =
    "urn:3leaps:identity-evidence:context-derivation:argv-bytes:v0";
/// Context derivation ID for `executable_digest`.
pub const EXEC_DIGEST_DERIVATION_ID: &str =
    "urn:3leaps:identity-evidence:context-derivation:executable-file-bytes:v0";
/// Context derivation ID for Linux `namespace_context_digest`.
pub const LINUX_NS_DIGEST_DERIVATION_ID: &str =
    "urn:3leaps:identity-evidence:context-derivation:linux-ns-inum-set:v0";
/// Context derivation ID for Linux `security_context_digest`.
pub const LINUX_SECCTX_DIGEST_DERIVATION_ID: &str =
    "urn:3leaps:identity-evidence:context-derivation:linux-procfs-status-secctx:v0";

const HOST_STABILITIES: &[&str] = &[
    "unknown",
    "process",
    "boot",
    "installation",
    "enrollment",
    "device",
];
const CLONE_BEHAVIORS: &[&str] = &["unknown", "copyable", "regenerated", "device-bound"];

/// Pinned `{id, revision}` reference to `native-process-observation:v0` `profile.json`.
pub fn accepted_observation_profile() -> ProfileRef {
    ProfileRef::new(
        NATIVE_PROCESS_OBSERVATION_PROFILE_ID,
        NATIVE_PROCESS_OBSERVATION_PROFILE_REVISION,
    )
}

/// Caller-supplied host-scope descriptor (`process.host_scope`).
///
/// This descriptor crosses an explicit caller trust boundary; it does not
/// self-authenticate physical-machine uniqueness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostScope {
    /// Optional report-local identifier ID (`^[a-z][a-z0-9-]{0,63}$`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier_ref: Option<String>,
    /// Identifier namespace (`1..=512` Unicode scalar values).
    pub namespace: String,
    /// Identifier value (`1..=512` Unicode scalar values).
    pub value: String,
    /// Pinned derivation `{id, revision}`.
    pub derivation: ProfileRef,
    /// Lifecycle stability (`unknown | process | boot | installation | enrollment | device`).
    pub stability: String,
    /// Clone behavior (`unknown | copyable | regenerated | device-bound`).
    pub clone_behavior: String,
}

impl HostScope {
    /// Validate against `native-process-observation/evidence.schema.json` and encode as JSON.
    pub fn to_json(&self) -> Result<Value, CollectorRefusal> {
        if let Some(ref id_ref) = self.identifier_ref {
            if !is_local_id(id_ref) {
                return Err(CollectorRefusal::error(
                    "invalid-host-scope",
                    "host_scope.identifier_ref must match ^[a-z][a-z0-9-]{0,63}$",
                ));
            }
        }
        if !is_nonempty_512(&self.namespace)
            || !is_nonempty_512(&self.value)
            || !is_nonempty_512(&self.derivation.id)
            || !is_sha256_digest(&self.derivation.revision)
            || !HOST_STABILITIES.contains(&self.stability.as_str())
            || !CLONE_BEHAVIORS.contains(&self.clone_behavior.as_str())
        {
            return Err(CollectorRefusal::error(
                "invalid-host-scope",
                "host_scope fields violate native-process-observation schema constraints",
            ));
        }
        let mut obj = Map::new();
        if let Some(ref id_ref) = self.identifier_ref {
            obj.insert("identifier_ref".into(), Value::String(id_ref.clone()));
        }
        obj.insert("namespace".into(), Value::String(self.namespace.clone()));
        obj.insert("value".into(), Value::String(self.value.clone()));
        obj.insert(
            "derivation".into(),
            json!({
                "id": self.derivation.id,
                "revision": self.derivation.revision,
            }),
        );
        obj.insert("stability".into(), Value::String(self.stability.clone()));
        obj.insert(
            "clone_behavior".into(),
            Value::String(self.clone_behavior.clone()),
        );
        Ok(Value::Object(obj))
    }
}

/// Disclosure and probe budget options for `observe_process` and `open_continuity_handle`.
///
/// By default, `probe_security_principal` is `true` and all four digest context
/// facts (`namespace_context_digest`, `executable_digest`, `arguments_digest`,
/// `security_context_digest`) are `false` (`status: "not-probed"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationOptions {
    /// Report-local process subject ID (`^[a-z][a-z0-9-]{0,63}$`, default `"process-target"`).
    pub subject_ref: String,
    /// Optional explicit UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`); defaults to current system UTC time.
    pub observed_at: Option<String>,
    /// Probe `execution_context.security_principal` (default `true`).
    pub probe_security_principal: bool,
    /// Opt in to `execution_context.namespace_context_digest` (default `false`).
    pub include_namespace_context_digest: bool,
    /// Opt in to `execution_context.executable_digest` (default `false`).
    pub include_executable_digest: bool,
    /// Opt in to `execution_context.arguments_digest` (default `false`).
    pub include_arguments_digest: bool,
    /// Opt in to `execution_context.security_context_digest` (default `false`).
    pub include_security_context_digest: bool,
}

impl Default for ObservationOptions {
    fn default() -> Self {
        Self {
            subject_ref: "process-target".to_string(),
            observed_at: None,
            probe_security_principal: true,
            include_namespace_context_digest: false,
            include_executable_digest: false,
            include_arguments_digest: false,
            include_security_context_digest: false,
        }
    }
}

impl ObservationOptions {
    /// Enable all opt-in execution-context facts supported by the platform.
    pub fn with_all_context_facts(mut self) -> Self {
        self.probe_security_principal = true;
        self.include_namespace_context_digest = true;
        self.include_executable_digest = true;
        self.include_arguments_digest = true;
        self.include_security_context_digest = true;
        self
    }
}

/// Deterministic collector or continuity-handle refusal.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{status}:{code}: {detail}")]
pub struct CollectorRefusal {
    /// 7-state capability/fact status category (`unsupported`, `inaccessible`, `not-present`, `error`, `unknown`).
    pub status: String,
    /// Stable machine-readable refusal code.
    pub code: String,
    /// Diagnostic detail (not exported into portable observation payloads).
    pub detail: String,
}

impl CollectorRefusal {
    /// Construct a new [`CollectorRefusal`].
    pub fn new(
        status: impl Into<String>,
        code: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            status: status.into(),
            code: code.into(),
            detail: detail.into(),
        }
    }

    /// Construct an `"unsupported"` collector refusal.
    pub fn unsupported(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new("unsupported", code, detail)
    }

    /// Construct an `"inaccessible"` collector refusal.
    pub fn inaccessible(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new("inaccessible", code, detail)
    }

    /// Construct a `"not-present"` collector refusal.
    pub fn not_present(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new("not-present", code, detail)
    }

    /// Construct an `"error"` collector refusal.
    pub fn error(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new("error", code, detail)
    }
}

/// Optional live continuity-handle probe outcome to record in `report.capabilities[]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityProbe {
    /// Adapter provider slug (e.g. `"macos-kqueue-proc"` or `"linux-pidfd"`).
    pub provider: String,
    /// Report capability status (`available | not-present | inaccessible | unsupported | not-probed | error | unknown`).
    pub status: String,
    /// Human-readable reason (required when `status != "available"`).
    pub reason: String,
}

/// Envelope metadata for wrapping a collected observation in an observation-only
/// `identity-evidence/v0` report (`assessments: []`, `evaluator` omitted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportMetadata {
    /// Report identifier (`^[a-z][a-z0-9-]{0,63}$`).
    pub report_id: String,
    /// Optional report `created_at` (`YYYY-MM-DDTHH:MM:SSZ`, defaults to `observation.observed_at`).
    pub created_at: Option<String>,
    /// Producer name (`1..=128` chars).
    pub producer_name: String,
    /// Producer revision (`sha256:<64 lowercase hex>`).
    pub producer_revision: String,
    /// Report-local host subject ID (`^[a-z][a-z0-9-]{0,63}$`, distinct from `process.subject_ref`).
    pub host_subject_id: String,
    /// Report-local evidence descriptor ID (`^[a-z][a-z0-9-]{0,63}$`).
    pub evidence_id: String,
    /// Evidence `source` label (e.g. `"macos-sysctl-proc"` or `"linux-procfs-openat2"`).
    pub source: String,
    /// Evidence `collector` label.
    pub collector: String,
    /// Optional capability entry describing live exit-handle acquisition.
    pub exit_handle_capability: Option<CapabilityProbe>,
}

impl Default for ReportMetadata {
    fn default() -> Self {
        Self {
            report_id: "native-process-observation".to_string(),
            created_at: None,
            producer_name: "sysprims-proc".to_string(),
            producer_revision:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            host_subject_id: "host-local".to_string(),
            evidence_id: "e-process-observation".to_string(),
            source: default_collector_source().to_string(),
            collector: "sysprims-proc".to_string(),
            exit_handle_capability: None,
        }
    }
}

/// Private, non-serializable read-only OS continuity handle (`kqueue` `EVFILT_PROC`
/// `NOTE_EXIT` on macOS; `pidfd` on Linux) bracketed at registration against
/// kernel start-identity metadata.
///
/// This type intentionally exposes no raw file descriptor accessor, implements
/// neither `Serialize` nor `Deserialize`, and provides no signal delivery API.
#[derive(Debug)]
pub struct ProcessContinuityHandle {
    pid: u32,
    baseline: Value,
    host_scope: HostScope,
    options: ObservationOptions,
    inner: imp::PlatformHandle,
}

impl ProcessContinuityHandle {
    /// Numeric PID (`1..=2147483647`) bound at handle creation.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Borrow the bracketed baseline observation captured across handle registration.
    pub fn baseline_observation(&self) -> &Value {
        &self.baseline
    }

    /// Non-blocking exit check (`true` once the bound incarnation has exited).
    pub fn poll_exited(&self) -> Result<bool, CollectorRefusal> {
        self.inner.poll_exited()
    }

    /// Wait up to `timeout` for the bound incarnation to exit (`Ok(true)` if exited,
    /// `Ok(false)` if `timeout` elapsed while still running).
    pub fn wait_exit(&self, timeout: Duration) -> Result<bool, CollectorRefusal> {
        self.inner.wait_exit(timeout)
    }

    /// Collect a fresh read-only observation for `self.pid` using the stored
    /// `HostScope` and `ObservationOptions`.
    pub fn reobserve(&self, observed_at: Option<&str>) -> Result<Value, CollectorRefusal> {
        let mut opts = self.options.clone();
        opts.observed_at = observed_at.map(str::to_string);
        observe_process(self.pid, &self.host_scope, &opts)
    }

    /// Re-observe `self.pid` at trusted UTC instant `now_utc`, enforce
    /// `admit_live_observation`, and evaluate `compare_native_observations`
    /// against [`Self::baseline_observation`].
    pub fn reverify(
        &self,
        policy: &ComparisonPolicy,
        now_utc: &str,
    ) -> Result<(Value, ComparisonOutcome), CollectorRefusal> {
        let candidate = self.reobserve(Some(now_utc))?;
        let accepted = accepted_observation_profile();
        admit_live_observation(&candidate, Some(now_utc), &accepted, None, None).map_err(|e| {
            CollectorRefusal::error(e.code(), "live observation freshness gate refused")
        })?;
        let outcome = compare_native_observations(&self.baseline, &candidate, policy, &accepted);
        Ok((candidate, outcome))
    }
}

/// Format current system wall clock as strict whole-second UTC (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn current_utc_timestamp() -> Result<String, CollectorRefusal> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            CollectorRefusal::error("trusted-clock-unavailable", "system clock pre-epoch")
        })?
        .as_secs();
    let secs_i64 = i64::try_from(secs)
        .map_err(|_| CollectorRefusal::error("trusted-clock-unavailable", "clock overflow"))?;
    let dt = OffsetDateTime::from_unix_timestamp(secs_i64)
        .map_err(|_| CollectorRefusal::error("trusted-clock-unavailable", "invalid unix time"))?;
    let formatted = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    );
    if parse_strict_utc_unix(&formatted).is_none() {
        return Err(CollectorRefusal::error(
            "trusted-clock-unavailable",
            "formatted timestamp outside four-digit UTC range",
        ));
    }
    Ok(formatted)
}

fn validate_common_inputs(
    pid: u32,
    host_scope: &HostScope,
    options: &ObservationOptions,
) -> Result<(Value, String), CollectorRefusal> {
    if !(MIN_OBSERVABLE_PID..=MAX_OBSERVABLE_PID).contains(&pid) {
        return Err(CollectorRefusal::error(
            "invalid-pid-range",
            "pid must be within 1..=2147483647",
        ));
    }
    if !is_local_id(&options.subject_ref) {
        return Err(CollectorRefusal::error(
            "invalid-subject-ref",
            "subject_ref must match ^[a-z][a-z0-9-]{0,63}$",
        ));
    }
    let host_scope_json = host_scope.to_json()?;
    let observed_at = match options.observed_at.as_deref() {
        Some(ts) => {
            if parse_strict_utc_unix(ts).is_none() {
                return Err(CollectorRefusal::error(
                    "invalid-observed-at",
                    "observed_at must be a valid YYYY-MM-DDTHH:MM:SSZ instant",
                ));
            }
            ts.to_string()
        }
        None => current_utc_timestamp()?,
    };
    Ok((host_scope_json, observed_at))
}

/// Collect a bracketed read-only `native-process-observation:v0` evidence payload
/// for `pid` (`1..=2147483647`).
///
/// Target absence (`ESRCH` / verified `ENOENT` without `hidepid` concealment)
/// returns an `Ok(Value)` payload whose `process_state` and `start_identity`
/// record `status: "not-present"` so downstream comparison deterministically
/// yields `mismatch:process-exited`. Collector-level precondition, platform, or
/// confinement failures return `Err(CollectorRefusal)`.
pub fn observe_process(
    pid: u32,
    host_scope: &HostScope,
    options: &ObservationOptions,
) -> Result<Value, CollectorRefusal> {
    let (host_scope_json, observed_at) = validate_common_inputs(pid, host_scope, options)?;
    let obs = imp::observe_process_impl(pid, host_scope_json, &observed_at, options)?;
    validate_observation_shape(&obs).map_err(|e| {
        CollectorRefusal::error(
            e.code(),
            "internal collector output failed native-process-observation schema validation",
        )
    })?;
    Ok(obs)
}

/// Open a private read-only continuity handle (`kqueue` `EVFILT_PROC` `NOTE_EXIT`
/// on macOS; `pidfd_open` on Linux) bracketed by pre/post incarnation checks.
///
/// Requires the target process to be observed in `"live"` state across handle
/// registration; absent or zombie targets return `Err(CollectorRefusal)`.
pub fn open_continuity_handle(
    pid: u32,
    host_scope: &HostScope,
    options: &ObservationOptions,
) -> Result<ProcessContinuityHandle, CollectorRefusal> {
    let (host_scope_json, observed_at) = validate_common_inputs(pid, host_scope, options)?;
    let (baseline, inner) =
        imp::open_continuity_handle_impl(pid, host_scope_json, &observed_at, options)?;
    validate_observation_shape(&baseline).map_err(|e| {
        CollectorRefusal::error(
            e.code(),
            "internal baseline output failed native-process-observation schema validation",
        )
    })?;
    Ok(ProcessContinuityHandle {
        pid,
        baseline,
        host_scope: host_scope.clone(),
        options: options.clone(),
        inner,
    })
}

/// Build and canonically admit an observation-only `identity-evidence/v0` report
/// (`assessments: []`, `evaluator` omitted) bound to `observation`.
pub fn build_observation_report(
    observation: &Value,
    metadata: &ReportMetadata,
) -> Result<AdmittedWire, CollectorRefusal> {
    validate_observation_shape(observation)
        .map_err(|e| CollectorRefusal::error(e.code(), "observation failed schema validation"))?;
    let observed_at = observation
        .get("observed_at")
        .and_then(Value::as_str)
        .ok_or_else(|| CollectorRefusal::error("invalid-observed-at", "missing observed_at"))?;
    let created_at = metadata.created_at.as_deref().unwrap_or(observed_at);
    if parse_strict_utc_unix(created_at).is_none() {
        return Err(CollectorRefusal::error(
            "invalid-report-metadata",
            "created_at must be strict UTC YYYY-MM-DDTHH:MM:SSZ",
        ));
    }
    let proc_obj = observation
        .get("process")
        .and_then(Value::as_object)
        .ok_or_else(|| CollectorRefusal::error("invalid-observation-shape", "missing process"))?;
    let proc_subject = proc_obj
        .get("subject_ref")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            CollectorRefusal::error("invalid-observation-shape", "missing subject_ref")
        })?;
    if !is_local_id(&metadata.report_id)
        || !is_local_id(&metadata.host_subject_id)
        || !is_local_id(&metadata.evidence_id)
        || metadata.host_subject_id == proc_subject
        || !is_sha256_digest(&metadata.producer_revision)
    {
        return Err(CollectorRefusal::error(
            "invalid-report-metadata",
            "report metadata IDs or digests violate report.schema.json constraints",
        ));
    }

    let payload_digest = canonical_sha256_value(observation)
        .map_err(|e| CollectorRefusal::error(e.code(), "failed to canonicalize observation"))?;

    let hs_obj = proc_obj
        .get("host_scope")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            CollectorRefusal::error("invalid-observation-shape", "missing host_scope")
        })?;
    let mut identifiers = Vec::new();
    if let Some(id_ref) = hs_obj.get("identifier_ref").and_then(Value::as_str) {
        identifiers.push(json!({
            "id": id_ref,
            "subject_ref": metadata.host_subject_id,
            "namespace": hs_obj.get("namespace").cloned().unwrap_or(Value::Null),
            "value": hs_obj.get("value").cloned().unwrap_or(Value::Null),
            "derivation": hs_obj.get("derivation").cloned().unwrap_or(Value::Null),
            "stability": hs_obj.get("stability").cloned().unwrap_or(Value::Null),
            "clone_behavior": hs_obj.get("clone_behavior").cloned().unwrap_or(Value::Null),
        }));
    }

    let provider = default_collector_source();
    let mut capabilities = Vec::new();
    if let Some(ref exit_cap) = metadata.exit_handle_capability {
        let mut cap = Map::new();
        cap.insert("id".into(), Value::String("cap-exit-handle".into()));
        cap.insert("subject_ref".into(), Value::String(proc_subject.into()));
        cap.insert("provider".into(), Value::String(exit_cap.provider.clone()));
        cap.insert(
            "capability".into(),
            Value::String("process-exit-observation".into()),
        );
        cap.insert("status".into(), Value::String(exit_cap.status.clone()));
        let reason = if exit_cap.reason.is_empty() {
            "Acquired private read-only process continuity handle.".to_string()
        } else {
            exit_cap.reason.clone()
        };
        cap.insert("reason".into(), Value::String(reason));
        cap.insert("observed_at".into(), Value::String(observed_at.into()));
        capabilities.push(Value::Object(cap));
    }

    if let Some(tpns) = proc_obj.get("target_pid_namespace") {
        capabilities.push(fact_to_capability(
            "cap-pid-namespace",
            proc_subject,
            provider,
            "pid-namespace-observation",
            tpns,
            observed_at,
        ));
    }
    if let Some(ec) = proc_obj.get("execution_context").and_then(Value::as_object) {
        for (cap_id, cap_name, key) in [
            (
                "cap-security-principal",
                "security-principal",
                "security_principal",
            ),
            (
                "cap-namespace-context-digest",
                "namespace-context-digest",
                "namespace_context_digest",
            ),
            (
                "cap-executable-digest",
                "executable-digest",
                "executable_digest",
            ),
            (
                "cap-arguments-digest",
                "arguments-digest",
                "arguments_digest",
            ),
            (
                "cap-security-context-digest",
                "security-context-digest",
                "security_context_digest",
            ),
        ] {
            if let Some(fact) = ec.get(key) {
                capabilities.push(fact_to_capability(
                    cap_id,
                    proc_subject,
                    provider,
                    cap_name,
                    fact,
                    observed_at,
                ));
            }
        }
    }

    let report_val = json!({
        "contract": IDENTITY_EVIDENCE_CONTRACT,
        "report_id": metadata.report_id,
        "created_at": created_at,
        "producer": {
            "name": metadata.producer_name,
            "revision": metadata.producer_revision,
        },
        "subjects": [
            {
                "id": metadata.host_subject_id,
                "kind": "os-installation",
            },
            {
                "id": proc_subject,
                "kind": "process",
            },
        ],
        "identifiers": identifiers,
        "evidence": [
            {
                "id": metadata.evidence_id,
                "subject_ref": proc_subject,
                "method": "os-observation",
                "source": metadata.source,
                "collector": metadata.collector,
                "collected_at": observed_at,
                "digest": payload_digest,
                "format_profile": {
                    "id": NATIVE_PROCESS_OBSERVATION_PROFILE_ID,
                    "revision": NATIVE_PROCESS_OBSERVATION_PROFILE_REVISION,
                },
            }
        ],
        "assessments": [],
        "capabilities": capabilities,
        "keys": [],
        "claims": [],
        "input_reports": [],
    });

    bind_native_observation_report(&report_val, observation, &accepted_observation_profile())
        .map_err(|e| CollectorRefusal::error(e.code(), "observation report binding failed"))?;

    let raw = serde_json::to_vec(&report_val)
        .map_err(|_| CollectorRefusal::error("invalid-json", "report serialization failed"))?;
    admit_and_validate_report(&raw, None)
        .map_err(|e| CollectorRefusal::error(e.code(), "report envelope validation failed"))
}

fn default_collector_source() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos-sysctl-proc"
    } else if cfg!(target_os = "linux") {
        "linux-procfs-openat2"
    } else {
        "unsupported-platform"
    }
}

fn fact_to_capability(
    id: &str,
    subject_ref: &str,
    provider: &str,
    capability: &str,
    fact: &Value,
    observed_at: &str,
) -> Value {
    let fact_status = fact
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let cap_status = if fact_status == "observed" {
        "available"
    } else {
        fact_status
    };
    let mut obj = Map::new();
    obj.insert("id".into(), Value::String(id.into()));
    obj.insert("subject_ref".into(), Value::String(subject_ref.into()));
    obj.insert("provider".into(), Value::String(provider.into()));
    obj.insert("capability".into(), Value::String(capability.into()));
    obj.insert("status".into(), Value::String(cap_status.into()));
    let reason = fact
        .get("reason")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("Observed within bracketed native process snapshot.");
    obj.insert("reason".into(), Value::String(reason.into()));
    obj.insert("observed_at".into(), Value::String(observed_at.into()));
    Value::Object(obj)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn observer_architecture() -> &'static str {
    std::env::consts::ARCH
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

pub(crate) fn is_nonempty_512(s: &str) -> bool {
    let n = s.chars().count();
    (1..=512).contains(&n)
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn is_lowercase_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 {
        return false;
    }
    for (idx, &ch) in b.iter().enumerate() {
        if matches!(idx, 8 | 13 | 18 | 23) {
            if ch != b'-' {
                return false;
            }
        } else if !(ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch)) {
            return false;
        }
    }
    true
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn fact_observed(value: impl Into<String>) -> Value {
    json!({
        "status": "observed",
        "value": value.into(),
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn fact_non_observed(status: &str, reason: &str) -> Value {
    json!({
        "status": status,
        "reason": reason,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn start_identity_observed(profile_id: &str, value: impl Into<String>) -> Value {
    json!({
        "status": "observed",
        "profile": {
            "id": profile_id,
            "revision": NATIVE_PROCESS_OBSERVATION_README_REVISION,
        },
        "value": value.into(),
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn start_identity_non_observed(profile_id: &str, status: &str, reason: &str) -> Value {
    json!({
        "status": status,
        "profile": {
            "id": profile_id,
            "revision": NATIVE_PROCESS_OBSERVATION_README_REVISION,
        },
        "reason": reason,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn principal_observed(
    derivation_id: &str,
    namespace: impl Into<String>,
    value: impl Into<String>,
) -> Value {
    json!({
        "status": "observed",
        "derivation": {
            "id": derivation_id,
            "revision": NATIVE_PROCESS_OBSERVATION_README_REVISION,
        },
        "principal": {
            "namespace": namespace.into(),
            "value": value.into(),
        },
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn digest_observed(derivation_id: &str, digest: impl Into<String>) -> Value {
    json!({
        "status": "observed",
        "derivation": {
            "id": derivation_id,
            "revision": NATIVE_PROCESS_OBSERVATION_README_REVISION,
        },
        "digest": digest.into(),
    })
}

/// Stream regular-file bytes from an already-opened descriptor up to
/// [`MAX_EXECUTABLE_FILE_BYTES`] (`67,108,864` bytes) into the domain-separated
/// `executable-file-bytes:v0` SHA-256 preimage without buffering the full image.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn stream_executable_digest_from_fd(raw_fd: libc::c_int) -> Value {
    use sha2::{Digest, Sha256};

    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    let fstat_rc = unsafe { libc::fstat(raw_fd, &mut st) };
    if fstat_rc != 0 {
        let err = std::io::Error::last_os_error();
        return errno_to_digest_fact(
            err.raw_os_error().unwrap_or(libc::EIO),
            "executable-fstat-failed",
        );
    }
    if (st.st_mode & libc::S_IFMT) != libc::S_IFREG {
        return fact_non_observed("error", "executable-not-regular-file");
    }
    if st.st_size < 0 {
        return fact_non_observed("error", "executable-negative-size");
    }
    if (st.st_size as u64) > MAX_EXECUTABLE_FILE_BYTES {
        return fact_non_observed("error", "executable-exceeds-size-cap");
    }

    let mut hasher = Sha256::new();
    hasher.update(b"3leaps.native-process-observation.executable-image.v0\0");

    let mut buf = [0u8; 65_536];
    let mut total: u64 = 0;
    loop {
        let n = unsafe { libc::read(raw_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n == 0 {
            break;
        }
        if n < 0 {
            let errno = std::io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or(libc::EIO);
            if errno == libc::EINTR {
                continue;
            }
            return errno_to_digest_fact(errno, "executable-read-failed");
        }
        total = total.saturating_add(n as u64);
        if total > MAX_EXECUTABLE_FILE_BYTES {
            return fact_non_observed("error", "executable-exceeds-size-cap");
        }
        hasher.update(&buf[..n as usize]);
    }

    let digest = format!("sha256:{:x}", hasher.finalize());
    digest_observed(EXEC_DIGEST_DERIVATION_ID, digest)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn errno_to_digest_fact(errno: i32, fallback_reason: &str) -> Value {
    match errno {
        libc::EPERM | libc::EACCES => fact_non_observed("inaccessible", "permission-denied"),
        libc::ENOENT | libc::ESRCH => fact_non_observed("not-present", "backing-entry-absent"),
        _ => fact_non_observed("error", fallback_reason),
    }
}
