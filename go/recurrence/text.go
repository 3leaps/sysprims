package recurrence

import (
	"strconv"
	"strings"
)

// parseTextInt applies the contract's integer lexeme grammar: "0", or an
// optional sign, a nonzero digit and at most eight more digits. The length
// is checked before conversion, so no unbounded integer is ever converted.
func parseTextInt(s string) (int64, error) {
	if s == "0" {
		return 0, nil
	}
	body := s
	if body != "" && (body[0] == '+' || body[0] == '-') {
		body = body[1:]
	}
	if len(body) < 1 || len(body) > 9 || body[0] < '1' || body[0] > '9' {
		return 0, refuse(InvalidText, "integer lexeme")
	}
	for i := 1; i < len(body); i++ {
		if body[i] < '0' || body[i] > '9' {
			return 0, refuse(InvalidText, "integer lexeme")
		}
	}
	n, err := strconv.ParseInt(s, 10, 64)
	if err != nil {
		return 0, refuse(InvalidText, "integer lexeme")
	}
	return n, nil
}

var unsupportedRRuleParts = map[string]bool{"BYSECOND": true, "BYWEEKNO": true, "BYYEARDAY": true}

// textRule collects text input; values are range-checked afterwards as
// schema checks, so an in-grammar out-of-bounds integer is invalid_rule.
type textRule struct {
	dtstart  []int
	tzid     string
	sawRRule bool
	freq     string
	interval *int64
	count    *int64
	until    []int
	wkst     int
	byDay    []struct {
		wd  int
		ord int64
	}
	lists         map[string][]int64
	rdate, exdate [][]int
	dateZones     []string
}

func onlyTZID(params []string) (string, error) {
	if len(params) != 1 || !strings.HasPrefix(params[0], "TZID=") || len(params[0]) == len("TZID=") {
		return "", refuse(InvalidText, "expected exactly one TZID parameter")
	}
	return params[0][len("TZID="):], nil
}

func parseLocalValues(v string) ([][]int, error) {
	var out [][]int
	for _, item := range strings.Split(v, ",") {
		f, ok := scanFields(item, layoutTextDate)
		if !ok {
			return nil, refuse(InvalidText, "local date-time")
		}
		out = append(out, f)
	}
	return out, nil
}

func (t *textRule) rrule(value string) error {
	seen := map[string]bool{}
	for _, part := range strings.Split(value, ";") {
		eq := strings.IndexByte(part, '=')
		if eq <= 0 || eq == len(part)-1 {
			return refuse(InvalidText, "RRULE part")
		}
		name, v := part[:eq], part[eq+1:]
		if seen[name] {
			return refuse(InvalidText, "repeated RRULE part")
		}
		seen[name] = true
		switch name {
		case "FREQ":
			if v == "SECONDLY" {
				return refuse(UnsupportedPart, "FREQ=SECONDLY")
			}
			if !isFreq(v) {
				return refuse(InvalidText, "FREQ")
			}
			t.freq = v
		case "INTERVAL", "COUNT":
			n, err := parseTextInt(v)
			if err != nil {
				return err
			}
			if name == "INTERVAL" {
				t.interval = &n
			} else {
				t.count = &n
			}
		case "UNTIL":
			f, ok := scanFields(v, layoutTextUTC)
			if !ok {
				return refuse(InvalidText, "UNTIL")
			}
			t.until = f
		case "WKST":
			if t.wkst = weekdayIndex(v); t.wkst < 0 {
				return refuse(InvalidText, "WKST")
			}
		case "BYDAY":
			for _, item := range strings.Split(v, ",") {
				if len(item) < 2 {
					return refuse(InvalidText, "BYDAY")
				}
				wd := weekdayIndex(item[len(item)-2:])
				if wd < 0 {
					return refuse(InvalidText, "BYDAY")
				}
				var ord int64
				if prefix := item[:len(item)-2]; prefix != "" {
					n, err := parseTextInt(prefix)
					if err != nil {
						return err
					}
					ord = n
					if ord == 0 {
						// The literal 0 is lexically valid but not an ordinal.
						return refuse(InvalidRule, "BYDAY ordinal")
					}
				}
				t.byDay = append(t.byDay, struct {
					wd  int
					ord int64
				}{wd, ord})
			}
		case "BYMONTH", "BYMONTHDAY", "BYHOUR", "BYMINUTE", "BYSETPOS":
			var ns []int64
			for _, item := range strings.Split(v, ",") {
				n, err := parseTextInt(item)
				if err != nil {
					return err
				}
				ns = append(ns, n)
			}
			t.lists[name] = ns
		default:
			if unsupportedRRuleParts[name] {
				return refuse(UnsupportedPart, name)
			}
			return refuse(UnsupportedPart, "RRULE part")
		}
	}
	if t.freq == "" {
		return refuse(InvalidText, "FREQ is required")
	}
	return nil
}

