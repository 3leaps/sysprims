package sysprims_test

import (
	"errors"
	"net"
	"os"
	"runtime"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/3leaps/sysprims/bindings/go/sysprims"
)

// TestVersion verifies that Version returns a non-empty semver string.
func TestVersion(t *testing.T) {
	v := sysprims.Version()
	if v == "" {
		t.Error("Version() returned empty string")
	}
	// Version should be semver format (contains at least one dot)
	if len(v) < 3 {
		t.Errorf("Version() returned unexpectedly short string: %q", v)
	}
	t.Logf("Version: %s", v)
}

// TestABIVersion verifies that ABIVersion returns a positive number.
func TestABIVersion(t *testing.T) {
	abi := sysprims.ABIVersion()
	if abi == 0 {
		t.Error("ABIVersion() returned 0")
	}
	t.Logf("ABI Version: %d", abi)
}

// TestPlatform verifies that Platform returns a valid platform name.
func TestPlatform(t *testing.T) {
	p := sysprims.Platform()
	if p == "" {
		t.Error("Platform() returned empty string")
	}

	// Verify it matches the Go runtime
	expected := runtime.GOOS
	if expected == "darwin" {
		expected = "macos"
	}
	if p != expected {
		t.Errorf("Platform() = %q, expected %q", p, expected)
	}
	t.Logf("Platform: %s", p)
}

// TestKillInvalidPID verifies that Kill rejects invalid PIDs.
func TestKillInvalidPID(t *testing.T) {
	// PID 0 should be rejected
	err := sysprims.Kill(0, sysprims.SIGTERM)
	if err == nil {
		t.Error("Kill(0, SIGTERM) should return error")
	}

	sErr, ok := err.(*sysprims.Error)
	if !ok {
		t.Errorf("Expected *sysprims.Error, got %T", err)
		return
	}
	if sErr.Code != sysprims.ErrInvalidArgument {
		t.Errorf("Expected ErrInvalidArgument, got %d (%s)", sErr.Code, sErr.Code)
	}
}

// TestKillNonexistentPID verifies that Kill returns appropriate error for nonexistent PIDs.
func TestKillNonexistentPID(t *testing.T) {
	// Use a very high PID that shouldn't exist
	err := sysprims.Kill(99999, sysprims.SIGTERM)
	if err == nil {
		t.Skip("PID 99999 unexpectedly exists on this system")
	}

	sErr, ok := err.(*sysprims.Error)
	if !ok {
		t.Errorf("Expected *sysprims.Error, got %T", err)
		return
	}

	// Should be NotFound or PermissionDenied depending on platform
	if sErr.Code != sysprims.ErrNotFound && sErr.Code != sysprims.ErrPermissionDenied {
		t.Errorf("Expected ErrNotFound or ErrPermissionDenied, got %d (%s)", sErr.Code, sErr.Code)
	}
}

// TestProcessList verifies that ProcessList returns processes.
func TestProcessList(t *testing.T) {
	snapshot, err := sysprims.ProcessList(nil)
	if err != nil {
		t.Fatalf("ProcessList(nil) failed: %v", err)
	}

	if len(snapshot.Processes) == 0 {
		t.Error("ProcessList returned empty list")
	}

	if snapshot.SchemaID == "" {
		t.Error("ProcessList returned empty schema_id")
	}

	t.Logf("Found %d processes, schema_id: %s", len(snapshot.Processes), snapshot.SchemaID)
}

// TestProcessListWithFilter verifies filtering works.
func TestProcessListWithFilter(t *testing.T) {
	// Filter by current process name
	currentName := "sysprims" // Test binary name will contain this

	snapshot, err := sysprims.ProcessList(&sysprims.ProcessFilter{
		NameContains: &currentName,
	})
	if err != nil {
		t.Fatalf("ProcessList with filter failed: %v", err)
	}

	// Should find at least one process (might not match depending on test binary name)
	t.Logf("Found %d processes matching %q", len(snapshot.Processes), currentName)
}

// TestProcessGetSelf verifies that ProcessGet works for the current process.
func TestProcessGetSelf(t *testing.T) {
	pid := uint32(os.Getpid())

	info, err := sysprims.ProcessGet(pid)
	if err != nil {
		t.Fatalf("ProcessGet(%d) failed: %v", pid, err)
	}

	if info.PID != pid {
		t.Errorf("ProcessGet returned wrong PID: got %d, expected %d", info.PID, pid)
	}

	if info.Name == "" {
		t.Error("ProcessGet returned empty name")
	}

	t.Logf("Process: %s (PID %d, PPID %d)", info.Name, info.PID, info.PPID)
}

