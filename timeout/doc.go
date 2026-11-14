// SPDX-License-Identifier: MIT OR Apache-2.0

// Package timeout provides process timeout execution, PID-based tree
// termination, and managed containment lifecycles for Go without cgo
// (CGO_ENABLED=0).
//
// This package is the pure-Go twin of crates/sysprims-timeout. It enforces
// ADR-0011 PID/PGID safety invariants at the API boundary, integrates with
// sibling packages (syserror, signal, session), and emits JSON-schema-aligned
// TimeoutResult, TerminateTreeResult, and ContainmentSnapshot receipts
// matching Rust, Python, and TypeScript.
package timeout

import (
	"context"
	"math"
	"os"
	"runtime"
	"sort"
	"strings"
	"sync/atomic"
	"time"

	"github.com/3leaps/sysprims/signal"
	"github.com/3leaps/sysprims/syserror"
)

// MaxSafePID is the maximum safe POSIX PID/PGID value (2147483647, math.MaxInt32).
const MaxSafePID uint32 = math.MaxInt32

// Schema URI constants matching crates/sysprims-core/src/schema.rs.
const (
	TimeoutResultSchemaID          = "https://schemas.3leaps.dev/sysprims/timeout/v1.1.0/timeout-result.schema.json"
	TerminateTreeConfigSchemaID    = "https://schemas.3leaps.dev/sysprims/process/v1.0.0/terminate-tree-config.schema.json"
	TerminateTreeResultSchemaID    = "https://schemas.3leaps.dev/sysprims/process/v1.0.0/terminate-tree-result.schema.json"
	ContainmentSpawnConfigSchemaID = "https://schemas.3leaps.dev/sysprims/timeout/v1.0.0/containment-spawn-config.schema.json"
	ContainmentSnapshotSchemaID    = "https://schemas.3leaps.dev/sysprims/timeout/v1.0.0/containment-snapshot.schema.json"
)

// Bounds constants matching crates/sysprims-timeout/src/managed.rs.
const (
	MaxRegistrySlots                       = 1024
	MaxArgvEntries                         = 256
	MaxArgvEntryBytes                      = 4096
	MaxArgvTotalBytes                      = 65_536
	MaxEnvEntries                          = 256
	MaxEnvKeyBytes                         = 1024
	MaxEnvValueBytes                       = 4096
	MaxEnvTotalBytes                       = 65_536
	MaxCwdBytes                            = 4096
	MaxDurationMS                   uint64 = 86_400_000
	MaxCompletionObservedPIDs              = 4096
	MaxCompletionObservationRetries        = 3

	pollInterval = 10 * time.Millisecond
)

// GroupingMode controls process group / Job Object creation for timeout execution.
type GroupingMode int32

const (
	// GroupByDefault creates a new session/process group (Unix) or Job Object
	// (Windows) to enable tree-kill on timeout. This is the recommended default.
	GroupByDefault GroupingMode = 0
	// Foreground runs without creating a new process group.
	// Only the direct child is killed on timeout; grandchildren may survive.
	Foreground GroupingMode = 1
)

// Tree-kill reliability and boundary strength string constants.
const (
	ReliabilityGuaranteed = "guaranteed"
	ReliabilityUnproven   = "unproven"
	ReliabilityBestEffort = "best_effort"

	BoundaryCooperativeGroup  = "cooperative_group"
	BoundaryKernelEnforcedJob = "kernel_enforced_job"
	BoundaryUnknown           = "unknown"

	ObservationLinuxProcfsProcessGroup  = "linux_procfs_process_group"
	ObservationMacosLibprocProcessGroup = "macos_libproc_process_group"
	ObservationWindowsJobProcessIDList  = "windows_job_process_id_list"
	ObservationUnsupportedPlatform      = "unsupported_platform"
)

