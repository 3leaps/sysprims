// SPDX-License-Identifier: MIT OR Apache-2.0

package evidence

import (
	"encoding/binary"
	"math"
	"strconv"
	"strings"
	"unicode/utf8"
)

const (
	// NativeProcessObservationProfileID is the native-process-observation:v0 profile identifier.
	NativeProcessObservationProfileID = "urn:3leaps:identity-evidence:native-process-observation:v0"
	// NativeProcessObservationProfileRevision is the pinned sha256 revision of profile.json.
	NativeProcessObservationProfileRevision = "sha256:0c116af8e558012f654be4c8ca772583b2d3b58c43cc157ce606e4f74f6b4a60"
	// NativeProcessObservationReadmeRevision is the pinned sha256 revision of README.md.
	NativeProcessObservationReadmeRevision = "sha256:8a4347fcec0ad51874f28c6c1dfc6ca0888e9ccf2e25f239ba46b9be9d3749cb"
	// NativeProcessObservationSchemaRevision is the pinned sha256 revision of evidence.schema.json.
	NativeProcessObservationSchemaRevision = "sha256:670e254ca9cdeab8071757f91cff2226089c1d96630ce0e8c70185ddbaec8e45"
	// LinuxProcfsStatTimensScopedProfileID is the projected Linux start_identity profile ID.
	LinuxProcfsStatTimensScopedProfileID = "urn:3leaps:identity-evidence:start-identity:linux-procfs-stat-timens-scoped:v0"

	maxNativeObsAgeSecs     int64  = 5
	maxNativeFutureSkewSecs int64  = 0
	minNativePID            uint32 = 1
	maxNativePID            uint32 = 2_147_483_647
)

var (
	// CanonicalContextFacts is the normative execution_context fact evaluation order.
	CanonicalContextFacts = []string{
		"security_principal",
		"namespace_context_digest",
		"executable_digest",
		"arguments_digest",
		"security_context_digest",
	}
	// LinuxNamespaceKinds is the fixed 9-entry Linux namespace order (linux-ns-inum-set:v0).
	LinuxNamespaceKinds = []string{
		"cgroup", "ipc", "mnt", "net", "pid", "time", "time_for_children", "user", "uts",
	}
	factStatuses = []string{
		"observed", "not-present", "inaccessible", "unsupported", "not-probed", "error", "unknown",
	}
	platformFamilies       = []string{"linux", "macos", "windows"}
	establishedStabilities = []string{"boot", "installation", "enrollment", "device"}
)

// ComparisonKind classifies a three-valued CompareNativeObservations outcome.
type ComparisonKind string

const (
	// OutcomeMatch indicates both observations refer to the same incarnation and satisfy policy.
	OutcomeMatch ComparisonKind = "match"
	// OutcomeMismatch indicates positive disagreement on incarnation, state, or watched context.
	OutcomeMismatch ComparisonKind = "mismatch"
	// OutcomeInsufficient indicates comparison cannot conclude safely.
	OutcomeInsufficient ComparisonKind = "insufficient"
)

// ComparisonOutcome holds the three-valued result of CompareNativeObservations.
type ComparisonOutcome struct {
	Kind   ComparisonKind
	Reason string
}

// Code returns the exact normative result string ("match", "mismatch:...", or "insufficient:...").
func (o ComparisonOutcome) Code() string {
	if o.Kind == OutcomeMatch {
		return "match"
	}
	return string(o.Kind) + ":" + o.Reason
}

// String implements fmt.Stringer.
func (o ComparisonOutcome) String() string {
	return o.Code()
}

func outcomeInsufficient(reason string) ComparisonOutcome {
	return ComparisonOutcome{Kind: OutcomeInsufficient, Reason: reason}
}

func outcomeMismatch(reason string) ComparisonOutcome {
	return ComparisonOutcome{Kind: OutcomeMismatch, Reason: reason}
}

// ComparisonPolicy specifies caller policy for CompareNativeObservations (no defaults).
type ComparisonPolicy struct {
	BaselineHostScopeTrusted   bool
	CandidateHostScopeTrusted  bool
	RequireLive                bool
	RequiredContextFacts       []string
	DetectObservedContextDrift bool
	AllowedCloneBehaviors      []string
	HasAllowedCloneBehaviors   bool
}

// ParseComparisonPolicyJSON validates a raw JSON policy value with strict types and no defaults.
func ParseComparisonPolicyJSON(rawPolicy any) (ComparisonPolicy, error) {
	m, ok := rawPolicy.(map[string]any)
	if !ok || m == nil {
		return ComparisonPolicy{}, newRefusal("invalid-evaluation-policy")
	}
	reqKeys := []string{
		"baseline_host_scope_trusted",
		"candidate_host_scope_trusted",
		"require_live",
		"required_context_facts",
		"detect_observed_context_drift",
	}
	if !checkAllowedKeys(m, reqKeys, []string{"allowed_clone_behaviors"}) {
		return ComparisonPolicy{}, newRefusal("invalid-evaluation-policy")
	}
	bTrust, ok1 := m["baseline_host_scope_trusted"].(bool)
	cTrust, ok2 := m["candidate_host_scope_trusted"].(bool)
	reqLive, ok3 := m["require_live"].(bool)
	detDrift, ok4 := m["detect_observed_context_drift"].(bool)
	if !ok1 || !ok2 || !ok3 || !ok4 {
		return ComparisonPolicy{}, newRefusal("invalid-evaluation-policy")
	}
	rfArr, ok := m["required_context_facts"].([]any)
	if !ok {
		return ComparisonPolicy{}, newRefusal("invalid-evaluation-policy")
	}
	reqFacts := make([]string, 0, len(rfArr))
	for _, item := range rfArr {
		s, ok := item.(string)
		if !ok || !containsStr(CanonicalContextFacts, s) {
			return ComparisonPolicy{}, newRefusal("invalid-evaluation-policy")
		}
		reqFacts = append(reqFacts, s)
	}
	pol := ComparisonPolicy{
		BaselineHostScopeTrusted:   bTrust,
		CandidateHostScopeTrusted:  cTrust,
		RequireLive:                reqLive,
		RequiredContextFacts:       reqFacts,
		DetectObservedContextDrift: detDrift,
	}
	if acVal, exists := m["allowed_clone_behaviors"]; exists {
		acArr, ok := acVal.([]any)
		if !ok || len(acArr) == 0 {
			return ComparisonPolicy{}, newRefusal("invalid-evaluation-policy")
		}
		clones := make([]string, 0, len(acArr))
		for _, item := range acArr {
			s, ok := item.(string)
			if !ok || !containsStr(cloneBehaviors, s) {
				return ComparisonPolicy{}, newRefusal("invalid-evaluation-policy")
			}
			clones = append(clones, s)
		}
		pol.AllowedCloneBehaviors = clones
		pol.HasAllowedCloneBehaviors = true
	}
	return pol, nil
}

func (p ComparisonPolicy) validate() (map[string]bool, map[string]bool, bool) {
	reqSet := make(map[string]bool, len(p.RequiredContextFacts))
	for _, f := range p.RequiredContextFacts {
		if !containsStr(CanonicalContextFacts, f) {
			return nil, nil, false
		}
		reqSet[f] = true
	}
	var allowSet map[string]bool
	if p.HasAllowedCloneBehaviors || p.AllowedCloneBehaviors != nil {
		if len(p.AllowedCloneBehaviors) == 0 {
			return nil, nil, false
		}
		allowSet = make(map[string]bool, len(p.AllowedCloneBehaviors))
		for _, c := range p.AllowedCloneBehaviors {
			if !containsStr(cloneBehaviors, c) {
				return nil, nil, false
			}
			allowSet[c] = true
		}
	}
	return reqSet, allowSet, true
}

