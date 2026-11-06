// SPDX-License-Identifier: MIT OR Apache-2.0

// Package procobserve implements pure-Go (CGO_ENABLED=0) read-only OS process
// collectors and private continuity handles for
// urn:3leaps:identity-evidence:native-process-observation:v0.
//
// Authority boundary:
//   - Collection and continuity handles use strictly non-destructive read-only
//     OS facilities (sysctl + kqueue EVFILT_PROC NOTE_EXIT on macOS;
//     descriptor-anchored /proc via openat2 + pidfd_open on Linux).
//   - Live OS handles are private in-memory resources: they expose no raw file
//     descriptor accessor, cannot be JSON-marshaled, and expose no signal API.
//   - HostScope is supplied across an explicit caller trust boundary and does
//     not self-authenticate physical-machine uniqueness.
package procobserve

import (
	"encoding/json"
	"fmt"
	"runtime"
	"sync"
	"time"
	"unicode/utf8"

	"github.com/3leaps/sysprims/evidence"
)

const (
	// MinObservablePID is the minimum numeric PID accepted by native-process-observation:v0.
	MinObservablePID uint32 = 1
	// MaxObservablePID is the maximum numeric PID accepted by native-process-observation:v0 (2^31 - 1).
	MaxObservablePID uint32 = 2_147_483_647
	// MaxArgumentsRawBytes is the maximum raw argv/cmdline/KERN_PROCARGS2 read size (256 KiB).
	MaxArgumentsRawBytes int = 262_144
	// MaxExecutableFileBytes is the maximum regular-file executable size hashed (64 MiB).
	MaxExecutableFileBytes int64 = 67_108_864

	// MacOSStartIdentityProfileID is the macOS start-identity sub-profile URN.
	MacOSStartIdentityProfileID = "urn:3leaps:identity-evidence:start-identity:macos-kinfo-starttime:v0"
	// LinuxStartIdentityProfileID is the Linux start-identity sub-profile URN.
	LinuxStartIdentityProfileID = "urn:3leaps:identity-evidence:start-identity:linux-procfs-stat:v0"
	// MacOSPrincipalDerivationID is the macOS security_principal derivation URN.
	MacOSPrincipalDerivationID = "urn:3leaps:identity-evidence:principal-derivation:macos-kinfo-ucred:v0"
	// LinuxPrincipalDerivationID is the Linux security_principal derivation URN.
	LinuxPrincipalDerivationID = "urn:3leaps:identity-evidence:principal-derivation:linux-procfs-status-uidgid:v0"
	// ArgvDigestDerivationID is the arguments_digest derivation URN.
	ArgvDigestDerivationID = "urn:3leaps:identity-evidence:context-derivation:argv-bytes:v0"
	// ExecDigestDerivationID is the executable_digest derivation URN.
	ExecDigestDerivationID = "urn:3leaps:identity-evidence:context-derivation:executable-file-bytes:v0"
	// LinuxNSDigestDerivationID is the Linux namespace_context_digest derivation URN.
	LinuxNSDigestDerivationID = "urn:3leaps:identity-evidence:context-derivation:linux-ns-inum-set:v0"
	// LinuxSecCtxDigestDerivationID is the Linux security_context_digest derivation URN.
	LinuxSecCtxDigestDerivationID = "urn:3leaps:identity-evidence:context-derivation:linux-procfs-status-secctx:v0"

	defaultNotProbedReason = "Omitted under default opt-in disclosure policy."
)

// AcceptedObservationProfile returns the pinned {id, revision} reference for
// native-process-observation:v0 profile.json.
func AcceptedObservationProfile() evidence.ProfileRef {
	return evidence.ProfileRef{
		ID:       evidence.NativeProcessObservationProfileID,
		Revision: evidence.NativeProcessObservationProfileRevision,
	}
}

