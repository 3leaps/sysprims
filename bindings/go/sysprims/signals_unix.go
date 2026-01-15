//go:build !windows

package sysprims

import "syscall"

// Unix-only signal constants.
//
// These use Go's per-platform syscall constants so values match the host OS
// (e.g., SIGUSR1 is 30 on macOS, 10 on Linux).
const (
	SIGHUP  = int(syscall.SIGHUP)
	SIGQUIT = int(syscall.SIGQUIT)
	SIGUSR1 = int(syscall.SIGUSR1)
	SIGUSR2 = int(syscall.SIGUSR2)
)