// TestProcessGetInvalidPID verifies that ProcessGet rejects PID 0.
func TestProcessGetInvalidPID(t *testing.T) {
	_, err := sysprims.ProcessGet(0)
	if err == nil {
		t.Error("ProcessGet(0) should return error")
	}

	sErr, ok := err.(*sysprims.Error)
	if !ok {
		t.Errorf("Expected *sysprims.Error, got %T", err)
		return
	}
	if sErr.Code != sysprims.ErrInvalidArgument {
		t.Errorf("Expected ErrInvalidArgument, got %d (%s)", sErr.Code, sErr.Code)
	}
}

// TestProcessGetNonexistent verifies that ProcessGet returns NotFound for nonexistent PIDs.
func TestProcessGetNonexistent(t *testing.T) {
	_, err := sysprims.ProcessGet(99999999)
	if err == nil {
		t.Skip("PID 99999999 unexpectedly exists on this system")
	}

	sErr, ok := err.(*sysprims.Error)
	if !ok {
		t.Errorf("Expected *sysprims.Error, got %T", err)
		return
	}
	if sErr.Code != sysprims.ErrNotFound {
		t.Errorf("Expected ErrNotFound, got %d (%s)", sErr.Code, sErr.Code)
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
	pid := uint32(os.Getpid())

	proto := sysprims.ProtocolTCP
	filter := &sysprims.PortFilter{Protocol: &proto, LocalPort: &port}

	snap, err := sysprims.ListeningPorts(filter)
	if err != nil {
		// Best-effort: on macOS SIP/TCC or constrained runners, this may fail.
		if sErr, ok := err.(*sysprims.Error); ok && sErr.Code == sysprims.ErrPermissionDenied {
			t.Skipf("ListeningPorts PermissionDenied in this environment: %v", err)
		}
		t.Fatalf("ListeningPorts failed: %v", err)
	}

	found := false
	for _, b := range snap.Bindings {
		if b.LocalPort == port && b.PID != nil && *b.PID == pid {
			found = true
			break
		}
	}

	if !found {
		// Best-effort: port-to-PID mapping requires elevated privileges on most platforms.
		// - macOS: SIP/TCC restrictions
		// - Linux: /proc/<pid>/fd requires root or same-user
		// - Windows: netstat access may be limited
		// CI runners typically don't have these privileges.
		hasPermissionWarnings := false
		for _, w := range snap.Warnings {
			if strings.Contains(w, "permission") || strings.Contains(w, "Permission") {
				hasPermissionWarnings = true
				break
			}
		}
		if runtime.GOOS == "darwin" || hasPermissionWarnings {
			t.Logf("Did not find self listener pid=%d port=%d; warnings=%v bindings=%d (best-effort: permission-limited)", pid, port, snap.Warnings, len(snap.Bindings))
			return
		}
		t.Fatalf("Did not find self listener pid=%d port=%d; warnings=%v bindings=%d", pid, port, snap.Warnings, len(snap.Bindings))
	}
}

// TestRunWithTimeoutCompletes verifies that a quick command completes normally.
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

	result, err := sysprims.RunWithTimeout(
		cmd, args,
		5*time.Second,
		sysprims.DefaultTimeoutConfig(),
	)
	if err != nil {
		t.Fatalf("RunWithTimeout failed: %v", err)
	}

	if !result.Completed() {
		t.Error("Expected command to complete")
	}

	if result.TimedOut() {
		t.Error("Command should not have timed out")
	}

	if result.ExitCode == nil {
		t.Error("Expected exit code for completed command")
	} else if *result.ExitCode != 0 {
		t.Errorf("Expected exit code 0, got %d", *result.ExitCode)
	}

	if result.SchemaID == "" {
		t.Error("Result has empty schema_id")
	}

	t.Logf("Command completed with exit code %d, schema_id: %s", *result.ExitCode, result.SchemaID)
}

// TestRunWithTimeoutTimesOut verifies that a slow command times out.
func TestRunWithTimeoutTimesOut(t *testing.T) {
	if testing.Short() {
		t.Skip("Skipping timeout test in short mode")
	}

	var cmd string
	var args []string

	if runtime.GOOS == "windows" {
		cmd = "cmd"
		args = []string{"/c", "ping -n 10 127.0.0.1"}
	} else {
		cmd = "sleep"
		args = []string{"10"}
	}

	config := sysprims.DefaultTimeoutConfig()
	config.KillAfter = 500 * time.Millisecond // Quick escalation for test

	result, err := sysprims.RunWithTimeout(
		cmd, args,
		1*time.Second,
		config,
	)
	if err != nil {
		t.Fatalf("RunWithTimeout failed: %v", err)
	}

	if result.Completed() {
		t.Error("Expected command to time out")
	}

	if !result.TimedOut() {
		t.Error("Command should have timed out")
	}

	if result.SignalSent == nil {
		t.Error("Expected signal_sent for timed out command")
	} else {
		t.Logf("Signal sent: %d", *result.SignalSent)
	}

	if result.TreeKillReliability != nil {
		t.Logf("Tree kill reliability: %s", *result.TreeKillReliability)
	}
}

