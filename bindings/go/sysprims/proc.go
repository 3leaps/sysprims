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
	// StateIn filters by process state.
	StateIn []string `json:"state_in,omitempty"`
	// CPUAbove filters to processes using more than this CPU percentage.
	CPUAbove *float64 `json:"cpu_above,omitempty"`
	// MemoryAboveKB filters to processes using more than this memory (KB).
	MemoryAboveKB *uint64 `json:"memory_above_kb,omitempty"`
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
		return C.sysprims_proc_list(filterCStr, &resultCStr)
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
	var resultCStr *C.char
	if err := callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_proc_get(C.uint32_t(pid), &resultCStr)
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