func isArchByte(c byte) bool {
	if c >= 'a' && c <= 'z' {
		return true
	}
	if c >= '0' && c <= '9' {
		return true
	}
	return c == '_'
}

func isArchString(s string) bool {
	if len(s) < 2 || len(s) > 32 {
		return false
	}
	for i := 0; i < len(s); i++ {
		if !isArchByte(s[i]) {
			return false
		}
	}
	return true
}

func parseCanonicalU32(s string) (uint32, bool) {
	if len(s) == 0 || len(s) > 10 || (len(s) > 1 && s[0] == '0') {
		return 0, false
	}
	for i := 0; i < len(s); i++ {
		if s[i] < '0' || s[i] > '9' {
			return 0, false
		}
	}
	v, err := strconv.ParseUint(s, 10, 32)
	if err != nil {
		return 0, false
	}
	return uint32(v), true
}

func parseCanonicalU64(s string) (uint64, bool) {
	if len(s) == 0 || len(s) > 20 || (len(s) > 1 && s[0] == '0') {
		return 0, false
	}
	for i := 0; i < len(s); i++ {
		if s[i] < '0' || s[i] > '9' {
			return 0, false
		}
	}
	v, err := strconv.ParseUint(s, 10, 64)
	if err != nil {
		return 0, false
	}
	return v, true
}

func parseCanonicalNonNegI64(s string) (int64, bool) {
	if len(s) == 0 || len(s) > 19 || (len(s) > 1 && s[0] == '0') {
		return 0, false
	}
	for i := 0; i < len(s); i++ {
		if s[i] < '0' || s[i] > '9' {
			return 0, false
		}
	}
	v, err := strconv.ParseInt(s, 10, 64)
	if err != nil || v < 0 {
		return 0, false
	}
	return v, true
}

func isValidPIDString(s string) bool {
	if len(s) == 0 || len(s) > 10 || s[0] < '1' || s[0] > '9' {
		return false
	}
	v, ok := parseCanonicalU32(s)
	return ok && v >= minNativePID && v <= maxNativePID
}

func isSchemaNamespaceValue(s string) bool {
	idx := strings.IndexByte(s, ':')
	if idx < 0 {
		return false
	}
	prefix, rest := s[:idx], s[idx+1:]
	if len(prefix) < 2 || len(prefix) > 32 || prefix[0] < 'a' || prefix[0] > 'z' {
		return false
	}
	for i := 1; i < len(prefix); i++ {
		if !isLowerAlphaNumDashByte(prefix[i]) {
			return false
		}
	}
	if len(rest) < 1 || len(rest) > 256 {
		return false
	}
	for i := 0; i < len(rest); i++ {
		if rest[i] < 0x21 || rest[i] > 0x7e {
			return false
		}
	}
	return true
}

func isLowercaseUUID(s string) bool {
	if len(s) != 36 {
		return false
	}
	for i := 0; i < 36; i++ {
		c := s[i]
		if i == 8 || i == 13 || i == 18 || i == 23 {
			if c != '-' {
				return false
			}
		} else if !isLowerHexByte(c) {
			return false
		}
	}
	return true
}

func isLinuxScopedNSID(s, expectedPrefix string) bool {
	parts := strings.Split(s, ":")
	if len(parts) != 4 || parts[0] != expectedPrefix {
		return false
	}
	_, ok1 := parseCanonicalU32(parts[1])
	_, ok2 := parseCanonicalU32(parts[2])
	_, ok3 := parseCanonicalU64(parts[3])
	return ok1 && ok2 && ok3
}

func validateFactReasonOpt(m map[string]any) bool {
	r, exists := m["reason"]
	if !exists {
		return true
	}
	rs, ok := r.(string)
	return ok && isNonEmpty512(rs)
}

func validateFactReasonReq(m map[string]any) bool {
	rs, ok := strField(m, "reason")
	return ok && isNonEmpty512(rs)
}

func validateNamespaceFact(v any) bool {
	m, ok := v.(map[string]any)
	if !ok {
		return false
	}
	st, ok := strField(m, "status")
	if !ok || !containsStr(factStatuses, st) {
		return false
	}
	if st == "observed" {
		val, ok := strField(m, "value")
		return checkAllowedKeys(m, []string{"status", "value"}, []string{"reason"}) &&
			ok && isSchemaNamespaceValue(val) && validateFactReasonOpt(m)
	}
	return checkAllowedKeys(m, []string{"status", "reason"}, nil) && validateFactReasonReq(m)
}

func validateBootIDFact(v any) bool {
	m, ok := v.(map[string]any)
	if !ok {
		return false
	}
	st, ok := strField(m, "status")
	if !ok || !containsStr(factStatuses, st) {
		return false
	}
	if st == "observed" {
		val, ok := strField(m, "value")
		return checkAllowedKeys(m, []string{"status", "value"}, []string{"reason"}) &&
			ok && isNonEmpty512(val) && validateFactReasonOpt(m)
	}
	return checkAllowedKeys(m, []string{"status", "reason"}, nil) && validateFactReasonReq(m)
}

func validateStateFact(v any) bool {
	m, ok := v.(map[string]any)
	if !ok {
		return false
	}
	st, ok := strField(m, "status")
	if !ok || !containsStr(factStatuses, st) {
		return false
	}
	if st == "observed" {
		val, ok := strField(m, "value")
		return checkAllowedKeys(m, []string{"status", "value"}, []string{"reason"}) &&
			ok && (val == "live" || val == "zombie") && validateFactReasonOpt(m)
	}
	return checkAllowedKeys(m, []string{"status", "reason"}, nil) && validateFactReasonReq(m)
}

func validateStartIdentityFact(v any) bool {
	m, ok := v.(map[string]any)
	if !ok {
		return false
	}
	st, ok := strField(m, "status")
	if !ok || !containsStr(factStatuses, st) || !isProfileRefObj(m["profile"]) {
		return false
	}
	if st == "observed" {
		val, ok := strField(m, "value")
		if !ok || !utf8.ValidString(val) {
			return false
		}
		n := utf8.RuneCountInString(val)
		return checkAllowedKeys(m, []string{"status", "profile", "value"}, []string{"reason"}) &&
			n >= 1 && n <= 128 && validateFactReasonOpt(m)
	}
	return checkAllowedKeys(m, []string{"status", "profile", "reason"}, nil) && validateFactReasonReq(m)
}

func validatePrincipalFact(v any) bool {
	m, ok := v.(map[string]any)
	if !ok {
		return false
	}
	st, ok := strField(m, "status")
	if !ok || !containsStr(factStatuses, st) {
		return false
	}
	if st == "observed" {
		return checkAllowedKeys(m, []string{"status", "derivation", "principal"}, []string{"reason"}) &&
			isProfileRefObj(m["derivation"]) && isPrincipalObj(m["principal"]) && validateFactReasonOpt(m)
	}
	return checkAllowedKeys(m, []string{"status", "reason"}, nil) && validateFactReasonReq(m)
}

func validateDigestFact(v any) bool {
	m, ok := v.(map[string]any)
	if !ok {
		return false
	}
	st, ok := strField(m, "status")
	if !ok || !containsStr(factStatuses, st) {
		return false
	}
	if st == "observed" {
		dig, ok := strField(m, "digest")
		return checkAllowedKeys(m, []string{"status", "derivation", "digest"}, []string{"reason"}) &&
			isProfileRefObj(m["derivation"]) && ok && isSHA256Digest(dig) && validateFactReasonOpt(m)
	}
	return checkAllowedKeys(m, []string{"status", "reason"}, nil) && validateFactReasonReq(m)
}