// HostScope describes the caller-supplied host installation/boot scope.
type HostScope struct {
	IdentifierRef string              `json:"identifier_ref,omitempty"`
	Namespace     string              `json:"namespace"`
	Value         string              `json:"value"`
	Derivation    evidence.ProfileRef `json:"derivation"`
	Stability     string              `json:"stability"`
	CloneBehavior string              `json:"clone_behavior"`
}

func isValidStability(s string) bool {
	switch s {
	case "unknown", "process", "boot", "installation", "enrollment", "device":
		return true
	default:
		return false
	}
}

func isValidCloneBehavior(s string) bool {
	switch s {
	case "unknown", "copyable", "regenerated", "device-bound":
		return true
	default:
		return false
	}
}

// ToMap validates HostScope against native-process-observation/evidence.schema.json
// and returns a fresh JSON-compatible map.
func (h HostScope) ToMap() (map[string]any, error) {
	if h.IdentifierRef != "" && !isLocalID(h.IdentifierRef) {
		return nil, newErrorRefusal("invalid-host-scope", "identifier_ref must match ^[a-z][a-z0-9-]{0,63}$")
	}
	validFields := isNonEmpty512(h.Namespace) &&
		isNonEmpty512(h.Value) &&
		isNonEmpty512(h.Derivation.ID) &&
		isSHA256Digest(h.Derivation.Revision) &&
		isValidStability(h.Stability) &&
		isValidCloneBehavior(h.CloneBehavior)
	if !validFields {
		return nil, newErrorRefusal("invalid-host-scope", "host_scope fields violate schema constraints")
	}
	out := map[string]any{
		"namespace": h.Namespace,
		"value":     h.Value,
		"derivation": map[string]any{
			"id":       h.Derivation.ID,
			"revision": h.Derivation.Revision,
		},
		"stability":      h.Stability,
		"clone_behavior": h.CloneBehavior,
	}
	if h.IdentifierRef != "" {
		out["identifier_ref"] = h.IdentifierRef
	}
	return out, nil
}

// ObservationOptions configures disclosure and opt-in context probes.
//
// Zero-value ObservationOptions uses SubjectRef "process-target", current UTC
// wall clock, probes security_principal (unless OmitSecurityPrincipal is true),
// and leaves all four digest facts not-probed.
type ObservationOptions struct {
	SubjectRef                    string
	ObservedAt                    string
	OmitSecurityPrincipal         bool
	IncludeNamespaceContextDigest bool
	IncludeExecutableDigest       bool
	IncludeArgumentsDigest        bool
	IncludeSecurityContextDigest  bool
}

// DefaultObservationOptions returns the profile-default ObservationOptions.
func DefaultObservationOptions() ObservationOptions {
	return ObservationOptions{
		SubjectRef: "process-target",
	}
}

// WithAllContextFacts returns a copy with all context facts opted in.
func (o ObservationOptions) WithAllContextFacts() ObservationOptions {
	o.OmitSecurityPrincipal = false
	o.IncludeNamespaceContextDigest = true
	o.IncludeExecutableDigest = true
	o.IncludeArgumentsDigest = true
	o.IncludeSecurityContextDigest = true
	return o
}

func (o ObservationOptions) probeSecurityPrincipal() bool {
	return !o.OmitSecurityPrincipal
}

func (o ObservationOptions) effectiveSubjectRef() string {
	if o.SubjectRef == "" {
		return "process-target"
	}
	return o.SubjectRef
}

// CollectorRefusal represents a typed refusal from collector or handle acquisition.
type CollectorRefusal struct {
	Status string
	Code   string
	Detail string
}

// Error implements error.
func (e *CollectorRefusal) Error() string {
	return fmt.Sprintf("%s:%s: %s", e.Status, e.Code, e.Detail)
}

func newRefusal(status, code, detail string) *CollectorRefusal {
	return &CollectorRefusal{Status: status, Code: code, Detail: detail}
}

func newErrorRefusal(code, detail string) *CollectorRefusal {
	return newRefusal("error", code, detail)
}