// TestRunWithTimeoutNotFound verifies error handling for nonexistent commands.
func TestRunWithTimeoutNotFound(t *testing.T) {
	_, err := sysprims.RunWithTimeout(
		"/nonexistent/command/that/does/not/exist",
		nil,
		1*time.Second,
		sysprims.DefaultTimeoutConfig(),
	)

	if err == nil {
		t.Fatal("Expected error for nonexistent command")
	}

	sErr, ok := err.(*sysprims.Error)
	if !ok {
		t.Errorf("Expected *sysprims.Error, got %T", err)
		return
	}

	// Should be NotFound or SpawnFailed
	if sErr.Code != sysprims.ErrNotFound && sErr.Code != sysprims.ErrSpawnFailed {
		t.Errorf("Expected ErrNotFound or ErrSpawnFailed, got %d (%s)", sErr.Code, sErr.Code)
	}
}

// TestKillGroupNotSupportedOnWindows verifies Windows platform behavior.
func TestKillGroupNotSupportedOnWindows(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("Skipping Windows-specific test")
	}

	err := sysprims.KillGroup(1234, sysprims.SIGTERM)
	if err == nil {
		t.Error("KillGroup should return error on Windows")
	}

	sErr, ok := err.(*sysprims.Error)
	if !ok {
		t.Errorf("Expected *sysprims.Error, got %T", err)
		return
	}
	if sErr.Code != sysprims.ErrNotSupported {
		t.Errorf("Expected ErrNotSupported, got %d (%s)", sErr.Code, sErr.Code)
	}
}

// TestDefaultTimeoutConfig verifies default config values.
func TestDefaultTimeoutConfig(t *testing.T) {
	config := sysprims.DefaultTimeoutConfig()

	if config.Signal != sysprims.SIGTERM {
		t.Errorf("Expected Signal=SIGTERM, got %d", config.Signal)
	}
	if config.KillAfter != 10*time.Second {
		t.Errorf("Expected KillAfter=10s, got %v", config.KillAfter)
	}
	if config.Grouping != sysprims.GroupByDefault {
		t.Errorf("Expected Grouping=GroupByDefault, got %d", config.Grouping)
	}
	if config.PreserveStatus {
		t.Error("Expected PreserveStatus=false")
	}
}

// TestErrorString verifies Error.Error() returns a meaningful string.
func TestErrorString(t *testing.T) {
	err := &sysprims.Error{
		Code:    sysprims.ErrNotFound,
		Message: "process not found: 12345",
	}

	s := err.Error()
	if s != "process not found: 12345" {
		t.Errorf("Error.Error() = %q, expected message", s)
	}

	// Test with empty message
	err2 := &sysprims.Error{Code: sysprims.ErrPermissionDenied}
	s2 := err2.Error()
	if s2 != "PermissionDenied" {
		t.Errorf("Error.Error() with empty message = %q, expected code name", s2)
	}
}

// TestErrorCodeString verifies ErrorCode.String() returns meaningful names.
func TestErrorCodeString(t *testing.T) {
	tests := []struct {
		code     sysprims.ErrorCode
		expected string
	}{
		{sysprims.ErrOK, "OK"},
		{sysprims.ErrInvalidArgument, "InvalidArgument"},
		{sysprims.ErrSpawnFailed, "SpawnFailed"},
		{sysprims.ErrTimeout, "Timeout"},
		{sysprims.ErrPermissionDenied, "PermissionDenied"},
		{sysprims.ErrNotFound, "NotFound"},
		{sysprims.ErrNotSupported, "NotSupported"},
		{sysprims.ErrGroupCreationFailed, "GroupCreationFailed"},
		{sysprims.ErrSystem, "System"},
		{sysprims.ErrInternal, "Internal"},
		{sysprims.ErrorCode(999), "Unknown"},
	}

	for _, tt := range tests {
		if got := tt.code.String(); got != tt.expected {
			t.Errorf("ErrorCode(%d).String() = %q, expected %q", tt.code, got, tt.expected)
		}
	}
}

// TestClearError verifies ClearError doesn't panic.
func TestClearError(t *testing.T) {
	// Should not panic
	sysprims.ClearError()
}