// ValidateObservationShape validates strict native-process-observation/evidence.schema.json
// structural and 7-state conditional rules.
func ValidateObservationShape(obs any) error {
	errShape := func() error { return newRefusal("invalid-observation-shape") }
	obj, ok := obs.(map[string]any)
	if !ok || !checkAllowedKeys(obj, []string{"profile", "observed_at", "observer", "process"}, nil) ||
		!isProfileRefObj(obj["profile"]) {
		return errShape()
	}
	obsAt, ok := strField(obj, "observed_at")
	if !ok {
		return errShape()
	}
	if _, ok := ParseStrictUTCUnix(obsAt); !ok {
		return errShape()
	}

	observer, ok := objField(obj, "observer")
	if !ok || !checkAllowedKeys(observer, []string{
		"platform_family", "architecture", "bracketed", "time_namespace", "user_namespace",
	}, nil) {
		return errShape()
	}
	pf, ok1 := strField(observer, "platform_family")
	arch, ok2 := strField(observer, "architecture")
	_, ok3 := observer["bracketed"].(bool)
	if !ok1 || !containsStr(platformFamilies, pf) || !ok2 || !isArchString(arch) || !ok3 ||
		!validateNamespaceFact(observer["time_namespace"]) ||
		!validateNamespaceFact(observer["user_namespace"]) {
		return errShape()
	}

	procObj, ok := objField(obj, "process")
	if !ok || !checkAllowedKeys(procObj, []string{
		"subject_ref", "host_scope", "boot_id", "pid_numbering_namespace",
		"target_pid_namespace", "pid", "process_state", "start_identity", "execution_context",
	}, nil) {
		return errShape()
	}
	sr, ok1 := strField(procObj, "subject_ref")
	pid, ok2 := strField(procObj, "pid")
	if !ok1 || !isLocalID(sr) || !ok2 || !isValidPIDString(pid) ||
		!validateBootIDFact(procObj["boot_id"]) ||
		!validateNamespaceFact(procObj["pid_numbering_namespace"]) ||
		!validateNamespaceFact(procObj["target_pid_namespace"]) ||
		!validateStateFact(procObj["process_state"]) ||
		!validateStartIdentityFact(procObj["start_identity"]) {
		return errShape()
	}

	hs, ok := objField(procObj, "host_scope")
	if !ok || !checkAllowedKeys(hs, []string{
		"namespace", "value", "derivation", "stability", "clone_behavior",
	}, []string{"identifier_ref"}) {
		return errShape()
	}
	hns, ok1 := strField(hs, "namespace")
	hval, ok2 := strField(hs, "value")
	hstab, ok3 := strField(hs, "stability")
	hclone, ok4 := strField(hs, "clone_behavior")
	if !ok1 || !isNonEmpty512(hns) || !ok2 || !isNonEmpty512(hval) ||
		!isProfileRefObj(hs["derivation"]) ||
		!ok3 || !containsStr(stabilities, hstab) ||
		!ok4 || !containsStr(cloneBehaviors, hclone) {
		return errShape()
	}
	if ir, exists := hs["identifier_ref"]; exists {
		irs, ok := ir.(string)
		if !ok || !isLocalID(irs) {
			return errShape()
		}
	}

	ec, ok := objField(procObj, "execution_context")
	if !ok || !checkAllowedKeys(ec, CanonicalContextFacts, nil) ||
		!validatePrincipalFact(ec["security_principal"]) ||
		!validateDigestFact(ec["namespace_context_digest"]) ||
		!validateDigestFact(ec["executable_digest"]) ||
		!validateDigestFact(ec["arguments_digest"]) ||
		!validateDigestFact(ec["security_context_digest"]) {
		return errShape()
	}

	return nil
}

func appendU32BE(out []byte, n int) []byte {
	if n < 0 || uint64(n) > math.MaxUint32 {
		panic("framed length fits in u32")
	}
	var b [4]byte
	binary.BigEndian.PutUint32(b[:], uint32(n))
	return append(out, b[:]...)
}

// DeriveArgumentsDigest computes arguments_digest (argv-bytes:v0) over raw NUL-free argv slices.
func DeriveArgumentsDigest(argv [][]byte) string {
	pre := []byte("3leaps.native-process-observation.arguments.v0\x00")
	pre = appendU32BE(pre, len(argv))
	for _, arg := range argv {
		pre = appendU32BE(pre, len(arg))
		pre = append(pre, arg...)
	}
	return sha256Prefixed(pre)
}

// DeriveExecutableDigest computes executable_digest (executable-file-bytes:v0) over imageBytes.
func DeriveExecutableDigest(imageBytes []byte) string {
	prefix := "3leaps.native-process-observation.executable-image.v0\x00"
	pre := make([]byte, 0, len(prefix)+len(imageBytes))
	pre = append(pre, prefix...)
	pre = append(pre, imageBytes...)
	return sha256Prefixed(pre)
}

// DeriveLinuxNamespaceContextDigest computes namespace_context_digest (linux-ns-inum-set:v0)
// over the complete 9-entry Linux namespace map (ns:<major_u32>:<minor_u32>:<ino_u64>).
func DeriveLinuxNamespaceContextDigest(nsMap map[string]string) (string, error) {
	if len(nsMap) != len(LinuxNamespaceKinds) {
		return "", newRefusal("incomplete-linux-namespace-set")
	}
	for _, k := range LinuxNamespaceKinds {
		if _, ok := nsMap[k]; !ok {
			return "", newRefusal("incomplete-linux-namespace-set")
		}
	}
	pre := []byte("3leaps.native-process-observation.namespaces.v0\x00")
	pre = appendU32BE(pre, len(LinuxNamespaceKinds))
	for _, kind := range LinuxNamespaceKinds {
		val := nsMap[kind]
		if !isLinuxScopedNSID(val, "ns") {
			return "", newRefusal("invalid-linux-ns-entry")
		}
		pre = appendU32BE(pre, len(kind))
		pre = append(pre, kind...)
		pre = appendU32BE(pre, len(val))
		pre = append(pre, val...)
	}
	return sha256Prefixed(pre), nil
}

func isLowerHex16(s string) bool {
	if len(s) != 16 {
		return false
	}
	for i := 0; i < 16; i++ {
		if !isLowerHexByte(s[i]) {
			return false
		}
	}
	return true
}

// DeriveLinuxSecurityContextDigest computes security_context_digest (linux-procfs-status-secctx:v0)
// over canonical ASCII security-context bytes.
func DeriveLinuxSecurityContextDigest(canonicalContextBytes []byte) (string, error) {
	errBad := func() (string, error) { return "", newRefusal("invalid-linux-security-context") }
	for _, c := range canonicalContextBytes {
		if c > 0x7f {
			return errBad()
		}
	}
	text := string(canonicalContextBytes)
	parts := strings.Split(text, ";")
	if len(parts) != 9 {
		return errBad()
	}
	userNS, ok := strings.CutPrefix(parts[0], "observer_user_ns=")
	if !ok || (userNS != "userns:initial" && !isLinuxScopedNSID(userNS, "userns")) {
		return errBad()
	}
	groupsStr, ok := strings.CutPrefix(parts[1], "groups=")
	if !ok {
		return errBad()
	}
	if len(groupsStr) > 0 {
		var prev uint32
		hasPrev := false
		for _, tok := range strings.Split(groupsStr, ",") {
			gid, ok := parseCanonicalU32(tok)
			if !ok || (hasPrev && gid <= prev) {
				return errBad()
			}
			prev = gid
			hasPrev = true
		}
	}
	nnp, ok := strings.CutPrefix(parts[2], "no_new_privs=")
	if !ok || (nnp != "0" && nnp != "1") {
		return errBad()
	}
	seccompStr, ok := strings.CutPrefix(parts[3], "seccomp=")
	if !ok {
		return errBad()
	}
	if _, ok := parseCanonicalU32(seccompStr); !ok {
		return errBad()
	}
	capKeys := [5]string{"cap_inh=", "cap_prm=", "cap_eff=", "cap_bnd=", "cap_amb="}
	for idx, ck := range capKeys {
		hexVal, ok := strings.CutPrefix(parts[4+idx], ck)
		if !ok || !isLowerHex16(hexVal) {
			return errBad()
		}
	}

	scheme := "urn:3leaps:identity-evidence:context-derivation:linux-procfs-status-secctx:v0"
	pre := []byte("3leaps.native-process-observation.security-context.v0\x00")
	pre = appendU32BE(pre, len(scheme))
	pre = append(pre, scheme...)
	pre = append(pre, 0)
	pre = appendU32BE(pre, len(canonicalContextBytes))
	pre = append(pre, canonicalContextBytes...)
	return sha256Prefixed(pre), nil
}

