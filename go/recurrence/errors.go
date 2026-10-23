package recurrence

import "errors"

// Reason is a refusal reason from the contract's refusal table.
type Reason string

// Refusal reasons. Every refused rule carries exactly one.
const (
	InvalidRule            Reason = "invalid_rule"
	InvalidText            Reason = "invalid_text"
	UnsupportedPart        Reason = "unsupported_part"
	InvalidTZID            Reason = "invalid_tzid"
	InvalidDate            Reason = "invalid_date"
	DTStartNotSynchronized Reason = "dtstart_not_synchronized"
)

// RefusalError reports why a rule was refused. Detail is diagnostic text that
// names the offending field; it never echoes input values and is not part of
// any conformance comparison.
type RefusalError struct {
	Reason Reason
	Detail string
}

func (e *RefusalError) Error() string { return string(e.Reason) + ": " + e.Detail }

func refuse(r Reason, detail string) error { return &RefusalError{Reason: r, Detail: detail} }

// ReasonOf returns the refusal reason of err, or "" if err is not a refusal.
func ReasonOf(err error) Reason {
	var re *RefusalError
	if errors.As(err, &re) {
		return re.Reason
	}
	return ""
}

var (
	// ErrReversedRange is returned when a query's from is after its to.
	ErrReversedRange = errors.New("recurrence: query range is reversed")
	// ErrInvalidLimit is returned when a query's result limit is not positive.
	ErrInvalidLimit = errors.New("recurrence: result limit must be positive")
	// ErrSearchLimit is returned when a query stops at its search budget.
	// It never means that no later occurrence exists.
	ErrSearchLimit = errors.New("recurrence: search limit exceeded; later occurrences are not ruled out")
)
