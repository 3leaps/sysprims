// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build linux

package timeout

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"unsafe"

	"github.com/3leaps/sysprims/syserror"
	"golang.org/x/sys/unix"
)

type linuxSiginfo struct {
	signo  int32
	errno  int32
	code   int32
	pad0   int32
	pid    int32
	uid    uint32
	status int32
	pad    [24]uint32
}

func verifyOwnedChildSlot(pid uint32) (bool, error) {
	var info linuxSiginfo
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

var (
	linuxBootTimeOnce sync.Once
	linuxBootTimeSecs uint64
)

func getLinuxBootTimeSecs() uint64 {
	linuxBootTimeOnce.Do(func() {
		data, err := os.ReadFile("/proc/stat")
		if err != nil {
			return
		}
		for _, line := range strings.Split(string(data), "\n") {
			if rest, ok := strings.CutPrefix(line, "btime "); ok {
				if v, pErr := strconv.ParseUint(strings.TrimSpace(rest), 10, 64); pErr == nil {
					linuxBootTimeSecs = v
					return
				}
			}
		}
	})
	return linuxBootTimeSecs
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
	statPath := fmt.Sprintf("/proc/%d/stat", pid)
	data, err := os.ReadFile(statPath) // #nosec G304 -- reading kernel procfs entry for validated uint32 PID
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return 0, "", false, syserror.NotFound(pid)
		}
		if errors.Is(err, os.ErrPermission) {
			return 0, "", false, syserror.PermissionDenied(pid, "observe process identity")
		}
		return 0, "", false, syserror.NotFound(pid)
	}
	state, startTicks, ok := parseLinuxStatStartTicks(data)
	if !ok || state == 'Z' || state == 'X' || state == 'x' {
		return 0, "", false, syserror.NotFound(pid)
	}
	bootSecs := getLinuxBootTimeSecs()
	const clockTicks = 100
	startMS := bootSecs*1000 + (startTicks*1000)/clockTicks
	exePath, _ := os.Readlink(fmt.Sprintf("/proc/%d/exe", pid))
	return startMS, exePath, true, nil
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
	statPath := fmt.Sprintf("/proc/%d/stat", pid)
	data, rErr := os.ReadFile(statPath) // #nosec G304 -- reading kernel procfs entry for validated uint32 PID
	if rErr != nil {
		return false, nil
	}
	state, _, ok := parseLinuxStatMembership(data)
	if !ok || state == 'Z' || state == 'X' || state == 'x' {
		return false, nil
	}
	return true, nil
}

func observeContainmentCompletion(pgid, sessionID uint32) ContainmentCompletion {
	procDir, err := os.Open("/proc")
	if err != nil {
		return unknownCompletion(ObservationLinuxProcfsProcessGroup)
	}
	defer func() { _ = procDir.Close() }()

	fd := procDir.Fd()
	fdinfo, err := os.ReadFile(fmt.Sprintf("/proc/self/fdinfo/%d", fd)) // #nosec G304 -- reading own fdinfo for open /proc descriptor
	if err != nil {
		return unknownCompletion(ObservationLinuxProcfsProcessGroup)
	}
	mountinfo, err := os.ReadFile("/proc/self/mountinfo")
	if err != nil {
		return unknownCompletion(ObservationLinuxProcfsProcessGroup)
	}
	mountID, ok := linuxProcfsMountID(fdinfo)
	if !ok {
		return unknownCompletion(ObservationLinuxProcfsProcessGroup)
	}
	procPath := fmt.Sprintf("/proc/self/fd/%d", fd)
	if !linuxProcfsVisibilityIsComplete(mountinfo, mountID) || !linuxProcfsMatchesCallerNamespace(procPath) {
		return unknownCompletion(ObservationLinuxProcfsProcessGroup)
	}
	return stabilizeCompletion(ObservationLinuxProcfsProcessGroup, func() ([]uint32, bool) {
		return linuxGroupSnapshot(procPath, pgid, sessionID)
	})
}

func linuxProcfsMatchesCallerNamespace(procPath string) bool {
	stat, err := os.ReadFile(filepath.Join(procPath, "self/stat")) // #nosec G304 -- reading self/stat under verified /proc fd path
	if err != nil {
		return false
	}
	callerPGID, err1 := unix.Getpgid(0)
	callerSID, err2 := unix.Getsid(0)
	if err1 != nil || err2 != nil || callerPGID <= 0 || callerSID <= 0 {
		return false
	}
	return linuxProcfsIdentityMatches(stat, uint32(os.Getpid()), uint32(callerPGID), uint32(callerSID)) // #nosec G115 -- validated positive PIDs
}

func linuxGroupSnapshot(procPath string, pgid, sessionID uint32) ([]uint32, bool) {
	entries, err := os.ReadDir(procPath)
	if err != nil {
		return nil, false
	}
	members := make([]uint32, 0)
	for _, entry := range entries {
		name := entry.Name()
		pid64, pErr := strconv.ParseUint(name, 10, 32)
		if pErr != nil {
			continue
		}
		pid := uint32(pid64)
		if pid == 0 || pid > MaxSafePID {
			return nil, false
		}
		statPath := filepath.Join(procPath, name, "stat")
		stat, rErr := os.ReadFile(statPath) // #nosec G304 -- reading stat under verified /proc fd path for numeric PID
		if rErr != nil {
			if errors.Is(rErr, os.ErrNotExist) {
				continue
			}
			return nil, false
		}
		state, pgrp, sid, ok := parseLinuxStatStateGroupSession(stat)
		if !ok {
			return nil, false
		}
		if pgrp == pgid && sid == sessionID && state != 'Z' && state != 'X' && state != 'x' {
			if len(members) == MaxCompletionObservedPIDs {
				return nil, false
			}
			members = append(members, pid)
		}
	}
	return members, true
}
