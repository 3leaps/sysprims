//go:build darwin

package proc

import (
	"bytes"
	"encoding/binary"
	"errors"
	"fmt"
	"math"
	"os"
	"runtime"
	"sort"
	"sync"
	"time"
	"unsafe"

	"github.com/3leaps/sysprims/syserror"
	"golang.org/x/sys/unix"
)

// Constants from <sys/proc_info.h> and <libproc.h>
const (
	procInfoCallListPIDs  = 1
	procInfoCallPIDInfo   = 2
	procInfoCallPIDFDInfo = 3

	procAllPIDs = 1

	procPIDListFDs   = 1
	procPIDTBSDInfo  = 3
	procPIDTaskInfo  = 4
	procPIDPathInfo  = 11
	procPIDFDVnode   = 2
	procPIDFDSocket  = 3
	proxFDTypeVnode  = 1
	proxFDTypeSocket = 2
	proxFDTypePipe   = 6

	darwinSIDL   = 1
	darwinSRUN   = 2
	darwinSSLEEP = 3
	darwinSSTOP  = 4
	darwinSZOMB  = 5

	darwinMaxComLen      = 16
	darwinMaxPathLen     = 1024
	darwinPIDPathBufSize = 4 * darwinMaxPathLen
	darwinBSDInfoSize    = 136
	darwinTaskInfoSize   = 96
	darwinFDInfoSize     = 8
)

type darwinBSDInfo struct {
	Flags       uint32
	Status      uint32
	XStatus     uint32
	PID         uint32
	PPID        uint32
	UID         uint32
	GID         uint32
	RUID        uint32
	RGID        uint32
	SVUID       uint32
	SVGID       uint32
	Comm        string
	Name        string
	PGID        uint32
	StartTVSec  uint64
	StartTVUsec uint64
}

type darwinTaskInfo struct {
	VirtualSize  uint64
	ResidentSize uint64
	TotalUser    uint64
	TotalSystem  uint64
	ThreadNum    int32
}

type darwinFDInfo struct {
	FD     int32
	FDType uint32
}

var (
	machTimebaseOnce   sync.Once
	machTimebaseFactor float64
)

func getMachToNsFactor() float64 {
	machTimebaseOnce.Do(func() {
		if freq, err := unix.SysctlUint64("hw.tbfrequency"); err == nil && freq > 0 {
			machTimebaseFactor = 1e9 / float64(freq)
			return
		}
		if runtime.GOARCH == "arm64" {
			machTimebaseFactor = 125.0 / 3.0
		} else {
			machTimebaseFactor = 1.0
		}
	})
	return machTimebaseFactor
}

func machTimeToNs(machTime uint64) uint64 {
	return uint64(float64(machTime) * getMachToNsFactor())
}

// sysProcInfo is the XNU syscall number for proc_info(2) (336 on darwin/arm64 and darwin/amd64).
// Defined locally so CGO_ENABLED=0 builds do not trigger SA1019 on unix.SYS_PROC_INFO.
const sysProcInfo = 336

func procInfoSyscall(callnum int, pid int32, flavor uint32, arg uint64, buf []byte) (int, error) {
	var ptr uintptr
	var size uintptr
	if len(buf) > 0 {
		ptr = uintptr(unsafe.Pointer(&buf[0])) // #nosec G103 -- bounded slice pointer for proc_info syscall
		size = uintptr(len(buf))
	}
	r1, _, errno := unix.Syscall6(
		sysProcInfo,
		uintptr(callnum),
		uintptr(uint32(pid)), // #nosec G115 -- pid validated in [0, MaxSafePID]
		uintptr(flavor),
		uintptr(arg),
		ptr,
		size,
	)
	if errno != 0 {
		return 0, errno
	}
	return int(int32(r1)), nil // #nosec G115 -- proc_info returns int32 byte count
}

func listAllPIDsDarwin() ([]int32, error) {
	byteSize, err := procInfoSyscall(procInfoCallListPIDs, procAllPIDs, 0, 0, nil)
	if err != nil || byteSize <= 0 {
		return nil, syserror.Internal("proc_listpids failed to get size")
	}
	count := (byteSize / 4) + 64
	buf := make([]byte, count*4)
	actual, err := procInfoSyscall(procInfoCallListPIDs, procAllPIDs, 0, 0, buf)
	if err != nil || actual <= 0 {
		return nil, syserror.Internal("proc_listpids failed")
	}
	n := actual / 4
	if n > count {
		n = count
	}
	pids := make([]int32, 0, n)
	for i := 0; i < n; i++ {
		pid := int32(binary.LittleEndian.Uint32(buf[i*4 : (i+1)*4])) // #nosec G115 -- C pid_t is int32
		if pid > 0 {
			pids = append(pids, pid)
		}
	}
	return pids, nil
}