// CapabilityProbe records a live continuity-handle probe outcome in report.capabilities[].
type CapabilityProbe struct {
	Provider string
	Status   string
	Reason   string
}

// ReportMetadata supplies envelope fields for BuildObservationReport.
type ReportMetadata struct {
	ReportID             string
	CreatedAt            string
	ProducerName         string
	ProducerRevision     string
	HostSubjectID        string
	EvidenceID           string
	Source               string
	Collector            string
	ExitHandleCapability *CapabilityProbe
}

// DefaultReportMetadata returns default envelope metadata for BuildObservationReport.
func DefaultReportMetadata() ReportMetadata {
	return ReportMetadata{
		ReportID:         "native-process-observation",
		ProducerName:     "sysprims-procobserve",
		ProducerRevision: "sha256:0000000000000000000000000000000000000000000000000000000000000000",
		HostSubjectID:    "host-local",
		EvidenceID:       "e-process-observation",
		Source:           defaultCollectorSource(),
		Collector:        "sysprims-procobserve",
	}
}

// ProcessContinuityHandle wraps a private read-only OS synchronization handle
// (kqueue EVFILT_PROC NOTE_EXIT on macOS; pidfd on Linux) bracketed at open.
//
// It intentionally exposes no raw descriptor, refuses JSON marshaling, and
// provides no signal delivery API.
type ProcessContinuityHandle struct {
	pid       uint32
	baseline  map[string]any
	hostScope HostScope
	options   ObservationOptions
	mu        sync.Mutex
	closed    bool
	inner     *platformHandle
}

// MarshalJSON always fails to prevent accidental serialization of live handles.
func (*ProcessContinuityHandle) MarshalJSON() ([]byte, error) {
	return nil, newErrorRefusal("continuity-handle-not-serializable", "ProcessContinuityHandle must never be serialized")
}

// PID returns the numeric PID bound when the handle was opened.
func (h *ProcessContinuityHandle) PID() uint32 {
	return h.pid
}

// BaselineObservation returns a deep copy of the bracketed observation captured at open.
func (h *ProcessContinuityHandle) BaselineObservation() map[string]any {
	return cloneMap(h.baseline)
}

// Close releases the private OS continuity descriptor (idempotent).
func (h *ProcessContinuityHandle) Close() error {
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.closed {
		return nil
	}
	h.closed = true
	return h.inner.closeHandle()
}

// PollExited performs a non-blocking exit check on the private continuity handle.
func (h *ProcessContinuityHandle) PollExited() (bool, error) {
	return h.WaitExit(0)
}

// WaitExit blocks up to timeout waiting for process exit (returning true on exit, false on timeout).
func (h *ProcessContinuityHandle) WaitExit(timeout time.Duration) (bool, error) {
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.closed {
		return false, newErrorRefusal("continuity-handle-closed", "ProcessContinuityHandle is closed")
	}
	return h.inner.waitExit(timeout)
}

// Reobserve collects a fresh read-only observation for h.PID() using the stored
// HostScope and ObservationOptions.
func (h *ProcessContinuityHandle) Reobserve(observedAt string) (map[string]any, error) {
	opts := h.options
	opts.ObservedAt = observedAt
	return ObserveProcess(h.pid, h.hostScope, opts)
}

// Reverify collects a fresh observation at nowUTC, enforces AdmitLiveObservation,
// and compares against BaselineObservation() under policy.
func (h *ProcessContinuityHandle) Reverify(policy evidence.ComparisonPolicy, nowUTC string) (map[string]any, evidence.ComparisonOutcome, error) {
	candidate, err := h.Reobserve(nowUTC)
	if err != nil {
		return nil, evidence.ComparisonOutcome{}, err
	}
	accepted := AcceptedObservationProfile()
	nowCopy := nowUTC
	if err := evidence.AdmitLiveObservation(candidate, &nowCopy, accepted, nil, nil); err != nil {
		code, ok := evidence.RefusalCode(err)
		if !ok {
			code = "live-observation-refused"
		}
		return nil, evidence.ComparisonOutcome{}, newErrorRefusal(code, "live observation freshness gate refused")
	}
	outcome := evidence.CompareNativeObservations(h.baseline, candidate, policy, accepted)
	return candidate, outcome, nil
}

