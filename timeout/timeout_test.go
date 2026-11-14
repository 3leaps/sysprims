// SPDX-License-Identifier: MIT OR Apache-2.0

package timeout_test

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/3leaps/sysprims/signal"
	"github.com/3leaps/sysprims/syserror"
	"github.com/3leaps/sysprims/timeout"
)

func uint64Ptr(v uint64) *uint64 { return &v }
func int32Ptr(v int32) *int32    { return &v }

func spawnTrue(t *testing.T) *timeout.ContainedProcess {
	t.Helper()
	handle, err := timeout.SpawnContained(timeout.SpawnContainedConfig{
		Argv:           []string{"true"},
		GraceTimeoutMS: uint64Ptr(50),
		KillTimeoutMS:  uint64Ptr(200),
	})
	if err != nil {
		t.Fatalf("SpawnContained(true): %v", err)
	}
	return handle
}

func spawnSleep(t *testing.T) *timeout.ContainedProcess {
	t.Helper()
	handle, err := timeout.SpawnContained(timeout.SpawnContainedConfig{
		Argv:           []string{"sleep", "30"},
		GraceTimeoutMS: uint64Ptr(50),
		KillTimeoutMS:  uint64Ptr(500),
	})
	if err != nil {
		t.Fatalf("SpawnContained(sleep): %v", err)
	}
	return handle
}

func TestDefaultTimeoutConfig(t *testing.T) {
	cfg := timeout.DefaultTimeoutConfig()
	if cfg.Signal != signal.SIGTERM {
		t.Errorf("Expected Signal=SIGTERM, got %d", cfg.Signal)
	}
	if cfg.KillAfter != 10*time.Second {
		t.Errorf("Expected KillAfter=10s, got %v", cfg.KillAfter)
	}
	if cfg.Grouping != timeout.GroupByDefault {
		t.Errorf("Expected Grouping=GroupByDefault, got %d", cfg.Grouping)
	}
	if cfg.PreserveStatus {
		t.Error("Expected PreserveStatus=false")
	}
}

func TestRunWithTimeoutCompletes(t *testing.T) {
	var cmd string
	var args []string
	if runtime.GOOS == "windows" {
		cmd = "cmd"
		args = []string{"/c", "echo hello"}
	} else {
		cmd = "echo"
		args = []string{"hello"}
	}

	result, err := timeout.RunWithTimeout(cmd, args, 5*time.Second, timeout.DefaultTimeoutConfig())
	if err != nil {
		t.Fatalf("RunWithTimeout failed: %v", err)
	}
	if !result.Completed() {
		t.Error("Expected command to complete")
	}
	if result.TimedOut() {
		t.Error("Command should not have timed out")
	}
	if result.ExitCode == nil || *result.ExitCode != 0 {
		t.Fatalf("Expected exit code 0, got %+v", result.ExitCode)
	}
	if result.SchemaID != timeout.TimeoutResultSchemaID {
		t.Fatalf("SchemaID = %q, want %q", result.SchemaID, timeout.TimeoutResultSchemaID)
	}
}

func TestRunWithTimeoutBoundedPipesAndTruncation(t *testing.T) {
	var cmd string
	var args []string
	if runtime.GOOS == "windows" {
		cmd = "cmd"
		args = []string{"/c", "echo stdout-payload-1234567890 & echo stderr-payload-1234567890 1>&2"}
	} else {
		cmd = "sh"
		args = []string{"-c", "printf 'stdout-payload-1234567890'; printf 'stderr-payload-1234567890' >&2"}
	}

	cfg := timeout.DefaultTimeoutConfig()
	cfg.MaxStdoutBytes = 14
	cfg.MaxStderrBytes = 64

	res, err := timeout.RunWithTimeout(cmd, args, 5*time.Second, cfg)
	if err != nil {
		t.Fatalf("RunWithTimeout with pipes failed: %v", err)
	}
	if !res.Completed() {
		t.Fatalf("Expected completed status, got %q", res.Status)
	}
	if !res.StdoutTruncated {
		t.Error("Expected StdoutTruncated=true")
	}
	if len(res.Stdout) != 14 {
		t.Errorf("len(Stdout) = %d, want 14", len(res.Stdout))
	}
	if !strings.HasPrefix(string(res.Stdout), "stdout-payload") {
		t.Errorf("Stdout = %q, expected prefix stdout-payload", string(res.Stdout))
	}
	if res.StderrTruncated {
		t.Error("Expected StderrTruncated=false")
	}
	if !strings.Contains(string(res.Stderr), "stderr-payload") {
		t.Errorf("Stderr = %q, expected stderr-payload", string(res.Stderr))
	}
}

