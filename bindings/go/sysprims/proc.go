package sysprims

/*
#include "sysprims.h"
#include <stdlib.h>
*/
import "C"
import (
	"encoding/json"
	"time"
	"unsafe"
)

// ProcessInfo contains information about a running process.
type ProcessInfo struct {
	// PID is the process ID.
	PID uint32 `json:"pid"`
	// PPID is the parent process ID.
	PPID uint32 `json:"ppid"`
	// Name is the process name (executable name without path).
	Name string `json:"name"`
	// User is the username running the process (may be nil if unavailable).
	User *string `json:"user,omitempty"`
	// CPUPercent is the CPU usage percentage (0-100).
	CPUPercent float64 `json:"cpu_percent"`
	// MemoryKB is the memory usage in kilobytes.
	MemoryKB uint64 `json:"memory_kb"`
	// ElapsedSeconds is the process runtime in seconds (may be nil if unavailable).
	ElapsedSeconds *uint64 `json:"elapsed_seconds,omitempty"`
	// StartTimeUnixMS is the process start time (Unix epoch ms), best-effort.
	StartTimeUnixMS *uint64 `json:"start_time_unix_ms,omitempty"`
	// ExePath is the absolute executable path, best-effort.
	ExePath *string `json:"exe_path,omitempty"`
	// State is the process state (may be nil if unavailable).
	State *string `json:"state,omitempty"`
	// Cmdline is the command line arguments (may be empty if unavailable).
	Cmdline []string `json:"cmdline,omitempty"`
	// Env is process environment variables (same-user best-effort, may be nil).
	Env map[string]string `json:"env,omitempty"`
	// ThreadCount is the best-effort thread count for this process.
	ThreadCount *uint32 `json:"thread_count,omitempty"`
}

// ProcessSnapshot represents a point-in-time listing of processes.
type ProcessSnapshot struct {
	// SchemaID identifies the JSON schema version.
	SchemaID string `json:"schema_id"`
	// Timestamp is the ISO 8601 timestamp when the snapshot was taken.
	Timestamp string `json:"timestamp"`
	// Processes is the list of process information.
	Processes []ProcessInfo `json:"processes"`
}

// WaitPidResult is the result of waiting for a PID to exit.
type WaitPidResult struct {
	SchemaID  string   `json:"schema_id"`
	Timestamp string   `json:"timestamp"`
	Platform  string   `json:"platform"`
	PID       uint32   `json:"pid"`
	Exited    bool     `json:"exited"`
	TimedOut  bool     `json:"timed_out"`
	ExitCode  *int32   `json:"exit_code,omitempty"`
	Warnings  []string `json:"warnings"`
}

type Protocol string

const (
	ProtocolTCP Protocol = "tcp"
	ProtocolUDP Protocol = "udp"
)

type CpuMode string

const (
	CpuModeLifetime CpuMode = "lifetime"
	CpuModeMonitor  CpuMode = "monitor"
)

// PortBinding contains information about a listening socket binding.
type PortBinding struct {
	Protocol  Protocol     `json:"protocol"`
	LocalAddr *string      `json:"local_addr,omitempty"`
	LocalPort uint16       `json:"local_port"`
	State     *string      `json:"state,omitempty"`
	PID       *uint32      `json:"pid,omitempty"`
	Process   *ProcessInfo `json:"process,omitempty"`
	// NOTE: warnings and best-effort behavior are surfaced at snapshot level.
}

// PortBindingsSnapshot represents a point-in-time listing of listening ports.
type PortBindingsSnapshot struct {
	SchemaID  string        `json:"schema_id"`
	Timestamp string        `json:"timestamp"`
	Platform  string        `json:"platform"`
	Bindings  []PortBinding `json:"bindings"`
	Warnings  []string      `json:"warnings"`
}

// PortFilter specifies criteria for filtering port bindings.
type PortFilter struct {
	Protocol  *Protocol `json:"protocol,omitempty"`
	LocalPort *uint16   `json:"local_port,omitempty"`
}