// TimeoutConfig configures the behavior of [RunWithTimeout] and [RunWithTimeoutContext].
type TimeoutConfig struct {
	// Signal is the signal to send on timeout (default: signal.SIGTERM).
	Signal int
	// KillAfter is the delay before escalating to SIGKILL if the process
	// does not terminate. Set to 0 for immediate escalation.
	KillAfter time.Duration
	// Grouping controls process group creation for tree-kill.
	Grouping GroupingMode
	// PreserveStatus retains the child's exit code on completion.
	PreserveStatus bool

	// Cwd optionally sets the child process working directory.
	Cwd string
	// Env optionally merges environment variable overrides onto the parent environment.
	Env map[string]string
	// MaxStdoutBytes enables bounded stdout capture when > 0.
	MaxStdoutBytes int
	// MaxStderrBytes enables bounded stderr capture when > 0.
	MaxStderrBytes int
	// PipeDrainTimeout bounds post-exit pipe draining when output capture is enabled
	// (defaults to 200ms when 0).
	PipeDrainTimeout time.Duration
}

// DefaultTimeoutConfig returns sensible defaults for timeout execution.
//
// Defaults:
//   - Signal: signal.SIGTERM
//   - KillAfter: 10 seconds
//   - Grouping: GroupByDefault
//   - PreserveStatus: false
func DefaultTimeoutConfig() TimeoutConfig {
	return TimeoutConfig{
		Signal:         signal.SIGTERM,
		KillAfter:      10 * time.Second,
		Grouping:       GroupByDefault,
		PreserveStatus: false,
	}
}

// TimeoutResult represents the outcome of a timeout execution
// (`timeout-result.schema.json`).
type TimeoutResult struct {
	SchemaID            string  `json:"schema_id"`
	Status              string  `json:"status"`
	ExitCode            *int    `json:"exit_code,omitempty"`
	SignalSent          *int    `json:"signal_sent,omitempty"`
	Escalated           *bool   `json:"escalated,omitempty"`
	TreeKillReliability *string `json:"tree_kill_reliability,omitempty"`

	// Bounded output capture fields populated when MaxStdoutBytes / MaxStderrBytes > 0.
	// Excluded from schema-bound JSON serialization.
	Stdout          []byte `json:"-"`
	Stderr          []byte `json:"-"`
	StdoutTruncated bool   `json:"-"`
	StderrTruncated bool   `json:"-"`
}

// Completed returns true if the command completed without timing out.
func (r *TimeoutResult) Completed() bool {
	return r != nil && r.Status == "completed"
}

// TimedOut returns true if the command was terminated due to timeout.
func (r *TimeoutResult) TimedOut() bool {
	return r != nil && r.Status == "timed_out"
}

// TerminateTreeConfig configures PID-based terminate-tree behavior
// (`terminate-tree-config.schema.json`).
type TerminateTreeConfig struct {
	SchemaID string `json:"schema_id,omitempty"`

	GraceTimeoutMS *uint64 `json:"grace_timeout_ms,omitempty"`
	KillTimeoutMS  *uint64 `json:"kill_timeout_ms,omitempty"`
	Signal         *int32  `json:"signal,omitempty"`
	KillSignal     *int32  `json:"kill_signal,omitempty"`
}

// TerminateTreeResult is the outcome of a terminate-tree operation
// (`terminate-tree-result.schema.json`).
type TerminateTreeResult struct {
	SchemaID            string   `json:"schema_id"`
	Timestamp           string   `json:"timestamp"`
	Platform            string   `json:"platform"`
	PID                 uint32   `json:"pid"`
	PGID                *uint32  `json:"pgid,omitempty"`
	SignalSent          int32    `json:"signal_sent"`
	KillSignal          *int32   `json:"kill_signal,omitempty"`
	Escalated           bool     `json:"escalated"`
	Exited              bool     `json:"exited"`
	TimedOut            bool     `json:"timed_out"`
	TreeKillReliability string   `json:"tree_kill_reliability"`
	Warnings            []string `json:"warnings"`
}

