// SPDX-License-Identifier: MIT OR Apache-2.0

package evidence

import (
	"sort"
	"strings"
)

const (
	// IdentityEvidenceContract is the closed contract identifier for identity-evidence/v0 reports.
	IdentityEvidenceContract = "identity-evidence/v0"
	// MaxProvenanceDepth is the maximum transitive input_reports depth.
	MaxProvenanceDepth = 4
)

var (
	subjectKinds = []string{
		"physical-device", "os-installation", "virtual-machine", "container",
		"process", "session", "key-holder",
	}
	stabilities = []string{
		"unknown", "process", "boot", "installation", "enrollment", "device",
	}
	cloneBehaviors = []string{
		"unknown", "copyable", "regenerated", "device-bound",
	}
	evidenceMethods = []string{
		"declaration", "os-observation", "enrollment", "possession-proof",
		"device-attestation", "workload-attestation", "authority-statement",
		"custody-observation", "isolation-observation",
	}
	dimensions = []string{
		"identity-binding", "key-custody", "execution-isolation", "platform-evidence",
	}
	identityBindingValues = []string{
		"unknown", "declared", "observed", "enrolled-key", "attested-key",
	}
	keyCustodyValues = []string{
		"unknown", "not-applicable", "software", "os-managed",
		"hardware-backed", "virtual-hardware-backed",
	}
	executionIsolationValues = []string{
		"unknown", "shared-account", "separate-account", "os-confined",
		"virtual-machine", "confidential-workload",
	}
	platformEvidenceValues = []string{
		"unknown", "declared", "locally-observed", "authority-verified", "hardware-attested",
	}
	assessmentStates = []string{
		"current", "stale", "revoked", "conflicting", "unknown",
	}
	capabilityStatuses = []string{
		"available", "not-present", "inaccessible", "unsupported",
		"not-probed", "error", "unknown",
	}
)

func validateUniqueLocalIDArray(v any, minItems, maxItems int) bool {
	arr, ok := v.([]any)
	if !ok || len(arr) < minItems || len(arr) > maxItems {
		return false
	}
	seen := make(map[string]struct{}, len(arr))
	for _, item := range arr {
		s, ok := item.(string)
		if !ok || !isLocalID(s) {
			return false
		}
		if _, dup := seen[s]; dup {
			return false
		}
		seen[s] = struct{}{}
	}
	return true
}

func validateUniqueStringArray(v any, minItems, maxItems int) bool {
	arr, ok := v.([]any)
	if !ok || len(arr) < minItems || len(arr) > maxItems {
		return false
	}
	seen := make(map[string]struct{}, len(arr))
	for _, item := range arr {
		s, ok := item.(string)
		if !ok || !isNonEmpty512(s) {
			return false
		}
		if _, dup := seen[s]; dup {
			return false
		}
		seen[s] = struct{}{}
	}
	return true
}

func validateSoftwareObj(v any) bool {
	m, ok := v.(map[string]any)
	if !ok || !checkAllowedKeys(m, []string{"name", "revision"}, nil) {
		return false
	}
	name, ok1 := strField(m, "name")
	rev, ok2 := strField(m, "revision")
	return ok1 && ok2 && isNonEmpty512(name) && isSHA256Digest(rev)
}

func validateOriginObj(v any) bool {
	m, ok := v.(map[string]any)
	if !ok || !checkAllowedKeys(m, []string{"input_ref", "record_ref"}, nil) {
		return false
	}
	ir, ok1 := strField(m, "input_ref")
	rr, ok2 := strField(m, "record_ref")
	return ok1 && ok2 && isLocalID(ir) && isLocalID(rr)
}

func validateBoundaryObj(v any) bool {
	m, ok := v.(map[string]any)
	if !ok || !checkAllowedKeys(m, []string{"profile", "comparison_subject_refs", "threats"}, []string{"external_boundary"}) {
		return false
	}
	if !isProfileRefObj(m["profile"]) ||
		!validateUniqueLocalIDArray(m["comparison_subject_refs"], 0, 256) ||
		!validateUniqueStringArray(m["threats"], 1, 256) {
		return false
	}
	if ext, exists := m["external_boundary"]; exists {
		s, ok := ext.(string)
		if !ok || !isNonEmpty512(s) {
			return false
		}
	}
	comp, _ := m["comparison_subject_refs"].([]any)
	_, hasExt := m["external_boundary"]
	return len(comp) > 0 || hasExt
}