// ProcessFilter specifies criteria for filtering processes.
//
// All fields are optional. When multiple fields are set, they are ANDed together.
type ProcessFilter struct {
	// NameContains filters by process name substring (case-insensitive).
	NameContains *string `json:"name_contains,omitempty"`
	// NameEquals filters by exact process name match.
	NameEquals *string `json:"name_equals,omitempty"`
	// UserEquals filters by exact username match.
	UserEquals *string `json:"user_equals,omitempty"`
	// PIDIn filters to only these PIDs.
	PIDIn []uint32 `json:"pid_in,omitempty"`
	// PPID filters by parent process ID.
	PPID *uint32 `json:"ppid,omitempty"`
	// StateIn filters by process state.
	StateIn []string `json:"state_in,omitempty"`
	// CPUAbove filters to processes using more than this CPU percentage.
	CPUAbove *float64 `json:"cpu_above,omitempty"`
	// MemoryAboveKB filters to processes using more than this memory (KB).
	MemoryAboveKB *uint64 `json:"memory_above_kb,omitempty"`
	// RunningForAtLeastSecs filters to processes running at least this many seconds.
	RunningForAtLeastSecs *uint64 `json:"running_for_at_least_secs,omitempty"`
}

// ProcessOptions controls optional process detail collection.
//
// Defaults are false/zero-value for all fields.
type ProcessOptions struct {
	// IncludeEnv requests collection of environment variables.
	IncludeEnv bool `json:"include_env,omitempty"`
	// IncludeThreads requests collection of process thread count.
	IncludeThreads bool `json:"include_threads,omitempty"`
}

// FdInfo describes an open file descriptor.
type FdInfo struct {
	Fd   uint32  `json:"fd"`
	Kind string  `json:"kind"`
	Path *string `json:"path,omitempty"`
}

// FdSnapshot represents a point-in-time listing of open file descriptors.
type FdSnapshot struct {
	SchemaID  string   `json:"schema_id"`
	Timestamp string   `json:"timestamp"`
	Platform  string   `json:"platform"`
	Pid       uint32   `json:"pid"`
	Fds       []FdInfo `json:"fds"`
	Warnings  []string `json:"warnings"`
}

// FdFilter specifies criteria for filtering file descriptors.
type FdFilter struct {
	Kind *string `json:"kind,omitempty"`
}

// ListFds returns a snapshot of open file descriptors for the given PID.
//
// Best-effort behavior:
// - Fields may be omitted
// - Warnings may be present
// - Windows returns ErrNotSupported
func ListFds(pid uint32, filter *FdFilter) (*FdSnapshot, error) {
	var filterCStr *C.char
	if filter != nil {
		filterJSON, err := json.Marshal(filter)
		if err != nil {
			return nil, &Error{Code: ErrInvalidArgument, Message: "failed to marshal filter: " + err.Error()}
		}
		filterCStr = C.CString(string(filterJSON))
		defer C.free(unsafe.Pointer(filterCStr))
	}

	var resultCStr *C.char
	if err := callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_proc_list_fds(C.uint32_t(pid), filterCStr, &resultCStr)
	}); err != nil {
		return nil, err
	}
	defer C.sysprims_free_string(resultCStr)

	var snapshot FdSnapshot
	if err := json.Unmarshal([]byte(C.GoString(resultCStr)), &snapshot); err != nil {
		return nil, &Error{Code: ErrInternal, Message: "failed to parse response: " + err.Error()}
	}

	return &snapshot, nil
}

// ProcessList returns a snapshot of running processes, optionally filtered.
//
// Pass nil for filter to return all processes.
//
// # Example
//
//	// Get all processes
//	snapshot, err := sysprims.ProcessList(nil)
//
//	// Get processes by name
//	name := "nginx"
//	snapshot, err := sysprims.ProcessList(&sysprims.ProcessFilter{
//	    NameContains: &name,
//	})
//
// # Errors
//
//   - [ErrInvalidArgument]: Invalid filter JSON
//   - [ErrSystem]: System error reading process information
func ProcessList(filter *ProcessFilter) (*ProcessSnapshot, error) {
	return ProcessListWithOptions(filter, nil)
}

