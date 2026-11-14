package proc

import (
	"encoding/binary"
	"errors"
	"math"
	"net"
	"os"
	"os/exec"
	"runtime"
	"slices"
	"syscall"
	"testing"
	"time"

	"github.com/3leaps/sysprims/signal"
	"github.com/3leaps/sysprims/syserror"
)

func codeOf(err error) syserror.ErrorCode {
	code, _ := syserror.CodeOf(err)
	return code
}

func TestValidatePID(t *testing.T) {
	if err := ValidatePID(0); codeOf(err) != syserror.ErrInvalidArgument {
		t.Fatalf("ValidatePID(0) = %v, want ErrInvalidArgument", err)
	}
	if err := ValidatePID(uint32(math.MaxInt32) + 1); codeOf(err) != syserror.ErrInvalidArgument {
		t.Fatalf("ValidatePID(MaxInt32+1) = %v, want ErrInvalidArgument", err)
	}
	if err := ValidatePID(1); err != nil {
		t.Fatalf("ValidatePID(1) = %v, want nil", err)
	}
}

func TestGetProcessSelfAndOptions(t *testing.T) {
	selfPID := uint32(os.Getpid())
	info, err := GetProcess(selfPID)
	if err != nil {
		t.Fatalf("GetProcess(%d) failed: %v", selfPID, err)
	}
	if info.PID != selfPID {
		t.Fatalf("PID = %d, want %d", info.PID, selfPID)
	}
	if info.Name == "" {
		t.Fatal("Name should not be empty")
	}
	if info.Cmdline == nil {
		t.Fatal("Cmdline should be non-nil slice")
	}

	withOpts, err := GetProcessWithOptions(selfPID, &ProcessOptions{
		IncludeEnv:     true,
		IncludeThreads: true,
	})
	if err != nil {
		t.Fatalf("GetProcessWithOptions(%d) failed: %v", selfPID, err)
	}
	if withOpts.ThreadCount != nil && *withOpts.ThreadCount == 0 {
		t.Fatalf("ThreadCount should be > 0 when present, got 0")
	}
	if runtime.GOOS != "windows" && len(withOpts.Env) == 0 {
		t.Logf("note: self env empty or restricted in this environment")
	}
}

func TestGetProcessInvalidAndNonexistent(t *testing.T) {
	if _, err := GetProcess(0); codeOf(err) != syserror.ErrInvalidArgument {
		t.Fatalf("GetProcess(0) = %v, want ErrInvalidArgument", err)
	}
	if _, err := GetProcess(uint32(math.MaxInt32) + 10); codeOf(err) != syserror.ErrInvalidArgument {
		t.Fatalf("GetProcess(MaxInt32+10) = %v, want ErrInvalidArgument", err)
	}
	if _, err := GetProcess(99999999); err != nil && codeOf(err) != syserror.ErrNotFound {
		t.Fatalf("GetProcess(99999999) = %v, want ErrNotFound", err)
	}
}

func TestSnapshotAndFiltering(t *testing.T) {
	snap, err := Snapshot()
	if err != nil {
		t.Fatalf("Snapshot() failed: %v", err)
	}
	if snap.SchemaID != ProcessInfoSchemaID {
		t.Fatalf("SchemaID = %q, want %q", snap.SchemaID, ProcessInfoSchemaID)
	}
	if len(snap.Processes) == 0 {
		t.Fatal("Snapshot returned 0 processes")
	}

	selfPID := uint32(os.Getpid())
	filtered, err := SnapshotFiltered(&ProcessFilter{
		PIDIn: []uint32{selfPID},
	})
	if err != nil {
		t.Fatalf("SnapshotFiltered failed: %v", err)
	}
	if len(filtered.Processes) != 1 || filtered.Processes[0].PID != selfPID {
		t.Fatalf("SnapshotFiltered(self) = %+v, want single process %d", filtered.Processes, selfPID)
	}

	badCPU := 101.0
	if _, err := SnapshotFiltered(&ProcessFilter{CPUAbove: &badCPU}); codeOf(err) != syserror.ErrInvalidArgument {
		t.Fatalf("SnapshotFiltered(cpu_above=101) = %v, want ErrInvalidArgument", err)
	}
	nanCPU := math.NaN()
	if _, err := SnapshotFiltered(&ProcessFilter{CPUAbove: &nanCPU}); codeOf(err) != syserror.ErrInvalidArgument {
		t.Fatalf("SnapshotFiltered(cpu_above=NaN) = %v, want ErrInvalidArgument", err)
	}
	if _, err := SnapshotFiltered(&ProcessFilter{StateIn: []ProcessState{"bogus"}}); codeOf(err) != syserror.ErrInvalidArgument {
		t.Fatalf("SnapshotFiltered(state_in=bogus) = %v, want ErrInvalidArgument", err)
	}
}