func validateTimeAttributionObj(v any) bool {
	m, ok := v.(map[string]any)
	if !ok || !checkAllowedKeys(m, []string{"instant", "source"}, []string{"evidence"}) {
		return false
	}
	inst, ok := strField(m, "instant")
	if !ok {
		return false
	}
	if _, ok := ParseStrictUTCUnix(inst); !ok {
		return false
	}
	source, ok := strField(m, "source")
	if !ok {
		return false
	}
	switch source {
	case "producer_clock":
		_, hasEv := m["evidence"]
		return !hasEv
	case "trusted_time":
		ev, ok := objField(m, "evidence")
		if !ok || !checkAllowedKeys(ev, []string{"format_profile", "digest"}, []string{"ref"}) {
			return false
		}
		dig, ok := strField(ev, "digest")
		if !isProfileRefObj(ev["format_profile"]) || !ok || !isSHA256Digest(dig) {
			return false
		}
		if r, exists := ev["ref"]; exists {
			rs, ok := r.(string)
			if !ok || !isNonEmpty512(rs) {
				return false
			}
		}
		return true
	default:
		return false
	}
}

// ValidateReportShape validates strict identity-evidence/v0 report.schema.json
// structural and conditional rules.
func ValidateReportShape(report any) error {
	errShape := func() error { return newRefusal("invalid-report-shape") }
	obj, ok := report.(map[string]any)
	if !ok {
		return errShape()
	}
	required := []string{
		"contract", "report_id", "created_at", "producer",
		"subjects", "identifiers", "evidence", "assessments",
		"capabilities", "keys", "claims", "input_reports",
	}
	if !checkAllowedKeys(obj, required, []string{"evaluator", "time_attribution"}) {
		return errShape()
	}
	if c, ok := strField(obj, "contract"); !ok || c != IdentityEvidenceContract {
		return errShape()
	}
	if rid, ok := strField(obj, "report_id"); !ok || !isLocalID(rid) {
		return errShape()
	}
	createdStr, ok := strField(obj, "created_at")
	if !ok {
		return errShape()
	}
	if _, ok := ParseStrictUTCUnix(createdStr); !ok {
		return errShape()
	}
	if !validateSoftwareObj(obj["producer"]) {
		return errShape()
	}
	if ta, exists := obj["time_attribution"]; exists && !validateTimeAttributionObj(ta) {
		return errShape()
	}

	assessments, ok := arrField(obj, "assessments")
	if !ok || len(assessments) > 256 {
		return errShape()
	}
	if len(assessments) > 0 {
		eval, ok := objField(obj, "evaluator")
		if !ok || !checkAllowedKeys(eval, []string{"software", "principal", "verification_profile"}, nil) ||
			!validateSoftwareObj(eval["software"]) ||
			!isPrincipalObj(eval["principal"]) ||
			!isProfileRefObj(eval["verification_profile"]) {
			return errShape()
		}
	} else if _, hasEval := obj["evaluator"]; hasEval {
		return errShape()
	}

	subjects, ok := arrField(obj, "subjects")
	if !ok || len(subjects) == 0 || len(subjects) > 256 {
		return errShape()
	}
	for _, s := range subjects {
		so, ok := s.(map[string]any)
		if !ok || !checkAllowedKeys(so, []string{"id", "kind"}, []string{"label"}) {
			return errShape()
		}
		sid, ok1 := strField(so, "id")
		sk, ok2 := strField(so, "kind")
		if !ok1 || !isLocalID(sid) || !ok2 || !containsStr(subjectKinds, sk) {
			return errShape()
		}
		if lbl, exists := so["label"]; exists {
			ls, ok := lbl.(string)
			if !ok || !isNonEmpty512(ls) {
				return errShape()
			}
		}
	}

	identifiers, ok := arrField(obj, "identifiers")
	if !ok || len(identifiers) > 256 {
		return errShape()
	}
	for _, ident := range identifiers {
		io, ok := ident.(map[string]any)
		if !ok || !checkAllowedKeys(io, []string{
			"id", "subject_ref", "namespace", "value", "derivation", "stability", "clone_behavior",
		}, nil) {
			return errShape()
		}
		iid, ok0 := strField(io, "id")
		isr, ok1 := strField(io, "subject_ref")
		ins, ok2 := strField(io, "namespace")
		ival, ok3 := strField(io, "value")
		istab, ok4 := strField(io, "stability")
		iclone, ok5 := strField(io, "clone_behavior")
		if !ok0 || !isLocalID(iid) || !ok1 || !isLocalID(isr) ||
			!ok2 || !isNonEmpty512(ins) || !ok3 || !isNonEmpty512(ival) ||
			!isProfileRefObj(io["derivation"]) ||
			!ok4 || !containsStr(stabilities, istab) ||
			!ok5 || !containsStr(cloneBehaviors, iclone) {
			return errShape()
		}
	}

	evidenceArr, ok := arrField(obj, "evidence")
	if !ok || len(evidenceArr) > 256 {
		return errShape()
	}
	for _, ev := range evidenceArr {
		eo, ok := ev.(map[string]any)
		if !ok || !checkAllowedKeys(eo, []string{
			"id", "subject_ref", "method", "source", "collector", "collected_at", "digest", "format_profile",
		}, []string{
			"identifier_ref", "key_ref", "claim_refs", "boundary", "origin", "time_attribution",
		}) {
			return errShape()
		}
		eid, ok0 := strField(eo, "id")
		esr, ok1 := strField(eo, "subject_ref")
		em, ok2 := strField(eo, "method")
		esrc, ok3 := strField(eo, "source")
		ecol, ok4 := strField(eo, "collector")
		eca, ok5 := strField(eo, "collected_at")
		edig, ok6 := strField(eo, "digest")
		if !ok0 || !isLocalID(eid) || !ok1 || !isLocalID(esr) ||
			!ok2 || !containsStr(evidenceMethods, em) ||
			!ok3 || !isNonEmpty512(esrc) || !ok4 || !isNonEmpty512(ecol) ||
			!ok5 || !ok6 || !isSHA256Digest(edig) || !isProfileRefObj(eo["format_profile"]) {
			return errShape()
		}
		if _, ok := ParseStrictUTCUnix(eca); !ok {
			return errShape()
		}
		if ir, exists := eo["identifier_ref"]; exists {
			s, ok := ir.(string)
			if !ok || !isLocalID(s) {
				return errShape()
			}
		}
		if kr, exists := eo["key_ref"]; exists {
			s, ok := kr.(string)
			if !ok || !isLocalID(s) {
				return errShape()
			}
		}
		if cr, exists := eo["claim_refs"]; exists && !validateUniqueLocalIDArray(cr, 1, 256) {
			return errShape()
		}
		if b, exists := eo["boundary"]; exists && !validateBoundaryObj(b) {
			return errShape()
		}
		if o, exists := eo["origin"]; exists && !validateOriginObj(o) {
			return errShape()
		}
		if ta, exists := eo["time_attribution"]; exists && !validateTimeAttributionObj(ta) {
			return errShape()
		}
	}

	for _, a := range assessments {
		ao, ok := a.(map[string]any)
		if !ok || !checkAllowedKeys(ao, []string{
			"subject_ref", "scope", "dimension", "value", "state",
			"rule", "assessed_at", "valid_until", "evidence_refs", "residuals",
		}, []string{
			"identifier_ref", "key_ref", "claim_refs", "boundary", "time_attribution",
		}) {
			return errShape()
		}
		asr, ok0 := strField(ao, "subject_ref")
		ascope, ok1 := strField(ao, "scope")
		dim, ok2 := strField(ao, "dimension")
		val, ok3 := strField(ao, "value")
		state, ok4 := strField(ao, "state")
		aat, ok5 := strField(ao, "assessed_at")
		avu, ok6 := strField(ao, "valid_until")
		if !ok0 || !isLocalID(asr) || !ok1 || !isNonEmpty512(ascope) ||
			!ok2 || !containsStr(dimensions, dim) ||
			!ok3 || !ok4 || !containsStr(assessmentStates, state) ||
			!isProfileRefObj(ao["rule"]) || !ok5 || !ok6 ||
			!validateUniqueLocalIDArray(ao["evidence_refs"], 0, 256) {
			return errShape()
		}
		if _, ok := ParseStrictUTCUnix(aat); !ok {
			return errShape()
		}
		if _, ok := ParseStrictUTCUnix(avu); !ok {
			return errShape()
		}
		residuals, ok := arrField(ao, "residuals")
		if !ok || len(residuals) > 256 {
			return errShape()
		}
		for _, r := range residuals {
			rs, ok := r.(string)
			if !ok || !isNonEmpty512(rs) {
				return errShape()
			}
		}
		if ir, exists := ao["identifier_ref"]; exists {
			s, ok := ir.(string)
			if !ok || !isLocalID(s) {
				return errShape()
			}
		}
		if kr, exists := ao["key_ref"]; exists {
			s, ok := kr.(string)
			if !ok || !isLocalID(s) {
				return errShape()
			}
		}
		if cr, exists := ao["claim_refs"]; exists && !validateUniqueLocalIDArray(cr, 1, 256) {
			return errShape()
		}
		if b, exists := ao["boundary"]; exists && !validateBoundaryObj(b) {
			return errShape()
		}
		if ta, exists := ao["time_attribution"]; exists && !validateTimeAttributionObj(ta) {
			return errShape()
		}

		var dimOK bool
		switch dim {
		case "identity-binding":
			dimOK = containsStr(identityBindingValues, val)
		case "key-custody":
			dimOK = containsStr(keyCustodyValues, val)
		case "execution-isolation":
			dimOK = containsStr(executionIsolationValues, val)
		case "platform-evidence":
			dimOK = containsStr(platformEvidenceValues, val)
		}
		if !dimOK {
			return errShape()
		}
		if val == "unknown" && state != "unknown" {
			return errShape()
		}
		evRefs, _ := ao["evidence_refs"].([]any)
		if val != "unknown" && val != "not-applicable" && len(evRefs) == 0 {
			return errShape()
		}
		_, hasIdent := ao["identifier_ref"]
		_, hasKey := ao["key_ref"]
		_, hasBoundary := ao["boundary"]
		_, hasClaims := ao["claim_refs"]
		if dim == "identity-binding" && val != "unknown" && !hasIdent {
			return errShape()
		}
		if dim == "key-custody" && val != "unknown" && val != "not-applicable" && !hasKey {
			return errShape()
		}
		if dim == "identity-binding" && (val == "enrolled-key" || val == "attested-key") && !hasKey {
			return errShape()
		}
		if (dim == "execution-isolation") != hasBoundary {
			return errShape()
		}
		if (dim == "platform-evidence") != hasClaims {
			return errShape()
		}
		if dim == "key-custody" && val == "not-applicable" && hasKey {
			return errShape()
		}
	}

	capabilities, ok := arrField(obj, "capabilities")
	if !ok || len(capabilities) > 256 {
		return errShape()
	}
	for _, c := range capabilities {
		co, ok := c.(map[string]any)
		if !ok || !checkAllowedKeys(co, []string{
			"id", "subject_ref", "provider", "capability", "status", "reason", "observed_at",
		}, []string{"origin", "time_attribution"}) {
			return errShape()
		}
		cid, ok0 := strField(co, "id")
		csr, ok1 := strField(co, "subject_ref")
		cprov, ok2 := strField(co, "provider")
		ccap, ok3 := strField(co, "capability")
		cst, ok4 := strField(co, "status")
		creason, ok5 := strField(co, "reason")
		coat, ok6 := strField(co, "observed_at")
		if !ok0 || !isLocalID(cid) || !ok1 || !isLocalID(csr) ||
			!ok2 || !isNonEmpty512(cprov) || !ok3 || !isNonEmpty512(ccap) ||
			!ok4 || !containsStr(capabilityStatuses, cst) ||
			!ok5 || !isNonEmpty512(creason) || !ok6 {
			return errShape()
		}
		if _, ok := ParseStrictUTCUnix(coat); !ok {
			return errShape()
		}
		if o, exists := co["origin"]; exists && !validateOriginObj(o) {
			return errShape()
		}
		if ta, exists := co["time_attribution"]; exists && !validateTimeAttributionObj(ta) {
			return errShape()
		}
	}

	keysArr, ok := arrField(obj, "keys")
	if !ok || len(keysArr) > 256 {
		return errShape()
	}
	for _, k := range keysArr {
		ko, ok := k.(map[string]any)
		if !ok || !checkAllowedKeys(ko, []string{"id", "subject_ref", "representation_profile", "public_key_digest"}, nil) {
			return errShape()
		}
		kid, ok0 := strField(ko, "id")
		ksr, ok1 := strField(ko, "subject_ref")
		kdig, ok2 := strField(ko, "public_key_digest")
		if !ok0 || !isLocalID(kid) || !ok1 || !isLocalID(ksr) ||
			!isProfileRefObj(ko["representation_profile"]) || !ok2 || !isSHA256Digest(kdig) {
			return errShape()
		}
	}

	claimsArr, ok := arrField(obj, "claims")
	if !ok || len(claimsArr) > 256 {
		return errShape()
	}
	for _, cl := range claimsArr {
		co, ok := cl.(map[string]any)
		if !ok || !checkAllowedKeys(co, []string{"id", "subject_ref", "property_profile", "property", "value"}, []string{"key_ref"}) {
			return errShape()
		}
		cid, ok0 := strField(co, "id")
		csr, ok1 := strField(co, "subject_ref")
		cprop, ok2 := strField(co, "property")
		if !ok0 || !isLocalID(cid) || !ok1 || !isLocalID(csr) ||
			!isProfileRefObj(co["property_profile"]) || !ok2 || !isNonEmpty512(cprop) {
			return errShape()
		}
		cv := co["value"]
		switch x := cv.(type) {
		case string:
			if len([]rune(x)) > 512 {
				return errShape()
			}
		case bool, nil:
		default:
			if _, ok := asJSONNumberFloat(cv); !ok {
				return errShape()
			}
		}
		if kr, exists := co["key_ref"]; exists {
			ks, ok := kr.(string)
			if !ok || !isLocalID(ks) {
				return errShape()
			}
		}
	}

	inputReports, ok := arrField(obj, "input_reports")
	if !ok || len(inputReports) > 256 {
		return errShape()
	}
	for _, ir := range inputReports {
		io, ok := ir.(map[string]any)
		if !ok || !checkAllowedKeys(io, []string{"id", "report_id", "digest"}, nil) {
			return errShape()
		}
		iid, ok0 := strField(io, "id")
		irid, ok1 := strField(io, "report_id")
		idig, ok2 := strField(io, "digest")
		if !ok0 || !isLocalID(iid) || !ok1 || !isLocalID(irid) || !ok2 || !isSHA256Digest(idig) {
			return errShape()
		}
	}

	return nil
}