// ProcessListWithOptions returns a snapshot of running processes, optionally filtered,
// with opt-in extended fields.
//
// Pass nil for opts to use defaults (`include_env=false`, `include_threads=false`).
func ProcessListWithOptions(filter *ProcessFilter, opts *ProcessOptions) (*ProcessSnapshot, error) {
	var filterCStr *C.char
	if filter != nil {
		filterJSON, err := json.Marshal(filter)
		if err != nil {
			return nil, &Error{Code: ErrInvalidArgument, Message: "failed to marshal filter: " + err.Error()}
		}
		filterCStr = C.CString(string(filterJSON))
		defer C.free(unsafe.Pointer(filterCStr))
	}

	var optionsCStr *C.char
	if opts != nil {
		optionsJSON, err := json.Marshal(opts)
		if err != nil {
			return nil, &Error{Code: ErrInvalidArgument, Message: "failed to marshal options: " + err.Error()}
		}
		optionsCStr = C.CString(string(optionsJSON))
		defer C.free(unsafe.Pointer(optionsCStr))
	}

	var resultCStr *C.char
	if err := callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_proc_list_ex(filterCStr, optionsCStr, &resultCStr)
	}); err != nil {
		return nil, err
	}
	defer C.sysprims_free_string(resultCStr)

	var snapshot ProcessSnapshot
	if err := json.Unmarshal([]byte(C.GoString(resultCStr)), &snapshot); err != nil {
		return nil, &Error{Code: ErrInternal, Message: "failed to parse response: " + err.Error()}
	}

	return &snapshot, nil
}

// ProcessGet returns information for a single process by PID.
//
// # Errors
//
//   - [ErrInvalidArgument]: pid is 0
//   - [ErrNotFound]: Process doesn't exist
//   - [ErrPermissionDenied]: Not permitted to read this process
func ProcessGet(pid uint32) (*ProcessInfo, error) {
	return ProcessGetWithOptions(pid, nil)
}

// ProcessGetWithOptions returns information for a single process by PID,
// with opt-in extended fields.
//
// Pass nil for opts to use defaults (`include_env=false`, `include_threads=false`).
func ProcessGetWithOptions(pid uint32, opts *ProcessOptions) (*ProcessInfo, error) {
	var optionsCStr *C.char
	if opts != nil {
		optionsJSON, err := json.Marshal(opts)
		if err != nil {
			return nil, &Error{Code: ErrInvalidArgument, Message: "failed to marshal options: " + err.Error()}
		}
		optionsCStr = C.CString(string(optionsJSON))
		defer C.free(unsafe.Pointer(optionsCStr))
	}

	var resultCStr *C.char
	if err := callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_proc_get_ex(C.uint32_t(pid), optionsCStr, &resultCStr)
	}); err != nil {
		return nil, err
	}
	defer C.sysprims_free_string(resultCStr)

	var info ProcessInfo
	if err := json.Unmarshal([]byte(C.GoString(resultCStr)), &info); err != nil {
		return nil, &Error{Code: ErrInternal, Message: "failed to parse response: " + err.Error()}
	}

	return &info, nil
}

// WaitPID waits for a PID to exit up to the provided timeout.
//
// Best-effort behavior:
// - On Unix, this uses polling (we are not necessarily the parent).
// - On Windows, this uses process wait APIs when available.
//
// # Errors
//
//   - [ErrInvalidArgument]: pid is 0
//   - [ErrNotFound]: pid does not exist at time of first check
//   - [ErrPermissionDenied]: not permitted to query liveness
func WaitPID(pid uint32, timeout time.Duration) (*WaitPidResult, error) {
	var resultCStr *C.char
	timeoutMs := uint64(timeout / time.Millisecond)

	if err := callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_proc_wait_pid(C.uint32_t(pid), C.uint64_t(timeoutMs), &resultCStr)
	}); err != nil {
		return nil, err
	}
	defer C.sysprims_free_string(resultCStr)

	var result WaitPidResult
	if err := json.Unmarshal([]byte(C.GoString(resultCStr)), &result); err != nil {
		return nil, &Error{Code: ErrInternal, Message: "failed to parse response: " + err.Error()}
	}

	return &result, nil
}

// DescendantsLevel represents a single depth level in a descendants result.
type DescendantsLevel struct {
	// Level is the depth (1 = direct children, 2 = grandchildren, etc.).
	Level uint32 `json:"level"`
	// Processes at this level.
	Processes []ProcessInfo `json:"processes"`
}

// DescendantsResult is the result of a descendants traversal.
type DescendantsResult struct {
	SchemaID        string             `json:"schema_id"`
	RootPID         uint32             `json:"root_pid"`
	MaxLevels       uint32             `json:"max_levels"`
	Levels          []DescendantsLevel `json:"levels"`
	TotalFound      int                `json:"total_found"`
	MatchedByFilter int                `json:"matched_by_filter"`
	Timestamp       string             `json:"timestamp"`
	Platform        string             `json:"platform"`
}

// KillDescendantsResult is the result of a kill-descendants operation.
type KillDescendantsResult struct {
	SchemaID      string                `json:"schema_id"`
	SignalSent    int                   `json:"signal_sent"`
	RootPID       uint32                `json:"root_pid"`
	Succeeded     []uint32              `json:"succeeded"`
	Failed        []KillDescendantsFail `json:"failed"`
	SkippedSafety int                   `json:"skipped_safety"`
}

