// SPDX-License-Identifier: MIT OR Apache-2.0

package syserror

import (
	"errors"
	"fmt"
	"testing"
)

func TestErrorCodeStringsAndCategories(t *testing.T) {
	cases := []struct {
		code     ErrorCode
		num      int32
		name     string
		category string
	}{
		{ErrOK, 0, "OK", "ok"},
		{ErrInvalidArgument, 1, "InvalidArgument", "invalid_argument"},
		{ErrSpawnFailed, 2, "SpawnFailed", "spawn_failed"},
		{ErrTimeout, 3, "Timeout", "timeout"},
		{ErrPermissionDenied, 4, "PermissionDenied", "permission_denied"},
		{ErrNotFound, 5, "NotFound", "not_found"},
		{ErrNotSupported, 6, "NotSupported", "not_supported"},
		{ErrGroupCreationFailed, 7, "GroupCreationFailed", "group_creation_failed"},
		{ErrSystem, 8, "System", "system"},
		{ErrInternal, 99, "Internal", "internal"},
		{ErrorCode(42), 42, "Unknown", "unknown"},
	}

	for _, tc := range cases {
		if int32(tc.code) != tc.num {
			t.Fatalf("code %v numeric mismatch: got %d, want %d", tc.code, int32(tc.code), tc.num)
		}
		if got := tc.code.String(); got != tc.name {
			t.Fatalf("code %d String() = %q, want %q", tc.num, got, tc.name)
		}
		if got := tc.code.Category(); got != tc.category {
			t.Fatalf("code %d Category() = %q, want %q", tc.num, got, tc.category)
		}
	}
}

func TestConstructorsAndIsCode(t *testing.T) {
	inv := InvalidArgument("pid must be > 0")
	if inv.Error() != "Invalid argument: pid must be > 0" {
		t.Fatalf("unexpected InvalidArgument message: %q", inv.Error())
	}
	if !IsCode(inv, ErrInvalidArgument) {
		t.Fatalf("expected IsCode(inv, ErrInvalidArgument) == true")
	}

	nf := NotFound(99999)
	if nf.Error() != "Process 99999 not found" || nf.PID != 99999 {
		t.Fatalf("unexpected NotFound: %+v", nf)
	}
	wrapped := fmt.Errorf("wrap: %w", nf)
	if !errors.Is(wrapped, &Error{Code: ErrNotFound}) {
		t.Fatalf("expected errors.Is(wrapped, ErrNotFound) == true")
	}
	if !IsCode(wrapped, ErrNotFound) {
		t.Fatalf("expected IsCode(wrapped, ErrNotFound) == true")
	}

	pd := PermissionDenied(4242, "signal")
	if pd.Error() != "Permission denied for 'signal' on PID 4242" {
		t.Fatalf("unexpected PermissionDenied: %q", pd.Error())
	}

	ns := NotSupported("killpg", "windows")
	if ns.Error() != "Operation 'killpg' not supported on windows" {
		t.Fatalf("unexpected NotSupported: %q", ns.Error())
	}

	sysErr := System("unexpected errno", 22)
	if sysErr.Error() != "System error: unexpected errno (errno: 22)" || sysErr.Errno != 22 {
		t.Fatalf("unexpected System: %+v", sysErr)
	}
}