func boundaryIdentity(boundary any) string {
	b, ok := boundary.(map[string]any)
	if !ok {
		return ""
	}
	norm := make(map[string]any, len(b))
	for k, v := range b {
		norm[k] = v
	}
	for _, field := range []string{"comparison_subject_refs", "threats"} {
		if arr, ok := b[field].([]any); ok {
			strs := make([]string, 0, len(arr))
			for _, item := range arr {
				if s, ok := item.(string); ok {
					strs = append(strs, s)
				}
			}
			sort.Strings(strs)
			sortedAny := make([]any, len(strs))
			for i, s := range strs {
				sortedAny[i] = s
			}
			norm[field] = sortedAny
		}
	}
	dig, err := CanonicalSHA256Value(norm)
	if err != nil {
		return ""
	}
	return dig
}

func stripOrigin(rec map[string]any) map[string]any {
	out := make(map[string]any, len(rec))
	for k, v := range rec {
		if k != "origin" {
			out[k] = v
		}
	}
	return out
}

// NecessaryConditions enforces identity-evidence/v0 reference, time,
// assessment-method, and input_reports carry-forward rules.
func NecessaryConditions(report any, inputReports map[string]any) error {
	if err := ValidateReportShape(report); err != nil {
		return err
	}
	obj, ok := report.(map[string]any)
	if !ok {
		return newRefusal("invalid-report-shape")
	}

	collections := []string{
		"subjects", "identifiers", "keys", "claims", "evidence", "capabilities", "input_reports",
	}
	indexes := make(map[string]map[string]map[string]any, len(collections))
	for _, coll := range collections {
		arr, ok := arrField(obj, coll)
		if !ok {
			return newRefusal("invalid-report-shape")
		}
		m := make(map[string]map[string]any, len(arr))
		for _, item := range arr {
			rm, ok := item.(map[string]any)
			if !ok {
				return newRefusal("invalid-report-shape")
			}
			id, ok := strField(rm, "id")
			if !ok {
				return newRefusal("invalid-report-shape")
			}
			if _, dup := m[id]; dup {
				return newRefusal("duplicate-reference")
			}
			m[id] = rm
		}
		indexes[coll] = m
	}

	inputArr, _ := arrField(obj, "input_reports")
	seenReportIDs := make(map[string]struct{}, len(inputArr))
	for _, item := range inputArr {
		rm, _ := item.(map[string]any)
		rid, ok := strField(rm, "report_id")
		if !ok {
			return newRefusal("invalid-report-shape")
		}
		if _, dup := seenReportIDs[rid]; dup {
			return newRefusal("duplicate-reference")
		}
		seenReportIDs[rid] = struct{}{}
	}

	if ta, ok := objField(obj, "time_attribution"); ok {
		if !JSONEqual(ta["instant"], obj["created_at"]) {
			return newRefusal("time-attribution-instant-mismatch")
		}
	}
	for _, pair := range [][2]string{
		{"evidence", "collected_at"},
		{"assessments", "assessed_at"},
		{"capabilities", "observed_at"},
	} {
		arr, _ := arrField(obj, pair[0])
		for _, item := range arr {
			rm, _ := item.(map[string]any)
			if ta, ok := objField(rm, "time_attribution"); ok {
				if !JSONEqual(ta["instant"], rm[pair[1]]) {
					return newRefusal("time-attribution-instant-mismatch")
				}
			}
		}
	}

	createdStr, ok := strField(obj, "created_at")
	if !ok {
		return newRefusal("invalid-report-shape")
	}
	created, ok := ParseStrictUTCUnix(createdStr)
	if !ok {
		return newRefusal("invalid-report-shape")
	}

	assArr, _ := arrField(obj, "assessments")
	for _, item := range assArr {
		a, _ := item.(map[string]any)
		dim, _ := strField(a, "dimension")
		val, _ := strField(a, "value")
		if dim == "key-custody" && val == "not-applicable" {
			if _, hasKey := a["key_ref"]; hasKey {
				return newRefusal("not-applicable-custody-has-key")
			}
		}
	}

	subjects := indexes["subjects"]
	identifiers := indexes["identifiers"]
	keys := indexes["keys"]
	claims := indexes["claims"]
	evidence := indexes["evidence"]

	for _, coll := range []string{"identifiers", "keys", "claims", "evidence", "assessments", "capabilities"} {
		arr, _ := arrField(obj, coll)
		for _, item := range arr {
			r, _ := item.(map[string]any)
			subj, ok := strField(r, "subject_ref")
			if !ok {
				return newRefusal("invalid-report-shape")
			}
			if _, exists := subjects[subj]; !exists {
				return newRefusal("unknown-subject")
			}
			if ir, ok := strField(r, "identifier_ref"); ok {
				im := identifiers[ir]
				if im == nil || !JSONEqual(im["subject_ref"], subj) {
					return newRefusal("identifier_ref-subject-mismatch")
				}
			}
			if kr, ok := strField(r, "key_ref"); ok {
				km := keys[kr]
				if km == nil || !JSONEqual(km["subject_ref"], subj) {
					return newRefusal("key_ref-subject-mismatch")
				}
			}
			if crs, ok := arrField(r, "claim_refs"); ok {
				for _, crItem := range crs {
					cr, ok := crItem.(string)
					if !ok {
						return newRefusal("invalid-report-shape")
					}
					c := claims[cr]
					if c == nil || !JSONEqual(c["subject_ref"], subj) {
						return newRefusal("claim-subject-mismatch")
					}
					if ck, hasCK := c["key_ref"]; hasCK {
						_, rHasK := r["key_ref"]
						if coll == "evidence" || rHasK {
							if !JSONEqual(ck, r["key_ref"]) {
								return newRefusal("claim-key-mismatch")
							}
						}
					}
				}
			}
			if b, ok := objField(r, "boundary"); ok {
				comp, ok := arrField(b, "comparison_subject_refs")
				if !ok {
					return newRefusal("invalid-report-shape")
				}
				for _, x := range comp {
					xs, ok := x.(string)
					if !ok || subjects[xs] == nil || xs == subj {
						return newRefusal("invalid-comparison-subject")
					}
				}
			}
		}
	}

	evArr, _ := arrField(obj, "evidence")
	for _, item := range evArr {
		e, _ := item.(map[string]any)
		tsStr, _ := strField(e, "collected_at")
		ts, ok := ParseStrictUTCUnix(tsStr)
		if !ok {
			return newRefusal("invalid-report-shape")
		}
		if ts > created {
			return newRefusal("future-evidence")
		}
	}
	capArr, _ := arrField(obj, "capabilities")
	for _, item := range capArr {
		c, _ := item.(map[string]any)
		tsStr, _ := strField(c, "observed_at")
		ts, ok := ParseStrictUTCUnix(tsStr)
		if !ok {
			return newRefusal("invalid-report-shape")
		}
		if ts > created {
			return newRefusal("future-capability")
		}
	}

	reportID, _ := strField(obj, "report_id")
	resolved := make(map[string]map[string]any, len(inputArr))
	for _, item := range inputArr {
		source, _ := item.(map[string]any)
		srcRID, _ := strField(source, "report_id")
		var origAny any
		if inputReports != nil {
			origAny = inputReports[srcRID]
		}
		orig, ok := origAny.(map[string]any)
		if !ok || orig == nil {
			return newRefusal("input-report-unavailable")
		}
		if !JSONEqual(orig["report_id"], srcRID) {
			return newRefusal("input-report-id-mismatch")
		}
		actualDig, err := CanonicalSHA256Value(orig)
		if err != nil {
			code, _ := RefusalCode(err)
			return newRefusal(code)
		}
		if !JSONEqual(actualDig, source["digest"]) {
			return newRefusal("input-report-digest-mismatch")
		}
		if err := ValidateReportShape(orig); err != nil {
			return newRefusal("invalid-input-report")
		}
		origCreatedStr, ok := strField(orig, "created_at")
		if !ok {
			return newRefusal("invalid-input-report")
		}
		origCreated, ok := ParseStrictUTCUnix(origCreatedStr)
		if !ok || srcRID == reportID || origCreated > created {
			return newRefusal("invalid-input-report")
		}
		localID, _ := strField(source, "id")
		resolved[localID] = orig
	}

	for _, coll := range []string{"evidence", "capabilities"} {
		arr, _ := arrField(obj, coll)
		for _, item := range arr {
			r, _ := item.(map[string]any)
			origin, ok := objField(r, "origin")
			if !ok {
				continue
			}
			inRef, _ := strField(origin, "input_ref")
			recRef, _ := strField(origin, "record_ref")
			source := resolved[inRef]
			if source == nil {
				return newRefusal("unknown-input-report")
			}
			var old map[string]any
			if sArr, ok := arrField(source, coll); ok {
				for _, x := range sArr {
					if xm, ok := x.(map[string]any); ok {
						if xid, _ := strField(xm, "id"); xid == recRef {
							old = xm
							break
						}
					}
				}
			}
			if old == nil || !JSONEqual(stripOrigin(r), stripOrigin(old)) {
				return newRefusal("input-observation-changed")
			}
			type refPair struct{ coll, id string }
			subjRef, _ := strField(r, "subject_ref")
			refs := []refPair{{"subjects", subjRef}}
			if ir, ok := strField(r, "identifier_ref"); ok {
				refs = append(refs, refPair{"identifiers", ir})
			}
			if kr, ok := strField(r, "key_ref"); ok {
				refs = append(refs, refPair{"keys", kr})
			}
			if crs, ok := arrField(r, "claim_refs"); ok {
				for _, x := range crs {
					xs, _ := x.(string)
					refs = append(refs, refPair{"claims", xs})
				}
			}
			if b, ok := objField(r, "boundary"); ok {
				if comp, ok := arrField(b, "comparison_subject_refs"); ok {
					for _, x := range comp {
						xs, _ := x.(string)
						refs = append(refs, refPair{"subjects", xs})
					}
				}
			}
			for _, rp := range refs {
				var oldItem map[string]any
				if sArr, ok := arrField(source, rp.coll); ok {
					for _, x := range sArr {
						if xm, ok := x.(map[string]any); ok {
							if xid, _ := strField(xm, "id"); xid == rp.id {
								oldItem = xm
								break
							}
						}
					}
				}
				curItem := indexes[rp.coll][rp.id]
				if oldItem == nil || curItem == nil || !JSONEqual(oldItem, curItem) {
					return newRefusal("input-reference-changed")
				}
			}
		}
	}

	seenScopes := make(map[string]struct{}, len(assArr))
	for _, item := range assArr {
		a, _ := item.(map[string]any)
		var sortedClaims []string
		if crs, ok := arrField(a, "claim_refs"); ok {
			for _, x := range crs {
				if xs, ok := x.(string); ok {
					sortedClaims = append(sortedClaims, xs)
				}
			}
			sort.Strings(sortedClaims)
		}
		aSubj, _ := strField(a, "subject_ref")
		aIdent, hasIdent := strField(a, "identifier_ref")
		aKey, hasKey := strField(a, "key_ref")
		aDim, _ := strField(a, "dimension")
		aScope, _ := strField(a, "scope")
		bID := boundaryIdentity(a["boundary"])
		scopeKey := strings.Join([]string{
			aSubj,
			boolPrefix(hasIdent, aIdent),
			boolPrefix(hasKey, aKey),
			aDim,
			aScope,
			bID,
			strings.Join(sortedClaims, ","),
		}, "\x00")
		if _, dup := seenScopes[scopeKey]; dup {
			return newRefusal("duplicate-assessment-scope")
		}
		seenScopes[scopeKey] = struct{}{}

		startStr, _ := strField(a, "assessed_at")
		endStr, _ := strField(a, "valid_until")
		start, ok1 := ParseStrictUTCUnix(startStr)
		end, ok2 := ParseStrictUTCUnix(endStr)
		if !ok1 || !ok2 {
			return newRefusal("invalid-report-shape")
		}
		if start > created || start >= end {
			return newRefusal("invalid-assessment-window")
		}
		if st, _ := strField(a, "state"); st == "current" && created >= end {
			return newRefusal("expired-current-assessment")
		}

		refs, _ := arrField(a, "evidence_refs")
		es := make([]map[string]any, 0, len(refs))
		for _, er := range refs {
			erID, _ := er.(string)
			ev := evidence[erID]
			if ev == nil {
				return newRefusal("unknown-evidence")
			}
			es = append(es, ev)
		}
		for _, e := range es {
			if !JSONEqual(e["subject_ref"], aSubj) {
				return newRefusal("evidence-subject-mismatch")
			}
		}
		for _, e := range es {
			tsStr, _ := strField(e, "collected_at")
			ts, ok := ParseStrictUTCUnix(tsStr)
			if !ok || ts > start {
				return newRefusal("evidence-after-assessment")
			}
		}
		if hasKey {
			for _, e := range es {
				if !JSONEqual(e["key_ref"], aKey) {
					return newRefusal("evidence-key-mismatch")
				}
			}
		}

		keyed := make(map[string]bool)
		named := make(map[string]bool)
		for _, e := range es {
			m, _ := strField(e, "method")
			if hasKey && JSONEqual(e["key_ref"], aKey) {
				keyed[m] = true
			}
			if hasIdent && JSONEqual(e["identifier_ref"], aIdent) {
				named[m] = true
			}
		}

		val, _ := strField(a, "value")
		switch aDim {
		case "identity-binding":
			switch val {
			case "declared":
				if !named["declaration"] {
					return newRefusal("declaration-required")
				}
			case "observed":
				if !named["os-observation"] {
					return newRefusal("observation-required")
				}
			case "enrolled-key":
				if !named["enrollment"] || !keyed["enrollment"] || !keyed["possession-proof"] {
					return newRefusal("enrollment-and-proof-required")
				}
			case "attested-key":
				hasKeyedAtt := keyed["device-attestation"] || keyed["workload-attestation"]
				hasNamedAtt := named["device-attestation"] || named["workload-attestation"]
				if !keyed["possession-proof"] || !hasKeyedAtt || !hasNamedAtt {
					return newRefusal("attestation-and-proof-required")
				}
			}
		case "key-custody":
			if (val == "software" || val == "os-managed") && !keyed["custody-observation"] {
				return newRefusal("custody-observation-required")
			}
			hasCustodyEv := keyed["device-attestation"] || keyed["workload-attestation"] || keyed["custody-observation"]
			if (val == "hardware-backed" || val == "virtual-hardware-backed") && !hasCustodyEv {
				return newRefusal("key-custody-evidence-required")
			}
		case "execution-isolation":
			if val != "unknown" {
				matching := make(map[string]bool)
				for _, e := range es {
					if boundaryIdentity(e["boundary"]) == bID {
						m, _ := strField(e, "method")
						matching[m] = true
					}
				}
				hasIsoEv := matching["isolation-observation"] || matching["workload-attestation"] || matching["authority-statement"]
				if !hasIsoEv {
					return newRefusal("isolation-boundary-evidence-required")
				}
				if val == "confidential-workload" && !matching["workload-attestation"] {
					return newRefusal("workload-attestation-required")
				}
			}
		case "platform-evidence":
			if val != "unknown" {
				var allowed []string
				switch val {
				case "declared":
					allowed = []string{"declaration"}
				case "locally-observed":
					allowed = []string{"os-observation"}
				case "authority-verified":
					allowed = []string{"authority-statement"}
				case "hardware-attested":
					allowed = []string{"device-attestation", "workload-attestation"}
				}
				crs, _ := arrField(a, "claim_refs")
				for _, crItem := range crs {
					cr, _ := crItem.(string)
					covered := false
					for _, e := range es {
						m, _ := strField(e, "method")
						if !containsStr(allowed, m) {
							continue
						}
						if ecrs, ok := arrField(e, "claim_refs"); ok {
							for _, x := range ecrs {
								if xs, _ := x.(string); xs == cr {
									covered = true
									break
								}
							}
						}
						if covered {
							break
						}
					}
					if !covered {
						return newRefusal("platform-claim-evidence-required")
					}
				}
			}
		}
	}

	return nil
}