func TestAncestorsSelf(t *testing.T) {
	selfPID := uint32(os.Getpid())
	res, err := Ancestors(selfPID, 10, nil)
	if err != nil {
		t.Fatalf("Ancestors(%d) failed: %v", selfPID, err)
	}
	if res.SchemaID != AncestorsResultSchemaID {
		t.Fatalf("SchemaID = %q, want %q", res.SchemaID, AncestorsResultSchemaID)
	}
	if res.PID != selfPID {
		t.Fatalf("PID = %d, want %d", res.PID, selfPID)
	}
	if len(res.Chain) == 0 || res.Chain[0].PID != selfPID {
		t.Fatalf("Ancestors chain = %+v, want first element PID %d", res.Chain, selfPID)
	}
}

func TestDescendantsAndSelectDescendantTargets(t *testing.T) {
	u1 := "alice"
	u2 := "bob"
	procs := []ProcessInfo{
		{PID: 100, PPID: 1, Name: "root-app", User: &u1, State: StateRunning, CPUPercent: 1.0, MemoryKB: 1024, ElapsedSeconds: 100, Cmdline: []string{"root-app"}},
		{PID: 200, PPID: 100, Name: "worker-a", User: &u1, State: StateRunning, CPUPercent: 25.0, MemoryKB: 4096, ElapsedSeconds: 50, Cmdline: []string{"worker-a"}},
		{PID: 201, PPID: 100, Name: "helper-b", User: &u2, State: StateSleeping, CPUPercent: 2.0, MemoryKB: 512, ElapsedSeconds: 20, Cmdline: []string{"helper-b"}},
		{PID: 300, PPID: 200, Name: "leaf-a1", User: &u1, State: StateRunning, CPUPercent: 5.0, MemoryKB: 256, ElapsedSeconds: 10, Cmdline: []string{"leaf-a1"}},
		{PID: 301, PPID: 201, Name: "leaf-b1", User: &u2, State: StateSleeping, CPUPercent: 0.5, MemoryKB: 128, ElapsedSeconds: 5, Cmdline: []string{"leaf-b1"}},
	}

	tree := buildDescendantsFromSnapshot(100, math.MaxUint32, nil, procs)
	if tree.TotalFound != 4 {
		t.Fatalf("TotalFound = %d, want 4", tree.TotalFound)
	}
	if len(tree.Levels) != 2 {
		t.Fatalf("len(Levels) = %d, want 2", len(tree.Levels))
	}

	nameNeedle := "worker"
	filter := &ProcessFilter{NameContains: &nameNeedle}

	nonCascade, err := SelectDescendantTargets(tree, filter, false)
	if err != nil {
		t.Fatalf("SelectDescendantTargets(cascade=false) failed: %v", err)
	}
	if !slices.Equal(nonCascade, []uint32{200}) {
		t.Fatalf("nonCascade = %v, want [200]", nonCascade)
	}

	withCascade, err := SelectDescendantTargets(tree, filter, true)
	if err != nil {
		t.Fatalf("SelectDescendantTargets(cascade=true) failed: %v", err)
	}
	if !slices.Equal(withCascade, []uint32{200, 300}) {
		t.Fatalf("withCascade = %v, want [200 300]", withCascade)
	}
}

