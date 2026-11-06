// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build linux

package procobserve

import (
	"errors"
	"runtime"
	"testing"

	"github.com/3leaps/sysprims/evidence"
	"golang.org/x/sys/unix"
)

func TestLinuxStatAndMountinfoParsers(t *testing.T) {
	stat := "4242 (weird (comm)\nname ) S 1 4242 4242 0 -1 4194304 10 0 0 0 1 2 0 0 20 0 1 0 987654321 12345 10 18446744073709551615\n"
	state, ticks, reason := parseProcStat(stat)
	if reason != "" || state != "live" || ticks != "987654321" {
		t.Fatalf("unexpected parseProcStat: state=%q ticks=%q reason=%q", state, ticks, reason)
	}

	zstat := "4242 (z) Z 1 4242 4242 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 0 0 0\n"
	zstate, zticks, zreason := parseProcStat(zstat)
	if zreason != "" || zstate != "zombie" || zticks != "0" {
		t.Fatalf("unexpected zombie parseProcStat: state=%q ticks=%q reason=%q", zstate, zticks, zreason)
	}

	normal := "22 28 0:21 / /proc rw,nosuid,nodev,noexec,relatime shared:12 - proc proc rw\n"
	if mountinfoConcealsPIDs(normal, 0, 21) {
		t.Fatal("normal /proc mount must not report concealment")
	}

	noaccess := "22 28 0:21 / /proc rw,relatime - proc proc rw,hidepid=noaccess\n"
	if mountinfoConcealsPIDs(noaccess, 0, 21) {
		t.Fatal("hidepid=noaccess does not conceal PID directories as ENOENT")
	}

	for _, concealing := range []string{
		"22 28 0:21 / /proc rw,relatime - proc proc rw,hidepid=invisible\n",
		"22 28 0:21 / /proc rw,relatime - proc proc rw,hidepid=2\n",
		"22 28 0:21 / /proc rw,relatime - proc proc rw,subset=pid\n",
	} {
		if !mountinfoConcealsPIDs(concealing, 0, 21) {
			t.Fatalf("expected mountinfo concealment true for %q", concealing)
		}
	}
}

func TestLinuxNonLeaderThreadTID(t *testing.T) {
	leaderPID := unix.Getpid()
	tidCh := make(chan int, 4)
	releaseCh := make(chan struct{})
	defer close(releaseCh)

	// Spawn up to 4 locked goroutines so at least one runs on a non-M0 OS thread (tid != getpid()).
	var workerTID int
	for range 4 {
		go func() {
			runtime.LockOSThread()
			defer runtime.UnlockOSThread()
			tidCh <- unix.Gettid()
			<-releaseCh
		}()
		tid := <-tidCh
		if tid != leaderPID && tid > 0 {
			workerTID = tid
			break
		}
	}
	if workerTID == 0 {
		t.Fatal("failed to obtain non-leader OS thread TID")
	}

	hs := HostScope{
		IdentifierRef: "id-host-local",
		Namespace:     "urn:3leaps:identity-evidence:host-scope:test-fixture:v0",
		Value:         "qualification-fixture-host",
		Derivation: evidence.ProfileRef{
			ID:       "urn:3leaps:identity-evidence:derivation:caller-supplied-host-scope:v0",
			Revision: "sha256:0000000000000000000000000000000000000000000000000000000000000000",
		},
		Stability:     "boot",
		CloneBehavior: "regenerated",
	}
	opts := DefaultObservationOptions()
	opts.ObservedAt = "2026-11-06T12:00:00Z"

	obs, err := ObserveProcess(uint32(workerTID), hs, opts) // #nosec G115 -- verified positive Linux TID
	if err != nil {
		t.Fatalf("ObserveProcess(non-leader TID): %v", err)
	}
	procObj, _ := obs["process"].(map[string]any)
	psObj, _ := procObj["process_state"].(map[string]any)
	if psObj["status"] != "not-present" || psObj["reason"] != "tid-not-thread-group-leader" {
		t.Fatalf("unexpected non-leader TID process_state: %v", psObj)
	}

	_, herr := OpenContinuityHandle(uint32(workerTID), hs, opts) // #nosec G115 -- verified positive Linux TID
	var ref *CollectorRefusal
	if !errors.As(herr, &ref) || ref.Status != "not-present" {
		t.Fatalf("expected not-present refusal on non-leader thread TID, got %v", herr)
	}
}
