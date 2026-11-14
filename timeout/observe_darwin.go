// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build darwin

package timeout

import (
	"encoding/binary"
	"errors"
	"syscall"
	"unsafe"

	"github.com/3leaps/sysprims/syserror"
	"golang.org/x/sys/unix"
)

const (
	darwinKinfoProcSize = 648
	darwinSIDL          = 1
	darwinSZOMB         = 5
)

type darwinSiginfo struct {
	signo  int32
	errno  int32
	code   int32
	pid    int32
	uid    uint32
	status int32
	addr   uintptr
	value  uintptr
	band   int64
	pad    [7]uint64
}

func verifyOwnedChildSlot(pid uint32) (bool, error) {
	var info darwinSiginfo
	const pPID = 1
	opts := unix.WEXITED | unix.WNOHANG | unix.WNOWAIT
	_, _, e1 := syscall.Syscall6(
		syscall.SYS_WAITID,
		uintptr(pPID),
		uintptr(pid),
		uintptr(unsafe.Pointer(&info)), // #nosec G103 -- kernel waitid populates stack-allocated siginfo_t
		uintptr(opts),
		0,
		0,
	)
	if e1 != 0 {
		if e1 == syscall.ECHILD {
			return false, syserror.InvalidArgument("owned child reap capability is unavailable; refusing guaranteed containment")
		}
		return false, syserror.System("cannot verify unreaped child ownership: "+e1.Error(), int32(e1)) // #nosec G115 -- POSIX errno fits in int32
	}
	if info.pid == 0 {
		return false, nil
	}
	if uint32(info.pid) == pid { // #nosec G115 -- pid validated positive
		return true, nil
	}
	return false, syserror.InvalidArgument("waitid returned a different child; refusing guaranteed containment")
}

func captureContainmentIdentity(pid uint32, receiptBound bool) (ContainmentIdentity, error) {
	startMS, exePath, present, err := observeProcessIdentity(pid)
	if err != nil {
		return ContainmentIdentity{}, err
	}
	if !present {
		return ContainmentIdentity{}, syserror.NotFound(pid)
	}
	if startMS == 0 {
		return ContainmentIdentity{}, syserror.InvalidArgument("process start time unavailable for containment identity")
	}
	if !receiptBound && exePath == "" {
		return ContainmentIdentity{}, syserror.InvalidArgument("process executable path unavailable for containment identity")
	}
	return ContainmentIdentity{
		PID:             pid,
		StartTimeUnixMS: startMS,
		ExePath:         exePath,
	}, nil
}

func observeProcessIdentity(pid uint32) (uint64, string, bool, error) {
	raw, err := unix.SysctlRaw("kern.proc.pid", int(pid))
	if err != nil {
		if errors.Is(err, syscall.ESRCH) || errors.Is(err, syscall.ENOENT) {
			return 0, "", false, syserror.NotFound(pid)
		}
		if errors.Is(err, syscall.EPERM) || errors.Is(err, syscall.EACCES) {
			return 0, "", false, syserror.PermissionDenied(pid, "observe process identity")
		}
		return 0, "", false, syserror.System("sysctl kern.proc.pid failed: "+err.Error(), errnoCode(err))
	}
	if len(raw) < darwinKinfoProcSize {
		return 0, "", false, syserror.NotFound(pid)
	}
	pStat := raw[36]
	pPID := int32(binary.LittleEndian.Uint32(raw[40:44]))         // #nosec G115 -- pid_t is int32
	if pPID != int32(pid) || pStat == 0 || pStat == darwinSZOMB { // #nosec G115 -- pid <= MaxSafePID
		return 0, "", false, syserror.NotFound(pid)
	}
	tvSec := binary.LittleEndian.Uint64(raw[0:8])
	tvUsec := int32(binary.LittleEndian.Uint32(raw[8:12])) // #nosec G115 -- suseconds_t
	var usec uint64
	if tvUsec > 0 {
		usec = uint64(tvUsec)
	}
	startMS := tvSec*1000 + usec/1000
	exePath := readDarwinExePath(pid)
	return startMS, exePath, true, nil
}

func readDarwinExePath(pid uint32) string {
	raw, err := unix.SysctlRaw("kern.procargs2", int(pid))
	if err != nil || len(raw) < 5 {
		return ""
	}
	pos := 4
	for pos < len(raw) && raw[pos] != 0 {
		pos++
	}
	if pos <= 4 {
		return ""
	}
	return string(raw[4:pos])
}

func isProcessLive(pid uint32) (bool, error) {
	if pid == 0 || pid > MaxSafePID {
		return false, syserror.InvalidArgument("pid must be in 1..=2147483647")
	}
	err := unix.Kill(int(pid), 0)
	if err != nil {
		if errors.Is(err, syscall.ESRCH) {
			return false, nil
		}
		if errors.Is(err, syscall.EPERM) {
			return false, syserror.PermissionDenied(pid, "liveness probe")
		}
		return false, syserror.System("kill(pid, 0) failed", errnoCode(err))
	}

	raw, sErr := unix.SysctlRaw("kern.proc.pid", int(pid))
	if sErr != nil || len(raw) < darwinKinfoProcSize {
		return false, nil
	}
	pStat := raw[36]
	pPID := int32(binary.LittleEndian.Uint32(raw[40:44]))         // #nosec G115 -- pid_t is int32
	if pPID != int32(pid) || pStat == 0 || pStat == darwinSZOMB { // #nosec G115 -- pid <= MaxSafePID
		return false, nil
	}
	return true, nil
}

func observeContainmentCompletion(pgid, _ uint32) ContainmentCompletion {
	return stabilizeCompletion(ObservationMacosLibprocProcessGroup, func() ([]uint32, bool) {
		return macosGroupSnapshot(pgid)
	})
}

func macosGroupSnapshot(pgid uint32) ([]uint32, bool) {
	raw, err := unix.SysctlRaw("kern.proc.pgrp", int(pgid))
	if err != nil {
		if errors.Is(err, syscall.ESRCH) || errors.Is(err, syscall.ENOENT) {
			return []uint32{}, true
		}
		return nil, false
	}
	if len(raw) == 0 {
		return []uint32{}, true
	}
	if len(raw)%darwinKinfoProcSize != 0 {
		return nil, false
	}
	count := len(raw) / darwinKinfoProcSize
	if count >= MaxCompletionObservedPIDs {
		return nil, false
	}
	members := make([]uint32, 0, count)
	for i := 0; i < count; i++ {
		off := i * darwinKinfoProcSize
		rec := raw[off : off+darwinKinfoProcSize]
		pStat := rec[36]
		pPID := int32(binary.LittleEndian.Uint32(rec[40:44])) // #nosec G115 -- pid_t is int32
		if pPID <= 0 {
			continue
		}
		pid := uint32(pPID) // #nosec G115 -- validated pPID > 0
		if pid > MaxSafePID {
			return nil, false
		}
		if pStat == 0 || pStat == darwinSZOMB || pStat == darwinSIDL {
			continue
		}
		live, lErr := isProcessLive(pid)
		if lErr != nil {
			return nil, false
		}
		if live {
			members = append(members, pid)
		}
	}
	return members, true
}