func TestGuardStepAndRunner(t *testing.T) {
	selfPID := uint32(os.Getpid())
	maxLevels := uint32(1)
	ev, err := GuardStep(&GuardConfig{
		Rule: GuardRule{
			RootPID:   selfPID,
			MaxLevels: &maxLevels,
		},
		ActionEnabled: false,
		MaxTargets:    8,
	})
	if err != nil {
		t.Fatalf("GuardStep failed: %v", err)
	}
	if ev.SchemaID != GuardEventSchemaID {
		t.Fatalf("SchemaID = %q, want %q", ev.SchemaID, GuardEventSchemaID)
	}
	if ev.Targeted != 0 || ev.Killed != 0 || ev.Failed != 0 {
		t.Fatalf("unexpected counts with action disabled: %+v", ev)
	}

	if got := GuardPresetInteractive.Interval(); got != 3*time.Second {
		t.Fatalf("interactive interval = %v, want 3s", got)
	}
	if got := GuardPresetBackground.Interval(); got != 3*time.Minute {
		t.Fatalf("background interval = %v, want 3m", got)
	}
	if got := GuardPresetWatchdog.SampleDuration(); got != 5*time.Second {
		t.Fatalf("watchdog sample = %v, want 5s", got)
	}

	maxIter := uint64(1)
	runner, err := NewGuardRunner(&GuardRunnerConfig{
		Guard: GuardConfig{
			Rule: GuardRule{
				RootPID:   selfPID,
				MaxLevels: &maxLevels,
			},
			ActionEnabled: false,
			MaxTargets:    8,
		},
		Interval:      10 * time.Millisecond,
		MaxIterations: &maxIter,
	})
	if err != nil {
		t.Fatalf("NewGuardRunner failed: %v", err)
	}

	ev1, err := runner.Tick()
	if err != nil || ev1 == nil {
		t.Fatalf("first Tick() = (%v, %v), want non-nil event", ev1, err)
	}
	ev2, err := runner.Tick()
	if err != nil || ev2 != nil {
		t.Fatalf("second Tick() = (%v, %v), want (nil, nil) after MaxIterations", ev2, err)
	}
	runner.Close()
	if _, err := runner.Tick(); codeOf(err) != syserror.ErrInvalidArgument {
		t.Fatalf("Tick() after Close() = %v, want ErrInvalidArgument", err)
	}
}

func TestListFdsSelf(t *testing.T) {
	selfPID := uint32(os.Getpid())
	snap, err := ListFds(selfPID, nil)
	if runtime.GOOS == "windows" {
		if codeOf(err) != syserror.ErrNotSupported {
			t.Fatalf("ListFds on windows = %v, want ErrNotSupported", err)
		}
		return
	}
	if err != nil {
		t.Fatalf("ListFds(%d) failed: %v", selfPID, err)
	}
	if snap.SchemaID != FdSnapshotSchemaID {
		t.Fatalf("SchemaID = %q, want %q", snap.SchemaID, FdSnapshotSchemaID)
	}
	if snap.PID != selfPID {
		t.Fatalf("PID = %d, want %d", snap.PID, selfPID)
	}
	if len(snap.Fds) == 0 {
		t.Fatalf("ListFds returned 0 fds; warnings=%v", snap.Warnings)
	}

	// Open a temporary file and verify FdKindFile visibility
	tmpFile, err := os.CreateTemp(t.TempDir(), "sysprims-fd-test-*")
	if err != nil {
		t.Fatalf("CreateTemp failed: %v", err)
	}
	defer func() { _ = tmpFile.Close() }()

	fileKind := FdKindFile
	fileSnap, err := ListFds(selfPID, &FdFilter{Kind: &fileKind})
	if err != nil {
		t.Fatalf("ListFds(file filter) failed: %v", err)
	}
	if len(fileSnap.Fds) == 0 {
		t.Fatalf("expected at least 1 file FD for self")
	}

	// Unlink the temporary file while keeping tmpFile open and verify IsDeleted()
	if err := os.Remove(tmpFile.Name()); err != nil {
		t.Fatalf("os.Remove(%s) failed: %v", tmpFile.Name(), err)
	}
	deletedSnap, err := ListFds(selfPID, &FdFilter{Kind: &fileKind})
	if err != nil {
		t.Fatalf("ListFds(after unlink) failed: %v", err)
	}
	foundDeleted := false
	for _, fd := range deletedSnap.Fds {
		if fd.Fd == uint32(tmpFile.Fd()) && fd.IsDeleted() {
			foundDeleted = true
			break
		}
	}
	if !foundDeleted {
		t.Fatalf("expected open unlinked file fd=%d to report IsDeleted()==true; fds=%+v", tmpFile.Fd(), deletedSnap.Fds)
	}

	badKind := FdKind("invalid")
	if _, err := ListFds(selfPID, &FdFilter{Kind: &badKind}); codeOf(err) != syserror.ErrInvalidArgument {
		t.Fatalf("ListFds(invalid kind) = %v, want ErrInvalidArgument", err)
	}
}

