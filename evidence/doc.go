// SPDX-License-Identifier: MIT OR Apache-2.0

// Package evidence implements portable identity-evidence/v0 wire admission,
// report envelope validation, local-process-session:v0 contract-fit checks,
// and native-process-observation:v0 comparison, freshness admission, context
// digest derivations, and session projection over RFC 8785 canonical JSON
// (github.com/3leaps/canonprims/canon).
//
// This package is pure Go (CGO_ENABLED=0) and performs no OS process syscalls.
package evidence

import (
	"encoding/json"
	"math"
	"strconv"
	"time"
	"unicode/utf8"

	"github.com/3leaps/canonprims/canon"
)

// CorpusDigest is the sha256 digest of the canonical-json/v0 MANIFEST.json
// pinned by github.com/3leaps/canonprims/canon.
const CorpusDigest = canon.CorpusDigest

// ProfileRef identifies a pinned {id, revision} contract profile.
type ProfileRef struct {
	ID       string `json:"id"`
	Revision string `json:"revision"`
}

// RefusalError carries a deterministic contract refusal code identical across
// Rust (sysprims-evidence) and Go (github.com/3leaps/sysprims/evidence).
type RefusalError struct {
	Code string
}

// Error implements error.
func (e *RefusalError) Error() string {
	return e.Code
}

func newRefusal(code string) error {
	return &RefusalError{Code: code}
}

// RefusalCode extracts the normative refusal code from err when err is a
// *RefusalError or *WireRefusal, returning ("", false) otherwise.
func RefusalCode(err error) (string, bool) {
	if err == nil {
		return "", false
	}
	if re, ok := err.(*RefusalError); ok {
		return re.Code, true
	}
	if we, ok := err.(*WireRefusal); ok {
		return we.Code, true
	}
	return "", false
}

// ParseStrictUTCUnix parses a strict YYYY-MM-DDTHH:MM:SSZ UTC instant (years
// 0001..=9999) into Unix whole seconds, rejecting year 0000, sub-second
// fractions, non-Z offsets, lowercase t/z, leap-second 60, and invalid
// calendar dates.
func ParseStrictUTCUnix(text string) (int64, bool) {
	if len(text) != 20 {
		return 0, false
	}
	digitIdx := [...]int{0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18}
	for _, idx := range digitIdx {
		if text[idx] < '0' || text[idx] > '9' {
			return 0, false
		}
	}
	if text[0:4] == "0000" {
		return 0, false
	}
	if text[4] != '-' || text[7] != '-' || text[10] != 'T' || text[13] != ':' || text[16] != ':' || text[19] != 'Z' {
		return 0, false
	}
	sec := (int(text[17]-'0') * 10) + int(text[18]-'0')
	if sec > 59 {
		return 0, false
	}
	t, err := time.Parse("2006-01-02T15:04:05Z", text)
	if err != nil {
		return 0, false
	}
	if t.Format("2006-01-02T15:04:05Z") != text {
		return 0, false
	}
	return t.Unix(), true
}

func asJSONNumberFloat(v any) (float64, bool) {
	switch x := v.(type) {
	case float64:
		if math.IsNaN(x) || math.IsInf(x, 0) {
			return 0, false
		}
		return x, true
	case json.Number:
		f, err := strconv.ParseFloat(string(x), 64)
		if err != nil || math.IsNaN(f) || math.IsInf(f, 0) {
			return 0, false
		}
		return f, true
	case int:
		return float64(x), true
	case int64:
		return float64(x), true
	case uint64:
		return float64(x), true
	default:
		return 0, false
	}
}