func cStringFromBytes(b []byte) string {
	if idx := bytes.IndexByte(b, 0); idx >= 0 {
		return string(b[:idx])
	}
	return string(b)
}

func getBSDInfoDarwin(pid uint32) (*darwinBSDInfo, error) {
	var buf [darwinBSDInfoSize]byte
	n, err := procInfoSyscall(procInfoCallPIDInfo, int32(pid), procPIDTBSDInfo, 0, buf[:]) // #nosec G115 -- pid validated in [1, MaxSafePID]
	if err != nil || n < darwinBSDInfoSize {
		if errors.Is(err, unix.ESRCH) {
			return nil, syserror.NotFound(pid)
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return nil, syserror.PermissionDenied(pid, "read process BSD info")
		}
		if err == nil && n <= 0 {
			// Verify whether the PID exists via kill(pid, 0)
			if killErr := unix.Kill(int(pid), 0); killErr != nil {
				if errors.Is(killErr, unix.ESRCH) {
					return nil, syserror.NotFound(pid)
				}
				if errors.Is(killErr, unix.EPERM) {
					return nil, syserror.PermissionDenied(pid, "read process BSD info")
				}
			}
			return nil, syserror.NotFound(pid)
		}
		return nil, syserror.Internal("proc_pidinfo(PROC_PIDTBSDINFO) failed")
	}

	info := &darwinBSDInfo{
		Flags:       binary.LittleEndian.Uint32(buf[0:4]),
		Status:      binary.LittleEndian.Uint32(buf[4:8]),
		XStatus:     binary.LittleEndian.Uint32(buf[8:12]),
		PID:         binary.LittleEndian.Uint32(buf[12:16]),
		PPID:        binary.LittleEndian.Uint32(buf[16:20]),
		UID:         binary.LittleEndian.Uint32(buf[20:24]),
		GID:         binary.LittleEndian.Uint32(buf[24:28]),
		RUID:        binary.LittleEndian.Uint32(buf[28:32]),
		RGID:        binary.LittleEndian.Uint32(buf[32:36]),
		SVUID:       binary.LittleEndian.Uint32(buf[36:40]),
		SVGID:       binary.LittleEndian.Uint32(buf[40:44]),
		Comm:        cStringFromBytes(buf[48:64]),
		Name:        cStringFromBytes(buf[64:96]),
		PGID:        binary.LittleEndian.Uint32(buf[100:104]),
		StartTVSec:  binary.LittleEndian.Uint64(buf[120:128]),
		StartTVUsec: binary.LittleEndian.Uint64(buf[128:136]),
	}
	return info, nil
}

func getTaskInfoDarwin(pid uint32) (*darwinTaskInfo, error) {
	var buf [darwinTaskInfoSize]byte
	n, err := procInfoSyscall(procInfoCallPIDInfo, int32(pid), procPIDTaskInfo, 0, buf[:]) // #nosec G115 -- pid validated in [1, MaxSafePID]
	if err != nil || n < darwinTaskInfoSize {
		if errors.Is(err, unix.ESRCH) {
			return nil, syserror.NotFound(pid)
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return nil, syserror.PermissionDenied(pid, "read process task info")
		}
		return nil, syserror.Internal("proc_pidinfo(PROC_PIDTASKINFO) failed")
	}
	return &darwinTaskInfo{
		VirtualSize:  binary.LittleEndian.Uint64(buf[0:8]),
		ResidentSize: binary.LittleEndian.Uint64(buf[8:16]),
		TotalUser:    binary.LittleEndian.Uint64(buf[16:24]),
		TotalSystem:  binary.LittleEndian.Uint64(buf[24:32]),
		ThreadNum:    int32(binary.LittleEndian.Uint32(buf[84:88])), // #nosec G115 -- C int32 pti_threadnum
	}, nil
}

