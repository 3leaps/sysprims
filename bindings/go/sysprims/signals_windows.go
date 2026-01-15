//go:build windows

package sysprims

// Windows does not support POSIX signal semantics. sysprims maps SIGTERM/SIGKILL
// to TerminateProcess and treats others as not supported.
//
// These values are provided for API completeness and align with common POSIX
// numbers (Linux), but callers should not expect them to be deliverable on Windows.
const (
	SIGHUP  = 1
	SIGQUIT = 3
	SIGUSR1 = 10
	SIGUSR2 = 12
)
