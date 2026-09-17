package sysprims_test

import (
	"runtime"
	"sync"
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
	wg.Add(3)
	go func() {
		defer wg.Done()
		_, _ = handle.Wait(2 * time.Second)
	}()
	go func() {
		defer wg.Done()
		_, _ = handle.Terminate()
	}()
	go func() {
		defer wg.Done()
		time.Sleep(20 * time.Millisecond)
		_ = handle.Close()
	}()
	wg.Wait()
	_ = handle.Close()
}

func TestSpawnContainedWindowsUnsupported(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("windows-only rejection fixture")
	}
	_, err := sysprims.SpawnContained(sysprims.SpawnContainedConfig{
		Argv: []string{"cmd", "/C", "echo should-not-run"},
	})
	if err == nil {
		t.Fatal("windows managed spawn must fail before spawn")
	}
	sErr, ok := err.(*sysprims.Error)
	if !ok || sErr.Code != sysprims.ErrNotSupported {
		t.Fatalf("expected ErrNotSupported, got %v", err)
	}
}