func getPIDPathDarwin(pid uint32) *string {
	var buf [darwinPIDPathBufSize]byte
	n, err := procInfoSyscall(procInfoCallPIDInfo, int32(pid), procPIDPathInfo, 0, buf[:]) // #nosec G115 -- pid validated in [1, MaxSafePID]
	if err != nil || n <= 0 {
		return nil
	}
	s := cStringFromBytes(buf[:n])
	if s == "" {
		return nil
	}
	return &s
}

func readProcargsDarwin(pid uint32) []byte {
	if pid == 0 || pid > MaxSafePID {
		return nil
	}
	raw, err := unix.SysctlRaw("kern.procargs2", int(pid))
	if err != nil || len(raw) == 0 {
		return nil
	}
	return raw
}

func calculateCPUPercentDarwin(task *darwinTaskInfo, elapsedSeconds uint64) float64 {
	if task == nil || elapsedSeconds == 0 {
		return 0.0
	}
	totalNs := machTimeToNs(task.TotalUser) + machTimeToNs(task.TotalSystem)
	elapsedNs := elapsedSeconds * 1_000_000_000
	if elapsedNs == 0 {
		return 0.0
	}
	numCPUs := runtime.NumCPU()
	if numCPUs < 1 {
		numCPUs = 1
	}
	pct := (float64(totalNs) / float64(elapsedNs)) * 100.0 / float64(numCPUs)
	if pct < 0.0 {
		pct = 0.0
	}
	if pct > 100.0 {
		pct = 100.0
	}
	return math.Round(pct*100.0) / 100.0
}

func readProcessInfoDarwin(pid uint32, opts ProcessOptions) (*ProcessInfo, error) {
	bsd, err := getBSDInfoDarwin(pid)
	if err != nil {
		return nil, err
	}
	task, _ := getTaskInfoDarwin(pid)

	name := bsd.Name
	if name == "" {
		name = bsd.Comm
	}

	userPtr := lookupUsername(bsd.UID)

	startTimeUnixMS := bsd.StartTVSec*1000 + (bsd.StartTVUsec / 1000)
	nowSec := uint64(time.Now().Unix()) // #nosec G115 -- Unix epoch seconds > 0
	var elapsedSeconds uint64
	if nowSec > bsd.StartTVSec {
		elapsedSeconds = nowSec - bsd.StartTVSec
	}

	exePath := getPIDPathDarwin(pid)
	cpuPercent := calculateCPUPercentDarwin(task, elapsedSeconds)

	var memoryKB uint64
	if task != nil {
		memoryKB = task.ResidentSize / 1024
	}

	var state ProcessState
	switch bsd.Status {
	case darwinSRUN:
		state = StateRunning
	case darwinSSLEEP, darwinSIDL:
		state = StateSleeping
	case darwinSSTOP:
		state = StateStopped
	case darwinSZOMB:
		state = StateZombie
	default:
		state = StateUnknown
	}

	procargs := readProcargsDarwin(pid)
	cmdline := parseDarwinProcargsCmdline(procargs)

	var env map[string]string
	if opts.IncludeEnv && len(procargs) > 0 {
		env = parseDarwinProcargsEnv(procargs)
	}

	var threadCount *uint32
	if opts.IncludeThreads && task != nil && task.ThreadNum > 0 {
		tc := uint32(task.ThreadNum)
		threadCount = &tc
	}

	name = guardDiscoveryName(cmdline, name)

	return &ProcessInfo{
		PID:             pid,
		PPID:            bsd.PPID,
		Name:            name,
		User:            userPtr,
		CPUPercent:      cpuPercent,
		MemoryKB:        memoryKB,
		ElapsedSeconds:  elapsedSeconds,
		StartTimeUnixMS: &startTimeUnixMS,
		ExePath:         exePath,
		State:           state,
		Cmdline:         cmdline,
		Env:             env,
		ThreadCount:     threadCount,
	}, nil
}

func getProcessImpl(pid uint32, opts ProcessOptions) (*ProcessInfo, error) {
	return readProcessInfoDarwin(pid, opts)
}

func snapshotAllImpl(opts ProcessOptions) ([]ProcessInfo, error) {
	pids, err := listAllPIDsDarwin()
	if err != nil {
		return nil, err
	}
	procs := make([]ProcessInfo, 0, len(pids))
	for _, pid := range pids {
		if pid <= 0 {
			continue
		}
		if info, err := readProcessInfoDarwin(uint32(pid), opts); err == nil && info != nil {
			procs = append(procs, *info)
		}
	}
	return procs, nil
}