// CurrentUTCTimestamp formats the current UTC clock as strict YYYY-MM-DDTHH:MM:SSZ.
func CurrentUTCTimestamp() (string, error) {
	formatted := time.Now().UTC().Truncate(time.Second).Format("2006-01-02T15:04:05Z")
	if _, ok := evidence.ParseStrictUTCUnix(formatted); !ok {
		return "", newErrorRefusal("trusted-clock-unavailable", "formatted UTC clock failed validation")
	}
	return formatted, nil
}

func validateCommonInputs(pid uint32, hostScope HostScope, opts ObservationOptions) (map[string]any, ObservationOptions, error) {
	if pid < MinObservablePID || pid > MaxObservablePID {
		return nil, opts, newErrorRefusal("invalid-pid-range", "pid must be within 1..=2147483647")
	}
	opts.SubjectRef = opts.effectiveSubjectRef()
	if !isLocalID(opts.SubjectRef) {
		return nil, opts, newErrorRefusal("invalid-subject-ref", "subject_ref must match ^[a-z][a-z0-9-]{0,63}$")
	}
	hsMap, err := hostScope.ToMap()
	if err != nil {
		return nil, opts, err
	}
	if opts.ObservedAt == "" {
		ts, err := CurrentUTCTimestamp()
		if err != nil {
			return nil, opts, err
		}
		opts.ObservedAt = ts
	} else if _, ok := evidence.ParseStrictUTCUnix(opts.ObservedAt); !ok {
		return nil, opts, newErrorRefusal("invalid-observed-at", "observed_at must be a valid YYYY-MM-DDTHH:MM:SSZ instant")
	}
	return hsMap, opts, nil
}

// ObserveProcess collects a bracketed read-only native-process-observation:v0
// payload for pid (1..=2147483647).
func ObserveProcess(pid uint32, hostScope HostScope, opts ObservationOptions) (map[string]any, error) {
	hsMap, normOpts, err := validateCommonInputs(pid, hostScope, opts)
	if err != nil {
		return nil, err
	}
	obs, err := observeProcessPlatform(pid, hsMap, normOpts)
	if err != nil {
		return nil, err
	}
	if err := evidence.ValidateObservationShape(obs); err != nil {
		code, _ := evidence.RefusalCode(err)
		return nil, newErrorRefusal(code, "internal collector output failed schema validation")
	}
	return obs, nil
}

// OpenContinuityHandle opens a private read-only continuity handle bracketed by
// pre/post start-identity snapshots, requiring a live target process.
func OpenContinuityHandle(pid uint32, hostScope HostScope, opts ObservationOptions) (*ProcessContinuityHandle, error) {
	hsMap, normOpts, err := validateCommonInputs(pid, hostScope, opts)
	if err != nil {
		return nil, err
	}
	baseline, ph, err := openContinuityHandlePlatform(pid, hsMap, normOpts)
	if err != nil {
		return nil, err
	}
	if err := evidence.ValidateObservationShape(baseline); err != nil {
		_ = ph.closeHandle()
		code, _ := evidence.RefusalCode(err)
		return nil, newErrorRefusal(code, "internal baseline output failed schema validation")
	}
	return &ProcessContinuityHandle{
		pid:       pid,
		baseline:  baseline,
		hostScope: hostScope,
		options:   normOpts,
		inner:     ph,
	}, nil
}

