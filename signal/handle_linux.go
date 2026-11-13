// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build linux

package signal

import (
	"errors"
	"fmt"
	"strconv"
	"strings"
	"sync/atomic"
	"syscall"

	"github.com/3leaps/sysprims/syserror"
	"golang.org/x/sys/unix"
)

var forceLinuxCheckedNumeric atomic.Bool

// WithForcedCheckedNumericForTest forces OpenProcessHandle on Linux to use the
// Linux < 5.3 ENOSYS (CheckedNumeric) fallback path for the duration of fn.
func WithForcedCheckedNumericForTest(fn func()) {
	forceLinuxCheckedNumeric.Store(true)
	defer forceLinuxCheckedNumeric.Store(false)
	fn()
}

type platformProcessHandle struct {
	pidfd atomic.Int32
	bound bool
}

func closeRawFD(fd int32) {
	if fd >= 0 {
		_ = unix.Close(int(fd))
	}
}

func errnoCode(err error) int32 {
	var errno syscall.Errno
	if errors.As(err, &errno) {
		return int32(errno)
	}
	return int32(unix.EIO)
}

type linuxStatSnapshot struct {
	startTicks string
	state      byte
	numThreads uint64
}

func (s linuxStatSnapshot) isWholeProcessDead() bool {
	return s.state == 'X' || s.state == 'x' || (s.state == 'Z' && s.numThreads <= 1)
}

func parseProcStatSnapshot(statText string) (linuxStatSnapshot, bool) {
	var zero linuxStatSnapshot
	closeIdx := strings.LastIndex(statText, ") ")
	if closeIdx < 0 {
		return zero, false
	}
	afterComm := statText[closeIdx+2:]
	fields := strings.Fields(afterComm)
	// Field 3 is state (index 0 after ") ").
	// Field 20 is num_threads (index 17 after ") ").
	// Field 22 is starttime (index 19 after ") ").
	if len(fields) <= 19 || len(fields[0]) == 0 {
		return zero, false
	}
	state := fields[0][0]
	numThreads, err := strconv.ParseUint(fields[17], 10, 64)
	if err != nil {
		return zero, false
	}
	startTicksStr := fields[19]
	if _, err := strconv.ParseUint(startTicksStr, 10, 64); err != nil {
		return zero, false
	}
	return linuxStatSnapshot{
		startTicks: startTicksStr,
		state:      state,
		numThreads: numThreads,
	}, true
}

func readLinuxStat(pid uint32, op string) (linuxStatSnapshot, error) {
	var zero linuxStatSnapshot
	path := fmt.Sprintf("/proc/%d/stat", pid)
	var fd int
	var err error
	for {
		fd, err = unix.Open(path, unix.O_RDONLY|unix.O_CLOEXEC|unix.O_NOFOLLOW, 0)
		if err == nil {
			break
		}
		if errors.Is(err, unix.EINTR) {
			continue
		}
		if errors.Is(err, unix.ENOENT) || errors.Is(err, unix.ESRCH) {
			return zero, syserror.NotFound(pid)
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return zero, syserror.PermissionDenied(pid, op)
		}
		return zero, syserror.System("open(/proc/<pid>/stat) failed", errnoCode(err))
	}
	defer func() {
		_ = unix.Close(fd)
	}()

	buf := make([]byte, 4096)
	total := 0
	for total < len(buf) {
		n, rErr := unix.Read(fd, buf[total:])
		if n > 0 {
			total += n
		}
		if rErr != nil {
			if errors.Is(rErr, unix.EINTR) {
				continue
			}
			if errors.Is(rErr, unix.ENOENT) || errors.Is(rErr, unix.ESRCH) {
				return zero, syserror.NotFound(pid)
			}
			if errors.Is(rErr, unix.EPERM) || errors.Is(rErr, unix.EACCES) {
				return zero, syserror.PermissionDenied(pid, op)
			}
			return zero, syserror.System("read(/proc/<pid>/stat) failed", errnoCode(rErr))
		}
		if n == 0 {
			break
		}
	}

	snap, ok := parseProcStatSnapshot(string(buf[:total]))
	if !ok {
		return zero, syserror.System("malformed /proc/<pid>/stat payload", int32(unix.EINVAL))
	}
	return snap, nil
}

func pollPidfdExited(fd int) (bool, error) {
	for {
		pfds := [1]unix.PollFd{{
			Fd:     int32(fd), // #nosec G115 -- valid Linux file descriptor fits in int32
			Events: unix.POLLIN,
		}}
		n, err := unix.Poll(pfds[:], 0)
		if err != nil {
			if errors.Is(err, unix.EINTR) {
				continue
			}
			return false, syserror.System("poll(pidfd) failed", errnoCode(err))
		}
		if n > 0 {
			rev := pfds[0].Revents
			if (rev & unix.POLLNVAL) != 0 {
				return false, syserror.System("poll(pidfd) POLLNVAL", int32(unix.EBADF))
			}
			if (rev & (unix.POLLIN | unix.POLLHUP)) != 0 {
				return true, nil
			}
			if (rev & unix.POLLERR) != 0 {
				return false, syserror.System("poll(pidfd) POLLERR", int32(unix.EIO))
			}
			return false, nil
		}
		return false, nil
	}
}