// IDMapExtent represents a single [First, First+Count) extent from Linux uid_map or gid_map.
type IDMapExtent struct {
	First uint32
	Count uint32
}

// LinuxIDIsUnambiguouslyMapped enforces the fail-closed check against Linux
// from_kuid_munged / from_kgid_munged overflow/unmapped ID conflation.
func LinuxIDIsUnambiguouslyMapped(renderedID uint32, extents []IDMapExtent, overflowID uint32) bool {
	if renderedID == math.MaxUint32 {
		return false
	}
	if len(extents) == 1 && extents[0].First == 0 && extents[0].Count == math.MaxUint32 {
		return true
	}
	id64 := uint64(renderedID)
	inExtent := false
	for _, ext := range extents {
		start := uint64(ext.First)
		end := start + uint64(ext.Count)
		if id64 >= start && id64 < end {
			inExtent = true
			break
		}
	}
	return inExtent && renderedID != overflowID
}

// ProjectHostScopeURN projects process.host_scope into the fixed 114-character URN
// urn:3leaps:identity-evidence:host-scope:v0:sha256:<64 lowercase hex>.
func ProjectHostScopeURN(hostScope any) (string, error) {
	hs, ok := hostScope.(map[string]any)
	if !ok {
		return "", newRefusal("invalid-observation-shape")
	}
	ns, ok1 := strField(hs, "namespace")
	val, ok2 := strField(hs, "value")
	drv, ok3 := objField(hs, "derivation")
	drvID, ok4 := strField(drv, "id")
	drvRev, ok5 := strField(drv, "revision")
	stab, ok6 := strField(hs, "stability")
	cb, ok7 := strField(hs, "clone_behavior")
	if !ok1 || !ok2 || !ok3 || !ok4 || !ok5 || !ok6 || !ok7 {
		return "", newRefusal("invalid-observation-shape")
	}
	pre := []byte("3leaps.native-process-observation.host-scope.v0\x00")
	for _, part := range []string{ns, val, drvID, drvRev, stab, cb} {
		pre = appendU32BE(pre, len(part))
		pre = append(pre, part...)
	}
	sha := sha256Prefixed(pre)
	return "urn:3leaps:identity-evidence:host-scope:v0:" + sha, nil
}

type startSubprofile int

const (
	subprofileInvalid startSubprofile = iota
	subprofileLinuxProcfsStatV0
	subprofileMacosKinfoStarttimeV0
)

func lookupStartSubprofile(obs map[string]any) startSubprofile {
	procObj, ok := objField(obs, "process")
	if !ok {
		return subprofileInvalid
	}
	si, ok := objField(procObj, "start_identity")
	if !ok {
		return subprofileInvalid
	}
	prof, ok := objField(si, "profile")
	if !ok {
		return subprofileInvalid
	}
	id, _ := strField(prof, "id")
	rev, _ := strField(prof, "revision")
	if rev != NativeProcessObservationReadmeRevision {
		return subprofileInvalid
	}
	switch id {
	case "urn:3leaps:identity-evidence:start-identity:linux-procfs-stat:v0":
		return subprofileLinuxProcfsStatV0
	case "urn:3leaps:identity-evidence:start-identity:macos-kinfo-starttime:v0":
		return subprofileMacosKinfoStarttimeV0
	default:
		return subprofileInvalid
	}
}

func (s startSubprofile) platformFamily() string {
	switch s {
	case subprofileLinuxProcfsStatV0:
		return "linux"
	case subprofileMacosKinfoStarttimeV0:
		return "macos"
	default:
		return ""
	}
}

func (s startSubprofile) validateStartValue(val string) bool {
	switch s {
	case subprofileLinuxProcfsStatV0:
		_, ok := parseCanonicalU64(val)
		return ok
	case subprofileMacosKinfoStarttimeV0:
		idx := strings.IndexByte(val, '.')
		if idx < 0 {
			return false
		}
		secStr, usecStr := val[:idx], val[idx+1:]
		if _, ok := parseCanonicalNonNegI64(secStr); !ok || len(usecStr) != 6 {
			return false
		}
		for i := 0; i < 6; i++ {
			if usecStr[i] < '0' || usecStr[i] > '9' {
				return false
			}
		}
		return true
	default:
		return false
	}
}

func observerUserNSScope(obs map[string]any) (string, bool) {
	observer, ok := objField(obs, "observer")
	if !ok {
		return "", false
	}
	uns, ok := objField(observer, "user_namespace")
	if !ok {
		return "", false
	}
	st, _ := strField(uns, "status")
	switch st {
	case "observed":
		val, ok := strField(uns, "value")
		return val, ok
	case "not-present":
		return "userns:initial", true
	default:
		return "", false
	}
}

func validatePrincipalTokens(val string, keys []string) bool {
	parts := strings.Split(val, ";")
	if len(parts) != len(keys) {
		return false
	}
	for i, key := range keys {
		numStr, ok := strings.CutPrefix(parts[i], key+"=")
		if !ok {
			return false
		}
		if _, ok := parseCanonicalU32(numStr); !ok {
			return false
		}
	}
	return true
}

func validateContextDerivation(factName string, factObj map[string]any, obs map[string]any) bool {
	observer, ok := objField(obs, "observer")
	if !ok {
		return false
	}
	pf, ok := strField(observer, "platform_family")
	if !ok {
		return false
	}
	drv, ok := objField(factObj, "derivation")
	if !ok {
		return false
	}
	drvID, _ := strField(drv, "id")
	drvRev, _ := strField(drv, "revision")
	if drvRev != NativeProcessObservationReadmeRevision {
		return false
	}
	switch factName {
	case "security_principal":
		p, ok := objField(factObj, "principal")
		if !ok {
			return false
		}
		ns, _ := strField(p, "namespace")
		val, _ := strField(p, "value")
		switch drvID {
		case "urn:3leaps:identity-evidence:principal-derivation:macos-kinfo-ucred:v0":
			return pf == "macos" &&
				ns == "urn:3leaps:posix-credentials:macos-kinfo:v0" &&
				validatePrincipalTokens(val, []string{"ruid", "euid", "rgid", "egid"})
		case "urn:3leaps:identity-evidence:principal-derivation:linux-procfs-status-uidgid:v0":
			if pf != "linux" {
				return false
			}
			scope, ok := observerUserNSScope(obs)
			if !ok {
				return false
			}
			return ns == "urn:3leaps:posix-credentials:linux-procfs-status:v0:"+scope &&
				validatePrincipalTokens(val, []string{"ruid", "euid", "suid", "fsuid", "rgid", "egid", "sgid", "fsgid"})
		default:
			return false
		}
	case "arguments_digest":
		return drvID == "urn:3leaps:identity-evidence:context-derivation:argv-bytes:v0" && (pf == "linux" || pf == "macos")
	case "executable_digest":
		return drvID == "urn:3leaps:identity-evidence:context-derivation:executable-file-bytes:v0" && (pf == "linux" || pf == "macos")
	case "namespace_context_digest":
		return drvID == "urn:3leaps:identity-evidence:context-derivation:linux-ns-inum-set:v0" && pf == "linux"
	case "security_context_digest":
		return drvID == "urn:3leaps:identity-evidence:context-derivation:linux-procfs-status-secctx:v0" && pf == "linux"
	default:
		return false
	}
}