// SpawnContainedConfig configures sysprims-owned contained spawn
// (`containment-spawn-config.schema.json`).
type SpawnContainedConfig struct {
	SchemaID           string            `json:"schema_id,omitempty"`
	Argv               []string          `json:"argv"`
	Cwd                *string           `json:"cwd,omitempty"`
	Env                map[string]string `json:"env,omitempty"`
	ExecutionTimeoutMS *uint64           `json:"execution_timeout_ms,omitempty"`
	GraceTimeoutMS     *uint64           `json:"grace_timeout_ms,omitempty"`
	KillTimeoutMS      *uint64           `json:"kill_timeout_ms,omitempty"`
	Signal             *int32            `json:"signal,omitempty"`
	KillSignal         *int32            `json:"kill_signal,omitempty"`
}

// ContainmentIdentity is diagnostic spawn evidence. PID is never authority.
type ContainmentIdentity struct {
	PID             uint32 `json:"pid"`
	StartTimeUnixMS uint64 `json:"start_time_unix_ms"`
	ExePath         string `json:"exe_path"`
}

// ContainmentCompletion is point-in-time membership evidence after cleanup.
type ContainmentCompletion struct {
	Status        string   `json:"status"`
	Observation   string   `json:"observation"`
	ObservedCount *uint32  `json:"observed_count,omitempty"`
	SurvivorPIDs  []uint32 `json:"survivor_pids,omitempty"`
}

// ContainmentSnapshot is a poll/wait/terminate result
// (`containment-snapshot.schema.json`).
type ContainmentSnapshot struct {
	SchemaID            string                 `json:"schema_id"`
	Timestamp           string                 `json:"timestamp"`
	Platform            string                 `json:"platform"`
	HandleState         string                 `json:"handle_state"`
	LeaderStatus        string                 `json:"leader_status"`
	Identity            ContainmentIdentity    `json:"identity"`
	TreeKillReliability string                 `json:"tree_kill_reliability"`
	BoundaryStrength    string                 `json:"boundary_strength"`
	PGID                *uint32                `json:"pgid,omitempty"`
	SignalSent          *int32                 `json:"signal_sent,omitempty"`
	KillSignal          *int32                 `json:"kill_signal,omitempty"`
	Escalated           *bool                  `json:"escalated,omitempty"`
	Exited              *bool                  `json:"exited,omitempty"`
	TimedOut            *bool                  `json:"timed_out,omitempty"`
	Completion          *ContainmentCompletion `json:"completion,omitempty"`
	Warnings            []string               `json:"warnings"`
}

// RunWithTimeout executes a command with a timeout.
//
// If the command does not complete within timeout, it is terminated. When using
// [GroupByDefault] (the default), the acquired process group or Windows Job
// Object is terminated.
func RunWithTimeout(command string, args []string, timeout time.Duration, config TimeoutConfig) (*TimeoutResult, error) {
	return RunWithTimeoutContext(context.Background(), command, args, timeout, config)
}

// RunWithTimeoutContext executes a command with a timeout and caller context.
//
// If either timeout elapses or ctx is canceled, the command (and its process
// group / Job Object when using [GroupByDefault]) is terminated with configured
// escalation.
func RunWithTimeoutContext(ctx context.Context, command string, args []string, timeout time.Duration, config TimeoutConfig) (*TimeoutResult, error) {
	if ctx == nil {
		ctx = context.Background()
	}
	if command == "" {
		return nil, syserror.InvalidArgument("command cannot be empty")
	}
	if indexByteZero(command) {
		return nil, syserror.InvalidArgument("command must not contain NUL bytes")
	}
	for i, arg := range args {
		if indexByteZero(arg) {
			return nil, syserror.InvalidArgumentf("args[%d] must not contain NUL bytes", i)
		}
	}
	if timeout <= 0 {
		return nil, syserror.InvalidArgument("timeout_ms must be > 0")
	}
	if uint64(timeout.Milliseconds()) > MaxDurationMS { // #nosec G115 -- timeout validated > 0 above
		return nil, syserror.InvalidArgumentf("timeout_ms exceeds maximum of %d", MaxDurationMS)
	}
	if config.KillAfter < 0 {
		return nil, syserror.InvalidArgument("kill_after_ms must not be negative")
	}
	if uint64(config.KillAfter.Milliseconds()) > MaxDurationMS { // #nosec G115 -- KillAfter validated >= 0 above
		return nil, syserror.InvalidArgumentf("kill_after_ms exceeds maximum of %d", MaxDurationMS)
	}
	if config.Grouping != GroupByDefault && config.Grouping != Foreground {
		return nil, syserror.InvalidArgumentf("invalid grouping mode %d", config.Grouping)
	}
	if config.Signal == 0 {
		config.Signal = signal.SIGTERM
	}
	if err := validatePlatformSignal(int32(config.Signal), "signal"); err != nil { // #nosec G115 -- signal number validated by validatePlatformSignal
		return nil, err
	}
	return runWithTimeoutImpl(ctx, command, args, timeout, config)
}

