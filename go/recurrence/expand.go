package recurrence

import (
	"sort"
	"time"
)

// SearchBudget is the number of consecutive periods without an eligible
// occurrence after which a query stops with ErrSearchLimit. Stopping is a
// resource refusal, never a claim that no later occurrence exists.
const SearchBudget = 2_000_000

// zoneSlack bounds the gap between a local label and its instant; tzdb
// offsets stay well inside a day, so a day and a quarter is safe.
const zoneSlack = 30 * 3600

const (
	firstInstant = -62135596800 // 0001-01-01T00:00:00Z
	lastInstant  = 253402300799 // 9999-12-31T23:59:59Z
)

type picked struct{ label, at int64 }

type expander struct {
	r        *Rule
	loc      *time.Location
	start    int64 // resolved DTSTART instant (explicit-time rules)
	startLbl int64
	// Period 0 cursor: the first label of the period containing DTSTART.
	baseDay  int64 // DAILY/WEEKLY: first day; HOURLY/MINUTELY: day of the label
	baseLbl  int64 // HOURLY/MINUTELY: first label of period 0
	baseYear int
	baseMon  int
}

func newExpander(r *Rule) *expander {
	loc, _ := zone(r.tzid)
	e := &expander{r: r, loc: loc, startLbl: r.dtstart.seconds()}
	e.start, _ = resolveLocal(loc, e.startLbl, true)
	d := dayNumber(r.dtstart.Year, r.dtstart.Month, r.dtstart.Day)
	switch r.freq {
	case Weekly:
		e.baseDay = d - int64((weekdayOfDay(d)-r.wkst+7)%7)
	case Daily:
		e.baseDay = d
	case Hourly:
		e.baseLbl = e.startLbl - int64(r.dtstart.Minute*60+r.dtstart.Second)
	case Minutely:
		e.baseLbl = e.startLbl - int64(r.dtstart.Second)
	}
	e.baseYear, e.baseMon = r.dtstart.Year, r.dtstart.Month
	return e
}

func inYears(day int64) bool {
	y, _, _ := dateOfDay(day)
	return y >= minYear && y <= maxYear
}

// times lists the time-of-day choices for an expanded day.
func (e *expander) times() (hours, minutes []int) {
	hours, minutes = e.r.byHour, e.r.byMinute
	if len(hours) == 0 {
		hours = []int{e.r.dtstart.Hour}
	}
	if len(minutes) == 0 {
		minutes = []int{e.r.dtstart.Minute}
	}
	return
}

func contains(list []int, v int) bool {
	for _, x := range list {
		if x == v {
			return true
		}
	}
	return false
}

// monthDayOf resolves a BYMONTHDAY selector in a month; ok=false when the
// month has no such day.
func monthDayOf(year, month, sel int) (int, bool) {
	n := monthLength(year, month)
	d := sel
	if sel < 0 {
		d = n + sel + 1
	}
	return d, d >= 1 && d <= n
}

// weekdayHits lists the days in [lo, hi] that a BYDAY entry selects, with
// its ordinal (if any) counted inside that range.
func weekdayHits(lo, hi int64, spec DaySpec) []int64 {
	var all []int64
	first := lo + int64((spec.Weekday-weekdayOfDay(lo)+7)%7)
	for d := first; d <= hi; d += 7 {
		all = append(all, d)
	}
	switch {
	case spec.Ordinal == 0:
		return all
	case spec.Ordinal > 0 && spec.Ordinal <= len(all):
		return []int64{all[spec.Ordinal-1]}
	case spec.Ordinal < 0 && -spec.Ordinal <= len(all):
		return []int64{all[len(all)+spec.Ordinal]}
	}
	return nil
}

// dayLimited applies the day-level limits used by DAILY and finer rules.
func (e *expander) dayLimited(day int64) bool {
	y, m, d := dateOfDay(day)
	if len(e.r.byMonth) > 0 && !contains(e.r.byMonth, m) {
		return false
	}
	if len(e.r.byMonthDay) > 0 {
		hit := false
		for _, sel := range e.r.byMonthDay {
			if md, ok := monthDayOf(y, m, sel); ok && md == d {
				hit = true
			}
		}
		if !hit {
			return false
		}
	}
	if len(e.r.byDay) > 0 {
		wd := weekdayOfDay(day)
		hit := false
		for _, s := range e.r.byDay {
			hit = hit || s.Weekday == wd
		}
		if !hit {
			return false
		}
	}
	return true
}