// JSONEqual compares two JSON-compatible Go values without conflating booleans
// and numbers, treating object key order as immaterial and numerically equal
// numbers (such as 1 and 1.0 or -0.0 and 0) as equal.
func JSONEqual(left, right any) bool {
	if left == nil || right == nil {
		return left == nil && right == nil
	}
	if lb, ok := left.(bool); ok {
		rb, rok := right.(bool)
		return rok && lb == rb
	}
	if _, ok := right.(bool); ok {
		return false
	}
	if ls, ok := left.(string); ok {
		rs, rok := right.(string)
		return rok && ls == rs
	}
	if lf, ok := asJSONNumberFloat(left); ok {
		rf, rok := asJSONNumberFloat(right)
		return rok && lf == rf
	}
	if la, ok := left.([]any); ok {
		ra, rok := right.([]any)
		if !rok || len(la) != len(ra) {
			return false
		}
		for i := range la {
			if !JSONEqual(la[i], ra[i]) {
				return false
			}
		}
		return true
	}
	if lm, ok := left.(map[string]any); ok {
		rm, rok := right.(map[string]any)
		if !rok || len(lm) != len(rm) {
			return false
		}
		for k, lv := range lm {
			rv, exists := rm[k]
			if !exists || !JSONEqual(lv, rv) {
				return false
			}
		}
		return true
	}
	return false
}

func isLowerHexByte(c byte) bool {
	if c >= '0' && c <= '9' {
		return true
	}
	return c >= 'a' && c <= 'f'
}

func isLowerAlphaNumDashByte(c byte) bool {
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
		if !isLowerAlphaNumDashByte(s[i]) {
			return false
		}
	}
	return true
}

func isSHA256Digest(s string) bool {
	if len(s) != 71 || s[:7] != "sha256:" {
		return false
	}
	for i := 7; i < 71; i++ {
		if !isLowerHexByte(s[i]) {
			return false
		}
	}
	return true
}

func isNonEmpty512(s string) bool {
	if len(s) == 0 || !utf8.ValidString(s) {
		return false
	}
	n := utf8.RuneCountInString(s)
	return n >= 1 && n <= 512
}

func isProfileRefObj(v any) bool {
	m, ok := v.(map[string]any)
	if !ok || len(m) != 2 {
		return false
	}
	id, ok1 := m["id"].(string)
	rev, ok2 := m["revision"].(string)
	return ok1 && ok2 && isNonEmpty512(id) && isSHA256Digest(rev)
}

func isPrincipalObj(v any) bool {
	m, ok := v.(map[string]any)
	if !ok || len(m) != 2 {
		return false
	}
	ns, ok1 := m["namespace"].(string)
	val, ok2 := m["value"].(string)
	return ok1 && ok2 && isNonEmpty512(ns) && isNonEmpty512(val)
}

func strField(m map[string]any, key string) (string, bool) {
	if m == nil {
		return "", false
	}
	v, ok := m[key]
	if !ok {
		return "", false
	}
	s, ok := v.(string)
	return s, ok
}

func objField(m map[string]any, key string) (map[string]any, bool) {
	if m == nil {
		return nil, false
	}
	v, ok := m[key]
	if !ok {
		return nil, false
	}
	o, ok := v.(map[string]any)
	return o, ok
}

func arrField(m map[string]any, key string) ([]any, bool) {
	if m == nil {
		return nil, false
	}
	v, ok := m[key]
	if !ok {
		return nil, false
	}
	a, ok := v.([]any)
	return a, ok
}

func checkAllowedKeys(m map[string]any, required []string, optional []string) bool {
	reqSet := make(map[string]struct{}, len(required))
	for _, r := range required {
		if _, ok := m[r]; !ok {
			return false
		}
		reqSet[r] = struct{}{}
	}
	optSet := make(map[string]struct{}, len(optional))
	for _, o := range optional {
		optSet[o] = struct{}{}
	}
	for k := range m {
		if _, ok := reqSet[k]; ok {
			continue
		}
		if _, ok := optSet[k]; ok {
			continue
		}
		return false
	}
	return true
}

func containsStr(list []string, target string) bool {
	for _, s := range list {
		if s == target {
			return true
		}
	}
	return false
}
