// SPDX-License-Identifier: MIT OR Apache-2.0

package evidence

const (
	// LocalProcessSessionProfileID is the local-process-session:v0 profile identifier.
	LocalProcessSessionProfileID = "urn:3leaps:identity-evidence:local-process-session:v0"
	// LocalProcessSessionProfileRevision is the pinned sha256 revision of local-process-session/profile.json.
	LocalProcessSessionProfileRevision = "sha256:5cf6300b565a7fca392003edd376f33c1c2b90b72e7436aa89a5c0533e96dc3d"
	// LocalProcessSessionTranscriptDomain is the local-process-session:v0 transcript domain.
	LocalProcessSessionTranscriptDomain = "3leaps.identity-evidence.local-process-session.v0"

	maxReceiptSeconds        int64 = 300
	maxChallengeSeconds      int64 = 60
	maxSessionFutureSkewSecs int64 = 5
	maxSessionObsAgeSecs     int64 = 5
)

func isSessionPIDString(s string) bool {
	if len(s) == 0 || len(s) > 20 || s[0] < '1' || s[0] > '9' {
		return false
	}
	for i := 1; i < len(s); i++ {
		if s[i] < '0' || s[i] > '9' {
			return false
		}
	}
	return true
}

func isHex64(s string) bool {
	if len(s) != 64 {
		return false
	}
	for i := 0; i < 64; i++ {
		if !isLowerHexByte(s[i]) {
			return false
		}
	}
	return true
}

func checkExactKeys(m map[string]any, keys []string) bool {
	if len(m) != len(keys) {
		return false
	}
	for _, k := range keys {
		if _, ok := m[k]; !ok {
			return false
		}
	}
	return true
}

// ValidateSessionBundleShape validates local-process-session/evidence.schema.json shape rules.
func ValidateSessionBundleShape(bundle any) error {
	errShape := func() error { return newRefusal("invalid-session-bundle-shape") }
	obj, ok := bundle.(map[string]any)
	if !ok {
		return errShape()
	}
	if !checkExactKeys(obj, []string{"profile", "process", "session", "launch", "challenge", "proof"}) ||
		!isProfileRefObj(obj["profile"]) {
		return errShape()
	}

	procObj, ok := objField(obj, "process")
	if !ok || !checkExactKeys(procObj, []string{
		"subject_ref", "host_scope", "boot_id", "pid_namespace", "pid", "start_identity", "execution_context",
	}) {
		return errShape()
	}
	subjRef, ok1 := strField(procObj, "host_scope")
	sr, ok0 := strField(procObj, "subject_ref")
	bootID, ok2 := strField(procObj, "boot_id")
	pidNS, ok3 := strField(procObj, "pid_namespace")
	pidStr, ok4 := strField(procObj, "pid")
	if !ok0 || !isLocalID(sr) || !ok1 || !isNonEmpty512(subjRef) || !ok2 || !isNonEmpty512(bootID) ||
		!ok3 || !isNonEmpty512(pidNS) || !ok4 || !isSessionPIDString(pidStr) {
		return errShape()
	}

	si, ok := objField(procObj, "start_identity")
	if !ok || !checkExactKeys(si, []string{"profile", "value"}) || !isProfileRefObj(si["profile"]) {
		return errShape()
	}
	if siVal, ok := strField(si, "value"); !ok || !isNonEmpty512(siVal) {
		return errShape()
	}

	ec, ok := objField(procObj, "execution_context")
	if !ok || !checkExactKeys(ec, []string{
		"security_principal", "namespace_context_digest", "executable_digest", "arguments_digest", "security_context_digest",
	}) || !isPrincipalObj(ec["security_principal"]) {
		return errShape()
	}
	for _, dk := range []string{"namespace_context_digest", "executable_digest", "arguments_digest", "security_context_digest"} {
		if dv, ok := strField(ec, dk); !ok || !isSHA256Digest(dv) {
			return errShape()
		}
	}

	sess, ok := objField(obj, "session")
	if !ok || !checkExactKeys(sess, []string{
		"subject_ref", "identifier_ref", "identifier_digest", "key_ref", "public_key_digest", "key_representation_profile",
	}) {
		return errShape()
	}
	sSubj, ok0 := strField(sess, "subject_ref")
	sIdent, ok1 := strField(sess, "identifier_ref")
	sIDig, ok2 := strField(sess, "identifier_digest")
	sKey, ok3 := strField(sess, "key_ref")
	sPKDig, ok4 := strField(sess, "public_key_digest")
	if !ok0 || !isLocalID(sSubj) || !ok1 || !isLocalID(sIdent) || !ok2 || !isSHA256Digest(sIDig) ||
		!ok3 || !isLocalID(sKey) || !ok4 || !isSHA256Digest(sPKDig) || !isProfileRefObj(sess["key_representation_profile"]) {
		return errShape()
	}

	launch, ok := objField(obj, "launch")
	if !ok || !checkExactKeys(launch, []string{
		"receipt_id", "issuer", "intent_digest", "process_digest", "session_digest", "issued_at", "expires_at",
	}) {
		return errShape()
	}
	if rid, ok := strField(launch, "receipt_id"); !ok || !isLocalID(rid) || !isPrincipalObj(launch["issuer"]) {
		return errShape()
	}
	for _, dk := range []string{"intent_digest", "process_digest", "session_digest"} {
		if dv, ok := strField(launch, dk); !ok || !isSHA256Digest(dv) {
			return errShape()
		}
	}
	for _, tk := range []string{"issued_at", "expires_at"} {
		ts, ok := strField(launch, tk)
		if !ok {
			return errShape()
		}
		if _, ok := ParseStrictUTCUnix(ts); !ok {
			return errShape()
		}
	}

	chal, ok := objField(obj, "challenge")
	if !ok || !checkExactKeys(chal, []string{"nonce", "audience", "receipt_digest", "issued_at", "expires_at"}) {
		return errShape()
	}
	nonce, ok0 := strField(chal, "nonce")
	aud, ok1 := strField(chal, "audience")
	rdig, ok2 := strField(chal, "receipt_digest")
	if !ok0 || !isHex64(nonce) || !ok1 || !isNonEmpty512(aud) || !ok2 || !isSHA256Digest(rdig) {
		return errShape()
	}
	for _, tk := range []string{"issued_at", "expires_at"} {
		ts, ok := strField(chal, tk)
		if !ok {
			return errShape()
		}
		if _, ok := ParseStrictUTCUnix(ts); !ok {
			return errShape()
		}
	}

	proof, ok := objField(obj, "proof")
	if !ok || !checkExactKeys(proof, []string{"key_ref", "transcript_digest", "proof_digest"}) {
		return errShape()
	}
	pk, ok0 := strField(proof, "key_ref")
	ptd, ok1 := strField(proof, "transcript_digest")
	ppd, ok2 := strField(proof, "proof_digest")
	if !ok0 || !isLocalID(pk) || !ok1 || !isSHA256Digest(ptd) || !ok2 || !isSHA256Digest(ppd) {
		return errShape()
	}

	return nil
}