func TestRunWithTimeoutTimesOut(t *testing.T) {
	var cmd string
	var args []string
	if runtime.GOOS == "windows" {
		cmd = "cmd"
		args = []string{"/c", "ping -n 10 127.0.0.1"}
	} else {
		cmd = "sleep"
		args = []string{"10"}
	}

	cfg := timeout.DefaultTimeoutConfig()
	cfg.KillAfter = 100 * time.Millisecond

	res, err := timeout.RunWithTimeout(cmd, args, 200*time.Millisecond, cfg)
	if err != nil {
		t.Fatalf("RunWithTimeout failed: %v", err)
	}
	if !res.TimedOut() || res.Completed() {
		t.Fatalf("Expected timed_out result, got %+v", res)
	}
	if res.SignalSent == nil || *res.SignalSent != signal.SIGTERM {
		t.Fatalf("SignalSent = %+v, want SIGTERM", res.SignalSent)
	}
	if runtime.GOOS != "windows" {
		if res.TreeKillReliability == nil || *res.TreeKillReliability != timeout.ReliabilityGuaranteed {
			t.Fatalf("TreeKillReliability = %+v, want guaranteed", res.TreeKillReliability)
		}
	}
}

func TestRunWithTimeoutContextCancellation(t *testing.T) {
	var cmd string
	var args []string
	if runtime.GOOS == "windows" {
		cmd = "cmd"
		args = []string{"/c", "ping -n 10 127.0.0.1"}
	} else {
		cmd = "sleep"
		args = []string{"10"}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 150*time.Millisecond)
	defer cancel()

	cfg := timeout.DefaultTimeoutConfig()
	cfg.KillAfter = 50 * time.Millisecond

	res, err := timeout.RunWithTimeoutContext(ctx, cmd, args, 10*time.Second, cfg)
	if err != nil {
		t.Fatalf("RunWithTimeoutContext failed: %v", err)
	}
	if !res.TimedOut() {
		t.Fatalf("Expected context deadline to trigger timed_out, got %+v", res)
	}
}

func TestRunWithTimeoutNotFound(t *testing.T) {
	_, err := timeout.RunWithTimeout(
		"/nonexistent/command/that/does/not/exist",
		nil,
		time.Second,
		timeout.DefaultTimeoutConfig(),
	)
	if err == nil {
		t.Fatal("Expected error for nonexistent command")
	}
	if !syserror.IsCode(err, syserror.ErrNotFound) && !syserror.IsCode(err, syserror.ErrSpawnFailed) {
		t.Fatalf("Expected ErrNotFound or ErrSpawnFailed, got %v", err)
	}
}