// TerminateTree sends a graceful signal to pid, waits up to GraceTimeoutMS,
// and escalates to KillSignal if the process is still running.
//
// This PID-returning compatibility API has no owned child or sealed containment
// receipt, so it signals only the supplied PID and always reports
// `best_effort`. Use [SpawnContained] for process-group or Job Object
// containment.
func TerminateTree(pid uint32, config *TerminateTreeConfig) (*TerminateTreeResult, error) {
	var cfg TerminateTreeConfig
	if config != nil {
		cfg = *config
	}
	if cfg.SchemaID != "" && cfg.SchemaID != TerminateTreeConfigSchemaID {
		return nil, syserror.InvalidArgumentf("invalid schema_id (expected %s)", TerminateTreeConfigSchemaID)
	}
	if pid == 0 {
		return nil, syserror.InvalidArgument("PID 0 is not valid (would target process group)")
	}
	if pid > MaxSafePID {
		return nil, syserror.InvalidArgumentf("PID %d exceeds i32::MAX (would overflow to negative pid_t)", pid)
	}
	resolved, err := resolveTerminateTreeConfig(cfg)
	if err != nil {
		return nil, err
	}
	return terminateTreeImpl(pid, resolved)
}

const (
	handleOpen uint32 = iota
	handleClosing
	handleClosed
)

// ContainedProcess is a Go projection of a managed containment handle.
//
// The wrapper stores only the generation-checked capability token. Close is
// deterministic; a finalizer is a leak backstop only.
type ContainedProcess struct {
	token atomic.Uint64
	state atomic.Uint32
}

var nativeCloseFn atomic.Value             // stores func(uint64) error
var beforeSnapshotCallForTest atomic.Value // stores func()

func callNativeClose(token uint64) error {
	if v := nativeCloseFn.Load(); v != nil {
		if fn, ok := v.(func(uint64) error); ok && fn != nil {
			return fn(token)
		}
	}
	return managedClose(token)
}

func callBeforeSnapshotHook() {
	if v := beforeSnapshotCallForTest.Load(); v != nil {
		if fn, ok := v.(func()); ok && fn != nil {
			fn()
		}
	}
}

func containedFinalizer(handle *ContainedProcess) {
	if !handle.state.CompareAndSwap(handleOpen, handleClosing) {
		return
	}
	token := handle.token.Swap(0)
	if token == 0 {
		handle.state.Store(handleClosed)
		return
	}
	_ = callNativeClose(token)
	handle.state.Store(handleClosed)
}

// SpawnContained spawns and owns a contained child process.
func SpawnContained(config SpawnContainedConfig) (*ContainedProcess, error) {
	if config.SchemaID != "" && config.SchemaID != ContainmentSpawnConfigSchemaID {
		return nil, syserror.InvalidArgumentf("invalid schema_id (expected %s)", ContainmentSpawnConfigSchemaID)
	}
	token, err := managedSpawn(config)
	if err != nil {
		return nil, err
	}
	proc := &ContainedProcess{}
	proc.token.Store(token)
	runtime.SetFinalizer(proc, containedFinalizer)
	return proc, nil
}

func (h *ContainedProcess) currentToken() (uint64, error) {
	if h == nil {
		return 0, syserror.InvalidArgument("containment handle is invalid")
	}
	token := h.token.Load()
	if token == 0 {
		return 0, syserror.InvalidArgument("containment handle is stale or closed")
	}
	return token, nil
}