func boolPrefix(present bool, s string) string {
	if !present {
		return "0:"
	}
	return "1:" + s
}

// CheckImportCollisions refuses multi-report imports that collide on
// (collection, id) with unequal records (no local-ID remapping).
func CheckImportCollisions(reports []any) error {
	collections := []string{
		"subjects", "identifiers", "keys", "claims", "evidence", "capabilities", "input_reports",
	}
	seen := make(map[string]any)
	for _, rep := range reports {
		if err := ValidateReportShape(rep); err != nil {
			return err
		}
		obj, ok := rep.(map[string]any)
		if !ok {
			return newRefusal("invalid-report-shape")
		}
		for _, coll := range collections {
			arr, ok := arrField(obj, coll)
			if !ok {
				return newRefusal("invalid-report-shape")
			}
			for _, rec := range arr {
				rm, ok := rec.(map[string]any)
				if !ok {
					return newRefusal("invalid-report-shape")
				}
				id, ok := strField(rm, "id")
				if !ok {
					return newRefusal("invalid-report-shape")
				}
				key := coll + "\x00" + id
				if prev, exists := seen[key]; exists {
					if !JSONEqual(prev, rm) {
						return newRefusal("import-id-collision")
					}
				} else {
					seen[key] = rm
				}
			}
		}
	}
	return nil
}

