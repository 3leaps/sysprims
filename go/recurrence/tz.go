package recurrence

import (
	"bytes"
	"embed"
	"fmt"
	"strconv"
	"strings"
	"sync"
	"time"
)

//go:embed tzdata/tzif-bundle-2026e.bin
var bundleFS embed.FS

const bundlePath = "tzdata/tzif-bundle-2026e.bin"

type tzBundle struct {
	version string
	index   map[string][2]int
	data    []byte
}

var (
	bundleOnce sync.Once
	theBundle  *tzBundle
	bundleErr  error

	zoneMu    sync.Mutex
	zoneCache = map[string]*time.Location{}
)

func loadBundle() (*tzBundle, error) {
	bundleOnce.Do(func() {
		raw, err := bundleFS.ReadFile(bundlePath)
		if err != nil {
			bundleErr = err
			return
		}
		theBundle, bundleErr = parseBundle(raw)
	})
	return theBundle, bundleErr
}

func parseBundle(raw []byte) (*tzBundle, error) {
	split := bytes.Index(raw, []byte("\n\n"))
	if split < 0 {
		return nil, fmt.Errorf("tz bundle: no header terminator")
	}
	lines := strings.Split(string(raw[:split]), "\n")
	data := raw[split+2:]
	if len(lines) < 3 || lines[0] != "SYSPRIMS-TZIF-BUNDLE 1" ||
		!strings.HasPrefix(lines[1], "tzdb ") || !strings.HasPrefix(lines[2], "zones ") {
		return nil, fmt.Errorf("tz bundle: bad header")
	}
	n, err := strconv.Atoi(strings.TrimPrefix(lines[2], "zones "))
	if err != nil || n != len(lines)-3 {
		return nil, fmt.Errorf("tz bundle: zone count")
	}
	b := &tzBundle{version: strings.TrimPrefix(lines[1], "tzdb "), index: make(map[string][2]int, n), data: data}
	for _, line := range lines[3:] {
		f := strings.Split(line, " ")
		if len(f) != 3 {
			return nil, fmt.Errorf("tz bundle: index line")
		}
		off, err1 := strconv.Atoi(f[1])
		size, err2 := strconv.Atoi(f[2])
		if err1 != nil || err2 != nil || off < 0 || size <= 0 || off+size > len(data) {
			return nil, fmt.Errorf("tz bundle: index range")
		}
		b.index[f[0]] = [2]int{off, size}
	}
	if len(b.index) != n {
		return nil, fmt.Errorf("tz bundle: duplicate zone name")
	}
	return b, nil
}

func mustBundle() *tzBundle {
	b, err := loadBundle()
	if err != nil {
		panic("recurrence: embedded tzdb bundle is malformed: " + err.Error())
	}
	return b
}

// TZDBVersion is the tzdb release this package expands with, read from the
// embedded bundle.
func TZDBVersion() string { return mustBundle().version }

func acceptedZone(name string) bool {
	_, ok := mustBundle().index[name]
	return ok
}

// zone returns a location built only from the embedded TZif bytes.
func zone(name string) (*time.Location, bool) {
	zoneMu.Lock()
	defer zoneMu.Unlock()
	if loc, ok := zoneCache[name]; ok {
		return loc, true
	}
	b := mustBundle()
	span, ok := b.index[name]
	if !ok {
		return nil, false
	}
	loc, err := time.LoadLocationFromTZData(name, b.data[span[0]:span[0]+span[1]])
	if err != nil {
		return nil, false
	}
	zoneCache[name] = loc
	return loc, true
}

// offsetAt is the zone's UTC offset in seconds at a Unix instant.
func offsetAt(loc *time.Location, unix int64) int64 {
	_, off := time.Unix(unix, 0).In(loc).Zone()
	return int64(off)
}

// resolveLocal maps a local wall-clock label (seconds since the civil
// epoch) to Unix seconds. Every offset that the zone uses in the two days
// around the label is a candidate; an instant is a valid reading of the
// label when the zone's offset at that instant equals the candidate.
//
//   - one valid reading: that instant;
//   - several (an overlap): the earliest;
//   - none (a gap): generated labels do not exist (ok=false); explicit labels
//     use the offset in force just before the gap.
func resolveLocal(loc *time.Location, label int64, explicit bool) (int64, bool) {
	before := offsetAt(loc, label-2*secondsPerDay)
	after := offsetAt(loc, label+2*secondsPerDay)
	candidates := []int64{before, after}
	// Zones can change more than once in a short window; include the offsets
	// at the naive readings too.
	candidates = append(candidates, offsetAt(loc, label-before), offsetAt(loc, label-after))
	best, found := int64(0), false
	for _, off := range candidates {
		t := label - off
		if offsetAt(loc, t) == off && (!found || t < best) {
			best, found = t, true
		}
	}
	if found {
		return best, true
	}
	if !explicit {
		return 0, false
	}
	// In a gap. The earlier naive reading lies before the transition, so the
	// zone's offset there is the one in force before the gap.
	return label - offsetAt(loc, label-max64(before, after)), true
}

func max64(a, b int64) int64 {
	if a > b {
		return a
	}
	return b
}
