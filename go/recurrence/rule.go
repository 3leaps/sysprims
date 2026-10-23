package recurrence

import (
	"crypto/sha256"
	"encoding/hex"
	"sort"
	"strconv"
	"strings"
	"time"
)

// Frequencies, in the contract's vocabulary.
const (
	Minutely = "MINUTELY"
	Hourly   = "HOURLY"
	Daily    = "DAILY"
	Weekly   = "WEEKLY"
	Monthly  = "MONTHLY"
	Yearly   = "YEARLY"
)

var freqs = []string{Minutely, Hourly, Daily, Weekly, Monthly, Yearly}

// weekdays in the contract's MO..SU order; the index is the weekday number.
var weekdays = []string{"MO", "TU", "WE", "TH", "FR", "SA", "SU"}

func weekdayIndex(s string) int {
	for i, w := range weekdays {
		if w == s {
			return i
		}
	}
	return -1
}

func isFreq(s string) bool {
	for _, f := range freqs {
		if f == s {
			return true
		}
	}
	return false
}

// DaySpec is one BYDAY entry. Ordinal 0 means "no ordinal" (the contract
// forbids an explicit zero).
type DaySpec struct {
	Weekday int // 0 = MO .. 6 = SU
	Ordinal int
}

// Rule is a validated recurrence rule in normal form. Obtain one only from
// ParseJSON, ParseText or ParseDocument.
type Rule struct {
	dtstart    Local
	tzid       string
	freq       string
	interval   int
	count      int   // 0 = absent
	until      int64 // valid when hasUntil
	hasUntil   bool
	wkst       int
	byDay      []DaySpec
	byMonth    []int
	byMonthDay []int
	byHour     []int
	byMinute   []int
	bySetPos   []int
	rdate      []Local
	exdate     []Local
}

// fields gathers decoded values before calendar checks and normalization.
type fields struct {
	dtstart    []int
	tzid       string
	freq       string
	interval   int64
	count      int64
	hasCount   bool
	until      []int
	wkst       int
	byDay      []DaySpec
	byMonth    []int64
	byMonthDay []int64
	byHour     []int64
	byMinute   []int64
	bySetPos   []int64
	rdate      [][]int
	exdate     [][]int
}

var ruleMembers = map[string]bool{
	"dtstart": true, "tzid": true, "freq": true, "interval": true, "count": true,
	"until": true, "wkst": true, "by_day": true, "by_month": true, "by_month_day": true,
	"by_hour": true, "by_minute": true, "by_set_pos": true, "rdate": true, "exdate": true,
	"calendar": true,
}

func tzidShapeOK(s string) bool {
	if len([]rune(s)) > 64 || s == "" {
		return false
	}
	allowed := func(c byte) bool {
		return c >= 'A' && c <= 'Z' || c >= 'a' && c <= 'z' || c >= '0' && c <= '9' || c == '_' || c == '+' || c == '-'
	}
	if !(s[0] >= 'A' && s[0] <= 'Z' || s[0] >= 'a' && s[0] <= 'z') {
		return false
	}
	for _, seg := range strings.Split(s, "/") {
		if seg == "" {
			return false
		}
		for i := 0; i < len(seg); i++ {
			if !allowed(seg[i]) {
				return false
			}
		}
	}
	return true
}

func badRule(detail string) error { return refuse(InvalidRule, detail) }