// KillDescendantsFail is a single failure in a kill-descendants operation.
type KillDescendantsFail struct {
	PID   uint32 `json:"pid"`
	Error string `json:"error"`
}

type DescendantsOptions struct {
	// MaxLevels controls traversal depth. Nil means all levels.
	MaxLevels *uint32
	// Filter applied to descendant processes.
	Filter *ProcessFilter
	// CpuMode controls CPU measurement semantics.
	CpuMode CpuMode
	// SampleDuration is used when CpuMode is monitor. 0 means default sample.
	SampleDuration time.Duration
}

type KillDescendantsOptions struct {
	// Signal to send. Zero defaults to SIGTERM (15).
	Signal int
	// MaxLevels controls traversal depth. Nil means all levels.
	MaxLevels *uint32
	// Filter applied to descendant processes.
	Filter *ProcessFilter
	// CpuMode controls CPU measurement semantics.
	CpuMode CpuMode
	// SampleDuration is used when CpuMode is monitor. 0 means default sample.
	SampleDuration time.Duration
}

// Descendants returns the process subtree rooted at pid.
//
// maxLevels controls the traversal depth (1 = children only). Pass 0 or
// math.MaxUint32 to traverse all levels.
//
// # Errors
//
//   - [ErrInvalidArgument]: root_pid is 0 or filter/config is invalid
//   - [ErrNotFound]: root process doesn't exist
func Descendants(pid uint32, maxLevels uint32, filter *ProcessFilter) (*DescendantsResult, error) {
	return DescendantsWithOptions(pid, &DescendantsOptions{MaxLevels: &maxLevels, Filter: filter})
}

func normalizeCpuMode(mode CpuMode) (CpuMode, error) {
	switch mode {
	case "", CpuModeLifetime:
		return CpuModeLifetime, nil
	case CpuModeMonitor:
		return CpuModeMonitor, nil
	default:
		return "", &Error{Code: ErrInvalidArgument, Message: "invalid cpu mode: " + string(mode)}
	}
}

func buildDescendantsConfigJSON(filter *ProcessFilter, mode CpuMode, sample time.Duration) (string, error) {
	config := make(map[string]interface{})
	if filter != nil {
		filterJSON, err := json.Marshal(filter)
		if err != nil {
			return "", &Error{Code: ErrInvalidArgument, Message: "failed to marshal filter: " + err.Error()}
		}
		if err := json.Unmarshal(filterJSON, &config); err != nil {
			return "", &Error{Code: ErrInvalidArgument, Message: "failed to decode filter JSON: " + err.Error()}
		}
	}

	normalizedMode, err := normalizeCpuMode(mode)
	if err != nil {
		return "", err
	}
	if normalizedMode != CpuModeLifetime {
		config["cpu_mode"] = string(normalizedMode)
	}

	if sample < 0 {
		return "", &Error{Code: ErrInvalidArgument, Message: "sample duration must be >= 0"}
	}
	if sample > 0 {
		config["sample_duration_ms"] = uint64(sample / time.Millisecond)
	}

	if len(config) == 0 {
		return "", nil
	}

	configJSON, err := json.Marshal(config)
	if err != nil {
		return "", &Error{Code: ErrInvalidArgument, Message: "failed to marshal descendants config: " + err.Error()}
	}
	return string(configJSON), nil
}

// DescendantsWithOptions returns descendants using optional cpu mode/sample config.
func DescendantsWithOptions(pid uint32, opts *DescendantsOptions) (*DescendantsResult, error) {
	maxLevels := uint32(^uint32(0))
	var filter *ProcessFilter
	cpuMode := CpuModeLifetime
	sampleDuration := time.Duration(0)

	if opts != nil {
		if opts.MaxLevels != nil {
			maxLevels = *opts.MaxLevels
		}
		filter = opts.Filter
		cpuMode = opts.CpuMode
		sampleDuration = opts.SampleDuration
	}

	configJSON, err := buildDescendantsConfigJSON(filter, cpuMode, sampleDuration)
	if err != nil {
		return nil, err
	}

	var configCStr *C.char
	if configJSON != "" {
		configCStr = C.CString(configJSON)
		defer C.free(unsafe.Pointer(configCStr))
	}

	var resultCStr *C.char
	if err := callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_proc_descendants_ex(
			C.uint32_t(pid),
			C.uint32_t(maxLevels),
			configCStr,
			nil,
			&resultCStr,
		)
	}); err != nil {
		return nil, err
	}
	defer C.sysprims_free_string(resultCStr)

	var result DescendantsResult
	if err := json.Unmarshal([]byte(C.GoString(resultCStr)), &result); err != nil {
		return nil, &Error{Code: ErrInternal, Message: "failed to parse response: " + err.Error()}
	}

	return &result, nil
}