func openPlatformHandle(pid uint32, expectedStartIdentity string) (string, SignalBinding, *platformProcessHandle, error) {
	snap0, err := readLinuxStat(pid, "open_process_handle")
	if err != nil {
		return "", "", nil, err
	}
	if snap0.isWholeProcessDead() {
		return "", "", nil, syserror.NotFound(pid)
	}
	if expectedStartIdentity != "" && snap0.startTicks != expectedStartIdentity {
		return "", "", nil, syserror.NotFound(pid)
	}

	if err := unix.Kill(int(pid), 0); err != nil {
		if errors.Is(err, unix.ESRCH) {
			return "", "", nil, syserror.NotFound(pid)
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return "", "", nil, syserror.PermissionDenied(pid, "open_process_handle")
		}
		return "", "", nil, syserror.System("kill(pid, 0) failed", errnoCode(err))
	}

	pidfd := -1
	binding := SignalBindingCheckedNumeric
	if forceLinuxCheckedNumeric.Load() {
		if snap0.state == 'Z' {
			return "", "", nil, syserror.NotFound(pid)
		}
	} else {
		var openErr error
		for {
			pidfd, openErr = unix.PidfdOpen(int(pid), 0)
			if openErr == nil {
				break
			}
			if errors.Is(openErr, unix.EINTR) {
				continue
			}
			if errors.Is(openErr, unix.ENOSYS) {
				pidfd = -1
				openErr = nil
				break
			}
			if errors.Is(openErr, unix.ESRCH) {
				return "", "", nil, syserror.NotFound(pid)
			}
			if errors.Is(openErr, unix.EPERM) || errors.Is(openErr, unix.EACCES) {
				return "", "", nil, syserror.PermissionDenied(pid, "open_process_handle")
			}
			return "", "", nil, syserror.System("pidfd_open failed", errnoCode(openErr))
		}

		if pidfd >= 0 {
			if _, fErr := unix.FcntlInt(uintptr(pidfd), unix.F_SETFD, unix.FD_CLOEXEC); fErr != nil {
				_ = unix.Close(pidfd)
				return "", "", nil, syserror.System("fcntl(FD_CLOEXEC) failed", errnoCode(fErr))
			}
			exited, pErr := pollPidfdExited(pidfd)
			if pErr != nil {
				_ = unix.Close(pidfd)
				return "", "", nil, pErr
			}
			if exited {
				_ = unix.Close(pidfd)
				return "", "", nil, syserror.NotFound(pid)
			}
			binding = SignalBindingBound
		} else if snap0.state == 'Z' {
			return "", "", nil, syserror.NotFound(pid)
		}
	}

	snap1, err := readLinuxStat(pid, "open_process_handle")
	if err != nil {
		if pidfd >= 0 {
			_ = unix.Close(pidfd)
		}
		return "", "", nil, err
	}
	if snap1.startTicks != snap0.startTicks || snap1.isWholeProcessDead() {
		if pidfd >= 0 {
			_ = unix.Close(pidfd)
		}
		return "", "", nil, syserror.NotFound(pid)
	}
	if pidfd >= 0 {
		exited, pErr := pollPidfdExited(pidfd)
		if pErr != nil {
			_ = unix.Close(pidfd)
			return "", "", nil, pErr
		}
		if exited {
			_ = unix.Close(pidfd)
			return "", "", nil, syserror.NotFound(pid)
		}
	}

	ph := &platformProcessHandle{
		bound: binding == SignalBindingBound,
	}
	ph.pidfd.Store(int32(pidfd)) // #nosec G115 -- valid Linux file descriptor fits in int32
	return snap0.startTicks, binding, ph, nil
}

func (p *platformProcessHandle) closeHandle() {
	fd := p.pidfd.Swap(-1)
	closeRawFD(fd)
}

func (p *platformProcessHandle) checkExited(pid uint32, startIdentity string) (bool, error) {
	if p.bound {
		fd := p.pidfd.Load()
		if fd < 0 {
			return false, syserror.InvalidArgument("process handle is closed")
		}
		return pollPidfdExited(int(fd))
	}

	snap, err := readLinuxStat(pid, "exited")
	if err != nil {
		if syserror.IsCode(err, ErrNotFound) {
			return true, nil
		}
		return false, err
	}
	return snap.isWholeProcessDead() || snap.startTicks != startIdentity, nil
}

func (p *platformProcessHandle) sendSignal(pid uint32, startIdentity string, sig int) (SignalBinding, error) {
	if p.bound {
		fd := p.pidfd.Load()
		if fd < 0 {
			return "", syserror.InvalidArgument("process handle is closed")
		}
		for {
			err := unix.PidfdSendSignal(int(fd), syscall.Signal(sig), nil, 0)
			if err == nil {
				return SignalBindingBound, nil
			}
			if errors.Is(err, unix.EINTR) {
				continue
			}
			if errors.Is(err, unix.ESRCH) {
				return "", syserror.NotFound(pid)
			}
			if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
				return "", syserror.PermissionDenied(pid, "signal")
			}
			if errors.Is(err, unix.EINVAL) {
				return "", syserror.InvalidArgument(fmt.Sprintf("invalid signal: %d", sig))
			}
			return "", syserror.System("pidfd_send_signal failed", errnoCode(err))
		}
	}

	snap, err := readLinuxStat(pid, "signal")
	if err != nil {
		if syserror.IsCode(err, ErrNotFound) {
			return "", syserror.NotFound(pid)
		}
		return "", err
	}
	if snap.isWholeProcessDead() || snap.startTicks != startIdentity {
		return "", syserror.NotFound(pid)
	}
	if err := killImpl(pid, sig); err != nil {
		return "", err
	}
	return SignalBindingCheckedNumeric, nil
}