func admitLiveObservationSteps(
	obs any,
	nowUTC *string,
	acceptedProfile ProfileRef,
	report any,
	inputReports map[string]any,
) error {
	if nowUTC == nil {
		return newRefusal("trusted-clock-unavailable")
	}
	nowUnix, ok := ParseStrictUTCUnix(*nowUTC)
	if !ok {
		return newRefusal("trusted-clock-unavailable")
	}
	obsObj, ok := obs.(map[string]any)
	if !ok {
		return newRefusal("unsupported-evidence-profile")
	}
	expectedProf := map[string]any{
		"id":       acceptedProfile.ID,
		"revision": acceptedProfile.Revision,
	}
	if !JSONEqual(obsObj["profile"], expectedProf) {
		return newRefusal("unsupported-evidence-profile")
	}
	obsAtStr, ok := strField(obsObj, "observed_at")
	if !ok {
		return newRefusal("invalid-observed-at")
	}
	obsUnix, ok := ParseStrictUTCUnix(obsAtStr)
	if !ok {
		return newRefusal("invalid-observed-at")
	}

	if report != nil {
		if err := NecessaryConditions(report, inputReports); err != nil {
			return err
		}
		repObj, ok := report.(map[string]any)
		if !ok {
			return newRefusal("invalid-report-shape")
		}
		obsDigest, err := CanonicalSHA256Value(obsObj)
		if err != nil {
			code, _ := RefusalCode(err)
			return newRefusal(code)
		}
		evArr, ok := arrField(repObj, "evidence")
		if !ok {
			return newRefusal("invalid-report-shape")
		}
		var matched []map[string]any
		for _, item := range evArr {
			if em, ok := item.(map[string]any); ok {
				if JSONEqual(em["format_profile"], expectedProf) || JSONEqual(em["digest"], obsDigest) {
					matched = append(matched, em)
				}
			}
		}
		if len(matched) != 1 {
			return newRefusal("observation-evidence-descriptor-missing")
		}
		ev := matched[0]
		procObj, _ := objField(obsObj, "process")
		if !JSONEqual(ev["format_profile"], expectedProf) ||
			!JSONEqual(ev["digest"], obsDigest) ||
			!JSONEqual(ev["method"], "os-observation") ||
			!JSONEqual(ev["subject_ref"], procObj["subject_ref"]) ||
			!JSONEqual(ev["collected_at"], obsObj["observed_at"]) {
			return newRefusal("observation-evidence-descriptor-mismatch")
		}
	}

	age := nowUnix - obsUnix
	if age < -maxNativeFutureSkewSecs {
		return newRefusal("future-process-observation")
	}
	if age > maxNativeObsAgeSecs {
		return newRefusal("stale-process-observation")
	}
	return nil
}

// AdmitLiveObservation enforces the standalone freshness and optional report-time
// binding gate for live native-process-observation:v0 consumers.
func AdmitLiveObservation(
	obs any,
	nowUTC *string,
	acceptedProfile ProfileRef,
	report any,
	inputReports map[string]any,
) error {
	if err := admitLiveObservationSteps(obs, nowUTC, acceptedProfile, report, inputReports); err != nil {
		return err
	}
	return ValidateObservationShape(obs)
}

// CompareNativeObservationsJSON evaluates CompareNativeObservations when policy
// arrives as an untrusted JSON value.
func CompareNativeObservationsJSON(
	baseline any,
	candidate any,
	rawPolicy any,
	acceptedProfile ProfileRef,
) ComparisonOutcome {
	pol, err := ParseComparisonPolicyJSON(rawPolicy)
	if err != nil {
		return outcomeInsufficient("invalid-evaluation-policy")
	}
	return CompareNativeObservations(baseline, candidate, pol, acceptedProfile)
}