// KillDescendants sends a signal to descendants of a process.
//
// Safety rules are enforced by the FFI layer: root PID, self, PID 1, and
// parent are excluded from the kill list. The result includes a
// SkippedSafety count indicating how many PIDs were excluded.
//
// # Errors
//
//   - [ErrInvalidArgument]: root_pid is 0 or filter/config is invalid
//   - [ErrNotFound]: root process doesn't exist
func KillDescendants(pid uint32, signal int, maxLevels uint32, filter *ProcessFilter) (*KillDescendantsResult, error) {
	return KillDescendantsWithOptions(pid, &KillDescendantsOptions{
		Signal:    signal,
		MaxLevels: &maxLevels,
		Filter:    filter,
	})
}

// KillDescendantsWithOptions sends a signal to descendants using optional
// cpu mode/sample config for filter evaluation.
func KillDescendantsWithOptions(pid uint32, opts *KillDescendantsOptions) (*KillDescendantsResult, error) {
	signal := 15
	maxLevels := uint32(^uint32(0))
	var filter *ProcessFilter
	cpuMode := CpuModeLifetime
	sampleDuration := time.Duration(0)

	if opts != nil {
		if opts.Signal != 0 {
			signal = opts.Signal
		}
		if opts.MaxLevels != nil {
			maxLevels = *opts.MaxLevels
		}
		filter = opts.Filter
		cpuMode = opts.CpuMode
		sampleDuration = opts.SampleDuration
	}

	configJSON, err := buildDescendantsConfigJSON(filter, cpuMode, sampleDuration)
	if err != nil {
		return nil, err
	}

	var configCStr *C.char
	if configJSON != "" {
		configCStr = C.CString(configJSON)
		defer C.free(unsafe.Pointer(configCStr))
	}

	var resultCStr *C.char
	if err := callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_proc_kill_descendants_ex(
			C.uint32_t(pid),
			C.uint32_t(maxLevels),
			C.int32_t(signal),
			configCStr,
			&resultCStr,
		)
	}); err != nil {
		return nil, err
	}
	defer C.sysprims_free_string(resultCStr)

	var result KillDescendantsResult
	if err := json.Unmarshal([]byte(C.GoString(resultCStr)), &result); err != nil {
		return nil, &Error{Code: ErrInternal, Message: "failed to parse response: " + err.Error()}
	}

	return &result, nil
}

// ListeningPorts returns a snapshot of listening ports, optionally filtered.
//
// Best-effort behavior:
//   - If successful, the returned snapshot may include warnings and may omit PIDs
//     or process attribution for some bindings.
//   - On macOS, SIP/TCC can restrict socket attribution even for same-user
//     processes. In those environments, callers should treat results as best-effort
//     and fall back to platform tooling if required.
//
// # Errors
//
//   - [ErrInvalidArgument]: Filter is invalid
//   - [ErrPermissionDenied]: The platform denies even self inspection
//   - [ErrNotSupported]: Port attribution is not supported on this platform
func ListeningPorts(filter *PortFilter) (*PortBindingsSnapshot, error) {
	var filterCStr *C.char
	if filter != nil {
		filterJSON, err := json.Marshal(filter)
		if err != nil {
			return nil, &Error{Code: ErrInvalidArgument, Message: "failed to marshal filter: " + err.Error()}
		}
		filterCStr = C.CString(string(filterJSON))
		defer C.free(unsafe.Pointer(filterCStr))
	}

	var resultCStr *C.char
	if err := callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_proc_listening_ports(filterCStr, &resultCStr)
	}); err != nil {
		return nil, err
	}
	defer C.sysprims_free_string(resultCStr)

	var snapshot PortBindingsSnapshot
	if err := json.Unmarshal([]byte(C.GoString(resultCStr)), &snapshot); err != nil {
		return nil, &Error{Code: ErrInternal, Message: "failed to parse response: " + err.Error()}
	}

	return &snapshot, nil
}
