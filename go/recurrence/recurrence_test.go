package recurrence

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"os"
	"strings"
	"testing"
	"time"
)

func mustRule(t *testing.T, js string) *Rule {
	t.Helper()
	r, err := ParseJSON([]byte(js))
	if err != nil {
		t.Fatalf("%s: %v", js, err)
	}
	return r
}

func at(t *testing.T, s string) time.Time {
	t.Helper()
	v, err := ParseInstant(s)
	if err != nil {
		t.Fatal(err)
	}
	return v
}

func TestBundleMatchesItsPinAndTheRoster(t *testing.T) {
	raw, err := bundleFS.ReadFile(bundlePath)
	if err != nil {
		t.Fatal(err)
	}
	pin, err := os.ReadFile("tzdata/tzif-bundle-2026e.sha256")
	if err != nil {
		t.Fatal(err)
	}
	sum := sha256.Sum256(raw)
	if got, want := hex.EncodeToString(sum[:]), strings.Fields(string(pin))[0]; got != want {
		t.Fatalf("bundle sha256 %s, pin %s", got, want)
	}
	if TZDBVersion() != "2026e" || len(mustBundle().index) != 446 {
		t.Fatalf("bundle %s with %d zones", TZDBVersion(), len(mustBundle().index))
	}
	for _, name := range []string{"Europe/Oslo", "Etc/UTC", "America/New_York"} {
		if _, ok := zone(name); !ok {
			t.Errorf("%s does not load", name)
		}
	}
	for _, alias := range []string{"US/Eastern", "Asia/Calcutta", "UTC", "Local", ""} {
		if acceptedZone(alias) {
			t.Errorf("%q accepted", alias)
		}
	}
}

// The host time-zone configuration must not change any answer: zones come
// only from the embedded bundle.
func TestHostTimeZoneIsIgnored(t *testing.T) {
	t.Setenv("TZ", "Asia/Tokyo")
	t.Setenv("ZONEINFO", t.TempDir())
	r := mustRule(t, `{"dtstart":"2026-03-07T02:30:00","tzid":"America/New_York","freq":"DAILY","count":3}`)
	got, err := r.Expand(at(t, "2026-03-01T00:00:00Z"), at(t, "2026-04-01T00:00:00Z"), 10)
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"2026-03-07T07:30:00Z", "2026-03-09T06:30:00Z", "2026-03-10T06:30:00Z"}
	for i := range want {
		if FormatInstant(got[i]) != want[i] {
			t.Fatalf("got %v", got)
		}
	}
}

func TestQueryErrors(t *testing.T) {
	r := mustRule(t, `{"dtstart":"2026-01-05T09:00:00","tzid":"Etc/UTC","freq":"DAILY"}`)
	a, b := at(t, "2026-01-06T00:00:00Z"), at(t, "2026-01-05T00:00:00Z")
	if _, err := r.Expand(a, b, 5); !errors.Is(err, ErrReversedRange) {
		t.Errorf("reversed: %v", err)
	}
	if _, err := r.Expand(b, a, 0); !errors.Is(err, ErrInvalidLimit) {
		t.Errorf("zero limit: %v", err)
	}
	if got, err := r.Expand(a, a, 5); err != nil || len(got) != 0 {
		t.Errorf("equal bounds: %v %v", got, err)
	}
}

func TestEndOfCalendarIsProofNotAResourceError(t *testing.T) {
	r := mustRule(t, `{"dtstart":"9999-12-31T09:00:00","tzid":"Etc/UTC","freq":"YEARLY"}`)
	_, found, err := r.NextAfter(at(t, "9999-12-31T09:00:00Z"))
	if found || err != nil {
		t.Fatalf("found=%v err=%v", found, err)
	}
}

func TestFarQueriesDoNotWalkFromTheStart(t *testing.T) {
	r := mustRule(t, `{"dtstart":"2026-01-05T09:00:00","tzid":"Etc/UTC","freq":"MINUTELY"}`)
	got, err := r.Expand(at(t, "9000-01-01T00:00:00Z"), at(t, "9000-01-01T00:03:00Z"), 10)
	if err != nil || len(got) != 3 || FormatInstant(got[0]) != "9000-01-01T00:00:00Z" {
		t.Fatalf("%v %v", got, err)
	}
}