func TestTerminateTreeValidationAndExecution(t *testing.T) {
	if _, err := timeout.TerminateTree(0, nil); !syserror.IsCode(err, syserror.ErrInvalidArgument) {
		t.Fatalf("TerminateTree(0) = %v, want ErrInvalidArgument", err)
	}
	if _, err := timeout.TerminateTree(timeout.MaxSafePID+1, nil); !syserror.IsCode(err, syserror.ErrInvalidArgument) {
		t.Fatalf("TerminateTree(MaxSafePID+1) = %v, want ErrInvalidArgument", err)
	}
	if _, err := timeout.TerminateTree(1, &timeout.TerminateTreeConfig{
		SchemaID: "https://schemas.3leaps.dev/sysprims/process/v9.9.9/terminate-tree-config.schema.json",
	}); !syserror.IsCode(err, syserror.ErrInvalidArgument) {
		t.Fatalf("bad schema_id = %v, want ErrInvalidArgument", err)
	}
	if _, err := timeout.TerminateTree(1, &timeout.TerminateTreeConfig{
		Signal: int32Ptr(0),
	}); !syserror.IsCode(err, syserror.ErrInvalidArgument) {
		t.Fatalf("signal 0 = %v, want ErrInvalidArgument", err)
	}

	if runtime.GOOS != "windows" {
		cmd := exec.Command("sleep", "30")
		if err := cmd.Start(); err != nil {
			t.Fatalf("spawn sleep: %v", err)
		}
		pid := uint32(cmd.Process.Pid) // #nosec G115 -- test process PID
		go func() {
			_ = cmd.Wait()
		}()

		res, err := timeout.TerminateTree(pid, &timeout.TerminateTreeConfig{
			GraceTimeoutMS: uint64Ptr(100),
			KillTimeoutMS:  uint64Ptr(500),
		})
		if err != nil {
			t.Fatalf("TerminateTree(%d): %v", pid, err)
		}
		if !res.Exited || res.TimedOut {
			t.Fatalf("TerminateTree result = %+v, want Exited=true TimedOut=false", res)
		}
		if res.TreeKillReliability != timeout.ReliabilityBestEffort {
			t.Fatalf("TreeKillReliability = %q, want best_effort", res.TreeKillReliability)
		}
	}
}

func TestSpawnContainedRejectsEmptyArgvAndSignalZero(t *testing.T) {
	_, err := timeout.SpawnContained(timeout.SpawnContainedConfig{})
	if !syserror.IsCode(err, syserror.ErrInvalidArgument) {
		t.Fatalf("empty argv should fail with ErrInvalidArgument, got %v", err)
	}
	zero := int32(0)
	_, err = timeout.SpawnContained(timeout.SpawnContainedConfig{
		Argv:   []string{"true"},
		Signal: &zero,
	})
	if !syserror.IsCode(err, syserror.ErrInvalidArgument) {
		t.Fatalf("signal 0 should fail with ErrInvalidArgument, got %v", err)
	}
}

func assertWindowsSpawnContainedUnsupported(t *testing.T) {
	t.Helper()
	marker := filepath.Join(t.TempDir(), "should-not-exist")
	_, err := timeout.SpawnContained(timeout.SpawnContainedConfig{
		Argv: []string{"cmd", "/C", "echo spawned>\"" + marker + "\""},
	})
	if _, statErr := os.Stat(marker); !os.IsNotExist(statErr) {
		t.Fatalf("spawn marker: %v", statErr)
	}
	if !syserror.IsCode(err, syserror.ErrNotSupported) {
		t.Fatalf("expected ErrNotSupported, got %v", err)
	}
}

func TestSpawnContainedInvalidForeignTokensFailClosed(t *testing.T) {
	if runtime.GOOS == "windows" {
		assertWindowsSpawnContainedUnsupported(t)
		return
	}
	handle := spawnTrue(t)
	if err := handle.Close(); err != nil {
		t.Fatal(err)
	}
	if err := handle.Close(); err != nil {
		t.Fatalf("language dispose-twice must be a no-op: %v", err)
	}
	if _, err := handle.Identity(); err == nil {
		t.Fatal("closed handle identity must fail closed")
	}
}