// fromObject applies the JSON Schema (A6) to a decoded rule object.
func fromObject(v jsonValue) (*fields, error) {
	obj, ok := v.(map[string]jsonValue)
	if !ok {
		return nil, badRule("rule is not an object")
	}
	for k := range obj {
		if !ruleMembers[k] {
			return nil, badRule("unknown member")
		}
	}
	str := func(k string) (string, bool, error) {
		raw, present := obj[k]
		if !present {
			return "", false, nil
		}
		s, ok := raw.(string)
		if !ok {
			return "", true, badRule(k)
		}
		return s, true, nil
	}
	intIn := func(k string, lo, hi int64) (int64, bool, error) {
		raw, present := obj[k]
		if !present {
			return 0, false, nil
		}
		n, ok := schemaInteger(raw)
		if !ok || n < lo || n > hi {
			return 0, true, badRule(k)
		}
		return n, true, nil
	}
	intList := func(k string, lo, hi int64, noZero bool) ([]int64, error) {
		raw, present := obj[k]
		if !present {
			return nil, nil
		}
		arr, ok := raw.([]jsonValue)
		if !ok {
			return nil, badRule(k)
		}
		out := make([]int64, 0, len(arr))
		for _, item := range arr {
			n, ok := schemaInteger(item)
			if !ok || n < lo || n > hi || noZero && n == 0 {
				return nil, badRule(k)
			}
			out = append(out, n)
		}
		return out, nil
	}
	localList := func(k string) ([][]int, error) {
		raw, present := obj[k]
		if !present {
			return nil, nil
		}
		arr, ok := raw.([]jsonValue)
		if !ok {
			return nil, badRule(k)
		}
		out := make([][]int, 0, len(arr))
		for _, item := range arr {
			s, ok := item.(string)
			if !ok {
				return nil, badRule(k)
			}
			f, ok := scanFields(s, layoutLocal)
			if !ok {
				return nil, badRule(k)
			}
			out = append(out, f)
		}
		return out, nil
	}

	f := &fields{interval: 1}
	s, present, err := str("dtstart")
	if err != nil || !present {
		return nil, badRule("dtstart")
	}
	if f.dtstart, ok = scanFields(s, layoutLocal); !ok {
		return nil, badRule("dtstart")
	}
	if f.tzid, present, err = str("tzid"); err != nil || !present || !tzidShapeOK(f.tzid) {
		return nil, badRule("tzid")
	}
	if f.freq, present, err = str("freq"); err != nil || !present || !isFreq(f.freq) {
		return nil, badRule("freq")
	}
	if n, present, err := intIn("interval", 1, 1000); err != nil {
		return nil, err
	} else if present {
		f.interval = n
	}
	if f.count, f.hasCount, err = intIn("count", 1, 100000); err != nil {
		return nil, err
	}
	if s, present, err := str("until"); err != nil {
		return nil, err
	} else if present {
		if f.until, ok = scanFields(s, layoutUTC); !ok {
			return nil, badRule("until")
		}
	}
	if f.hasCount && f.until != nil {
		return nil, badRule("count and until together")
	}
	f.wkst = 0
	if s, present, err := str("wkst"); err != nil {
		return nil, err
	} else if present {
		if f.wkst = weekdayIndex(s); f.wkst < 0 {
			return nil, badRule("wkst")
		}
	}
	if s, present, err := str("calendar"); err != nil {
		return nil, err
	} else if present && s != "gregorian" {
		return nil, badRule("calendar")
	}
	if raw, present := obj["by_day"]; present {
		arr, ok := raw.([]jsonValue)
		if !ok {
			return nil, badRule("by_day")
		}
		for _, item := range arr {
			entry, ok := item.(map[string]jsonValue)
			if !ok {
				return nil, badRule("by_day")
			}
			spec := DaySpec{Weekday: -1}
			for k, val := range entry {
				switch k {
				case "day":
					if d, ok := val.(string); ok {
						spec.Weekday = weekdayIndex(d)
					}
				case "ordinal":
					n, ok := schemaInteger(val)
					if !ok || n == 0 || n < -53 || n > 53 {
						return nil, badRule("by_day ordinal")
					}
					spec.Ordinal = int(n)
				default:
					return nil, badRule("by_day member")
				}
			}
			if spec.Weekday < 0 {
				return nil, badRule("by_day day")
			}
			f.byDay = append(f.byDay, spec)
		}
	}
	if f.byMonth, err = intList("by_month", 1, 12, false); err != nil {
		return nil, err
	}
	if f.byMonthDay, err = intList("by_month_day", -31, 31, true); err != nil {
		return nil, err
	}
	if f.byHour, err = intList("by_hour", 0, 23, false); err != nil {
		return nil, err
	}
	if f.byMinute, err = intList("by_minute", 0, 59, false); err != nil {
		return nil, err
	}
	if f.bySetPos, err = intList("by_set_pos", -366, 366, true); err != nil {
		return nil, err
	}
	if f.rdate, err = localList("rdate"); err != nil {
		return nil, err
	}
	if f.exdate, err = localList("exdate"); err != nil {
		return nil, err
	}
	return f, f.crossChecks()
}

// crossChecks are the schema's conditional rules.
func (f *fields) crossChecks() error {
	if len(f.bySetPos) > 0 && len(f.byDay)+len(f.byMonth)+len(f.byMonthDay)+len(f.byHour)+len(f.byMinute) == 0 {
		return badRule("by_set_pos without a nonempty companion")
	}
	if f.freq != Monthly && f.freq != Yearly {
		for _, d := range f.byDay {
			if d.Ordinal != 0 {
				return badRule("by_day ordinal needs MONTHLY or YEARLY")
			}
		}
	}
	if f.freq == Weekly && len(f.byMonthDay) > 0 {
		return badRule("by_month_day with WEEKLY")
	}
	return nil
}

