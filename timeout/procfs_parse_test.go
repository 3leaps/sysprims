// SPDX-License-Identifier: MIT OR Apache-2.0

package timeout

import (
	"reflect"
	"testing"
)

func TestLinuxStatMembershipParserHandlesComplexCommandNames(t *testing.T) {
	state, pgrp, sid, ok := parseLinuxStatStateGroupSession([]byte("17 (worker ) pool) S 3 17 9 0 0"))
	if !ok || state != 'S' || pgrp != 17 || sid != 9 {
		t.Fatalf("got (%c, %d, %d, %v), want ('S', 17, 9, true)", state, pgrp, sid, ok)
	}
	mState, mPgrp, mOk := parseLinuxStatMembership([]byte("17 (worker ) pool) S 3 17 9 0 0"))
	if !mOk || mState != 'S' || mPgrp != 17 {
		t.Fatalf("membership got (%c, %d, %v), want ('S', 17, true)", mState, mPgrp, mOk)
	}
	tState, ticks, tOk := parseLinuxStatStartTicks([]byte("17 (worker ) pool) S 3 17 9 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 4200 0"))
	if !tOk || tState != 'S' || ticks != 4200 {
		t.Fatalf("startTicks got (%c, %d, %v), want ('S', 4200, true)", tState, ticks, tOk)
	}
}

func TestLinuxStatMembershipParserIgnoresNonUTF8CommandNames(t *testing.T) {
	stat := []byte("17 (worker \xff pool) S 3 17 9 0 0")
	state, pgrp, sid, ok := parseLinuxStatStateGroupSession(stat)
	if !ok || state != 'S' || pgrp != 17 || sid != 9 {
		t.Fatalf("got (%c, %d, %d, %v), want ('S', 17, 9, true)", state, pgrp, sid, ok)
	}
}

func TestLinuxProcfsVisibilityRejectsRestrictiveOrUnverifiableMounts(t *testing.T) {
	unrestricted := []byte("29 24 0:25 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw\n")
	explicitUnrestricted := []byte("29 24 0:25 / /proc rw,nosuid,nodev,noexec,relatime,hidepid=0 - proc proc rw,hidepid=0\n")
	omittedPIDVisibility := []byte("29 24 0:25 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw,hidepid=2\n")
	bindMount := []byte("29 24 0:25 /123 /proc rw - proc proc rw\n")
	maskedPID := []byte("29 24 0:25 / /proc rw - proc proc rw\n30 29 0:26 / /proc/123 rw - tmpfs tmpfs rw\n")

	if !linuxProcfsVisibilityIsComplete(unrestricted, 29) {
		t.Fatal("expected unrestricted procfs to be complete")
	}
	if !linuxProcfsVisibilityIsComplete(explicitUnrestricted, 29) {
		t.Fatal("expected explicit hidepid=0 procfs to be complete")
	}
	if linuxProcfsVisibilityIsComplete(omittedPIDVisibility, 29) {
		t.Fatal("expected hidepid=2 procfs to be incomplete")
	}
	if linuxProcfsVisibilityIsComplete(bindMount, 29) {
		t.Fatal("expected bind mount procfs to be incomplete")
	}
	if linuxProcfsVisibilityIsComplete(maskedPID, 29) {
		t.Fatal("expected masked PID procfs to be incomplete")
	}
	if linuxProcfsVisibilityIsComplete([]byte("29 24 0:25 / /host-proc rw - proc proc rw\n"), 29) {
		t.Fatal("expected non-/proc mount point to be incomplete")
	}
	if linuxProcfsVisibilityIsComplete([]byte("malformed"), 29) {
		t.Fatal("expected malformed mountinfo to be incomplete")
	}
	if linuxProcfsVisibilityIsComplete(unrestricted, 30) {
		t.Fatal("expected mismatched mount ID to be incomplete")
	}
}

func TestLinuxProcfsMountIDParserReadsEffectiveMount(t *testing.T) {
	id, ok := linuxProcfsMountID([]byte("pos:\t0\nflags:\t0100000\nmnt_id:\t29\n"))
	if !ok || id != 29 {
		t.Fatalf("got (%d, %v), want (29, true)", id, ok)
	}
	if _, ok := linuxProcfsMountID([]byte("mnt_id:\tnot-a-number\n")); ok {
		t.Fatal("expected invalid mnt_id to fail")
	}
}

func TestLinuxProcfsIdentityMustMatchCallerNamespace(t *testing.T) {
	stat := []byte("17 (observer) S 3 17 9 0 0")
	if !linuxProcfsIdentityMatches(stat, 17, 17, 9) {
		t.Fatal("expected matching identity")
	}
	if linuxProcfsIdentityMatches(stat, 1, 17, 9) {
		t.Fatal("expected mismatched PID to fail")
	}
	if linuxProcfsIdentityMatches(stat, 17, 4, 9) {
		t.Fatal("expected mismatched PGID to fail")
	}
	if linuxProcfsIdentityMatches(stat, 17, 17, 5) {
		t.Fatal("expected mismatched SID to fail")
	}
	if linuxProcfsIdentityMatches([]byte("malformed"), 17, 17, 9) {
		t.Fatal("expected malformed stat to fail")
	}
}

func TestCompletionObservationStabilization(t *testing.T) {
	attempts := 0
	completion := stabilizeCompletion(ObservationLinuxProcfsProcessGroup, func() ([]uint32, bool) {
		attempts++
		if attempts == 1 {
			return []uint32{41}, true
		}
		return []uint32{29, 41}, true
	})
	if attempts != 3 {
		t.Fatalf("attempts = %d, want 3", attempts)
	}
	count := uint32(2)
	want := ContainmentCompletion{
		Status:        "survivors",
		Observation:   ObservationLinuxProcfsProcessGroup,
		ObservedCount: &count,
		SurvivorPIDs:  []uint32{29, 41},
	}
	if !reflect.DeepEqual(completion, want) {
		t.Fatalf("got %+v, want %+v", completion, want)
	}

	unknown := stabilizeCompletion(ObservationLinuxProcfsProcessGroup, func() ([]uint32, bool) {
		return nil, false
	})
	if unknown.Status != "unknown" || unknown.Observation != ObservationLinuxProcfsProcessGroup {
		t.Fatalf("unexpected unknown completion: %+v", unknown)
	}
}
