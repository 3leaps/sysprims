package sysprims

/*
#include "sysprims.h"
*/
import "C"

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