func livenessImpl(pid uint32) Liveness {
	bsd, err := getBSDInfoDarwin(pid)
	if err == nil && bsd != nil {
		if bsd.Status == darwinSZOMB {
			return Liveness{Live: false, TablePresent: true}
		}
		return Liveness{Live: true, TablePresent: true}
	}
	if syserror.IsCode(err, syserror.ErrPermissionDenied) {
		return Liveness{Live: true, TablePresent: true}
	}
	// Fallback check via kill(pid, 0)
	if killErr := unix.Kill(int(pid), 0); killErr == nil || errors.Is(killErr, unix.EPERM) {
		return Liveness{Live: true, TablePresent: true}
	}
	return Liveness{Live: false, TablePresent: false}
}

func waitPIDImpl(pid uint32, timeoutMS uint64) (*WaitPidResult, error) {
	bsd, err := getBSDInfoDarwin(pid)
	if err != nil {
		return nil, err
	}
	if bsd.Status == darwinSZOMB {
		return &WaitPidResult{
			SchemaID:  WaitPidResultSchemaID,
			Timestamp: nowRFC3339(),
			Platform:  CurrentPlatform(),
			PID:       pid,
			Exited:    true,
			TimedOut:  false,
			Warnings:  []string{"process is zombie; exit code unavailable without parent wait"},
		}, nil
	}

	deadline := time.Now().Add(durationFromTimeoutMS(timeoutMS))
	pollInterval := 10 * time.Millisecond

	for {
		cur, err := getBSDInfoDarwin(pid)
		if err != nil {
			if syserror.IsCode(err, syserror.ErrNotFound) {
				return &WaitPidResult{
					SchemaID:  WaitPidResultSchemaID,
					Timestamp: nowRFC3339(),
					Platform:  CurrentPlatform(),
					PID:       pid,
					Exited:    true,
					TimedOut:  false,
					Warnings:  []string{"exit code unavailable (not a child process)"},
				}, nil
			}
			return nil, err
		}
		if cur.Status == darwinSZOMB {
			return &WaitPidResult{
				SchemaID:  WaitPidResultSchemaID,
				Timestamp: nowRFC3339(),
				Platform:  CurrentPlatform(),
				PID:       pid,
				Exited:    true,
				TimedOut:  false,
				Warnings:  []string{"process became zombie; exit code unavailable without parent wait"},
			}, nil
		}

		now := time.Now()
		if !now.Before(deadline) {
			return &WaitPidResult{
				SchemaID:  WaitPidResultSchemaID,
				Timestamp: nowRFC3339(),
				Platform:  CurrentPlatform(),
				PID:       pid,
				Exited:    false,
				TimedOut:  true,
				Warnings:  []string{},
			}, nil
		}
		remaining := deadline.Sub(now)
		if remaining < pollInterval {
			time.Sleep(remaining)
		} else {
			time.Sleep(pollInterval)
		}
	}
}

func cpuTotalTimeNsImpl(pid uint32) (uint64, error) {
	task, err := getTaskInfoDarwin(pid)
	if err != nil {
		return 0, err
	}
	return machTimeToNs(task.TotalUser) + machTimeToNs(task.TotalSystem), nil
}

func listAllFDsDarwin(pid uint32) ([]darwinFDInfo, error) {
	bufSize := 4096
	const maxBufSize = 1024 * 1024
	for {
		buf := make([]byte, bufSize)
		actual, err := procInfoSyscall(procInfoCallPIDInfo, int32(pid), procPIDListFDs, 0, buf) // #nosec G115 -- pid validated in [1, MaxSafePID]
		if err != nil || actual <= 0 {
			if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
				return nil, syserror.PermissionDenied(pid, "list fds")
			}
			if errors.Is(err, unix.ESRCH) {
				return nil, syserror.NotFound(pid)
			}
			// Check whether the process even exists
			if _, bsdErr := getBSDInfoDarwin(pid); bsdErr != nil {
				return nil, bsdErr
			}
			return nil, syserror.Internal("proc_pidinfo(PROC_PIDLISTFDS) failed")
		}
		if actual < bufSize || bufSize >= maxBufSize {
			count := actual / darwinFDInfoSize
			out := make([]darwinFDInfo, 0, count)
			for i := 0; i < count; i++ {
				off := i * darwinFDInfoSize
				fd := int32(binary.LittleEndian.Uint32(buf[off : off+4])) // #nosec G115 -- C int32 proc_fd
				fdType := binary.LittleEndian.Uint32(buf[off+4 : off+8])
				out = append(out, darwinFDInfo{FD: fd, FDType: fdType})
			}
			return out, nil
		}
		bufSize *= 2
		if bufSize > maxBufSize {
			bufSize = maxBufSize
		}
	}
}