// Identity returns immutable spawn evidence without consuming reap authority.
func (h *ContainedProcess) Identity() (*ContainmentSnapshot, error) {
	token, err := h.currentToken()
	if err != nil {
		return nil, err
	}
	callBeforeSnapshotHook()
	snap, err := managedIdentity(token)
	runtime.KeepAlive(h)
	return snap, err
}

// Poll is non-destructive while the leader is running. If the leader has
// already exited, poll commits the terminal outcome.
func (h *ContainedProcess) Poll() (*ContainmentSnapshot, error) {
	token, err := h.currentToken()
	if err != nil {
		return nil, err
	}
	callBeforeSnapshotHook()
	snap, err := managedPoll(token)
	runtime.KeepAlive(h)
	return snap, err
}

// Wait waits for a terminal outcome or until timeout. A timeout of 0 waits
// until the lifecycle finishes. Negative timeouts are rejected. A positive
// sub-millisecond timeout is rounded up to 1ms so it is never treated as 0
// (infinite).
func (h *ContainedProcess) Wait(timeout time.Duration) (*ContainmentSnapshot, error) {
	token, err := h.currentToken()
	if err != nil {
		return nil, err
	}
	if timeout < 0 {
		return nil, syserror.InvalidArgument("wait timeout must not be negative")
	}
	var timeoutMS uint64
	if timeout > 0 {
		ms := timeout.Milliseconds()
		if ms <= 0 {
			timeoutMS = 1
		} else {
			timeoutMS = uint64(ms)
		}
	}
	callBeforeSnapshotHook()
	snap, err := managedWait(token, timeoutMS)
	runtime.KeepAlive(h)
	return snap, err
}

// Terminate sends the configured grace/escalate policy once.
func (h *ContainedProcess) Terminate() (*ContainmentSnapshot, error) {
	token, err := h.currentToken()
	if err != nil {
		return nil, err
	}
	callBeforeSnapshotHook()
	snap, err := managedTerminate(token)
	runtime.KeepAlive(h)
	return snap, err
}

// Close releases the registry entry. The second call is a no-op.
// The wrapper token is cleared only after close succeeds; a failed close
// restores Open so the caller can retry.
func (h *ContainedProcess) Close() error {
	if h == nil {
		return nil
	}
	for {
		switch h.state.Load() {
		case handleClosed:
			return nil
		case handleClosing:
			runtime.Gosched()
		default:
			if h.state.CompareAndSwap(handleOpen, handleClosing) {
				return h.finishClose()
			}
		}
	}
}

func (h *ContainedProcess) finishClose() error {
	token := h.token.Load()
	if token == 0 {
		h.state.Store(handleClosed)
		return nil
	}
	runtime.SetFinalizer(h, nil)
	if err := callNativeClose(token); err != nil {
		runtime.SetFinalizer(h, containedFinalizer)
		h.state.Store(handleOpen)
		return err
	}
	h.token.Store(0)
	h.state.Store(handleClosed)
	return nil
}

func currentPlatform() string {
	switch runtime.GOOS {
	case "darwin":
		return "macos"
	case "linux":
		return "linux"
	case "windows":
		return "windows"
	default:
		return runtime.GOOS
	}
}

func nowRFC3339() string {
	return time.Now().UTC().Format(time.RFC3339Nano)
}

func indexByteZero(s string) bool {
	return strings.IndexByte(s, 0) >= 0
}

func mergeEnv(base []string, overrides map[string]string) []string {
	envMap := make(map[string]string, len(base)+len(overrides))
	for _, entry := range base {
		if k, v, ok := strings.Cut(entry, "="); ok {
			envMap[k] = v
		}
	}
	for k, v := range overrides {
		envMap[k] = v
	}
	keys := make([]string, 0, len(envMap))
	for k := range envMap {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	out := make([]string, 0, len(keys))
	for _, k := range keys {
		out = append(out, k+"="+envMap[k])
	}
	return out
}

func parentEnv() []string {
	return os.Environ()
}
