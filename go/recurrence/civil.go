package recurrence

import (
	"fmt"
	"time"
)

const (
	secondsPerDay = 86400
	minYear       = 1
	maxYear       = 9999
)

// Local is a civil date-time label with whole seconds and no offset.
type Local struct {
	Year                 int
	Month, Day           int
	Hour, Minute, Second int
}

// valid reports whether l names a real Gregorian date-time in years 0001..9999.
func (l Local) valid() bool {
	if l.Year < minYear || l.Year > maxYear || l.Month < 1 || l.Month > 12 {
		return false
	}
	return l.Day >= 1 && l.Day <= monthLength(l.Year, l.Month) &&
		l.Hour >= 0 && l.Hour <= 23 && l.Minute >= 0 && l.Minute <= 59 && l.Second >= 0 && l.Second <= 59
}

// seconds is the label read as if it were UTC: an ordering key and the input
// to offset arithmetic. time.Date with UTC is exact for valid labels.
func (l Local) seconds() int64 {
	return time.Date(l.Year, time.Month(l.Month), l.Day, l.Hour, l.Minute, l.Second, 0, time.UTC).Unix()
}

func localFromSeconds(s int64) Local {
	t := time.Unix(s, 0).UTC()
	return Local{t.Year(), int(t.Month()), t.Day(), t.Hour(), t.Minute(), t.Second()}
}

// String is the extended form YYYY-MM-DDTHH:MM:SS.
func (l Local) String() string {
	return fmt.Sprintf("%04d-%02d-%02dT%02d:%02d:%02d", l.Year, l.Month, l.Day, l.Hour, l.Minute, l.Second)
}

func (l Local) basic() string {
	return fmt.Sprintf("%04d%02d%02dT%02d%02d%02d", l.Year, l.Month, l.Day, l.Hour, l.Minute, l.Second)
}

func monthLength(year, month int) int {
	return time.Date(year, time.Month(month)+1, 0, 0, 0, 0, 0, time.UTC).Day()
}

// dayNumber counts days from 1970-01-01 for a valid date.
func dayNumber(year, month, day int) int64 {
	return time.Date(year, time.Month(month), day, 0, 0, 0, 0, time.UTC).Unix() / secondsPerDay
}

func dateOfDay(n int64) (year, month, day int) {
	t := time.Unix(n*secondsPerDay, 0).UTC()
	return t.Year(), int(t.Month()), t.Day()
}

// weekdayOfDay is 0 for Monday through 6 for Sunday.
func weekdayOfDay(n int64) int {
	return (int(time.Unix(n*secondsPerDay, 0).UTC().Weekday()) + 6) % 7
}

// scanFields reads a fixed layout. In layout, the letters Y M D h m s mark
// digit positions of the year, month, day, hour, minute and second fields;
// any other byte must match literally. It returns the six fields in that
// order, or ok=false.
func scanFields(s, layout string) (fields []int, ok bool) {
	if len(s) != len(layout) {
		return nil, false
	}
	const order = "YMDhms"
	out := make([]int, len(order))
	for i := 0; i < len(layout); i++ {
		k := -1
		for j := 0; j < len(order); j++ {
			if layout[i] == order[j] {
				k = j
			}
		}
		if k < 0 {
			if s[i] != layout[i] {
				return nil, false
			}
			continue
		}
		c := s[i]
		if c < '0' || c > '9' {
			return nil, false
		}
		out[k] = out[k]*10 + int(c-'0')
	}
	return out, true
}

const (
	layoutLocal    = "YYYY-MM-DDThh:mm:ss"
	layoutUTC      = "YYYY-MM-DDThh:mm:ssZ"
	layoutTextDate = "YYYYMMDDThhmmss"
	layoutTextUTC  = "YYYYMMDDThhmmssZ"
)

func localOf(f []int) Local { return Local{f[0], f[1], f[2], f[3], f[4], f[5]} }

// ParseInstant parses YYYY-MM-DDTHH:MM:SSZ into Unix seconds.
func ParseInstant(s string) (time.Time, error) {
	f, ok := scanFields(s, layoutUTC)
	if !ok || !localOf(f).valid() {
		return time.Time{}, fmt.Errorf("recurrence: %q is not a whole-second UTC instant", s)
	}
	return time.Unix(localOf(f).seconds(), 0).UTC(), nil
}

// FormatInstant renders an instant as YYYY-MM-DDTHH:MM:SSZ.
func FormatInstant(t time.Time) string {
	return t.UTC().Format("2006-01-02T15:04:05Z")
}
