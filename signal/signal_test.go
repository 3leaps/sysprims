// SPDX-License-Identifier: MIT OR Apache-2.0

package signal

import (
	"encoding/json"
	"math"
	"os"
	"os/exec"
	"runtime"
	"strings"
	"testing"
	"time"

	"github.com/3leaps/sysprims/syserror"
)

func spawnSleepChild(t *testing.T) *exec.Cmd {
	t.Helper()
	var cmd *exec.Cmd
	if runtime.GOOS == "windows" {
		cmd = exec.Command("ping", "-n", "30", "127.0.0.1")
	} else {
		cmd = exec.Command("sleep", "30")
	}
	if err := cmd.Start(); err != nil {
		t.Fatalf("failed to start child process: %v", err)
	}
	t.Cleanup(func() {
		if cmd.Process != nil {
			_ = cmd.Process.Kill()
			_, _ = cmd.Process.Wait()
		}
	})
	return cmd
}

func TestPIDValidationSafetyBounds(t *testing.T) {
	if MaxSafePID != math.MaxInt32 {
		t.Fatalf("MaxSafePID = %d, want %d", MaxSafePID, math.MaxInt32)
	}

	// PID 0 rejected.
	err := Kill(0, SIGTERM)
	if !syserror.IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Kill(0) err = %v, want ErrInvalidArgument", err)
	}
	if !strings.Contains(err.Error(), "pid must be > 0") {
		t.Fatalf("Kill(0) message = %q, want 'pid must be > 0'", err.Error())
	}

	// PGID 0 rejected.
	err = KillGroup(0, SIGTERM)
	if !syserror.IsCode(err, ErrInvalidArgument) {
		t.Fatalf("KillGroup(0) err = %v, want ErrInvalidArgument", err)
	}
	if !strings.Contains(err.Error(), "pgid must be > 0") {
		t.Fatalf("KillGroup(0) message = %q, want 'pgid must be > 0'", err.Error())
	}

	// Boundary MaxSafePID+1 and math.MaxUint32 (-1 cast hazard) rejected.
	for _, unsafePID := range []uint32{MaxSafePID + 1, math.MaxUint32} {
		err = Kill(unsafePID, SIGTERM)
		if !syserror.IsCode(err, ErrInvalidArgument) {
			t.Fatalf("Kill(%d) err = %v, want ErrInvalidArgument", unsafePID, err)
		}
		if !strings.Contains(err.Error(), "exceeds maximum safe value") {
			t.Fatalf("Kill(%d) message = %q, want 'exceeds maximum safe value'", unsafePID, err.Error())
		}

		err = KillGroup(unsafePID, SIGTERM)
		if !syserror.IsCode(err, ErrInvalidArgument) {
			t.Fatalf("KillGroup(%d) err = %v, want ErrInvalidArgument", unsafePID, err)
		}
	}

	// MaxSafePID passes boundary check (returns NotFound or PermissionDenied, never InvalidArgument).
	err = Kill(MaxSafePID, SIGTERM)
	if syserror.IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Kill(MaxSafePID) returned ErrInvalidArgument: %v", err)
	}

	// Empty batch slice rejected.
	if _, err := KillMany(nil, SIGTERM); !syserror.IsCode(err, ErrInvalidArgument) {
		t.Fatalf("KillMany(nil) err = %v, want ErrInvalidArgument", err)
	}
	if _, err := KillMany([]uint32{99999, 0}, SIGTERM); !syserror.IsCode(err, ErrInvalidArgument) {
		t.Fatalf("KillMany with invalid element err = %v, want ErrInvalidArgument", err)
	}
}

func TestSignalCatalogAndMatching(t *testing.T) {
	if SIGTERM != 15 || SIGKILL != 9 || SIGINT != 2 || SIGHUP != 1 {
		t.Fatalf("unexpected signal constants: SIGTERM=%d SIGKILL=%d SIGINT=%d SIGHUP=%d", SIGTERM, SIGKILL, SIGINT, SIGHUP)
	}
	if runtime.GOOS == "darwin" {
		if SIGUSR1 != 30 || SIGUSR2 != 31 {
			t.Fatalf("darwin SIGUSR1=%d SIGUSR2=%d, want 30, 31", SIGUSR1, SIGUSR2)
		}
	} else {
		if SIGUSR1 != 10 || SIGUSR2 != 12 {
			t.Fatalf("non-darwin SIGUSR1=%d SIGUSR2=%d, want 10, 12", SIGUSR1, SIGUSR2)
		}
	}

	for _, variant := range []string{"SIGTERM", "TERM", "term", " sigterm "} {
		num, err := ResolveSignalName(variant)
		if err != nil || num != SIGTERM {
			t.Fatalf("ResolveSignalName(%q) = (%d, %v), want (%d, nil)", variant, num, err, SIGTERM)
		}
	}
	if _, err := ResolveSignalName("SIGDOESNOTEXIST"); !syserror.IsCode(err, ErrInvalidArgument) {
		t.Fatalf("ResolveSignalName(unknown) = %v, want ErrInvalidArgument", err)
	}

	matches := MatchSignalNames("SIGU*")
	if len(matches) != 2 || matches[0] != "SIGUSR1" || matches[1] != "SIGUSR2" {
		t.Fatalf("MatchSignalNames(SIGU*) = %v, want [SIGUSR1 SIGUSR2]", matches)
	}
}