func TestSearchBudgetIsARefusal(t *testing.T) {
	// January 31 then March 31: after the first occurrence, January (now
	// spent) and February are empty periods before March.
	r := mustRule(t, `{"dtstart":"2026-01-31T09:00:00","tzid":"Etc/UTC","freq":"MONTHLY","by_month_day":[31],"by_month":[1,3]}`)
	after := at(t, "2026-01-31T09:00:01Z").Unix()
	run := func(budget int) (string, error) {
		s := newExpander(r).newStream(after)
		s.budget = budget
		for {
			v, ok, err := s.next(lastInstant + 1)
			if err != nil || !ok {
				return "", err
			}
			if v >= after {
				return FormatInstant(time.Unix(v, 0)), nil
			}
		}
	}
	if got, err := run(3); err != nil || got != "2026-03-31T09:00:00Z" {
		t.Fatalf("budget 3: %q %v", got, err)
	}
	if _, err := run(1); !errors.Is(err, ErrSearchLimit) {
		t.Fatalf("budget 1: %v", err)
	}
}

func TestTextGrammar(t *testing.T) {
	parse := func(rr string) error {
		_, err := ParseText([]string{"DTSTART;TZID=Etc/UTC:20260105T090000", rr})
		return err
	}
	for rr, want := range map[string]Reason{
		"RRULE:FREQ=DAILY;COUNT=+5":         "",
		"RRULE:FREQ=DAILY;BYMINUTE=0":       "",
		"RRULE:FREQ=DAILY;BYMINUTE=-0":      InvalidText,
		"RRULE:FREQ=DAILY;COUNT=05":         InvalidText,
		"RRULE:FREQ=DAILY;COUNT=2;COUNT=2":  InvalidText,
		"RRULE:FREQ=DAILY;BYYEARDAY=1":      UnsupportedPart,
		"RRULE:FREQ=SECONDLY":               UnsupportedPart,
		"RRULE:FREQ=MONTHLY;BYDAY=é":        InvalidText,
		"RRULE:FREQ=MONTHLY;BYDAY=0MO":      InvalidRule,
		"RRULE:FREQ=DAILY;COUNT=999999999":  InvalidRule,
		"RRULE:FREQ=DAILY;COUNT=1000000000": InvalidText,
	} {
		if got := ReasonOf(parse(rr)); got != want {
			t.Errorf("%s: %q, want %q", rr, got, want)
		}
	}
}

func TestJSONStrictness(t *testing.T) {
	base := `"dtstart":"2026-01-05T09:00:00","tzid":"Etc/UTC","freq":"DAILY"`
	for js, want := range map[string]Reason{
		`{` + base + `,"freq":"DAILY"}`:                     InvalidRule,
		`{` + base + `,"count":2.5}`:                        InvalidRule,
		`{` + base + `,"count":null}`:                       InvalidRule,
		`{` + base + `,"by_day":[{"day":"MO","x":1}]}`:      InvalidRule,
		`{` + base + `,"by_day":[{"day":"MO","day":"TU"}]}`: InvalidRule,
		`{` + base + `,"count":2.0}`:                        "",
		`{` + base + `,"count":1e300}`:                      InvalidRule,
		`{` + base + `} `:                                   "",
		`{` + base + `} {}`:                                 InvalidRule,
	} {
		_, err := ParseJSON([]byte(js))
		if got := ReasonOf(err); got != want {
			t.Errorf("%s: %q, want %q", js, got, want)
		}
	}
}

func TestExpansionPropertiesAcrossGapsAndFolds(t *testing.T) {
	for _, js := range []string{
		`{"dtstart":"2026-03-07T20:00:00","tzid":"America/New_York","freq":"HOURLY","interval":5,"by_minute":[0,30]}`,
		`{"dtstart":"2026-04-03T01:45:00","tzid":"Australia/Lord_Howe","freq":"DAILY","by_hour":[1,2],"by_minute":[15,45]}`,
		`{"dtstart":"2026-01-02T17:00:00","tzid":"Europe/Berlin","freq":"MONTHLY","by_day":[{"day":"MO"},{"day":"FR"}],"by_set_pos":[-1,1]}`,
	} {
		r := mustRule(t, js)
		from, to := at(t, "2026-01-01T00:00:00Z"), at(t, "2028-01-01T00:00:00Z")
		all, err := r.Expand(from, to, 500)
		if err != nil {
			t.Fatal(err)
		}
		cursor := from.Add(-time.Second)
		for i, want := range all {
			if i > 0 && !all[i-1].Before(want) {
				t.Fatalf("%s: not strictly increasing at %d", js, i)
			}
			got, found, err := r.NextAfter(cursor)
			if err != nil || !found || !got.Equal(want) {
				t.Fatalf("%s: next_after %v = %v, want %v", js, cursor, got, want)
			}
			cursor = got
		}
		if len(all) > 4 {
			mid, _ := r.Expand(all[2], all[4], 500)
			if len(mid) != 2 || !mid[0].Equal(all[2]) || !mid[1].Equal(all[3]) {
				t.Fatalf("%s: narrowed query changed selection", js)
			}
		}
	}
}