// CompareNativeObservations deterministically compares baseline and candidate
// native-process-observation:v0 payloads in strict top-to-bottom row order (1a..9).
func CompareNativeObservations(
	baseline any,
	candidate any,
	policy ComparisonPolicy,
	acceptedProfile ProfileRef,
) ComparisonOutcome {
	// Row 1a
	reqFactsSet, allowedClones, ok := policy.validate()
	if !ok {
		return outcomeInsufficient("invalid-evaluation-policy")
	}

	baseObj, ok1 := baseline.(map[string]any)
	candObj, ok2 := candidate.(map[string]any)
	if !ok1 || !ok2 {
		return outcomeInsufficient("unsupported-evidence-profile")
	}

	// Row 1b
	expectedProf := map[string]any{
		"id":       acceptedProfile.ID,
		"revision": acceptedProfile.Revision,
	}
	if !JSONEqual(baseObj["profile"], expectedProf) || !JSONEqual(candObj["profile"], expectedProf) {
		return outcomeInsufficient("unsupported-evidence-profile")
	}

	// Row 1c
	bObs, _ := objField(baseObj, "observer")
	cObs, _ := objField(candObj, "observer")
	bBr, _ := bObs["bracketed"].(bool)
	cBr, _ := cObs["bracketed"].(bool)
	if !bBr || !cBr {
		return outcomeInsufficient("unbracketed-observation")
	}

	// Row 1d
	baseSub := lookupStartSubprofile(baseObj)
	candSub := lookupStartSubprofile(candObj)
	if baseSub == subprofileInvalid || candSub == subprofileInvalid {
		return outcomeInsufficient("unsupported-start-identity-profile")
	}

	// Row 1e
	bPlat, _ := strField(bObs, "platform_family")
	cPlat, _ := strField(cObs, "platform_family")
	if bPlat != baseSub.platformFamily() || cPlat != candSub.platformFamily() {
		return outcomeInsufficient("inconsistent-observation")
	}

	bProc, _ := objField(baseObj, "process")
	cProc, _ := objField(candObj, "process")
	bHS, _ := objField(bProc, "host_scope")
	cHS, _ := objField(cProc, "host_scope")

	// Row 2a
	bStab, _ := strField(bHS, "stability")
	cStab, _ := strField(cHS, "stability")
	if !policy.BaselineHostScopeTrusted || !policy.CandidateHostScopeTrusted ||
		!containsStr(establishedStabilities, bStab) || !containsStr(establishedStabilities, cStab) {
		return outcomeInsufficient("unestablished-host-scope")
	}

	// Row 2b
	for _, hs := range []map[string]any{bHS, cHS} {
		ns, okNS := strField(hs, "namespace")
		val, okVal := strField(hs, "value")
		validHS := okNS && isNonEmpty512(ns) && okVal && isNonEmpty512(val) && isProfileRefObj(hs["derivation"])
		if !validHS {
			return outcomeInsufficient("invalid-observation-shape")
		}
	}
	for _, key := range []string{"namespace", "value", "derivation"} {
		if !JSONEqual(bHS[key], cHS[key]) {
			return outcomeMismatch("host-scope-mismatch")
		}
	}

	// Row 2c
	bClone, _ := strField(bHS, "clone_behavior")
	cClone, _ := strField(cHS, "clone_behavior")
	if bStab != cStab || bClone != cClone {
		return outcomeInsufficient("inconsistent-host-scope-metadata")
	}

	// Row 2d
	if allowedClones != nil && (!allowedClones[bClone] || !allowedClones[cClone]) {
		return outcomeInsufficient("disallowed-clone-behavior")
	}

	// Rows 3a-3c
	bBootFact, _ := objField(bProc, "boot_id")
	cBootFact, _ := objField(cProc, "boot_id")
	bBootSt, _ := strField(bBootFact, "status")
	cBootSt, _ := strField(cBootFact, "status")
	if bBootSt != "observed" || cBootSt != "observed" {
		return outcomeInsufficient("missing-boot-id")
	}
	bBoot, _ := strField(bBootFact, "value")
	cBoot, _ := strField(cBootFact, "value")
	if !isLowercaseUUID(bBoot) || !isLowercaseUUID(cBoot) {
		return outcomeInsufficient("invalid-boot-id")
	}
	if bBoot != cBoot {
		return outcomeMismatch("boot-id-mismatch")
	}

	// Row 4a
	bStartFact, _ := objField(bProc, "start_identity")
	cStartFact, _ := objField(cProc, "start_identity")
	if !JSONEqual(bStartFact["profile"], cStartFact["profile"]) {
		return outcomeInsufficient("incompatible-start-identity-profile")
	}

	// Rows 4b-4k
	switch baseSub {
	case subprofileLinuxProcfsStatV0:
		bPNSFact, _ := objField(bProc, "pid_numbering_namespace")
		cPNSFact, _ := objField(cProc, "pid_numbering_namespace")
		bPNSSt, _ := strField(bPNSFact, "status")
		cPNSSt, _ := strField(cPNSFact, "status")
		if bPNSSt != "observed" || cPNSSt != "observed" {
			return outcomeInsufficient("missing-pid-numbering-namespace")
		}
		bPNS, _ := strField(bPNSFact, "value")
		cPNS, _ := strField(cPNSFact, "value")
		if !isLinuxScopedNSID(bPNS, "pidns") || !isLinuxScopedNSID(cPNS, "pidns") {
			return outcomeInsufficient("invalid-pid-namespace")
		}
		if bPNS != cPNS {
			return outcomeInsufficient("pid-numbering-namespace-mismatch")
		}

		// Rows 4e-4g
		bTNS, _ := objField(bObs, "time_namespace")
		cTNS, _ := objField(cObs, "time_namespace")
		bTNSSt, _ := strField(bTNS, "status")
		cTNSSt, _ := strField(cTNS, "status")
		if (bTNSSt != "observed" && bTNSSt != "not-present") ||
			(cTNSSt != "observed" && cTNSSt != "not-present") {
			return outcomeInsufficient("missing-observer-time-namespace")
		}
		bTNSVal, _ := strField(bTNS, "value")
		cTNSVal, _ := strField(cTNS, "value")
		if (bTNSSt == "observed" && !isLinuxScopedNSID(bTNSVal, "timens")) ||
			(cTNSSt == "observed" && !isLinuxScopedNSID(cTNSVal, "timens")) {
			return outcomeInsufficient("invalid-observer-time-namespace")
		}
		if bTNSSt != cTNSSt || (bTNSSt == "observed" && bTNSVal != cTNSVal) {
			return outcomeInsufficient("observer-time-namespace-mismatch")
		}

		// Rows 4h-4j
		bUNS, _ := objField(bObs, "user_namespace")
		cUNS, _ := objField(cObs, "user_namespace")
		bUNSSt, _ := strField(bUNS, "status")
		cUNSSt, _ := strField(cUNS, "status")
		if (bUNSSt != "observed" && bUNSSt != "not-present") ||
			(cUNSSt != "observed" && cUNSSt != "not-present") {
			return outcomeInsufficient("missing-observer-user-namespace")
		}
		bUNSVal, _ := strField(bUNS, "value")
		cUNSVal, _ := strField(cUNS, "value")
		if (bUNSSt == "observed" && !isLinuxScopedNSID(bUNSVal, "userns")) ||
			(cUNSSt == "observed" && !isLinuxScopedNSID(cUNSVal, "userns")) {
			return outcomeInsufficient("invalid-observer-user-namespace")
		}
		if bUNSSt != cUNSSt || (bUNSSt == "observed" && bUNSVal != cUNSVal) {
			return outcomeInsufficient("observer-user-namespace-mismatch")
		}
	case subprofileMacosKinfoStarttimeV0:
		for _, item := range []map[string]any{baseObj, candObj} {
			pItem, _ := objField(item, "process")
			oItem, _ := objField(item, "observer")
			pns, _ := objField(pItem, "pid_numbering_namespace")
			tpns, _ := objField(pItem, "target_pid_namespace")
			tns, _ := objField(oItem, "time_namespace")
			uns, _ := objField(oItem, "user_namespace")
			s1, _ := strField(pns, "status")
			s2, _ := strField(tpns, "status")
			s3, _ := strField(tns, "status")
			s4, _ := strField(uns, "status")
			if s1 != "unsupported" || s2 != "unsupported" || s3 != "unsupported" || s4 != "unsupported" {
				return outcomeInsufficient("inconsistent-observation")
			}
		}
	}

	// Rows 5a-5b
	bPID, _ := strField(bProc, "pid")
	cPID, _ := strField(cProc, "pid")
	if !isValidPIDString(bPID) || !isValidPIDString(cPID) {
		return outcomeInsufficient("invalid-pid-range")
	}
	if bPID != cPID {
		return outcomeMismatch("pid-mismatch")
	}

	// Row 5c
	bStateFact, _ := objField(bProc, "process_state")
	cStateFact, _ := objField(cProc, "process_state")
	bStateSt, _ := strField(bStateFact, "status")
	cStateSt, _ := strField(cStateFact, "status")
	bStartSt, _ := strField(bStartFact, "status")
	cStartSt, _ := strField(cStartFact, "status")
	if bStateSt == "observed" && bStartSt == "observed" &&
		(cStateSt == "not-present" || cStartSt == "not-present") {
		return outcomeMismatch("process-exited")
	}

	// Rows 5d-5f
	if baseSub == subprofileLinuxProcfsStatV0 {
		bTPNSFact, _ := objField(bProc, "target_pid_namespace")
		cTPNSFact, _ := objField(cProc, "target_pid_namespace")
		bTPNSSt, _ := strField(bTPNSFact, "status")
		cTPNSSt, _ := strField(cTPNSFact, "status")
		if bTPNSSt != "observed" || cTPNSSt != "observed" {
			return outcomeInsufficient("missing-target-pid-namespace")
		}
		bTPNS, _ := strField(bTPNSFact, "value")
		cTPNS, _ := strField(cTPNSFact, "value")
		if !isLinuxScopedNSID(bTPNS, "pidns") || !isLinuxScopedNSID(cTPNS, "pidns") {
			return outcomeInsufficient("invalid-pid-namespace")
		}
		if bTPNS != cTPNS {
			return outcomeMismatch("target-pid-namespace-mismatch")
		}
	}

	// Rows 6a-6c
	if bStartSt != "observed" || cStartSt != "observed" {
		return outcomeInsufficient("missing-start-identity")
	}
	bStartVal, _ := strField(bStartFact, "value")
	cStartVal, _ := strField(cStartFact, "value")
	if !baseSub.validateStartValue(bStartVal) || !candSub.validateStartValue(cStartVal) {
		return outcomeInsufficient("invalid-start-identity")
	}
	if bStartVal != cStartVal {
		return outcomeMismatch("start-identity-mismatch")
	}

	// Rows 7a-7b
	if bStateSt != "observed" || cStateSt != "observed" {
		return outcomeInsufficient("missing-process-state")
	}
	cStateVal, _ := strField(cStateFact, "value")
	if policy.RequireLive && cStateVal != "live" {
		return outcomeMismatch("process-not-live")
	}

	// Rows 8a-8d
	bCtx, _ := objField(bProc, "execution_context")
	cCtx, _ := objField(cProc, "execution_context")

	for _, fact := range CanonicalContextFacts {
		for _, pair := range [][2]map[string]any{{baseObj, bCtx}, {candObj, cCtx}} {
			fo, _ := objField(pair[1], fact)
			if st, _ := strField(fo, "status"); st == "observed" {
				if !validateContextDerivation(fact, fo, pair[0]) {
					return outcomeInsufficient("unsupported-context-derivation-" + fact)
				}
			}
		}
	}
	for _, fact := range CanonicalContextFacts {
		if reqFactsSet[fact] {
			bf, _ := objField(bCtx, fact)
			cf, _ := objField(cCtx, fact)
			bs, _ := strField(bf, "status")
			cs, _ := strField(cf, "status")
			if bs != "observed" || cs != "observed" {
				return outcomeInsufficient("missing-context-" + fact)
			}
		}
	}
	for _, fact := range CanonicalContextFacts {
		bf, _ := objField(bCtx, fact)
		cf, _ := objField(cCtx, fact)
		bs, _ := strField(bf, "status")
		cs, _ := strField(cf, "status")
		if bs == "observed" && cs == "observed" && (reqFactsSet[fact] || policy.DetectObservedContextDrift) {
			if !JSONEqual(bf["derivation"], cf["derivation"]) {
				return outcomeInsufficient("incompatible-context-derivation-" + fact)
			}
		}
	}
	for _, fact := range CanonicalContextFacts {
		bf, _ := objField(bCtx, fact)
		cf, _ := objField(cCtx, fact)
		bs, _ := strField(bf, "status")
		cs, _ := strField(cf, "status")
		if bs == "observed" && cs == "observed" && (reqFactsSet[fact] || policy.DetectObservedContextDrift) {
			if fact != "security_principal" {
				bd, okB := strField(bf, "digest")
				cd, okC := strField(cf, "digest")
				validDigests := okB && isSHA256Digest(bd) && okC && isSHA256Digest(cd)
				if !validDigests {
					return outcomeInsufficient("invalid-observation-shape")
				}
			}
			payloadKey := "digest"
			if fact == "security_principal" {
				payloadKey = "principal"
			}
			if !JSONEqual(bf[payloadKey], cf[payloadKey]) {
				return outcomeMismatch("context-changed-" + fact)
			}
		}
	}

	if err := ValidateObservationShape(baseline); err != nil {
		return outcomeInsufficient("invalid-observation-shape")
	}
	if err := ValidateObservationShape(candidate); err != nil {
		return outcomeInsufficient("invalid-observation-shape")
	}

	return ComparisonOutcome{Kind: OutcomeMatch}
}