func TestListeningPortsSelfListener(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		var opErr *net.OpError
		if errors.As(err, &opErr) && (errors.Is(opErr.Err, syscall.EPERM) || errors.Is(opErr.Err, syscall.EACCES)) {
			t.Skipf("net.Listen denied in this environment: %v", err)
		}
		t.Fatalf("net.Listen failed: %v", err)
	}
	defer func() { _ = listener.Close() }()

	addr := listener.Addr().(*net.TCPAddr)
	port := uint16(addr.Port)
	selfPID := uint32(os.Getpid())

	proto := ProtocolTCP
	filter := &PortFilter{Protocol: &proto, LocalPort: &port}

	snap, err := ListeningPorts(filter)
	if err != nil {
		if codeOf(err) == syserror.ErrPermissionDenied || codeOf(err) == syserror.ErrNotSupported {
			t.Skipf("ListeningPorts restricted in this environment: %v", err)
		}
		t.Fatalf("ListeningPorts failed: %v", err)
	}
	if snap.SchemaID != PortBindingsSchemaID {
		t.Fatalf("SchemaID = %q, want %q", snap.SchemaID, PortBindingsSchemaID)
	}

	found := false
	for _, b := range snap.Bindings {
		if b.LocalPort == port && b.PID != nil && *b.PID == selfPID {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("expected to find self listener pid=%d port=%d; bindings=%+v warnings=%v", selfPID, port, snap.Bindings, snap.Warnings)
	}

	procByPort, err := ProcessByPort(port, &proto)
	if err != nil {
		t.Fatalf("ProcessByPort(%d) failed: %v", port, err)
	}
	if procByPort.PID != selfPID {
		t.Fatalf("ProcessByPort(%d).PID = %d, want %d", port, procByPort.PID, selfPID)
	}

	zeroPort := uint16(0)
	if _, err := ListeningPorts(&PortFilter{LocalPort: &zeroPort}); codeOf(err) != syserror.ErrInvalidArgument {
		t.Fatalf("ListeningPorts(port=0) = %v, want ErrInvalidArgument", err)
	}
}

func TestWaitPIDAndKillDescendantsLiveChild(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("skipping unix sleep child test on windows")
	}
	cmd := exec.Command("sleep", "30")
	if err := cmd.Start(); err != nil {
		t.Fatalf("failed to start sleep child: %v", err)
	}
	childPID := uint32(cmd.Process.Pid)
	reaped := make(chan struct{})
	go func() {
		_ = cmd.Wait()
		close(reaped)
	}()
	defer func() {
		_ = cmd.Process.Kill()
		<-reaped
	}()

	if !IsLive(childPID) {
		t.Fatalf("expected child PID %d to be live", childPID)
	}

	selfPID := uint32(os.Getpid())
	desc, err := Descendants(selfPID, 1, &ProcessFilter{PIDIn: []uint32{childPID}})
	if err != nil {
		t.Fatalf("Descendants failed: %v", err)
	}
	if desc.MatchedByFilter != 1 {
		t.Fatalf("MatchedByFilter = %d, want 1", desc.MatchedByFilter)
	}

	killRes, err := KillDescendants(selfPID, signal.SIGKILL, 1, &ProcessFilter{PIDIn: []uint32{childPID}})
	if err != nil {
		t.Fatalf("KillDescendants failed: %v", err)
	}
	if !slices.Contains(killRes.Succeeded, childPID) {
		t.Fatalf("KillDescendants succeeded = %v, want %d", killRes.Succeeded, childPID)
	}

	<-reaped
	waitRes, err := WaitPID(childPID, 500*time.Millisecond)
	if err != nil && codeOf(err) != syserror.ErrNotFound {
		t.Fatalf("WaitPID after reap failed: %v", err)
	}
	if err == nil && !waitRes.Exited {
		t.Fatalf("expected WaitPID to report exited, got %+v", waitRes)
	}
}