// daysInMonthScope expands day candidates of one month. Ordinals count
// within [ordLo, ordHi] (the month, or the year for yearly rules without
// BYMONTH).
func (e *expander) daysInMonthScope(year, month int, ordLo, ordHi int64) []int64 {
	r := e.r
	monLo := dayNumber(year, month, 1)
	monHi := monLo + int64(monthLength(year, month)) - 1
	var out []int64
	switch {
	case len(r.byMonthDay) > 0:
		for _, sel := range r.byMonthDay {
			md, ok := monthDayOf(year, month, sel)
			if !ok {
				continue
			}
			day := dayNumber(year, month, md)
			if len(r.byDay) == 0 {
				out = append(out, day)
				continue
			}
			for _, s := range r.byDay {
				for _, h := range weekdayHits(ordLo, ordHi, s) {
					if h == day {
						out = append(out, day)
					}
				}
			}
		}
	case len(r.byDay) > 0:
		for _, s := range r.byDay {
			out = append(out, weekdayHits(monLo, monHi, s)...)
		}
	default:
		if r.dtstart.Day <= monthLength(year, month) {
			out = append(out, dayNumber(year, month, r.dtstart.Day))
		}
	}
	return out
}

// labels returns the sorted, unique candidate labels of period k, and
// ok=false once period k begins after year 9999.
func (e *expander) labels(k int64) ([]int64, bool) {
	r := e.r
	step := int64(r.interval) * k
	var days []int64
	var direct []int64 // HOURLY/MINUTELY build labels directly
	switch r.freq {
	case Yearly:
		y := int64(e.baseYear) + step
		if y > maxYear {
			return nil, false
		}
		year := int(y)
		if len(r.byMonthDay) == 0 && len(r.byDay) > 0 && len(r.byMonth) == 0 {
			lo, hi := dayNumber(year, 1, 1), dayNumber(year, 12, 31)
			for _, s := range r.byDay {
				days = append(days, weekdayHits(lo, hi, s)...)
			}
			break
		}
		months := r.byMonth
		if len(months) == 0 {
			if len(r.byMonthDay) > 0 {
				months = []int{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12}
			} else {
				months = []int{r.dtstart.Month}
			}
		}
		for _, m := range months {
			lo, hi := dayNumber(year, m, 1), dayNumber(year, m, monthLength(year, m))
			if len(r.byMonth) == 0 {
				lo, hi = dayNumber(year, 1, 1), dayNumber(year, 12, 31)
			}
			days = append(days, e.daysInMonthScope(year, m, lo, hi)...)
		}
	case Monthly:
		idx := int64(e.baseYear)*12 + int64(e.baseMon-1) + step
		if idx/12 > maxYear {
			return nil, false
		}
		year, month := int(idx/12), int(idx%12)+1
		if len(r.byMonth) > 0 && !contains(r.byMonth, month) {
			return nil, true
		}
		lo := dayNumber(year, month, 1)
		days = e.daysInMonthScope(year, month, lo, lo+int64(monthLength(year, month))-1)
	case Weekly:
		first := e.baseDay + 7*step
		if y, _, _ := dateOfDay(first); y > maxYear {
			return nil, false
		}
		want := []int{weekdayOfDay(dayNumber(r.dtstart.Year, r.dtstart.Month, r.dtstart.Day))}
		if len(r.byDay) > 0 {
			want = want[:0]
			for _, s := range r.byDay {
				want = append(want, s.Weekday)
			}
		}
		for d := first; d < first+7; d++ {
			_, m, _ := dateOfDay(d)
			if contains(want, weekdayOfDay(d)) && (len(r.byMonth) == 0 || contains(r.byMonth, m)) {
				days = append(days, d)
			}
		}
	case Daily:
		d := e.baseDay + step
		if !inYears(d) {
			return nil, false
		}
		if e.dayLimited(d) {
			days = []int64{d}
		}
	case Hourly, Minutely:
		unit := int64(3600)
		if r.freq == Minutely {
			unit = 60
		}
		lbl := e.baseLbl + step*unit
		day := floorDiv(lbl, secondsPerDay)
		if !inYears(day) {
			return nil, false
		}
		tod := lbl - day*secondsPerDay
		hour, minute := int(tod/3600), int(tod%3600/60)
		if !e.dayLimited(day) || len(r.byHour) > 0 && !contains(r.byHour, hour) {
			return nil, true
		}
		if r.freq == Minutely {
			if len(r.byMinute) == 0 || contains(r.byMinute, minute) {
				direct = []int64{lbl + int64(r.dtstart.Second)}
			}
		} else {
			mins := r.byMinute
			if len(mins) == 0 {
				mins = []int{r.dtstart.Minute}
			}
			for _, mi := range mins {
				direct = append(direct, day*secondsPerDay+int64(hour*3600+mi*60+r.dtstart.Second))
			}
		}
	}
	out := direct
	if out == nil {
		hours, minutes := e.times()
		for _, d := range uniqueSorted(days) {
			if !inYears(d) {
				continue
			}
			for _, h := range hours {
				for _, mi := range minutes {
					out = append(out, d*secondsPerDay+int64(h*3600+mi*60+r.dtstart.Second))
				}
			}
		}
	}
	return uniqueSorted(out), true
}

