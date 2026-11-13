// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build windows

package signal

import (
	"errors"
	"fmt"
	"strconv"
	"sync/atomic"
	"syscall"

	"github.com/3leaps/sysprims/syserror"
	"golang.org/x/sys/windows"
)

// WithForcedCheckedNumericForTest is a no-op on Windows (used for Linux
// ENOSYS fallback testing).
func WithForcedCheckedNumericForTest(fn func()) {
	fn()
}

type platformProcessHandle struct {
	handle atomic.Uintptr
}

func closeRawHandle(raw uintptr) {
	if raw != 0 {
		_ = windows.CloseHandle(windows.Handle(raw))
	}
}

func winErrnoCode(err error) int32 {
	var errno syscall.Errno
	if errors.As(err, &errno) {
		return int32(errno)
	}
	return 0
}

func readHandleCreationTime(handle windows.Handle, pid uint32, op string) (string, error) {
	var creation, exit, kernel, user windows.Filetime
	if err := windows.GetProcessTimes(handle, &creation, &exit, &kernel, &user); err != nil {
		var errno syscall.Errno
		if errors.As(err, &errno) {
			switch errno {
			case windows.ERROR_ACCESS_DENIED:
				return "", syserror.PermissionDenied(pid, op)
			case windows.ERROR_INVALID_PARAMETER:
				return "", syserror.NotFound(pid)
			default:
				return "", syserror.System("GetProcessTimes failed", int32(errno))
			}
		}
		return "", syserror.System("GetProcessTimes failed", 0)
	}
	ticks := (uint64(creation.HighDateTime) << 32) | uint64(creation.LowDateTime)
	return strconv.FormatUint(ticks, 10), nil
}

func readPIDCreationTime(pid uint32, op string) (string, error) {
	h, err := windows.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
	if err != nil {
		var errno syscall.Errno
		if errors.As(err, &errno) {
			switch errno {
			case windows.ERROR_ACCESS_DENIED:
				return "", syserror.PermissionDenied(pid, op)
			case windows.ERROR_INVALID_PARAMETER:
				return "", syserror.NotFound(pid)
			default:
				return "", syserror.System("OpenProcess failed", int32(errno))
			}
		}
		return "", syserror.System("OpenProcess failed", 0)
	}
	defer func() {
		_ = windows.CloseHandle(h)
	}()
	return readHandleCreationTime(h, pid, op)
}

func openPlatformHandle(pid uint32, expectedStartIdentity string) (string, SignalBinding, *platformProcessHandle, error) {
	birth0, err := readPIDCreationTime(pid, "open_process_handle")
	if err != nil {
		return "", "", nil, err
	}
	if expectedStartIdentity != "" && birth0 != expectedStartIdentity {
		return "", "", nil, syserror.NotFound(pid)
	}

	access := uint32(windows.PROCESS_TERMINATE | windows.SYNCHRONIZE | windows.PROCESS_QUERY_LIMITED_INFORMATION)
	handle, err := windows.OpenProcess(access, false, pid)
	if err != nil {
		var errno syscall.Errno
		if errors.As(err, &errno) {
			switch errno {
			case windows.ERROR_ACCESS_DENIED:
				return "", "", nil, syserror.PermissionDenied(pid, "open_process_handle")
			case windows.ERROR_INVALID_PARAMETER:
				return "", "", nil, syserror.NotFound(pid)
			default:
				return "", "", nil, syserror.System("OpenProcess failed", int32(errno))
			}
		}
		return "", "", nil, syserror.System("OpenProcess failed", 0)
	}
	_ = windows.SetHandleInformation(handle, windows.HANDLE_FLAG_INHERIT, 0)

	handleBirth, err := readHandleCreationTime(handle, pid, "open_process_handle")
	if err != nil {
		_ = windows.CloseHandle(handle)
		return "", "", nil, err
	}
	waitEvent, wErr := windows.WaitForSingleObject(handle, 0)
	if wErr == nil && waitEvent == windows.WAIT_OBJECT_0 || handleBirth != birth0 {
		_ = windows.CloseHandle(handle)
		return "", "", nil, syserror.NotFound(pid)
	}

	birth1, err := readPIDCreationTime(pid, "open_process_handle")
	if err != nil {
		_ = windows.CloseHandle(handle)
		return "", "", nil, err
	}
	if birth1 != birth0 {
		_ = windows.CloseHandle(handle)
		return "", "", nil, syserror.NotFound(pid)
	}

	ph := &platformProcessHandle{}
	ph.handle.Store(uintptr(handle))
	return birth0, SignalBindingBound, ph, nil
}

func (p *platformProcessHandle) closeHandle() {
	raw := p.handle.Swap(0)
	closeRawHandle(raw)
}

func (p *platformProcessHandle) checkExited(_ uint32, _ string) (bool, error) {
	raw := p.handle.Load()
	if raw == 0 {
		return false, syserror.InvalidArgument("process handle is closed")
	}
	ev, err := windows.WaitForSingleObject(windows.Handle(raw), 0)
	if err != nil {
		return false, syserror.System("WaitForSingleObject failed", winErrnoCode(err))
	}
	return ev == windows.WAIT_OBJECT_0, nil
}

func (p *platformProcessHandle) sendSignal(pid uint32, startIdentity string, sig int) (SignalBinding, error) {
	raw := p.handle.Load()
	if raw == 0 {
		return "", syserror.InvalidArgument("process handle is closed")
	}
	handle := windows.Handle(raw)
	if ev, err := windows.WaitForSingleObject(handle, 0); err == nil && ev == windows.WAIT_OBJECT_0 {
		return "", syserror.NotFound(pid)
	}

	switch sig {
	case SIGTERM, SIGKILL:
		if err := windows.TerminateProcess(handle, 1); err != nil {
			if ev, wErr := windows.WaitForSingleObject(handle, 0); wErr == nil && ev == windows.WAIT_OBJECT_0 {
				return "", syserror.NotFound(pid)
			}
			return "", syserror.System("TerminateProcess failed", winErrnoCode(err))
		}
		return SignalBindingBound, nil

	case SIGINT:
		currentBirth, err := readPIDCreationTime(pid, "signal")
		if err != nil {
			return "", err
		}
		if currentBirth != startIdentity {
			return "", syserror.NotFound(pid)
		}
		if err := windows.GenerateConsoleCtrlEvent(windows.CTRL_C_EVENT, pid); err != nil {
			var errno syscall.Errno
			if errors.As(err, &errno) {
				switch errno {
				case windows.ERROR_ACCESS_DENIED:
					return "", syserror.PermissionDenied(pid, "signal")
				case windows.ERROR_INVALID_PARAMETER:
					return "", syserror.NotFound(pid)
				default:
					return "", syserror.System("GenerateConsoleCtrlEvent failed", int32(errno))
				}
			}
			return "", syserror.System("GenerateConsoleCtrlEvent failed", 0)
		}
		return SignalBindingCheckedNumeric, nil

	default:
		return "", syserror.NotSupported(fmt.Sprintf("signal %d", sig), "windows")
	}
}