// ProjectToLocalProcessSessionProcessJSON validates untrusted JSON policy/clock
// arguments before invoking ProjectToLocalProcessSessionProcess.
func ProjectToLocalProcessSessionProcessJSON(
	obs any,
	hostScopeTrusted any,
	nowUTC any,
	acceptedProfile ProfileRef,
	report any,
	inputReports map[string]any,
	allowedCloneBehaviors any,
) (map[string]any, error) {
	trusted, ok := hostScopeTrusted.(bool)
	if !ok {
		return nil, newRefusal("invalid-evaluation-policy")
	}
	var clones []string
	hasClones := false
	if allowedCloneBehaviors != nil {
		arr, ok := allowedCloneBehaviors.([]any)
		if !ok || len(arr) == 0 {
			return nil, newRefusal("invalid-evaluation-policy")
		}
		clones = make([]string, 0, len(arr))
		for _, item := range arr {
			s, ok := item.(string)
			if !ok || !containsStr(cloneBehaviors, s) {
				return nil, newRefusal("invalid-evaluation-policy")
			}
			clones = append(clones, s)
		}
		hasClones = true
	}
	var nowPtr *string
	if s, ok := nowUTC.(string); ok {
		nowPtr = &s
	} else if nowUTC != nil {
		return nil, newRefusal("trusted-clock-unavailable")
	}
	var clonesArg []string
	if hasClones {
		clonesArg = clones
	}
	return ProjectToLocalProcessSessionProcess(obs, trusted, nowPtr, acceptedProfile, report, inputReports, clonesArg)
}