func TestCrossPlatformParsers(t *testing.T) {
	// 1. Linux /proc/[pid]/stat with spaces and parens inside comm
	statLine := "1234 (my (weird) proc) S 1 1234 1234 0 -1 4194304 100 0 0 0 150 50 0 0 20 0 4 0 98765 12345678 1024 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 0 0 0 0 0 0"
	parsedStat, err := parseLinuxStat(statLine)
	if err != nil {
		t.Fatalf("parseLinuxStat failed: %v", err)
	}
	if parsedStat.Name != "my (weird) proc" || parsedStat.State != StateSleeping || parsedStat.PPID != 1 || parsedStat.UTimeTicks != 150 || parsedStat.STimeTicks != 50 || parsedStat.StartTicks != 98765 {
		t.Fatalf("parsedStat = %+v", parsedStat)
	}

	// 2. Linux /proc/[pid]/status
	statusText := "Name:\tmyproc\nUid:\t1000\t1000\t1000\t1000\nThreads:\t7\n"
	uid, hasUID, threads, hasThreads := parseLinuxStatus(statusText)
	if !hasUID || uid != 1000 || !hasThreads || threads != 7 {
		t.Fatalf("parseLinuxStatus = (%d, %v, %d, %v)", uid, hasUID, threads, hasThreads)
	}

	// 3. Linux /proc/net/tcp
	netTCP := "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n" +
		"   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 54321 1 0000000000000000 100 0 0 10 0\n"
	entries, errs := parseProcNetTable(netTCP, ProtocolTCP, false)
	if errs != 0 || len(entries) != 1 {
		t.Fatalf("parseProcNetTable = (%+v, %d)", entries, errs)
	}
	if entries[0].LocalPort != 8080 || entries[0].LocalAddr == nil || *entries[0].LocalAddr != "127.0.0.1" || entries[0].Inode != 54321 {
		t.Fatalf("entries[0] = %+v", entries[0])
	}

	// 4. Darwin kern.procargs2
	var procargs []byte
	argcBuf := make([]byte, 4)
	binary.LittleEndian.PutUint32(argcBuf, 2)
	procargs = append(procargs, argcBuf...)
	procargs = append(procargs, []byte("/usr/bin/sysprims\x00\x00/usr/bin/sysprims\x00guard\x00FOO=bar\x00BAZ=qux\x00")...)
	cmdline := parseDarwinProcargsCmdline(procargs)
	if !slices.Equal(cmdline, []string{"/usr/bin/sysprims", "guard"}) {
		t.Fatalf("parseDarwinProcargsCmdline = %v", cmdline)
	}
	env := parseDarwinProcargsEnv(procargs)
	if env["FOO"] != "bar" || env["BAZ"] != "qux" {
		t.Fatalf("parseDarwinProcargsEnv = %v", env)
	}

	// 5. guardDiscoveryName
	if got := guardDiscoveryName([]string{"/usr/local/bin/sysprims", "guard", "4242"}, "sysprims"); got != "sysprims-guard:4242" {
		t.Fatalf("guardDiscoveryName = %q, want sysprims-guard:4242", got)
	}

	// 6. FdInfo.IsDeleted
	deletedPath := "/tmp/foo.log (deleted)"
	fdInfo := FdInfo{Fd: 3, Kind: FdKindFile, Path: &deletedPath}
	if !fdInfo.IsDeleted() {
		t.Fatal("expected IsDeleted() == true")
	}
	// 7. Additional cross-platform Linux parser helpers
	if got := parseLinuxStatmRSSPages("4096 512 256 1 0 100 0\n"); got != 512 {
		t.Fatalf("parseLinuxStatmRSSPages = %d, want 512", got)
	}
	if btime, ok := parseLinuxBootTimeSecs("cpu  1 2 3\nbtime 1700000000\nprocesses 42\n"); !ok || btime != 1700000000 {
		t.Fatalf("parseLinuxBootTimeSecs = (%d, %v)", btime, ok)
	}
	if got := parseNulSeparatedStrings([]byte("/bin/sh\x00-c\x00echo hi\x00")); !slices.Equal(got, []string{"/bin/sh", "-c", "echo hi"}) {
		t.Fatalf("parseNulSeparatedStrings = %v", got)
	}
	if envMap := parseNulSeparatedEnv([]byte("A=1\x00B=two\x00")); envMap["A"] != "1" || envMap["B"] != "two" {
		t.Fatalf("parseNulSeparatedEnv = %v", envMap)
	}
	if ino, ok := parseLinuxSocketInode("socket:[98765]"); !ok || ino != 98765 {
		t.Fatalf("parseLinuxSocketInode = (%d, %v)", ino, ok)
	}
}