func TestProcessHandleLifecycleAndInvariants(t *testing.T) {
	// Reject invalid PIDs.
	for _, bad := range []uint32{0, MaxSafePID + 1, math.MaxUint32} {
		if _, err := OpenProcessHandle(bad); !syserror.IsCode(err, ErrInvalidArgument) {
			t.Fatalf("OpenProcessHandle(%d) = %v, want ErrInvalidArgument", bad, err)
		}
	}

	selfPID := uint32(os.Getpid()) // #nosec G115 -- valid host PID fits in uint32
	if _, err := OpenProcessHandleVerified(selfPID, "   "); !syserror.IsCode(err, ErrInvalidArgument) {
		t.Fatalf("OpenProcessHandleVerified(empty) = %v, want ErrInvalidArgument", err)
	}

	selfHandle, err := OpenProcessHandle(selfPID)
	if err != nil {
		t.Fatalf("OpenProcessHandle(self) failed: %v", err)
	}
	defer func() {
		_ = selfHandle.Close()
	}()

	// Fail-closed JSON/text serialization and redacted String().
	if _, err := json.Marshal(selfHandle); err == nil {
		t.Fatal("expected json.Marshal(ProcessHandle) to fail closed")
	}
	if strings.Contains(selfHandle.String(), "kqFD") || strings.Contains(selfHandle.String(), "pidfd") {
		t.Fatalf("ProcessHandle.String() leaked internal descriptor field: %s", selfHandle.String())
	}

	// Verified open succeeds with matching start_identity and fails with mismatch.
	verified, err := OpenProcessHandleVerified(selfPID, selfHandle.StartIdentity())
	if err != nil {
		t.Fatalf("OpenProcessHandleVerified(self, matching) failed: %v", err)
	}
	_ = verified.Close()

	if _, err := OpenProcessHandleVerified(selfPID, "0000000000.999999"); !syserror.IsCode(err, ErrNotFound) {
		t.Fatalf("OpenProcessHandleVerified(self, mismatched) = %v, want ErrNotFound", err)
	}

	// Spawn a live child, open handle, force-kill, and verify latched exit + refusal on second signal.
	cmd := spawnSleepChild(t)
	childPID := uint32(cmd.Process.Pid) // #nosec G115 -- valid child PID fits in uint32

	h, err := OpenProcessHandle(childPID)
	if err != nil {
		t.Fatalf("OpenProcessHandle(child) failed: %v", err)
	}
	defer func() {
		_ = h.Close()
	}()

	exited, err := h.Exited()
	if err != nil || exited {
		t.Fatalf("h.Exited() on live child = (%v, %v), want (false, nil)", exited, err)
	}

	switch runtime.GOOS {
	case "darwin":
		if h.Binding() != SignalBindingCheckedNumeric {
			t.Fatalf("darwin Binding() = %q, want %q", h.Binding(), SignalBindingCheckedNumeric)
		}
	case "linux", "windows":
		if h.Binding() != SignalBindingBound {
			t.Fatalf("%s Binding() = %q, want %q", runtime.GOOS, h.Binding(), SignalBindingBound)
		}
	}

	b, err := h.ForceKill()
	if err != nil {
		t.Fatalf("h.ForceKill() failed: %v", err)
	}
	if b != h.Binding() {
		t.Fatalf("h.ForceKill() binding = %q, want %q", b, h.Binding())
	}

	_, _ = cmd.Process.Wait()

	observed := false
	for i := 0; i < 50; i++ {
		ex, exErr := h.Exited()
		if exErr != nil {
			t.Fatalf("h.Exited() after wait failed: %v", exErr)
		}
		if ex {
			observed = true
			break
		}
		time.Sleep(10 * time.Millisecond)
	}
	if !observed {
		t.Fatal("expected h.Exited() to become true after child exit")
	}

	// Latched true on subsequent call.
	if ex, err := h.Exited(); err != nil || !ex {
		t.Fatalf("latched h.Exited() = (%v, %v), want (true, nil)", ex, err)
	}

	// Second signal after exit refuses with ErrNotFound.
	if _, err := h.ForceKill(); !syserror.IsCode(err, ErrNotFound) {
		t.Fatalf("second ForceKill after exit = %v, want ErrNotFound", err)
	}

	// Close is idempotent and subsequent operations return ErrInvalidArgument.
	if err := h.Close(); err != nil {
		t.Fatalf("h.Close() failed: %v", err)
	}
	if err := h.Close(); err != nil {
		t.Fatalf("second h.Close() failed: %v", err)
	}
	if _, err := h.Exited(); !syserror.IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Exited() on closed handle = %v, want ErrInvalidArgument", err)
	}
}

func TestProcessHandleDetectsUnreapedZombie(t *testing.T) {
	cmd := spawnSleepChild(t)
	childPID := uint32(cmd.Process.Pid) // #nosec G115 -- valid child PID fits in uint32

	h, err := OpenProcessHandle(childPID)
	if err != nil {
		t.Fatalf("OpenProcessHandle(child) failed: %v", err)
	}
	defer func() {
		_ = h.Close()
	}()

	_ = cmd.Process.Kill()
	// Deliberately do not Wait() yet so child becomes an unreaped zombie on Unix.
	observed := false
	for i := 0; i < 100; i++ {
		ex, exErr := h.Exited()
		if exErr != nil {
			t.Fatalf("h.Exited() on zombie failed: %v", exErr)
		}
		if ex {
			observed = true
			break
		}
		time.Sleep(10 * time.Millisecond)
	}
	if !observed {
		t.Fatal("expected h.Exited() to observe unreaped zombie exit")
	}

	if _, err := h.Terminate(); !syserror.IsCode(err, ErrNotFound) {
		t.Fatalf("Terminate() on unreaped zombie = %v, want ErrNotFound", err)
	}

	if _, err := OpenProcessHandle(childPID); !syserror.IsCode(err, ErrNotFound) {
		t.Fatalf("OpenProcessHandle on unreaped zombie = %v, want ErrNotFound", err)
	}

	_, _ = cmd.Process.Wait()
}