func TestSpawnContainedHappyPathAndStaleReuse(t *testing.T) {
	if runtime.GOOS == "windows" {
		assertWindowsSpawnContainedUnsupported(t)
		return
	}
	first := spawnTrue(t)
	snap, err := first.Wait(5 * time.Second)
	if err != nil {
		t.Fatal(err)
	}
	if snap.LeaderStatus != "completed" {
		t.Fatalf("leader_status=%q", snap.LeaderStatus)
	}
	if snap.TreeKillReliability != "guaranteed" {
		t.Fatalf("reliability=%q", snap.TreeKillReliability)
	}
	if snap.BoundaryStrength != "cooperative_group" {
		t.Fatalf("boundary=%q", snap.BoundaryStrength)
	}
	if snap.Completion == nil || (snap.Completion.Status != "empty" && snap.Completion.Status != "unknown") {
		t.Fatalf("completion=%+v", snap.Completion)
	}
	if err := first.Close(); err != nil {
		t.Fatal(err)
	}

	second := spawnTrue(t)
	if _, err := first.Poll(); err == nil {
		t.Fatal("stale token after slot reuse must fail closed")
	}
	if err := second.Close(); err != nil {
		t.Fatal(err)
	}
}

func TestSpawnContainedTerminateKeepsSpawnReliability(t *testing.T) {
	if runtime.GOOS == "windows" {
		assertWindowsSpawnContainedUnsupported(t)
		return
	}
	handle := spawnSleep(t)
	defer func() { _ = handle.Close() }()
	before, err := handle.Identity()
	if err != nil {
		t.Fatal(err)
	}
	snap, err := handle.Terminate()
	if err != nil {
		t.Fatal(err)
	}
	if snap.LeaderStatus != "terminated" {
		t.Fatalf("leader_status=%q", snap.LeaderStatus)
	}
	if snap.TreeKillReliability != before.TreeKillReliability {
		t.Fatalf("reliability mutated after terminate")
	}
	again, err := handle.Terminate()
	if err != nil {
		t.Fatal(err)
	}
	if again.HandleState != "inert" {
		t.Fatalf("second terminate should be inert, got %q", again.HandleState)
	}
}

func TestSpawnContainedCloseRacesWaitAndTerminate(t *testing.T) {
	if runtime.GOOS == "windows" {
		assertWindowsSpawnContainedUnsupported(t)
		return
	}
	handle := spawnSleep(t)
	var wg sync.WaitGroup
	var closeErr, waitErr, termErr error
	var waited, terminated *timeout.ContainmentSnapshot
	wg.Add(3)
	go func() {
		defer wg.Done()
		waited, waitErr = handle.Wait(2 * time.Second)
	}()
	go func() {
		defer wg.Done()
		terminated, termErr = handle.Terminate()
	}()
	go func() {
		defer wg.Done()
		time.Sleep(20 * time.Millisecond)
		closeErr = handle.Close()
	}()
	wg.Wait()
	for name, result := range map[string]struct {
		snapshot *timeout.ContainmentSnapshot
		err      error
	}{
		"wait": {waited, waitErr}, "terminate": {terminated, termErr},
	} {
		if result.err != nil {
			if !syserror.IsCode(result.err, syserror.ErrInvalidArgument) {
				t.Fatalf("%s unexpected error: %v", name, result.err)
			}
		} else if result.snapshot == nil || result.snapshot.HandleState != "inert" || result.snapshot.LeaderStatus != "terminated" {
			t.Fatalf("%s unexpected successful outcome: %+v", name, result.snapshot)
		}
	}
	if waited != nil && terminated != nil && waited.LeaderStatus != terminated.LeaderStatus {
		t.Fatal("racing methods returned different terminal outcomes")
	}

	if closeErr != nil {
		if _, err := handle.Identity(); err != nil {
			t.Fatalf("failed close must preserve owner (wait=%v terminate=%v identity=%v)", waitErr, termErr, err)
		}
		if err := handle.Close(); err != nil {
			t.Fatalf("retry close after failure: %v", err)
		}
	}
	if err := handle.Close(); err != nil {
		t.Fatalf("final close: %v", err)
	}
	if _, err := handle.Identity(); err == nil {
		t.Fatal("closed handle identity must fail")
	}
}