func floorDiv(a, b int64) int64 {
	q := a / b
	if a%b != 0 && (a < 0) != (b < 0) {
		q--
	}
	return q
}

func uniqueSorted(v []int64) []int64 {
	sort.Slice(v, func(i, j int) bool { return v[i] < v[j] })
	out := v[:0]
	for i, x := range v {
		if i == 0 || x != v[i-1] {
			out = append(out, x)
		}
	}
	return out
}

// pick resolves period k's candidates (gap labels dropped, except the
// explicit DTSTART seed in period 0) and applies BYSETPOS.
func (e *expander) pick(k int64) ([]picked, bool) {
	labels, ok := e.labels(k)
	if !ok {
		return nil, false
	}
	var set []picked
	for _, l := range labels {
		if k == 0 && l == e.startLbl {
			set = append(set, picked{l, e.start})
			continue
		}
		if at, exists := resolveLocal(e.loc, l, false); exists {
			set = append(set, picked{l, at})
		}
	}
	if len(e.r.bySetPos) == 0 {
		return set, true
	}
	chosen := map[int]bool{}
	for _, p := range e.r.bySetPos {
		i := p - 1
		if p < 0 {
			i = len(set) + p
		}
		if i >= 0 && i < len(set) {
			chosen[i] = true
		}
	}
	var out []picked
	for i, s := range set {
		if chosen[i] {
			out = append(out, s)
		}
	}
	return out, true
}

func (e *expander) synchronized() bool {
	set, ok := e.pick(0)
	if !ok {
		return false
	}
	for _, s := range set {
		if s.label == e.startLbl {
			return true
		}
	}
	return false
}

// periodStart is the first local label of period k (an ordering bound).
func (e *expander) periodStart(k int64) int64 {
	r := e.r
	step := int64(r.interval) * k
	switch r.freq {
	case Yearly:
		y := int64(e.baseYear) + step
		if y > maxYear {
			y = maxYear + 1
		}
		return time.Date(int(y), 1, 1, 0, 0, 0, 0, time.UTC).Unix()
	case Monthly:
		idx := int64(e.baseYear)*12 + int64(e.baseMon-1) + step
		if idx/12 > maxYear {
			idx = (maxYear + 1) * 12
		}
		return time.Date(int(idx/12), time.Month(idx%12+1), 1, 0, 0, 0, 0, time.UTC).Unix()
	case Weekly:
		return (e.baseDay + 7*step) * secondsPerDay
	case Daily:
		return (e.baseDay + step) * secondsPerDay
	case Hourly:
		return e.baseLbl + step*3600
	default:
		return e.baseLbl + step*60
	}
}

// skipTo returns a period index no later than the first one that can hold
// an instant at or after unix. Only used when COUNT does not need the
// occurrences before it.
func (e *expander) skipTo(unix int64) int64 {
	lbl := unix - zoneSlack
	var k int64
	switch e.r.freq {
	case Minutely:
		k = floorDiv(lbl-e.baseLbl, 60*int64(e.r.interval))
	case Hourly:
		k = floorDiv(lbl-e.baseLbl, 3600*int64(e.r.interval))
	case Daily:
		k = floorDiv(floorDiv(lbl, secondsPerDay)-e.baseDay, int64(e.r.interval))
	case Weekly:
		k = floorDiv(floorDiv(lbl, secondsPerDay)-e.baseDay, 7*int64(e.r.interval))
	case Monthly:
		y, m, _ := dateOfDay(floorDiv(lbl, secondsPerDay))
		k = floorDiv(int64(y)*12+int64(m-1)-(int64(e.baseYear)*12+int64(e.baseMon-1)), int64(e.r.interval))
	case Yearly:
		y, _, _ := dateOfDay(floorDiv(lbl, secondsPerDay))
		k = floorDiv(int64(y-e.baseYear), int64(e.r.interval))
	}
	if k < 1 {
		return 0
	}
	return k - 1
}

// stream yields RRULE occurrences in increasing order.
type stream struct {
	e       *expander
	k       int64
	queue   []int64
	emitted map[int64]bool
	high    int64 // largest instant emitted so far
	n       int   // occurrences emitted (for COUNT)
	ended   bool
	budget  int // consecutive empty periods allowed
}

func (e *expander) newStream(from int64) *stream {
	s := &stream{e: e, emitted: map[int64]bool{}, high: e.start - 1, budget: SearchBudget}
	if e.r.count == 0 {
		s.k = e.skipTo(from)
	}
	return s
}

