package sysprims

/*
#include "sysprims.h"
*/
import "C"

import "math"

const (
	SIGINT  = 2  // Interrupt
	SIGKILL = 9  // Kill (cannot be caught)
	SIGTERM = 15 // Terminate (graceful)
)

// Kill sends a signal to a process.
//
// On Unix, this calls kill(pid, signal).
// On Windows, SIGTERM and SIGKILL are mapped to TerminateProcess.
// Other signals return [ErrNotSupported] on Windows.
//
// # Arguments
//
//   - pid: Process ID (must be > 0 and <= math.MaxInt32)
//   - signal: Signal number (e.g., [SIGTERM], [SIGKILL])
//
// # Errors
//
//   - [ErrInvalidArgument]: pid is 0 or > math.MaxInt32
//   - [ErrNotFound]: Process doesn't exist
//   - [ErrPermissionDenied]: Not permitted to signal this process
//   - [ErrNotSupported]: Signal not supported on this platform
func Kill(pid uint32, signal int) error {
	return callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_signal_send(C.uint32_t(pid), C.int32_t(signal))
	})
}

type BatchKillFailure struct {
	PID   uint32
	Error *Error
}

type BatchKillResult struct {
	Succeeded []uint32
	Failed    []BatchKillFailure
}

func validatePidList(pids []uint32) error {
	if len(pids) == 0 {
		return &Error{Code: ErrInvalidArgument, Message: "pids must not be empty"}
	}
	for _, pid := range pids {
		if pid == 0 {
			return &Error{Code: ErrInvalidArgument, Message: "pid must be > 0"}
		}
		if pid > uint32(math.MaxInt32) {
			return &Error{Code: ErrInvalidArgument, Message: "pid exceeds maximum safe value"}
		}
	}
	return nil
}

// KillMany sends a signal to multiple processes.
//
// PID validation happens for the entire slice before any signals are sent.
// Individual send failures are collected and returned in the aggregate result.
//
// This is implemented in Go (not a single FFI call) to avoid introducing new
// FFI surface area.
func KillMany(pids []uint32, signal int) (*BatchKillResult, error) {
	if err := validatePidList(pids); err != nil {
		return nil, err
	}

	r := &BatchKillResult{}
	for _, pid := range pids {
		err := Kill(pid, signal)
		if err == nil {
			r.Succeeded = append(r.Succeeded, pid)
			continue
		}

		sErr, ok := err.(*Error)
		if !ok {
			return nil, err
		}
		r.Failed = append(r.Failed, BatchKillFailure{PID: pid, Error: sErr})
	}

	return r, nil
}

// TerminateMany sends SIGTERM to multiple processes.
func TerminateMany(pids []uint32) (*BatchKillResult, error) {
	return KillMany(pids, SIGTERM)
}

// ForceKillMany sends SIGKILL to multiple processes.
func ForceKillMany(pids []uint32) (*BatchKillResult, error) {
	return KillMany(pids, SIGKILL)
}

// Terminate sends SIGTERM to a process.
//
// This is a convenience wrapper for Kill(pid, SIGTERM).
//
// On Windows, this calls TerminateProcess.
func Terminate(pid uint32) error {
	return callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_terminate(C.uint32_t(pid))
	})
}

// ForceKill sends SIGKILL to a process.
//
// This is a convenience wrapper for Kill(pid, SIGKILL).
//
// On Unix, SIGKILL cannot be caught or ignored.
// On Windows, this calls TerminateProcess.
func ForceKill(pid uint32) error {
	return callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_force_kill(C.uint32_t(pid))
	})
}

// KillGroup sends a signal to a process group.
//
// On Unix, this calls killpg(pgid, signal).
//
// # Platform Notes
//
// This function returns [ErrNotSupported] on Windows, which does not
// have the concept of process groups. Use Job Objects via [RunWithTimeout]
// with [GroupByDefault] for tree-kill behavior on Windows.
//
// # Arguments
//
//   - pgid: Process group ID (must be > 0 and <= math.MaxInt32)
//   - signal: Signal number
//
// # Errors
//
//   - [ErrInvalidArgument]: pgid is invalid
//   - [ErrNotSupported]: Always on Windows
func KillGroup(pgid uint32, signal int) error {
	return callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_signal_send_group(C.uint32_t(pgid), C.int32_t(signal))
	})
}