// BuildObservationReport wraps a validated native-process-observation:v0 payload
// in a canonically admitted observation-only identity-evidence/v0 report.
func BuildObservationReport(observation map[string]any, meta ReportMetadata) (evidence.AdmittedWire, error) {
	if err := evidence.ValidateObservationShape(observation); err != nil {
		code, _ := evidence.RefusalCode(err)
		return evidence.AdmittedWire{}, newErrorRefusal(code, "observation failed schema validation")
	}
	observedAt, _ := observation["observed_at"].(string)
	createdAt := meta.CreatedAt
	if createdAt == "" {
		createdAt = observedAt
	}
	if _, ok := evidence.ParseStrictUTCUnix(createdAt); !ok {
		return evidence.AdmittedWire{}, newErrorRefusal("invalid-report-metadata", "created_at must be strict UTC YYYY-MM-DDTHH:MM:SSZ")
	}
	procObj, _ := observation["process"].(map[string]any)
	procSubject, _ := procObj["subject_ref"].(string)
	validMeta := isLocalID(meta.ReportID) &&
		isLocalID(meta.HostSubjectID) &&
		isLocalID(meta.EvidenceID) &&
		meta.HostSubjectID != procSubject &&
		isSHA256Digest(meta.ProducerRevision)
	if !validMeta {
		return evidence.AdmittedWire{}, newErrorRefusal("invalid-report-metadata", "report metadata IDs or digests violate schema constraints")
	}

	payloadDigest, err := evidence.CanonicalSHA256Value(observation)
	if err != nil {
		code, _ := evidence.RefusalCode(err)
		return evidence.AdmittedWire{}, newErrorRefusal(code, "failed to canonicalize observation")
	}

	hsObj, _ := procObj["host_scope"].(map[string]any)
	identifiers := []any{}
	if idRef, ok := hsObj["identifier_ref"].(string); ok && idRef != "" {
		identifiers = append(identifiers, map[string]any{
			"id":             idRef,
			"subject_ref":    meta.HostSubjectID,
			"namespace":      hsObj["namespace"],
			"value":          hsObj["value"],
			"derivation":     deepCloneAny(hsObj["derivation"]),
			"stability":      hsObj["stability"],
			"clone_behavior": hsObj["clone_behavior"],
		})
	}

	provider := defaultCollectorSource()
	capabilities := []any{}
	if meta.ExitHandleCapability != nil {
		reason := meta.ExitHandleCapability.Reason
		if reason == "" {
			reason = "Acquired private read-only process continuity handle."
		}
		capEntry := map[string]any{
			"id":          "cap-exit-handle",
			"subject_ref": procSubject,
			"provider":    meta.ExitHandleCapability.Provider,
			"capability":  "process-exit-observation",
			"status":      meta.ExitHandleCapability.Status,
			"reason":      reason,
			"observed_at": observedAt,
		}
		capabilities = append(capabilities, capEntry)
	}
	if tpns, ok := procObj["target_pid_namespace"].(map[string]any); ok {
		capabilities = append(capabilities, factToCapability("cap-pid-namespace", procSubject, provider, "pid-namespace-observation", tpns, observedAt))
	}
	if ec, ok := procObj["execution_context"].(map[string]any); ok {
		specs := [][3]string{
			{"cap-security-principal", "security-principal", "security_principal"},
			{"cap-namespace-context-digest", "namespace-context-digest", "namespace_context_digest"},
			{"cap-executable-digest", "executable-digest", "executable_digest"},
			{"cap-arguments-digest", "arguments-digest", "arguments_digest"},
			{"cap-security-context-digest", "security-context-digest", "security_context_digest"},
		}
		for _, sp := range specs {
			if fact, ok := ec[sp[2]].(map[string]any); ok {
				capabilities = append(capabilities, factToCapability(sp[0], procSubject, provider, sp[1], fact, observedAt))
			}
		}
	}

	reportVal := map[string]any{
		"contract":   evidence.IdentityEvidenceContract,
		"report_id":  meta.ReportID,
		"created_at": createdAt,
		"producer": map[string]any{
			"name":     meta.ProducerName,
			"revision": meta.ProducerRevision,
		},
		"subjects": []any{
			map[string]any{"id": meta.HostSubjectID, "kind": "os-installation"},
			map[string]any{"id": procSubject, "kind": "process"},
		},
		"identifiers": identifiers,
		"evidence": []any{
			map[string]any{
				"id":           meta.EvidenceID,
				"subject_ref":  procSubject,
				"method":       "os-observation",
				"source":       meta.Source,
				"collector":    meta.Collector,
				"collected_at": observedAt,
				"digest":       payloadDigest,
				"format_profile": map[string]any{
					"id":       evidence.NativeProcessObservationProfileID,
					"revision": evidence.NativeProcessObservationProfileRevision,
				},
			},
		},
		"assessments":   []any{},
		"capabilities":  capabilities,
		"keys":          []any{},
		"claims":        []any{},
		"input_reports": []any{},
	}

	if err := evidence.BindNativeObservationReport(reportVal, observation, AcceptedObservationProfile()); err != nil {
		code, _ := evidence.RefusalCode(err)
		return evidence.AdmittedWire{}, newErrorRefusal(code, "observation report binding failed")
	}

	raw, err := json.Marshal(reportVal)
	if err != nil {
		return evidence.AdmittedWire{}, newErrorRefusal("invalid-json", "report serialization failed")
	}
	admitted, err := evidence.AdmitAndValidateReport(raw, nil)
	if err != nil {
		code, _ := evidence.RefusalCode(err)
		return evidence.AdmittedWire{}, newErrorRefusal(code, "report envelope validation failed")
	}
	return admitted, nil
}