func TestSpawnContainedWaitRejectsNegativeAndKeepsSubMillisecondFinite(t *testing.T) {
	if runtime.GOOS == "windows" {
		assertWindowsSpawnContainedUnsupported(t)
		return
	}
	handle := spawnSleep(t)
	defer func() { _ = handle.Close() }()
	if _, err := handle.Wait(-time.Second); err == nil {
		t.Fatal("negative wait must fail")
	}
	started := time.Now()
	if _, err := handle.Wait(500 * time.Microsecond); err != nil {
		t.Fatalf("positive sub-ms wait must stay finite: %v", err)
	}
	if time.Since(started) > 3*time.Second {
		t.Fatal("positive sub-ms wait was treated as infinite")
	}
	if _, err := handle.Terminate(); err != nil {
		t.Fatalf("terminate after sub-ms wait: %v", err)
	}
}

func TestSpawnContainedCloseFailureRestoresToken(t *testing.T) {
	if runtime.GOOS == "windows" {
		assertWindowsSpawnContainedUnsupported(t)
		return
	}
	handle := spawnSleep(t)
	timeout.SetNativeCloseForTest(func(uint64) error {
		return syserror.InvalidArgument("forced close failure")
	})
	err := handle.Close()
	timeout.SetNativeCloseForTest(nil)
	if err == nil {
		t.Fatal("forced native close failure must surface")
	}
	if _, idErr := handle.Identity(); idErr != nil {
		_ = handle.Close()
		t.Fatalf("failed close must keep the token usable: %v", idErr)
	}
	if err := handle.Close(); err != nil {
		t.Fatalf("retry close: %v", err)
	}
}

func TestSpawnContainedInvalidHighSignalDoesNotSpawn(t *testing.T) {
	marker := filepath.Join(t.TempDir(), "should-not-exist")
	sig := int32(99)
	_, err := timeout.SpawnContained(timeout.SpawnContainedConfig{
		Argv:   []string{"touch", marker},
		Signal: &sig,
	})
	if err == nil {
		t.Fatal("signal 99 must fail before spawn")
	}
	if _, statErr := os.Stat(marker); statErr == nil {
		t.Fatal("invalid high signal must not start argv")
	}
}

func TestContainedNativeExecutionDeadline(t *testing.T) {
	if runtime.GOOS == "windows" {
		assertWindowsSpawnContainedUnsupported(t)
		return
	}
	for _, test := range []struct {
		name   string
		argv   []string
		status string
	}{
		{"running", []string{"sleep", "30"}, "timed_out"},
		{"fast", []string{"true"}, "completed"},
	} {
		t.Run(test.name, func(t *testing.T) {
			h, err := timeout.SpawnContained(timeout.SpawnContainedConfig{
				Argv:               test.argv,
				ExecutionTimeoutMS: uint64Ptr(200),
				GraceTimeoutMS:     uint64Ptr(20),
				KillTimeoutMS:      uint64Ptr(200),
			})
			if err != nil {
				t.Fatal(err)
			}
			defer func() { _ = h.Close() }()
			time.Sleep(450 * time.Millisecond)
			observed, err := h.Identity()
			deadline := time.Now().Add(3 * time.Second)
			for err == nil && observed.HandleState != "inert" && time.Now().Before(deadline) {
				time.Sleep(10 * time.Millisecond)
				observed, err = h.Identity()
			}
			if err != nil || observed.HandleState != "inert" {
				t.Fatalf("native monitor did not finalize: %+v %v", observed, err)
			}
			result, err := h.Wait(time.Second)
			if err != nil || result.LeaderStatus != test.status {
				t.Fatalf("delayed wait: %+v %v", result, err)
			}
			if test.status == "completed" && (result.TimedOut == nil || *result.TimedOut) {
				t.Fatal("natural completion reported cleanup timeout")
			}
		})
	}
}