// next returns the next occurrence; ok=false when the RRULE is exhausted or
// the next period starts at or after horizon (labels can precede their
// instants by at most zoneSlack).
func (s *stream) next(horizon int64) (int64, bool, error) {
	empty := 0
	for {
		if s.ended || s.e.r.count > 0 && s.n >= s.e.r.count {
			s.ended = true
			return 0, false, nil
		}
		if len(s.queue) > 0 {
			at := s.queue[0]
			s.queue = s.queue[1:]
			if s.e.r.hasUntil && at > s.e.r.until {
				s.ended = true
				return 0, false, nil
			}
			s.n++
			return at, true, nil
		}
		set, ok := s.e.pick(s.k)
		if !ok {
			s.ended = true
			return 0, false, nil
		}
		var fresh []int64
		for _, p := range set {
			if p.at >= s.e.start && p.at > s.high && !s.emitted[p.at] && p.at >= firstInstant && p.at <= lastInstant {
				fresh = append(fresh, p.at)
			}
		}
		fresh = uniqueSorted(fresh)
		if len(fresh) == 0 && s.e.periodStart(s.k)-zoneSlack >= horizon {
			return 0, false, nil
		}
		s.k = s.e.advance(s.k, len(fresh) == 0)
		if len(fresh) > 0 {
			for _, at := range fresh {
				s.emitted[at] = true
			}
			s.high = fresh[len(fresh)-1]
			s.queue = fresh
			empty = 0
			continue
		}
		empty++
		if empty >= s.budget {
			return 0, false, ErrSearchLimit
		}
	}
}

// advance moves past period k. After an empty HOURLY or MINUTELY period
// whose day (or, for MINUTELY, hour) fails the limits, it jumps to the
// first period of the next day (or hour).
func (e *expander) advance(k int64, wasEmpty bool) int64 {
	r := e.r
	if !wasEmpty || r.freq != Hourly && r.freq != Minutely {
		return k + 1
	}
	unit := int64(3600)
	if r.freq == Minutely {
		unit = 60
	}
	stepSecs := unit * int64(r.interval)
	lbl := e.baseLbl + k*stepSecs
	day := floorDiv(lbl, secondsPerDay)
	var target int64
	switch {
	case !e.dayLimited(day):
		target = (day + 1) * secondsPerDay
	case r.freq == Minutely && len(r.byHour) > 0 && !contains(r.byHour, int((lbl-day*secondsPerDay)/3600)):
		target = floorDiv(lbl, 3600)*3600 + 3600
	default:
		return k + 1
	}
	next := (target - e.baseLbl + stepSecs - 1) / stepSecs
	if next <= k {
		return k + 1
	}
	return next
}

func (r *Rule) explicitInstants(e *expander, ls []Local) []int64 {
	var out []int64
	for _, l := range ls {
		if at, ok := resolveLocal(e.loc, l.seconds(), true); ok && at >= firstInstant && at <= lastInstant {
			out = append(out, at)
		}
	}
	return uniqueSorted(out)
}

// Expand returns the occurrences in the half-open UTC range [from, to),
// earliest first, at most limit of them.
func (r *Rule) Expand(from, to time.Time, limit int) ([]time.Time, error) {
	if limit <= 0 {
		return nil, ErrInvalidLimit
	}
	if from.After(to) {
		return nil, ErrReversedRange
	}
	return r.query(from.Unix(), to.Unix(), limit)
}

// NextAfter returns the first occurrence strictly after t. found=false
// with a nil error means no later occurrence exists.
func (r *Rule) NextAfter(t time.Time) (next time.Time, found bool, err error) {
	got, err := r.query(t.Unix()+1, lastInstant+1, 1)
	if err != nil || len(got) == 0 {
		return time.Time{}, false, err
	}
	return got[0], true, nil
}

func (r *Rule) query(from, to int64, limit int) ([]time.Time, error) {
	out := []time.Time{}
	if from >= to {
		return out, nil
	}
	e := newExpander(r)
	excluded := map[int64]bool{}
	for _, at := range r.explicitInstants(e, r.exdate) {
		excluded[at] = true
	}
	var extra []int64
	for _, at := range r.explicitInstants(e, r.rdate) {
		if at >= from {
			extra = append(extra, at)
		}
	}
	s := e.newStream(from)
	ruleNext, ruleOK, err := s.next(to)
	if err != nil {
		return nil, err
	}
	for {
		var at int64
		switch {
		case ruleOK && (len(extra) == 0 || ruleNext <= extra[0]):
			at = ruleNext
		case len(extra) > 0:
			at = extra[0]
		default:
			return out, nil
		}
		if at >= to {
			return out, nil
		}
		fromRule := ruleOK && ruleNext == at
		if len(extra) > 0 && extra[0] == at {
			extra = extra[1:]
		}
		if at >= from && !excluded[at] {
			out = append(out, time.Unix(at, 0).UTC())
			if len(out) == limit {
				return out, nil
			}
		}
		if fromRule {
			if ruleNext, ruleOK, err = s.next(to); err != nil {
				return nil, err
			}
		}
	}
}
