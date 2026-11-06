// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build !darwin && !linux

package procobserve_test

import (
	"errors"
	"testing"

	"github.com/3leaps/sysprims/procobserve"
)

func TestUnsupportedPlatformFailsClosed(t *testing.T) {
	hs := sampleHostScope()
	opts := procobserve.DefaultObservationOptions()
	opts.ObservedAt = "2026-11-06T12:00:00Z"

	_, err := procobserve.ObserveProcess(4242, hs, opts)
	var ref *procobserve.CollectorRefusal
	if !errors.As(err, &ref) {
		t.Fatalf("expected *CollectorRefusal, got %v", err)
	}
	if ref.Status != "unsupported" || ref.Code != "windows-collector-not-qualified" {
		t.Fatalf("unexpected refusal: %+v", ref)
	}

	_, herr := procobserve.OpenContinuityHandle(4242, hs, opts)
	if !errors.As(herr, &ref) {
		t.Fatalf("expected *CollectorRefusal, got %v", herr)
	}
	if ref.Status != "unsupported" || ref.Code != "windows-collector-not-qualified" {
		t.Fatalf("unexpected handle refusal: %+v", ref)
	}
}