// build runs the semantic checks in contract order (tzid, calendar dates,
// synchronization) and normalizes.
func (f *fields) build() (*Rule, error) {
	if !acceptedZone(f.tzid) {
		return nil, refuse(InvalidTZID, "tzid is not in the pinned roster")
	}
	asLocal := func(v []int, what string) (Local, error) {
		l := localOf(v)
		if !l.valid() {
			return Local{}, refuse(InvalidDate, what)
		}
		return l, nil
	}
	r := &Rule{tzid: f.tzid, freq: f.freq, interval: int(f.interval), count: int(f.count), wkst: f.wkst}
	var err error
	if r.dtstart, err = asLocal(f.dtstart, "dtstart"); err != nil {
		return nil, err
	}
	for _, v := range f.rdate {
		l, err := asLocal(v, "rdate")
		if err != nil {
			return nil, err
		}
		r.rdate = append(r.rdate, l)
	}
	for _, v := range f.exdate {
		l, err := asLocal(v, "exdate")
		if err != nil {
			return nil, err
		}
		r.exdate = append(r.exdate, l)
	}
	if f.until != nil {
		u, err := asLocal(f.until, "until")
		if err != nil {
			return nil, err
		}
		r.until, r.hasUntil = u.seconds(), true
	}
	r.byMonth = sortedInts(f.byMonth)
	r.byMonthDay = sortedInts(f.byMonthDay)
	r.byHour = sortedInts(f.byHour)
	r.byMinute = sortedInts(f.byMinute)
	r.bySetPos = sortedInts(f.bySetPos)
	r.byDay = normalizeDays(f.byDay)
	r.rdate = sortedLocals(r.rdate)
	r.exdate = sortedLocals(r.exdate)
	if !newExpander(r).synchronized() {
		return nil, refuse(DTStartNotSynchronized, "dtstart is not selected by its first period")
	}
	return r, nil
}

func sortedInts(in []int64) []int {
	seen := map[int64]bool{}
	var out []int
	for _, n := range in {
		if !seen[n] {
			seen[n] = true
			out = append(out, int(n))
		}
	}
	sort.Ints(out)
	return out
}

func normalizeDays(in []DaySpec) []DaySpec {
	seen := map[DaySpec]bool{}
	var out []DaySpec
	for _, d := range in {
		if !seen[d] {
			seen[d] = true
			out = append(out, d)
		}
	}
	sort.Slice(out, func(i, j int) bool {
		a, b := out[i], out[j]
		if a.Weekday != b.Weekday {
			return a.Weekday < b.Weekday
		}
		if (a.Ordinal == 0) != (b.Ordinal == 0) {
			return a.Ordinal == 0 // the unqualified entry comes first
		}
		return a.Ordinal < b.Ordinal
	})
	return out
}

func sortedLocals(in []Local) []Local {
	seen := map[Local]bool{}
	var out []Local
	for _, l := range in {
		if !seen[l] {
			seen[l] = true
			out = append(out, l)
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].seconds() < out[j].seconds() })
	return out
}

// ParseJSON decodes and validates a rule in its JSON form.
func ParseJSON(data []byte) (*Rule, error) {
	v, err := decodeStrict(data)
	if err != nil {
		return nil, badRule("malformed JSON")
	}
	f, err := fromObject(v)
	if err != nil {
		return nil, err
	}
	return f.build()
}

// ParseDocument decodes and validates a rule-document envelope
// ({"capabilities": [...], "rule": {...}}). The envelope never affects the
// rule's digest.
func ParseDocument(data []byte) (*Rule, error) {
	v, err := decodeStrict(data)
	if err != nil {
		return nil, badRule("malformed JSON")
	}
	doc, ok := v.(map[string]jsonValue)
	if !ok || len(doc) != 2 {
		return nil, badRule("document shape")
	}
	caps, ok := doc["capabilities"].([]jsonValue)
	rule, hasRule := doc["rule"]
	if !ok || !hasRule || len(caps) == 0 {
		return nil, badRule("document shape")
	}
	seen, found := map[string]bool{}, false
	for _, c := range caps {
		s, ok := c.(string)
		if !ok || s == "" || seen[s] {
			return nil, badRule("capabilities")
		}
		seen[s] = true
		found = found || s == Contract
	}
	if !found {
		return nil, badRule("capabilities lacks " + Contract)
	}
	f, err := fromObject(rule)
	if err != nil {
		return nil, err
	}
	return f.build()
}