func readVnodeFDPathDarwin(pid uint32, fd int32) *string {
	bufSize := 2048
	const maxBufSize = 64 * 1024
	for {
		buf := make([]byte, bufSize)
		written, err := procInfoSyscall(procInfoCallPIDFDInfo, int32(pid), procPIDFDVnode, uint64(uint32(fd)), buf) // #nosec G115 -- pid validated in [1, MaxSafePID], fd >= 0
		if err != nil || written <= 0 {
			return nil
		}
		if written < bufSize || bufSize >= maxBufSize {
			if written < darwinMaxPathLen {
				return nil
			}
			tail := buf[written-darwinMaxPathLen : written]
			s := cStringFromBytes(tail)
			if s != "" && !bytes.HasSuffix([]byte(s), []byte(" (deleted)")) {
				isDeleted := false
				if written >= 32 {
					mode := binary.LittleEndian.Uint16(buf[28:30])
					nlink := binary.LittleEndian.Uint16(buf[30:32])
					if (mode&unix.S_IFMT) == unix.S_IFREG && nlink == 0 {
						isDeleted = true
					}
				}
				if !isDeleted && len(s) > 0 && s[0] == '/' {
					var st unix.Stat_t
					if lstatErr := unix.Lstat(s, &st); errors.Is(lstatErr, unix.ENOENT) {
						isDeleted = true
					}
				}
				if isDeleted {
					s += " (deleted)"
				}
			}
			return &s
		}
		bufSize *= 2
		if bufSize > maxBufSize {
			bufSize = maxBufSize
		}
	}
}

func listFdsImpl(pid uint32, filter *FdFilter) (*FdSnapshot, error) {
	rawFDs, err := listAllFDsDarwin(pid)
	if err != nil {
		return nil, err
	}

	fds := make([]FdInfo, 0, len(rawFDs))
	pathMissing := 0

	for _, info := range rawFDs {
		if info.FD < 0 {
			continue
		}
		var kind FdKind
		switch info.FDType {
		case proxFDTypeVnode:
			kind = FdKindFile
		case proxFDTypeSocket:
			kind = FdKindSocket
		case proxFDTypePipe:
			kind = FdKindPipe
		default:
			kind = FdKindUnknown
		}

		var path *string
		if kind == FdKindFile {
			path = readVnodeFDPathDarwin(pid, info.FD)
			if path == nil {
				pathMissing++
			}
		}

		entry := FdInfo{
			Fd:   uint32(info.FD), // #nosec G115 -- info.FD >= 0 checked above
			Kind: kind,
			Path: path,
		}
		if filter == nil || filter.Matches(&entry) {
			fds = append(fds, entry)
		}
	}

	sort.Slice(fds, func(i, j int) bool {
		return fds[i].Fd < fds[j].Fd
	})

	warnings := []string{}
	if pathMissing > 0 {
		warnings = append(warnings, fmt.Sprintf("Failed to resolve paths for %d file fds", pathMissing))
	}
	if len(fds) == 0 {
		warnings = append(warnings, "No file descriptors visible")
	}

	return &FdSnapshot{
		SchemaID:  FdSnapshotSchemaID,
		Timestamp: nowRFC3339(),
		Platform:  CurrentPlatform(),
		PID:       pid,
		Fds:       fds,
		Warnings:  warnings,
	}, nil
}