// ProcessSessionFit evaluates local-process-session:v0 contract fit over a
// validated report, evidence bundle, and caller-supplied trusted evaluation context.
func ProcessSessionFit(report any, bundle any, context any, acceptedProfile ProfileRef) error {
	repObj, ok := report.(map[string]any)
	if !ok {
		return newRefusal("invalid-report-shape")
	}
	bunObj, ok := bundle.(map[string]any)
	if !ok {
		return newRefusal("invalid-session-bundle-shape")
	}
	ctxObj, ok := context.(map[string]any)
	if !ok {
		return newRefusal("trusted-clock-unavailable")
	}
	expectedProf := map[string]any{
		"id":       acceptedProfile.ID,
		"revision": acceptedProfile.Revision,
	}
	if !JSONEqual(bunObj["profile"], expectedProf) {
		return newRefusal("unsupported-evidence-profile")
	}

	process, _ := objField(bunObj, "process")
	session, _ := objField(bunObj, "session")
	receipt, _ := objField(bunObj, "launch")
	challenge, _ := objField(bunObj, "challenge")
	proof, _ := objField(bunObj, "proof")

	nowStr, ok := strField(ctxObj, "now")
	if !ok {
		return newRefusal("trusted-clock-unavailable")
	}
	now, ok := ParseStrictUTCUnix(nowStr)
	if !ok {
		return newRefusal("trusted-clock-unavailable")
	}

	windows := []struct {
		w   map[string]any
		max int64
	}{
		{receipt, maxReceiptSeconds},
		{challenge, maxChallengeSeconds},
	}
	for _, item := range windows {
		issStr, ok1 := strField(item.w, "issued_at")
		expStr, ok2 := strField(item.w, "expires_at")
		if !ok1 || !ok2 {
			return newRefusal("expired-or-invalid-window")
		}
		issued, ok1 := ParseStrictUTCUnix(issStr)
		expires, ok2 := ParseStrictUTCUnix(expStr)
		validWin := ok1 && ok2 && issued < expires && (expires-issued) <= item.max && (issued-now) <= maxSessionFutureSkewSecs && now < expires
		if !validWin {
			return newRefusal("expired-or-invalid-window")
		}
	}

	if live, ok := ctxObj["process_live"].(bool); !ok || !live {
		return newRefusal("process-not-live")
	}
	obsAtStr, ok := strField(ctxObj, "observed_at")
	if !ok {
		return newRefusal("stale-process-observation")
	}
	obsAt, ok := ParseStrictUTCUnix(obsAtStr)
	if !ok {
		return newRefusal("stale-process-observation")
	}
	obsAge := now - obsAt
	if obsAge < 0 || obsAge > maxSessionObsAgeSecs {
		return newRefusal("stale-process-observation")
	}
	if !JSONEqual(process, ctxObj["current_process"]) {
		return newRefusal("process-incarnation-or-context-changed")
	}
	if !JSONEqual(session, ctxObj["expected_session"]) {
		return newRefusal("session-key-or-enrollment-changed")
	}

	subjArr, ok := arrField(repObj, "subjects")
	if !ok {
		return newRefusal("invalid-report-shape")
	}
	subjects := make(map[string]map[string]any, len(subjArr))
	for _, item := range subjArr {
		if sm, ok := item.(map[string]any); ok {
			if id, ok := strField(sm, "id"); ok {
				subjects[id] = sm
			}
		}
	}
	procSubj, _ := strField(process, "subject_ref")
	sessSubj, _ := strField(session, "subject_ref")
	pKind, _ := strField(subjects[procSubj], "kind")
	sKind, _ := strField(subjects[sessSubj], "kind")
	if pKind != "process" || sKind != "session" {
		return newRefusal("process-session-subject-mismatch")
	}

	sessKeyRef, _ := strField(session, "key_ref")
	sessIdentRef, _ := strField(session, "identifier_ref")
	var keyObj map[string]any
	if keysArr, ok := arrField(repObj, "keys"); ok {
		for _, item := range keysArr {
			if km, ok := item.(map[string]any); ok {
				if kid, _ := strField(km, "id"); kid == sessKeyRef {
					keyObj = km
					break
				}
			}
		}
	}
	var identObj map[string]any
	if idArr, ok := arrField(repObj, "identifiers"); ok {
		for _, item := range idArr {
			if im, ok := item.(map[string]any); ok {
				if iid, _ := strField(im, "id"); iid == sessIdentRef {
					identObj = im
					break
				}
			}
		}
	}
	var identDigest string
	if identObj != nil {
		var err error
		identDigest, err = CanonicalSHA256Value(identObj)
		if err != nil {
			code, _ := RefusalCode(err)
			return newRefusal(code)
		}
	}
	if keyObj == nil || identObj == nil ||
		!JSONEqual(keyObj["subject_ref"], session["subject_ref"]) ||
		!JSONEqual(keyObj["public_key_digest"], session["public_key_digest"]) ||
		!JSONEqual(keyObj["representation_profile"], session["key_representation_profile"]) ||
		!JSONEqual(identObj["subject_ref"], session["subject_ref"]) ||
		!JSONEqual(identDigest, session["identifier_digest"]) {
		return newRefusal("report-session-key-mismatch")
	}

	procDigest, err := CanonicalSHA256Value(process)
	if err != nil {
		code, _ := RefusalCode(err)
		return newRefusal(code)
	}
	sessDigest, err := CanonicalSHA256Value(session)
	if err != nil {
		code, _ := RefusalCode(err)
		return newRefusal(code)
	}
	if !JSONEqual(receipt["process_digest"], procDigest) || !JSONEqual(receipt["session_digest"], sessDigest) {
		return newRefusal("receipt-binding-mismatch")
	}
	if !JSONEqual(receipt["issuer"], ctxObj["accepted_launch_issuer"]) ||
		!JSONEqual(receipt["intent_digest"], ctxObj["expected_intent_digest"]) {
		return newRefusal("launch-intent-or-issuer-mismatch")
	}

	consumedReceipts, ok := arrField(ctxObj, "consumed_receipts")
	if !ok {
		return newRefusal("launch-receipt-replayed")
	}
	for _, item := range consumedReceipts {
		if cm, ok := item.(map[string]any); ok {
			if JSONEqual(cm["issuer"], receipt["issuer"]) && JSONEqual(cm["receipt_id"], receipt["receipt_id"]) {
				return newRefusal("launch-receipt-replayed")
			}
		}
	}
	consumedChallenges, ok := arrField(ctxObj, "consumed_challenges")
	if !ok {
		return newRefusal("launch-challenge-replayed")
	}
	for _, item := range consumedChallenges {
		if JSONEqual(item, challenge["nonce"]) {
			return newRefusal("launch-challenge-replayed")
		}
	}

	receiptDigest, err := CanonicalSHA256Value(receipt)
	if err != nil {
		code, _ := RefusalCode(err)
		return newRefusal(code)
	}
	if !JSONEqual(ctxObj["verified_receipt_digest"], receiptDigest) {
		return newRefusal("receipt-verification-binding-mismatch")
	}
	if !JSONEqual(challenge["receipt_digest"], receiptDigest) {
		return newRefusal("challenge-receipt-mismatch")
	}
	if !JSONEqual(challenge["audience"], ctxObj["audience"]) || !JSONEqual(challenge["nonce"], ctxObj["nonce"]) {
		return newRefusal("challenge-audience-or-nonce-mismatch")
	}

	transcript := map[string]any{
		"domain":         LocalProcessSessionTranscriptDomain,
		"profile":        expectedProf,
		"process_digest": procDigest,
		"session_digest": sessDigest,
		"receipt_digest": receiptDigest,
		"challenge":      challenge,
	}
	transcriptDigest, err := CanonicalSHA256Value(transcript)
	if err != nil {
		code, _ := RefusalCode(err)
		return newRefusal(code)
	}
	if !JSONEqual(proof["key_ref"], session["key_ref"]) ||
		!JSONEqual(proof["transcript_digest"], transcriptDigest) ||
		!JSONEqual(ctxObj["verified_transcript_digest"], transcriptDigest) ||
		!JSONEqual(ctxObj["verified_key_digest"], keyObj["public_key_digest"]) ||
		!JSONEqual(ctxObj["verified_proof_digest"], proof["proof_digest"]) {
		return newRefusal("possession-verification-binding-mismatch")
	}

	claims := make(map[string]map[string]any)
	if cArr, ok := arrField(repObj, "claims"); ok {
		for _, item := range cArr {
			if cm, ok := item.(map[string]any); ok {
				if cid, ok := strField(cm, "id"); ok {
					claims[cid] = cm
				}
			}
		}
	}
	bundleDigest, err := CanonicalSHA256Value(bunObj)
	if err != nil {
		code, _ := RefusalCode(err)
		return newRefusal(code)
	}

	launchClaimMatches := func(cl map[string]any) bool {
		if cl == nil {
			return false
		}
		_, hasKey := cl["key_ref"]
		return !hasKey &&
			JSONEqual(cl["subject_ref"], process["subject_ref"]) &&
			JSONEqual(cl["property_profile"], expectedProf) &&
			JSONEqual(cl["property"], "launch-bound-session") &&
			JSONEqual(cl["value"], session["subject_ref"])
	}

	bound := make(map[string]map[string]any)
	evArr, _ := arrField(repObj, "evidence")
	for _, item := range evArr {
		ev, _ := item.(map[string]any)
		fpMatch := JSONEqual(ev["format_profile"], expectedProf)
		digMatch := JSONEqual(ev["digest"], bundleDigest)
		if !fpMatch && !digMatch {
			continue
		}
		if !fpMatch || !digMatch {
			return newRefusal("report-evidence-binding-mismatch")
		}
		method, _ := strField(ev, "method")
		switch method {
		case "possession-proof":
			crs, _ := arrField(ev, "claim_refs")
			_, hasBoundary := ev["boundary"]
			if !JSONEqual(ev["subject_ref"], session["subject_ref"]) ||
				!JSONEqual(ev["identifier_ref"], session["identifier_ref"]) ||
				!JSONEqual(ev["key_ref"], session["key_ref"]) ||
				len(crs) != 0 || hasBoundary {
				return newRefusal("report-possession-descriptor-mismatch")
			}
		case "os-observation":
			_, hasIdent := ev["identifier_ref"]
			_, hasKey := ev["key_ref"]
			_, hasBoundary := ev["boundary"]
			if !JSONEqual(ev["subject_ref"], process["subject_ref"]) || hasIdent || hasKey || hasBoundary {
				return newRefusal("report-process-descriptor-mismatch")
			}
			crs, ok := arrField(ev, "claim_refs")
			if !ok || len(crs) == 0 {
				return newRefusal("process-session-claim-mismatch")
			}
			for _, crItem := range crs {
				crID, _ := crItem.(string)
				if !launchClaimMatches(claims[crID]) {
					return newRefusal("process-session-claim-mismatch")
				}
			}
		default:
			return newRefusal("report-evidence-method-mismatch")
		}
		evID, _ := strField(ev, "id")
		bound[evID] = ev
	}

	usedMethods := make(map[string]bool)
	assArr, _ := arrField(repObj, "assessments")
	for _, item := range assArr {
		a, _ := item.(map[string]any)
		evRefs, _ := arrField(a, "evidence_refs")
		var selected []map[string]any
		for _, er := range evRefs {
			erID, _ := er.(string)
			if bev, exists := bound[erID]; exists {
				selected = append(selected, bev)
			}
		}
		var selectedClaims []map[string]any
		if acrs, ok := arrField(a, "claim_refs"); ok {
			for _, cr := range acrs {
				crID, _ := cr.(string)
				if cm, exists := claims[crID]; exists {
					selectedClaims = append(selectedClaims, cm)
				}
			}
		}
		if len(selected) == 0 {
			if JSONEqual(a["rule"], expectedProf) {
				return newRefusal("assessment-bundle-evidence-required")
			}
			for _, sc := range selectedClaims {
				if JSONEqual(sc["property_profile"], expectedProf) {
					return newRefusal("assessment-bundle-evidence-required")
				}
			}
			continue
		}
		for _, ev := range selected {
			method, _ := strField(ev, "method")
			if method == "possession-proof" {
				if !JSONEqual(a["dimension"], "identity-binding") ||
					!JSONEqual(a["value"], "enrolled-key") ||
					!JSONEqual(a["subject_ref"], session["subject_ref"]) ||
					!JSONEqual(a["identifier_ref"], session["identifier_ref"]) ||
					!JSONEqual(a["key_ref"], session["key_ref"]) {
					return newRefusal("assessment-possession-binding-mismatch")
				}
			} else {
				_, hasIdent := a["identifier_ref"]
				_, hasKey := a["key_ref"]
				if !JSONEqual(a["dimension"], "platform-evidence") ||
					!JSONEqual(a["value"], "locally-observed") ||
					!JSONEqual(a["rule"], expectedProf) ||
					!JSONEqual(a["subject_ref"], process["subject_ref"]) ||
					hasIdent || hasKey {
					return newRefusal("assessment-process-binding-mismatch")
				}
				if len(selectedClaims) == 0 {
					return newRefusal("assessment-process-claim-mismatch")
				}
				for _, sc := range selectedClaims {
					if !launchClaimMatches(sc) {
						return newRefusal("assessment-process-claim-mismatch")
					}
				}
				evClaimSet := make(map[string]bool)
				if ecrs, ok := arrField(ev, "claim_refs"); ok {
					for _, x := range ecrs {
						if xs, ok := x.(string); ok {
							evClaimSet[xs] = true
						}
					}
				}
				if acrs, ok := arrField(a, "claim_refs"); ok {
					for _, x := range acrs {
						xs, _ := x.(string)
						if !evClaimSet[xs] {
							return newRefusal("assessment-process-claim-mismatch")
						}
					}
				}
			}
			usedMethods[method] = true
		}
	}
	if len(usedMethods) != 2 || !usedMethods["possession-proof"] || !usedMethods["os-observation"] {
		return newRefusal("process-session-assessments-required")
	}

	rExpStr, _ := strField(receipt, "expires_at")
	cExpStr, _ := strField(challenge, "expires_at")
	rExp, _ := ParseStrictUTCUnix(rExpStr)
	cExp, _ := ParseStrictUTCUnix(cExpStr)
	expiry := rExp
	if cExp < expiry {
		expiry = cExp
	}
	for _, item := range assArr {
		a, _ := item.(map[string]any)
		vuStr, _ := strField(a, "valid_until")
		vu, _ := ParseStrictUTCUnix(vuStr)
		if vu > expiry {
			return newRefusal("assessment-outlives-binding")
		}
	}
	for _, item := range assArr {
		a, _ := item.(map[string]any)
		vuStr, _ := strField(a, "valid_until")
		vu, _ := ParseStrictUTCUnix(vuStr)
		if now >= vu {
			return newRefusal("assessment-expired")
		}
	}
	for _, item := range assArr {
		a, _ := item.(map[string]any)
		st, _ := strField(a, "state")
		atStr, _ := strField(a, "assessed_at")
		at, _ := ParseStrictUTCUnix(atStr)
		if st != "current" || at > now {
			return newRefusal("assessment-not-current")
		}
	}

	return nil
}