// TZID is the rule's time-zone name, as written.
func (r *Rule) TZID() string { return r.tzid }

// DTStart is the rule's local start label.
func (r *Rule) DTStart() Local { return r.dtstart }

// Freq is the rule's frequency.
func (r *Rule) Freq() string { return r.freq }

// CanonicalJSON is the RFC 8785 serialization of the normalized rule.
func (r *Rule) CanonicalJSON() []byte {
	m := jcsObject{}
	m.str("dtstart", r.dtstart.String())
	m.str("tzid", r.tzid)
	m.str("freq", r.freq)
	if r.interval != 1 {
		m.num("interval", r.interval)
	}
	if r.count > 0 {
		m.num("count", r.count)
	}
	if r.hasUntil {
		m.str("until", FormatInstant(time.Unix(r.until, 0)))
	}
	if r.wkst != 0 {
		m.str("wkst", weekdays[r.wkst])
	}
	if len(r.byDay) > 0 {
		var items []string
		for _, d := range r.byDay {
			e := jcsObject{}
			e.str("day", weekdays[d.Weekday])
			if d.Ordinal != 0 {
				e.num("ordinal", d.Ordinal)
			}
			items = append(items, e.render())
		}
		m["by_day"] = "[" + strings.Join(items, ",") + "]"
	}
	m.ints("by_month", r.byMonth)
	m.ints("by_month_day", r.byMonthDay)
	m.ints("by_hour", r.byHour)
	m.ints("by_minute", r.byMinute)
	m.ints("by_set_pos", r.bySetPos)
	m.locals("rdate", r.rdate)
	m.locals("exdate", r.exdate)
	return []byte(m.render())
}

// Digest is "sha256:" and the lowercase hex SHA-256 of CanonicalJSON.
func (r *Rule) Digest() string {
	sum := sha256.Sum256(r.CanonicalJSON())
	return "sha256:" + hex.EncodeToString(sum[:])
}

// Text is the canonical RFC 5545 form: DTSTART, RRULE, then nonempty
// RDATE and EXDATE.
func (r *Rule) Text() []string {
	lines := []string{"DTSTART;TZID=" + r.tzid + ":" + r.dtstart.basic()}
	parts := []string{"FREQ=" + r.freq}
	if r.interval != 1 {
		parts = append(parts, "INTERVAL="+strconv.Itoa(r.interval))
	}
	if r.count > 0 {
		parts = append(parts, "COUNT="+strconv.Itoa(r.count))
	}
	if r.hasUntil {
		parts = append(parts, "UNTIL="+localFromSeconds(r.until).basic()+"Z")
	}
	joinInts := func(name string, v []int) {
		if len(v) == 0 {
			return
		}
		s := make([]string, len(v))
		for i, n := range v {
			s[i] = strconv.Itoa(n)
		}
		parts = append(parts, name+"="+strings.Join(s, ","))
	}
	joinInts("BYMONTH", r.byMonth)
	joinInts("BYMONTHDAY", r.byMonthDay)
	if len(r.byDay) > 0 {
		s := make([]string, len(r.byDay))
		for i, d := range r.byDay {
			if d.Ordinal != 0 {
				s[i] = strconv.Itoa(d.Ordinal)
			}
			s[i] += weekdays[d.Weekday]
		}
		parts = append(parts, "BYDAY="+strings.Join(s, ","))
	}
	joinInts("BYHOUR", r.byHour)
	joinInts("BYMINUTE", r.byMinute)
	joinInts("BYSETPOS", r.bySetPos)
	if r.wkst != 0 {
		parts = append(parts, "WKST="+weekdays[r.wkst])
	}
	lines = append(lines, "RRULE:"+strings.Join(parts, ";"))
	for _, p := range []struct {
		name string
		v    []Local
	}{{"RDATE", r.rdate}, {"EXDATE", r.exdate}} {
		if len(p.v) == 0 {
			continue
		}
		s := make([]string, len(p.v))
		for i, l := range p.v {
			s[i] = l.basic()
		}
		lines = append(lines, p.name+";TZID="+r.tzid+":"+strings.Join(s, ","))
	}
	return lines
}