func readSocketBindingDarwin(pid uint32, fd int32) (*PortBinding, error) {
	var buf [2048]byte
	written, err := procInfoSyscall(procInfoCallPIDFDInfo, int32(pid), procPIDFDSocket, uint64(uint32(fd)), buf[:]) // #nosec G115 -- pid validated in [1, MaxSafePID], fd >= 0
	if err != nil || written <= 0 {
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return nil, syserror.PermissionDenied(pid, "read socket info")
		}
		return nil, syserror.Internal("proc_pidfdinfo socket failed")
	}
	binding, err := parseDarwinSocketFdInfo(buf[:written], pid)
	if err != nil {
		return nil, err
	}
	if procInfo, err := readProcessInfoDarwin(pid, ProcessOptions{}); err == nil {
		binding.Process = procInfo
	}
	return binding, nil
}

func listeningPortsImpl(filter *PortFilter) (*PortBindingsSnapshot, error) {
	pids, err := listAllPIDsDarwin()
	if err != nil {
		return nil, err
	}

	currentUID := uint32(os.Geteuid()) // #nosec G115 -- POSIX uid_t
	var bindings []PortBinding

	permissionDenied := 0
	readErrors := 0
	skippedOtherUser := 0
	socketPermDenied := 0
	socketReadErrors := 0
	socketUnsupported := 0
	socketMissingPort := 0

	for _, rawPID := range pids {
		if rawPID <= 0 {
			continue
		}
		pid := uint32(rawPID) // #nosec G115 -- rawPID > 0 checked above
		bsd, err := getBSDInfoDarwin(pid)
		if err != nil {
			if syserror.IsCode(err, syserror.ErrPermissionDenied) {
				permissionDenied++
			} else {
				readErrors++
			}
			continue
		}
		if bsd.UID != currentUID {
			skippedOtherUser++
			continue
		}

		fds, err := listAllFDsDarwin(pid)
		if err != nil {
			if syserror.IsCode(err, syserror.ErrPermissionDenied) {
				permissionDenied++
			} else {
				readErrors++
			}
			continue
		}

		for _, fdInfo := range fds {
			if fdInfo.FDType != proxFDTypeSocket {
				continue
			}
			b, err := readSocketBindingDarwin(pid, fdInfo.FD)
			if err != nil {
				if syserror.IsCode(err, syserror.ErrPermissionDenied) {
					socketPermDenied++
				} else {
					msg := err.Error()
					if bytes.Contains([]byte(msg), []byte("unsupported socket")) {
						socketUnsupported++
					} else if bytes.Contains([]byte(msg), []byte("no local port")) {
						socketMissingPort++
					} else if bytes.Contains([]byte(msg), []byte("not listening")) {
						// Ignore non-listening TCP sockets silently
					} else {
						socketReadErrors++
					}
				}
				continue
			}
			if filter == nil || filter.Matches(b) {
				bindings = append(bindings, *b)
			}
		}
	}

	sort.Slice(bindings, func(i, j int) bool {
		if bindings[i].LocalPort != bindings[j].LocalPort {
			return bindings[i].LocalPort < bindings[j].LocalPort
		}
		return bindings[i].Protocol < bindings[j].Protocol
	})

	warnings := []string{}
	if skippedOtherUser > 0 {
		warnings = append(warnings, fmt.Sprintf("macos port bindings are best-effort; scanning current user processes only (uid=%d)", currentUID))
		warnings = append(warnings, fmt.Sprintf("Skipped %d pid entries owned by other users", skippedOtherUser))
	}
	if w := aggregatePermissionWarning(permissionDenied, "pid entries"); w != "" {
		warnings = append(warnings, w)
	}
	if w := aggregateErrorWarning(readErrors, "pid entries"); w != "" {
		warnings = append(warnings, w)
	}
	if w := aggregatePermissionWarning(socketPermDenied, "socket entries"); w != "" {
		warnings = append(warnings, w)
	}
	if w := aggregateErrorWarning(socketReadErrors, "socket entries"); w != "" {
		warnings = append(warnings, w)
	}
	if socketUnsupported > 0 {
		warnings = append(warnings, fmt.Sprintf("Skipped %d socket entries due to unsupported socket kinds", socketUnsupported))
	}
	if socketMissingPort > 0 {
		warnings = append(warnings, fmt.Sprintf("Skipped %d socket entries with no local port", socketMissingPort))
	}
	if len(bindings) == 0 {
		warnings = append(warnings, "No listening ports visible")
	}

	return &PortBindingsSnapshot{
		SchemaID:  PortBindingsSchemaID,
		Timestamp: nowRFC3339(),
		Platform:  CurrentPlatform(),
		Bindings:  bindings,
		Warnings:  warnings,
	}, nil
}
