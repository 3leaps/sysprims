// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build darwin

package signal

import (
	"encoding/binary"
	"errors"
	"fmt"
	"math"
	"strconv"
	"sync/atomic"
	"syscall"

	"github.com/3leaps/sysprims/syserror"
	"golang.org/x/sys/unix"
)

const (
	darwinKinfoProcSize64   = 648
	darwinOffPStarttimeSec  = 0
	darwinOffPStarttimeUsec = 8
	darwinOffPStat          = 36
	darwinOffPPID           = 40

	darwinSIDL   byte = 1
	darwinSRUN   byte = 2
	darwinSSLEEP byte = 3
	darwinSSTOP  byte = 4
	darwinSZOMB  byte = 5
)

type platformProcessHandle struct {
	kqFD atomic.Int32
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

func readDarwinStartIdentity(pid uint32, op string) (string, bool, error) {
	buf, err := unix.SysctlRaw("kern.proc.pid", int(pid))
	if err != nil {
		if errors.Is(err, unix.ESRCH) {
			return "", false, syserror.NotFound(pid)
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return "", false, syserror.PermissionDenied(pid, op)
		}
		return "", false, syserror.System("sysctl(KERN_PROC_PID) failed", errnoCode(err))
	}
	if len(buf) == 0 {
		return "", false, syserror.NotFound(pid)
	}
	if len(buf) != darwinKinfoProcSize64 {
		return "", false, syserror.System("unexpected kinfo_proc size", int32(unix.EINVAL))
	}

	tvSecBits := binary.LittleEndian.Uint64(buf[darwinOffPStarttimeSec : darwinOffPStarttimeSec+8])
	tvUsecBits := binary.LittleEndian.Uint32(buf[darwinOffPStarttimeUsec : darwinOffPStarttimeUsec+4])
	pStat := buf[darwinOffPStat]
	pPIDBits := binary.LittleEndian.Uint32(buf[darwinOffPPID : darwinOffPPID+4])

	if pPIDBits != pid {
		return "", false, syserror.NotFound(pid)
	}
	if tvSecBits > math.MaxInt64 || tvUsecBits > 999_999 {
		return "", false, syserror.System("invalid kinfo_proc p_starttime", int32(unix.EINVAL))
	}

	var isZombie bool
	switch pStat {
	case darwinSZOMB:
		isZombie = true
	case darwinSRUN, darwinSSLEEP, darwinSSTOP:
		isZombie = false
	case darwinSIDL:
		return "", false, syserror.NotFound(pid)
	default:
		return "", false, syserror.System("unexpected kinfo_proc p_stat", int32(unix.EINVAL))
	}

	startIdentity := fmt.Sprintf("%s.%06d", strconv.FormatUint(tvSecBits, 10), tvUsecBits)
	return startIdentity, isZombie, nil
}

func openPlatformHandle(pid uint32, expectedStartIdentity string) (string, SignalBinding, *platformProcessHandle, error) {
	birth0, zombie0, err := readDarwinStartIdentity(pid, "open_process_handle")
	if err != nil {
		return "", "", nil, err
	}
	if zombie0 {
		return "", "", nil, syserror.NotFound(pid)
	}
	if expectedStartIdentity != "" && birth0 != expectedStartIdentity {
		return "", "", nil, syserror.NotFound(pid)
	}

	// Enforce the same UID/signal permission rule as kill(pid, 0) without
	// sending a signal.
	if err := unix.Kill(int(pid), 0); err != nil {
		if errors.Is(err, unix.ESRCH) {
			return "", "", nil, syserror.NotFound(pid)
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return "", "", nil, syserror.PermissionDenied(pid, "open_process_handle")
		}
		return "", "", nil, syserror.System("kill(pid, 0) failed", errnoCode(err))
	}

	syscall.ForkLock.RLock()
	kq, err := unix.Kqueue()
	if err != nil {
		syscall.ForkLock.RUnlock()
		return "", "", nil, syserror.System("kqueue() failed", errnoCode(err))
	}
	unix.CloseOnExec(kq)
	syscall.ForkLock.RUnlock()

	changes := [1]unix.Kevent_t{{
		Ident:  uint64(pid),
		Filter: unix.EVFILT_PROC,
		Flags:  unix.EV_ADD | unix.EV_ENABLE | unix.EV_ONESHOT,
		Fflags: unix.NOTE_EXIT,
	}}
	var outEvs [1]unix.Kevent_t
	var zeroTS unix.Timespec
	var n int
	for {
		n, err = unix.Kevent(kq, changes[:], outEvs[:], &zeroTS)
		if err == nil {
			break
		}
		if errors.Is(err, unix.EINTR) {
			continue
		}
		_ = unix.Close(kq)
		if errors.Is(err, unix.ESRCH) {
			return "", "", nil, syserror.NotFound(pid)
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return "", "", nil, syserror.PermissionDenied(pid, "open_process_handle")
		}
		return "", "", nil, syserror.System("kevent registration failed", errnoCode(err))
	}

	if n > 0 {
		outEv := outEvs[0]
		if (outEv.Flags & unix.EV_ERROR) != 0 {
			_ = unix.Close(kq)
			code := int32(outEv.Data)          // #nosec G115 -- kevent EV_ERROR data holds errno
			switch syscall.Errno(outEv.Data) { // #nosec G115 -- kevent EV_ERROR data holds errno
			case unix.ESRCH:
				return "", "", nil, syserror.NotFound(pid)
			case unix.EPERM, unix.EACCES:
				return "", "", nil, syserror.PermissionDenied(pid, "open_process_handle")
			default:
				return "", "", nil, syserror.System("kevent registration EV_ERROR", code)
			}
		}
		if (outEv.Fflags & unix.NOTE_EXIT) != 0 {
			_ = unix.Close(kq)
			return "", "", nil, syserror.NotFound(pid)
		}
	}

	birth1, zombie1, err := readDarwinStartIdentity(pid, "open_process_handle")
	if err != nil {
		_ = unix.Close(kq)
		return "", "", nil, err
	}
	if zombie1 || birth1 != birth0 {
		_ = unix.Close(kq)
		return "", "", nil, syserror.NotFound(pid)
	}

	ph := &platformProcessHandle{}
	ph.kqFD.Store(int32(kq)) // #nosec G115 -- valid macOS file descriptor fits in int32
	return birth0, SignalBindingCheckedNumeric, ph, nil
}

func (p *platformProcessHandle) closeHandle() {
	fd := p.kqFD.Swap(-1)
	closeRawFD(fd)
}

func (p *platformProcessHandle) checkExited(pid uint32, startIdentity string) (bool, error) {
	kq := p.kqFD.Load()
	if kq < 0 {
		return false, syserror.InvalidArgument("process handle is closed")
	}

	var zeroTS unix.Timespec
	for {
		var outEvs [1]unix.Kevent_t
		n, err := unix.Kevent(int(kq), nil, outEvs[:], &zeroTS)
		if err != nil {
			if errors.Is(err, unix.EINTR) {
				continue
			}
			return false, syserror.System("kevent poll failed", errnoCode(err))
		}
		if n > 0 {
			ev := outEvs[0]
			if (ev.Flags & unix.EV_ERROR) != 0 {
				return false, syserror.System("kevent poll EV_ERROR", int32(ev.Data)) // #nosec G115 -- kevent EV_ERROR data holds errno
			}
			if (ev.Fflags & unix.NOTE_EXIT) != 0 {
				return true, nil
			}
		}
		break
	}

	birth, isZombie, err := readDarwinStartIdentity(pid, "exited")
	if err != nil {
		if syserror.IsCode(err, ErrNotFound) {
			return true, nil
		}
		return false, err
	}
	return isZombie || birth != startIdentity, nil
}

func (p *platformProcessHandle) sendSignal(pid uint32, startIdentity string, sig int) (SignalBinding, error) {
	birth, isZombie, err := readDarwinStartIdentity(pid, "signal")
	if err != nil {
		if syserror.IsCode(err, ErrNotFound) {
			return "", syserror.NotFound(pid)
		}
		return "", err
	}
	if isZombie || birth != startIdentity {
		return "", syserror.NotFound(pid)
	}
	if err := killImpl(pid, sig); err != nil {
		return "", err
	}
	return SignalBindingCheckedNumeric, nil
}