// ValidateProvenanceDAG enforces input_reports DAG acyclicity and dependency_depth <= maxDepth.
func ValidateProvenanceDAG(rootReport any, inputReports map[string]any, maxDepth int) error {
	visiting := make(map[string]bool)
	var dfs func(cur any, depth int) error
	dfs = func(cur any, depth int) error {
		if depth > maxDepth {
			return newRefusal("provenance-depth-exceeded")
		}
		if err := ValidateReportShape(cur); err != nil {
			return err
		}
		obj, ok := cur.(map[string]any)
		if !ok {
			return newRefusal("invalid-report-shape")
		}
		rid, ok := strField(obj, "report_id")
		if !ok {
			return newRefusal("invalid-report-shape")
		}
		if visiting[rid] {
			return newRefusal("provenance-cycle")
		}
		visiting[rid] = true
		refs, ok := arrField(obj, "input_reports")
		if !ok {
			return newRefusal("invalid-report-shape")
		}
		for _, item := range refs {
			rm, ok := item.(map[string]any)
			if !ok {
				return newRefusal("invalid-report-shape")
			}
			childID, ok := strField(rm, "report_id")
			if !ok {
				return newRefusal("invalid-report-shape")
			}
			var child any
			if inputReports != nil {
				child = inputReports[childID]
			}
			if child == nil {
				return newRefusal("input-report-unavailable")
			}
			if err := dfs(child, depth+1); err != nil {
				return err
			}
		}
		delete(visiting, rid)
		return nil
	}
	return dfs(rootReport, 0)
}

// AdmitAndValidateReport admits raw report wire bytes (report-admission:v0),
// validates report.schema.json shape, enforces NecessaryConditions, and checks
// provenance DAG depth when inputReports are present.
func AdmitAndValidateReport(raw []byte, inputReports map[string]any) (AdmittedWire, error) {
	admitted, err := AdmitReportBytes(raw)
	if err != nil {
		code, _ := RefusalCode(err)
		return AdmittedWire{}, newRefusal(code)
	}
	if err := ValidateReportShape(admitted.Value); err != nil {
		return AdmittedWire{}, err
	}
	if err := NecessaryConditions(admitted.Value, inputReports); err != nil {
		return AdmittedWire{}, err
	}
	if inputReports != nil {
		if obj, ok := admitted.Value.(map[string]any); ok {
			if inArr, ok := arrField(obj, "input_reports"); ok && len(inArr) > 0 {
				if err := ValidateProvenanceDAG(admitted.Value, inputReports, MaxProvenanceDepth); err != nil {
					return AdmittedWire{}, err
				}
			}
		}
	}
	return admitted, nil
}