// ParseText decodes and validates a rule from RFC 5545 content lines.
func ParseText(lines []string) (*Rule, error) {
	t := &textRule{lists: map[string][]int64{}}
	for _, line := range lines {
		colon := strings.IndexByte(line, ':')
		if colon < 0 || colon == len(line)-1 {
			return nil, refuse(InvalidText, "content line")
		}
		head := strings.Split(line[:colon], ";")
		name, params, value := head[0], head[1:], line[colon+1:]
		switch name {
		case "RRULE":
			if t.sawRRule {
				return nil, refuse(UnsupportedPart, "more than one RRULE")
			}
			if len(params) != 0 {
				return nil, refuse(InvalidText, "RRULE parameter")
			}
			t.sawRRule = true
			if err := t.rrule(value); err != nil {
				return nil, err
			}
		case "DTSTART":
			if t.dtstart != nil {
				return nil, refuse(InvalidText, "repeated DTSTART")
			}
			zone, err := onlyTZID(params)
			if err != nil {
				return nil, err
			}
			f, ok := scanFields(value, layoutTextDate)
			if !ok {
				return nil, refuse(InvalidText, "DTSTART must be local time with TZID")
			}
			t.dtstart, t.tzid = f, zone
		case "RDATE", "EXDATE":
			zone, err := onlyTZID(params)
			if err != nil {
				return nil, err
			}
			vals, err := parseLocalValues(value)
			if err != nil {
				return nil, err
			}
			t.dateZones = append(t.dateZones, zone)
			if name == "RDATE" {
				t.rdate = append(t.rdate, vals...)
			} else {
				t.exdate = append(t.exdate, vals...)
			}
		default:
			return nil, refuse(UnsupportedPart, "property")
		}
	}
	if t.dtstart == nil || !t.sawRRule {
		return nil, refuse(InvalidText, "DTSTART and RRULE are required")
	}
	for _, z := range t.dateZones {
		if z != t.tzid {
			return nil, refuse(InvalidText, "RDATE/EXDATE TZID differs from DTSTART")
		}
	}
	f, err := t.toFields()
	if err != nil {
		return nil, err
	}
	return f.build()
}

// toFields applies the schema's value bounds to text input.
func (t *textRule) toFields() (*fields, error) {
	if !tzidShapeOK(t.tzid) {
		return nil, badRule("tzid")
	}
	f := &fields{dtstart: t.dtstart, tzid: t.tzid, freq: t.freq, interval: 1, until: t.until, wkst: t.wkst, rdate: t.rdate, exdate: t.exdate}
	bound := func(n, lo, hi int64, noZero bool, what string) (int64, error) {
		if n < lo || n > hi || noZero && n == 0 {
			return 0, badRule(what)
		}
		return n, nil
	}
	var err error
	if t.interval != nil {
		if f.interval, err = bound(*t.interval, 1, 1000, false, "INTERVAL"); err != nil {
			return nil, err
		}
	}
	if t.count != nil {
		if f.count, err = bound(*t.count, 1, 100000, false, "COUNT"); err != nil {
			return nil, err
		}
		f.hasCount = true
	}
	if f.hasCount && f.until != nil {
		return nil, badRule("COUNT and UNTIL together")
	}
	for _, d := range t.byDay {
		if d.ord < -53 || d.ord > 53 {
			return nil, badRule("BYDAY ordinal")
		}
		f.byDay = append(f.byDay, DaySpec{Weekday: d.wd, Ordinal: int(d.ord)})
	}
	type spec struct {
		name   string
		lo, hi int64
		noZero bool
		dst    *[]int64
	}
	for _, s := range []spec{
		{"BYMONTH", 1, 12, false, &f.byMonth},
		{"BYMONTHDAY", -31, 31, true, &f.byMonthDay},
		{"BYHOUR", 0, 23, false, &f.byHour},
		{"BYMINUTE", 0, 59, false, &f.byMinute},
		{"BYSETPOS", -366, 366, true, &f.bySetPos},
	} {
		for _, n := range t.lists[s.name] {
			if _, err := bound(n, s.lo, s.hi, s.noZero, s.name); err != nil {
				return nil, err
			}
			*s.dst = append(*s.dst, n)
		}
	}
	return f, f.crossChecks()
}