// ProjectToLocalProcessSessionProcess is the fail-closed adapter projecting a
// native-process-observation:v0 payload into a local-process-session:v0 process object.
func ProjectToLocalProcessSessionProcess(
	obs any,
	hostScopeTrusted bool,
	nowUTC *string,
	acceptedProfile ProfileRef,
	report any,
	inputReports map[string]any,
	allowedCloneBehaviors []string,
) (map[string]any, error) {
	if allowedCloneBehaviors != nil {
		if len(allowedCloneBehaviors) == 0 {
			return nil, newRefusal("invalid-evaluation-policy")
		}
		for _, c := range allowedCloneBehaviors {
			if !containsStr(cloneBehaviors, c) {
				return nil, newRefusal("invalid-evaluation-policy")
			}
		}
	}
	if err := admitLiveObservationSteps(obs, nowUTC, acceptedProfile, report, inputReports); err != nil {
		return nil, err
	}
	obsObj, _ := obs.(map[string]any)
	observer, _ := objField(obsObj, "observer")
	if br, _ := observer["bracketed"].(bool); !br {
		return nil, newRefusal("unbracketed-observation")
	}
	sub := lookupStartSubprofile(obsObj)
	if sub == subprofileInvalid {
		return nil, newRefusal("insufficient:unsupported-start-identity-profile")
	}
	if pf, _ := strField(observer, "platform_family"); pf != sub.platformFamily() {
		return nil, newRefusal("insufficient:inconsistent-observation")
	}

	procObj, _ := objField(obsObj, "process")
	hs, _ := objField(procObj, "host_scope")
	stab, _ := strField(hs, "stability")
	if !hostScopeTrusted || !containsStr(establishedStabilities, stab) {
		return nil, newRefusal("unestablished-host-scope")
	}
	if allowedCloneBehaviors != nil {
		cb, _ := strField(hs, "clone_behavior")
		if !containsStr(allowedCloneBehaviors, cb) {
			return nil, newRefusal("disallowed-clone-behavior")
		}
	}

	stateFact, _ := objField(procObj, "process_state")
	stateSt, _ := strField(stateFact, "status")
	stateVal, _ := strField(stateFact, "value")
	if stateSt != "observed" || stateVal != "live" {
		return nil, newRefusal("process-not-live")
	}

	bootFact, _ := objField(procObj, "boot_id")
	bootSt, _ := strField(bootFact, "status")
	bootVal, _ := strField(bootFact, "value")
	if bootSt != "observed" || !isLowercaseUUID(bootVal) {
		return nil, newRefusal("missing-or-invalid-boot-id")
	}

	startFact, _ := objField(procObj, "start_identity")
	startSt, _ := strField(startFact, "status")
	startVal, _ := strField(startFact, "value")
	if startSt != "observed" || !sub.validateStartValue(startVal) {
		return nil, newRefusal("missing-or-invalid-start-identity")
	}

	pnsFact, _ := objField(procObj, "pid_numbering_namespace")
	tpnsFact, _ := objField(procObj, "target_pid_namespace")
	pnsSt, _ := strField(pnsFact, "status")
	tpnsSt, _ := strField(tpnsFact, "status")
	if pnsSt != "observed" || tpnsSt != "observed" {
		return nil, newRefusal("missing-pid-namespace-for-session-bundle")
	}
	pnsVal, _ := strField(pnsFact, "value")
	tpnsVal, _ := strField(tpnsFact, "value")
	if !isLinuxScopedNSID(pnsVal, "pidns") || !isLinuxScopedNSID(tpnsVal, "pidns") || pnsVal != tpnsVal {
		return nil, newRefusal("ambiguous-pid-namespace-for-session-bundle")
	}

	tns, _ := objField(observer, "time_namespace")
	tnsSt, _ := strField(tns, "status")
	tnsVal, _ := strField(tns, "value")
	if (tnsSt != "observed" && tnsSt != "not-present") ||
		(tnsSt == "observed" && !isLinuxScopedNSID(tnsVal, "timens")) {
		return nil, newRefusal("invalid-observer-time-namespace-for-session-bundle")
	}

	uns, _ := objField(observer, "user_namespace")
	unsSt, _ := strField(uns, "status")
	unsVal, _ := strField(uns, "value")
	if (unsSt != "observed" && unsSt != "not-present") ||
		(unsSt == "observed" && !isLinuxScopedNSID(unsVal, "userns")) {
		return nil, newRefusal("invalid-observer-user-namespace-for-session-bundle")
	}

	pidStr, okPID := strField(procObj, "pid")
	if !okPID || !isValidPIDString(pidStr) {
		return nil, newRefusal("invalid-pid-range")
	}

	ctx, _ := objField(procObj, "execution_context")
	for _, fact := range CanonicalContextFacts {
		fo, _ := objField(ctx, fact)
		if st, _ := strField(fo, "status"); st != "observed" {
			return nil, newRefusal("missing-context-for-session-bundle:" + fact)
		}
		if !validateContextDerivation(fact, fo, obsObj) {
			return nil, newRefusal("unsupported-context-derivation:" + fact)
		}
	}

	projectedHostScope, err := ProjectHostScopeURN(hs)
	if err != nil {
		return nil, err
	}
	if len(projectedHostScope) > 512 {
		return nil, newRefusal("projected-host-scope-too-long")
	}

	clockDomain := "host-boottime"
	if tnsSt == "observed" {
		clockDomain = tnsVal
	}
	projectedStartVal := clockDomain + ";ticks=" + startVal
	if len(projectedStartVal) > 512 {
		return nil, newRefusal("projected-start-identity-too-long")
	}

	if err := ValidateObservationShape(obs); err != nil {
		return nil, err
	}

	spFact, _ := objField(ctx, "security_principal")
	nsFact, _ := objField(ctx, "namespace_context_digest")
	exeFact, _ := objField(ctx, "executable_digest")
	argFact, _ := objField(ctx, "arguments_digest")
	secFact, _ := objField(ctx, "security_context_digest")
	spPrin, _ := objField(spFact, "principal")
	spCopy := map[string]any{
		"namespace": spPrin["namespace"],
		"value":     spPrin["value"],
	}

	return map[string]any{
		"subject_ref":   procObj["subject_ref"],
		"host_scope":    projectedHostScope,
		"boot_id":       bootVal,
		"pid_namespace": tpnsVal,
		"pid":           procObj["pid"],
		"start_identity": map[string]any{
			"profile": map[string]any{
				"id":       LinuxProcfsStatTimensScopedProfileID,
				"revision": NativeProcessObservationReadmeRevision,
			},
			"value": projectedStartVal,
		},
		"execution_context": map[string]any{
			"security_principal":       spCopy,
			"namespace_context_digest": nsFact["digest"],
			"executable_digest":        exeFact["digest"],
			"arguments_digest":         argFact["digest"],
			"security_context_digest":  secFact["digest"],
		},
	}, nil
}

// BindNativeObservationReport verifies observation-only identity-evidence/v0
// report descriptor binding to a native-process-observation:v0 payload.
func BindNativeObservationReport(report any, obs any, acceptedProfile ProfileRef) error {
	obsObj, ok := obs.(map[string]any)
	if !ok {
		return newRefusal("unsupported-evidence-profile")
	}
	expectedProf := map[string]any{
		"id":       acceptedProfile.ID,
		"revision": acceptedProfile.Revision,
	}
	if !JSONEqual(obsObj["profile"], expectedProf) {
		return newRefusal("unsupported-evidence-profile")
	}
	repObj, ok := report.(map[string]any)
	if !ok {
		return newRefusal("invalid-report-shape")
	}
	assArr, ok := arrField(repObj, "assessments")
	if !ok {
		return newRefusal("invalid-report-shape")
	}
	_, hasEval := repObj["evaluator"]
	if len(assArr) != 0 || hasEval {
		return newRefusal("observation-report-not-observation-only")
	}

	subjArr, ok := arrField(repObj, "subjects")
	if !ok {
		return newRefusal("invalid-report-shape")
	}
	subjects := make(map[string]map[string]any, len(subjArr))
	for _, item := range subjArr {
		if sm, ok := item.(map[string]any); ok {
			if sid, ok := strField(sm, "id"); ok {
				subjects[sid] = sm
			}
		}
	}
	procObj, _ := objField(obsObj, "process")
	procSubj, _ := strField(procObj, "subject_ref")
	if kind, _ := strField(subjects[procSubj], "kind"); kind != "process" {
		return newRefusal("observation-subject-not-process")
	}

	hs, _ := objField(procObj, "host_scope")
	if identRef, hasIR := strField(hs, "identifier_ref"); hasIR {
		var identObj map[string]any
		if idArr, ok := arrField(repObj, "identifiers"); ok {
			for _, item := range idArr {
				if im, ok := item.(map[string]any); ok {
					if iid, _ := strField(im, "id"); iid == identRef {
						identObj = im
						break
					}
				}
			}
		}
		if identObj == nil {
			return newRefusal("host-scope-identifier-mismatch")
		}
		idSubj, _ := strField(identObj, "subject_ref")
		idSubjKind, _ := strField(subjects[idSubj], "kind")
		validHostKinds := []string{"os-installation", "physical-device", "virtual-machine", "container"}
		if !containsStr(validHostKinds, idSubjKind) {
			return newRefusal("host-scope-identifier-mismatch")
		}
		for _, k := range []string{"namespace", "value", "derivation", "stability", "clone_behavior"} {
			if !JSONEqual(identObj[k], hs[k]) {
				return newRefusal("host-scope-identifier-mismatch")
			}
		}
	}

	obsDigest, err := CanonicalSHA256Value(obsObj)
	if err != nil {
		code, _ := RefusalCode(err)
		return newRefusal(code)
	}
	evArr, ok := arrField(repObj, "evidence")
	if !ok {
		return newRefusal("invalid-report-shape")
	}
	var matched []map[string]any
	for _, item := range evArr {
		if em, ok := item.(map[string]any); ok {
			if JSONEqual(em["format_profile"], expectedProf) || JSONEqual(em["digest"], obsDigest) {
				matched = append(matched, em)
			}
		}
	}
	if len(matched) != 1 {
		return newRefusal("observation-evidence-descriptor-missing")
	}
	ev := matched[0]
	_, hasKey := ev["key_ref"]
	_, hasIdent := ev["identifier_ref"]
	_, hasBound := ev["boundary"]
	if !JSONEqual(ev["format_profile"], expectedProf) ||
		!JSONEqual(ev["digest"], obsDigest) ||
		!JSONEqual(ev["method"], "os-observation") ||
		!JSONEqual(ev["subject_ref"], procObj["subject_ref"]) ||
		!JSONEqual(ev["collected_at"], obsObj["observed_at"]) ||
		hasKey || hasIdent || hasBound {
		return newRefusal("observation-evidence-descriptor-mismatch")
	}
	if err := ValidateReportShape(report); err != nil {
		return err
	}
	return ValidateObservationShape(obs)
}
