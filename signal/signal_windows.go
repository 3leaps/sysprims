// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build windows

package signal

import (
	"errors"
	"fmt"
	"syscall"

	"github.com/3leaps/sysprims/syserror"
	"golang.org/x/sys/windows"
)

func killImpl(pid uint32, sig int) error {
	switch sig {
	case SIGTERM, SIGKILL:
		handle, err := windows.OpenProcess(windows.PROCESS_TERMINATE, false, pid)
		if err != nil {
			var errno syscall.Errno
			if errors.As(err, &errno) {
				switch errno {
				case windows.ERROR_ACCESS_DENIED:
					return syserror.PermissionDenied(pid, "terminate")
				case windows.ERROR_INVALID_PARAMETER:
					return syserror.NotFound(pid)
				default:
					return syserror.System("OpenProcess failed", int32(errno))
				}
			}
			return syserror.System("OpenProcess failed", 0)
		}
		defer func() {
			_ = windows.CloseHandle(handle)
		}()

		if err := windows.TerminateProcess(handle, 1); err != nil {
			var errno syscall.Errno
			if errors.As(err, &errno) {
				return syserror.System("TerminateProcess failed", int32(errno))
			}
			return syserror.System("TerminateProcess failed", 0)
		}
		return nil

	case SIGINT:
		if err := windows.GenerateConsoleCtrlEvent(windows.CTRL_C_EVENT, pid); err != nil {
			var errno syscall.Errno
			if errors.As(err, &errno) {
				switch errno {
				case windows.ERROR_ACCESS_DENIED:
					return syserror.PermissionDenied(pid, "signal")
				case windows.ERROR_INVALID_PARAMETER:
					return syserror.NotFound(pid)
				default:
					return syserror.System("GenerateConsoleCtrlEvent failed", int32(errno))
				}
			}
			return syserror.System("GenerateConsoleCtrlEvent failed", 0)
		}
		return nil

	default:
		return syserror.NotSupported(fmt.Sprintf("signal %d", sig), "windows")
	}
}

func killGroupImpl(_ uint32, _ int) error {
	return syserror.NotSupported("killpg", "windows")
}