func TestContainedBoundedWaitDuringTerminate(t *testing.T) {
	if runtime.GOOS == "windows" {
		assertWindowsSpawnContainedUnsupported(t)
		return
	}
	h, err := timeout.SpawnContained(timeout.SpawnContainedConfig{
		Argv:           []string{"sleep", "30"},
		GraceTimeoutMS: uint64Ptr(800),
		KillTimeoutMS:  uint64Ptr(200),
	})
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = h.Close() }()
	done := make(chan error, 1)
	go func() { _, err := h.Terminate(); done <- err }()
	time.Sleep(100 * time.Millisecond)
	started := time.Now()
	result, waitErr := h.Wait(10 * time.Millisecond)
	elapsed := time.Since(started)
	termErr := <-done
	if termErr != nil {
		t.Fatal(termErr)
	}
	if waitErr != nil || result.HandleState != "active" || result.LeaderStatus != "running" {
		t.Fatalf("wait: %+v %v", result, waitErr)
	}
	if elapsed > 300*time.Millisecond {
		t.Fatalf("10ms wait took %v", elapsed)
	}
}

func TestContainedFinalizerAfterExplicitCloseDoesNotCloseAgain(t *testing.T) {
	if runtime.GOOS == "windows" {
		assertWindowsSpawnContainedUnsupported(t)
		return
	}
	h := spawnTrue(t)
	if err := h.Close(); err != nil {
		t.Fatal(err)
	}
	calls := 0
	timeout.SetNativeCloseForTest(func(uint64) error { calls++; return nil })
	defer timeout.SetNativeCloseForTest(nil)
	timeout.RunContainedFinalizerForTest(h)
	if err := h.Close(); err != nil {
		t.Fatal(err)
	}
	if calls != 0 {
		t.Fatalf("native close called %d times after disposal", calls)
	}
}

//go:noinline
func callWithLastReference(t *testing.T, method string, finalized *atomic.Bool) error {
	h := spawnSleep(t)
	timeout.ObserveFinalizerForTest(h, func() { finalized.Store(true) })
	var err error
	switch method {
	case "identity":
		_, err = h.Identity()
	case "poll":
		_, err = h.Poll()
	case "wait":
		_, err = h.Wait(time.Millisecond)
	case "terminate":
		_, err = h.Terminate()
	}
	return err
}

func TestContainedMethodsKeepReceiverAlive(t *testing.T) {
	if runtime.GOOS == "windows" {
		assertWindowsSpawnContainedUnsupported(t)
		return
	}
	for _, method := range []string{"identity", "poll", "wait", "terminate"} {
		t.Run(method, func(t *testing.T) {
			var finalized atomic.Bool
			var premature bool
			timeout.SetSnapshotHookForTest(func() {
				for i := 0; i < 5; i++ {
					runtime.GC()
					time.Sleep(10 * time.Millisecond)
				}
				premature = finalized.Load()
			})
			err := callWithLastReference(t, method, &finalized)
			timeout.SetSnapshotHookForTest(nil)
			deadline := time.Now().Add(3 * time.Second)
			for !finalized.Load() && time.Now().Before(deadline) {
				runtime.GC()
				time.Sleep(10 * time.Millisecond)
			}
			if premature {
				t.Fatal("receiver finalized during native method")
			}
			if err != nil {
				t.Fatal(err)
			}
			if !finalized.Load() {
				t.Fatal("fixture did not become collectible after method")
			}
		})
	}
}

func TestTimeoutConformanceVectors(t *testing.T) {
	cmd := exec.Command("go", "run", "./internal/difftool")
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("timeout difftool failed: %v\n%s", err, string(out))
	}
	lines := strings.Split(strings.TrimSpace(string(out)), "\n")
	if len(lines) != 24 {
		t.Fatalf("expected 24 NDJSON lines, got %d:\n%s", len(lines), string(out))
	}
}