func defaultCollectorSource() string {
	switch runtime.GOOS {
	case "darwin":
		return "macos-sysctl-proc"
	case "linux":
		return "linux-procfs-openat2"
	default:
		return "unsupported-platform"
	}
}

func observerArchitecture() string {
	switch runtime.GOARCH {
	case "amd64":
		return "x86_64"
	case "arm64":
		return "aarch64"
	case "386":
		return "x86"
	default:
		return runtime.GOARCH
	}
}

func factToCapability(id, subjectRef, provider, capability string, fact map[string]any, observedAt string) map[string]any {
	factStatus, _ := fact["status"].(string)
	capStatus := factStatus
	if factStatus == "observed" {
		capStatus = "available"
	}
	reason, _ := fact["reason"].(string)
	if reason == "" {
		reason = "Observed within bracketed native process snapshot."
	}
	out := map[string]any{
		"id":          id,
		"subject_ref": subjectRef,
		"provider":    provider,
		"capability":  capability,
		"status":      capStatus,
		"reason":      reason,
		"observed_at": observedAt,
	}
	return out
}

func deepCloneAny(v any) any {
	switch x := v.(type) {
	case map[string]any:
		return cloneMap(x)
	case []any:
		cp := make([]any, len(x))
		for i, el := range x {
			cp[i] = deepCloneAny(el)
		}
		return cp
	default:
		return x
	}
}

func cloneMap(m map[string]any) map[string]any {
	out := make(map[string]any, len(m))
	for k, v := range m {
		out[k] = deepCloneAny(v)
	}
	return out
}

func isLowerHex(c byte) bool {
	if c >= '0' && c <= '9' {
		return true
	}
	return c >= 'a' && c <= 'f'
}

func isLowerIDChar(c byte) bool {
	if c >= 'a' && c <= 'z' {
		return true
	}
	if c >= '0' && c <= '9' {
		return true
	}
	return c == '-'
}

func isLocalID(s string) bool {
	if len(s) == 0 || len(s) > 64 || s[0] < 'a' || s[0] > 'z' {
		return false
	}
	for i := 1; i < len(s); i++ {
		if !isLowerIDChar(s[i]) {
			return false
		}
	}
	return true
}

func isNonEmpty512(s string) bool {
	n := utf8.RuneCountInString(s)
	return n >= 1 && n <= 512
}

func isSHA256Digest(s string) bool {
	if len(s) != 71 || s[:7] != "sha256:" {
		return false
	}
	for i := 7; i < 71; i++ {
		if !isLowerHex(s[i]) {
			return false
		}
	}
	return true
}
