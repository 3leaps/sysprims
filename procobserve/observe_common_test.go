// SPDX-License-Identifier: MIT OR Apache-2.0

package procobserve_test

import (
	"encoding/json"
	"errors"
	"math"
	"testing"

	"github.com/3leaps/sysprims/evidence"
	"github.com/3leaps/sysprims/procobserve"
)

const zeroDigest = "sha256:0000000000000000000000000000000000000000000000000000000000000000"

func sampleHostScope() procobserve.HostScope {
	return procobserve.HostScope{
		IdentifierRef: "id-host-local",
		Namespace:     "urn:3leaps:identity-evidence:host-scope:test-fixture:v0",
		Value:         "qualification-fixture-host",
		Derivation: evidence.ProfileRef{
			ID:       "urn:3leaps:identity-evidence:derivation:caller-supplied-host-scope:v0",
			Revision: zeroDigest,
		},
		Stability:     "boot",
		CloneBehavior: "regenerated",
	}
}

func assertRefusalCode(t *testing.T, err error, wantCode string) {
	t.Helper()
	if err == nil {
		t.Fatalf("expected refusal %q, got nil", wantCode)
	}
	var ref *procobserve.CollectorRefusal
	if !errors.As(err, &ref) {
		t.Fatalf("expected *CollectorRefusal, got %T (%v)", err, err)
	}
	if ref.Code != wantCode {
		t.Fatalf("got refusal code %q (status=%q detail=%q), want %q", ref.Code, ref.Status, ref.Detail, wantCode)
	}
}

func TestRefusesOutOfRangePIDsAndMalformedInputsBeforeSyscalls(t *testing.T) {
	hs := sampleHostScope()
	opts := procobserve.DefaultObservationOptions()
	opts.ObservedAt = "2026-11-06T12:00:00Z"

	for _, badPID := range []uint32{0, procobserve.MaxObservablePID + 1, math.MaxUint32} {
		_, err := procobserve.ObserveProcess(badPID, hs, opts)
		assertRefusalCode(t, err, "invalid-pid-range")
		_, herr := procobserve.OpenContinuityHandle(badPID, hs, opts)
		assertRefusalCode(t, herr, "invalid-pid-range")
	}

	badSubj := opts
	badSubj.SubjectRef = "Invalid_Subject"
	_, err := procobserve.ObserveProcess(42, hs, badSubj)
	assertRefusalCode(t, err, "invalid-subject-ref")

	badTime := opts
	badTime.ObservedAt = "2026-11-06T12:00:00.123Z"
	_, err = procobserve.ObserveProcess(42, hs, badTime)
	assertRefusalCode(t, err, "invalid-observed-at")

	badHS := hs
	badHS.Stability = "forever"
	_, err = procobserve.ObserveProcess(42, badHS, opts)
	assertRefusalCode(t, err, "invalid-host-scope")

	badHS2 := hs
	badHS2.IdentifierRef = "UPPERCASE"
	_, err = procobserve.ObserveProcess(42, badHS2, opts)
	assertRefusalCode(t, err, "invalid-host-scope")
}

func TestContinuityHandleRefusesJSONAndTextSerializationAndInvalidZeroState(t *testing.T) {
	var zero procobserve.ProcessContinuityHandle

	// Value, pointer, and embedded struct field JSON marshaling must all fail closed.
	_, err := json.Marshal(zero)
	assertRefusalCode(t, err, "continuity-handle-not-serializable")

	_, err = json.Marshal(&zero)
	assertRefusalCode(t, err, "continuity-handle-not-serializable")

	type wrapper struct {
		H procobserve.ProcessContinuityHandle `json:"h"`
	}
	_, err = json.Marshal(wrapper{H: zero})
	assertRefusalCode(t, err, "continuity-handle-not-serializable")

	// JSON unmarshaling into zero handle must fail closed.
	err = json.Unmarshal([]byte(`{}`), &zero)
	assertRefusalCode(t, err, "continuity-handle-not-serializable")

	// Text marshaling and unmarshaling must fail closed on both value and pointer forms.
	_, err = zero.MarshalText()
	assertRefusalCode(t, err, "continuity-handle-not-serializable")
	_, err = (&zero).MarshalText()
	assertRefusalCode(t, err, "continuity-handle-not-serializable")
	err = (&zero).UnmarshalText([]byte("handle"))
	assertRefusalCode(t, err, "continuity-handle-not-serializable")

	// Zero and nil handles fail closed without panicking and are safe to copy by value.
	copiedByValue := zero
	var nilPtr *procobserve.ProcessContinuityHandle
	for _, candidate := range []*procobserve.ProcessContinuityHandle{&zero, &copiedByValue, nilPtr} {
		if got := candidate.PID(); got != 0 {
			t.Fatalf("expected unopened PID()=0, got %d", got)
		}
		if obs := candidate.BaselineObservation(); obs != nil {
			t.Fatalf("expected unopened BaselineObservation()=nil, got %v", obs)
		}
		assertRefusalCode(t, candidate.Close(), "continuity-handle-invalid")
		_, pErr := candidate.PollExited()
		assertRefusalCode(t, pErr, "continuity-handle-invalid")
		_, wErr := candidate.WaitExit(0)
		assertRefusalCode(t, wErr, "continuity-handle-invalid")
		_, rErr := candidate.Reobserve("2026-11-06T12:00:00Z")
		assertRefusalCode(t, rErr, "continuity-handle-invalid")
		_, _, vErr := candidate.Reverify(evidence.ComparisonPolicy{}, "2026-11-06T12:00:00Z")
		assertRefusalCode(t, vErr, "continuity-handle-invalid")
	}
}
