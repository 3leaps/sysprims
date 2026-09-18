package sysprims_test

import (
	"os"
	"path/filepath"
	"runtime"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/3leaps/sysprims/bindings/go/sysprims"
)

func spawnTrue(t *testing.T) *sysprims.ContainedProcess {
	t.Helper()
	handle, err := sysprims.SpawnContained(sysprims.SpawnContainedConfig{
		Argv:           []string{"true"},
		GraceTimeoutMS: uint64Ptr(50),
		KillTimeoutMS:  uint64Ptr(200),
	})
	if err != nil {
		t.Fatalf("SpawnContained(true): %v", err)
	}
	return handle
}

func spawnSleep(t *testing.T) *sysprims.ContainedProcess {
	t.Helper()
	handle, err := sysprims.SpawnContained(sysprims.SpawnContainedConfig{
		Argv:           []string{"sleep", "30"},
		GraceTimeoutMS: uint64Ptr(50),
		KillTimeoutMS:  uint64Ptr(500),
	})
	if err != nil {
		t.Fatalf("SpawnContained(sleep): %v", err)
	}
	return handle
}

func uint64Ptr(v uint64) *uint64 { return &v }

func TestSpawnContainedRejectsEmptyArgvAndSignalZero(t *testing.T) {
	_, err := sysprims.SpawnContained(sysprims.SpawnContainedConfig{})
	if err == nil {
		t.Fatal("empty argv should fail")
	}
	zero := int32(0)
	_, err = sysprims.SpawnContained(sysprims.SpawnContainedConfig{
		Argv:   []string{"true"},
		Signal: &zero,
	})
	if err == nil {
		t.Fatal("signal 0 should fail")
	}
}

func TestSpawnContainedInvalidForeignTokensFailClosed(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("managed spawn is unix-only until Job assignment exists")
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
		t.Skip("managed spawn is unix-only until Job assignment exists")
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
		t.Skip("managed spawn is unix-only until Job assignment exists")
	}
	handle := spawnSleep(t)
	defer handle.Close()
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
		t.Skip("managed spawn is unix-only until Job assignment exists")
	}
	handle := spawnSleep(t)
	var wg sync.WaitGroup
	var closeErr, waitErr, termErr error
	var waited, terminated *sysprims.ContainmentSnapshot
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
		snapshot *sysprims.ContainmentSnapshot
		err      error
	}{
		"wait": {waited, waitErr}, "terminate": {terminated, termErr},
	} {
		if result.err != nil {
			nativeErr, ok := result.err.(*sysprims.Error)
			if !ok || nativeErr.Code != sysprims.ErrInvalidArgument {
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
		t.Skip("managed spawn is unix-only until Job assignment exists")
	}
	handle := spawnSleep(t)
	defer handle.Close()
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
		t.Skip("managed spawn is unix-only until Job assignment exists")
	}
	handle := spawnSleep(t)
	sysprims.SetNativeCloseForTest(func(uint64) error {
		return &sysprims.Error{Code: sysprims.ErrInvalidArgument, Message: "forced close failure"}
	})
	err := handle.Close()
	sysprims.SetNativeCloseForTest(nil)
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
	if runtime.GOOS == "windows" {
		t.Skip("managed spawn is unix-only until Job assignment exists")
	}
	marker := t.TempDir() + "/should-not-exist"
	sig := int32(99)
	_, err := sysprims.SpawnContained(sysprims.SpawnContainedConfig{
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

func TestSpawnContainedWindowsUnsupported(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("windows-only rejection fixture")
	}
	marker := filepath.Join(t.TempDir(), "should-not-exist")
	_, err := sysprims.SpawnContained(sysprims.SpawnContainedConfig{
		Argv: []string{"cmd", "/C", "echo spawned>\"" + marker + "\""},
	})
	if _, statErr := os.Stat(marker); !os.IsNotExist(statErr) {
		t.Fatalf("spawn marker: %v", statErr)
	}
	if err == nil {
		t.Fatal("windows managed spawn must fail before spawn")
	}
	sErr, ok := err.(*sysprims.Error)
	if !ok || sErr.Code != sysprims.ErrNotSupported {
		t.Fatalf("expected ErrNotSupported, got %v", err)
	}
}

func TestContainedNativeExecutionDeadline(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Unix containment")
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
			h, err := sysprims.SpawnContained(sysprims.SpawnContainedConfig{Argv: test.argv, ExecutionTimeoutMS: uint64Ptr(200), GraceTimeoutMS: uint64Ptr(20), KillTimeoutMS: uint64Ptr(200)})
			if err != nil {
				t.Fatal(err)
			}
			defer h.Close()
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
		t.Skip("Unix containment")
	}
	h, err := sysprims.SpawnContained(sysprims.SpawnContainedConfig{Argv: []string{"sleep", "30"}, GraceTimeoutMS: uint64Ptr(800), KillTimeoutMS: uint64Ptr(200)})
	if err != nil {
		t.Fatal(err)
	}
	defer h.Close()
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
		t.Skip("Unix containment")
	}
	h := spawnTrue(t)
	if err := h.Close(); err != nil {
		t.Fatal(err)
	}
	calls := 0
	sysprims.SetNativeCloseForTest(func(uint64) error { calls++; return nil })
	defer sysprims.SetNativeCloseForTest(nil)
	sysprims.RunContainedFinalizerForTest(h)
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
	sysprims.ObserveFinalizerForTest(h, func() { finalized.Store(true) })
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
		t.Skip("Unix containment")
	}
	for _, method := range []string{"identity", "poll", "wait", "terminate"} {
		t.Run(method, func(t *testing.T) {
			var finalized atomic.Bool
			var premature bool
			sysprims.SetSnapshotHookForTest(func() {
				for i := 0; i < 5; i++ {
					runtime.GC()
					time.Sleep(10 * time.Millisecond)
				}
				premature = finalized.Load()
			})
			err := callWithLastReference(t, method, &finalized)
			sysprims.SetSnapshotHookForTest(nil)
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
