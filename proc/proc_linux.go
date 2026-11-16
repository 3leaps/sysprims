//go:build linux

package proc

import (
	"errors"
	"fmt"
	"io"
	"math"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/3leaps/sysprims/syserror"
	"golang.org/x/sys/unix"
)

const (
	linuxClkTck            uint64 = 100
	maxProcSmallFileBytes  int64  = 256 * 1024
	maxProcMediumFileBytes int64  = maxEnvTotalBytes + 1
	maxProcNetFileBytes    int64  = 8 * 1024 * 1024
)

func readProcFileBounded(path string, maxBytes int64) ([]byte, error) {
	f, err := os.Open(path) // #nosec G304 G703 -- bounded /proc pseudo-filesystem read
	if err != nil {
		return nil, err
	}
	defer func() { _ = f.Close() }()
	return io.ReadAll(io.LimitReader(f, maxBytes))
}

var (
	bootTimeOnce sync.Once
	bootTimeSecs uint64
	hasBootTime  bool
)

func getBootTimeSecsLinux() (uint64, bool) {
	bootTimeOnce.Do(func() {
		data, err := readProcFileBounded("/proc/stat", maxProcMediumFileBytes)
		if err != nil {
			return
		}
		bootTimeSecs, hasBootTime = parseLinuxBootTimeSecs(string(data))
	})
	return bootTimeSecs, hasBootTime
}

func getUptimeSecsLinux() uint64 {
	var si unix.Sysinfo_t
	if err := unix.Sysinfo(&si); err == nil && si.Uptime > 0 {
		return uint64(si.Uptime) // #nosec G115 -- si.Uptime > 0 checked
	}
	if data, err := readProcFileBounded("/proc/uptime", maxProcSmallFileBytes); err == nil {
		fields := strings.Fields(string(data))
		if len(fields) >= 1 {
			if v, err := strconv.ParseFloat(fields[0], 64); err == nil && v > 0 {
				return uint64(v)
			}
		}
	}
	return 0
}

func mapReadErrorLinux(pid uint32, err error, action string) error {
	if errors.Is(err, os.ErrNotExist) || errors.Is(err, unix.ESRCH) {
		return syserror.NotFound(pid)
	}
	if errors.Is(err, os.ErrPermission) || errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
		return syserror.PermissionDenied(pid, action)
	}
	return syserror.System(err.Error(), 0)
}

func readProcessInfoLinux(pid uint32, opts ProcessOptions) (*ProcessInfo, error) {
	procDir := fmt.Sprintf("/proc/%d", pid)
	statBytes, err := readProcFileBounded(filepath.Join(procDir, "stat"), maxProcSmallFileBytes)
	if err != nil {
		return nil, mapReadErrorLinux(pid, err, "read /proc/pid/stat")
	}
	stat, err := parseLinuxStat(string(statBytes))
	if err != nil {
		return nil, err
	}

	var userPtr *string
	var threadCount *uint32
	if statusBytes, err := readProcFileBounded(filepath.Join(procDir, "status"), maxProcSmallFileBytes); err == nil {
		uid, hasUID, threads, hasThreads := parseLinuxStatus(string(statusBytes))
		if hasUID {
			userPtr = lookupUsername(uid)
		}
		if opts.IncludeThreads && hasThreads {
			tc := threads
			threadCount = &tc
		}
	}

	pageSizeKB := uint64(os.Getpagesize()) / 1024 // #nosec G115 -- page size > 0
	if pageSizeKB == 0 {
		pageSizeKB = 4
	}
	var memoryKB uint64
	if statmBytes, err := readProcFileBounded(filepath.Join(procDir, "statm"), maxProcSmallFileBytes); err == nil {
		rssPages := parseLinuxStatmRSSPages(string(statmBytes))
		memoryKB = rssPages * pageSizeKB
	}

	uptimeSecs := getUptimeSecsLinux()
	startSecsSinceBoot := stat.StartTicks / linuxClkTck
	var elapsedSeconds uint64
	if uptimeSecs > startSecsSinceBoot {
		elapsedSeconds = uptimeSecs - startSecsSinceBoot
	}

	var startTimeUnixMS *uint64
	if btime, ok := getBootTimeSecsLinux(); ok {
		ms := btime*1000 + (stat.StartTicks*1000)/linuxClkTck
		startTimeUnixMS = &ms
	}

	var cpuPercent float64
	if elapsedSeconds > 0 {
		totalTicks := stat.UTimeTicks + stat.STimeTicks
		cpuSecs := float64(totalTicks) / float64(linuxClkTck)
		numCPUs := runtime.NumCPU()
		if numCPUs < 1 {
			numCPUs = 1
		}
		pct := (cpuSecs / float64(elapsedSeconds)) * 100.0 / float64(numCPUs)
		if pct < 0.0 {
			pct = 0.0
		}
		if pct > 100.0 {
			pct = 100.0
		}
		cpuPercent = math.Round(pct*100.0) / 100.0
	}

	var exePath *string
	if link, err := os.Readlink(filepath.Join(procDir, "exe")); err == nil && link != "" {
		clean := strings.TrimSuffix(link, " (deleted)")
		exePath = &clean
	}

	var cmdline []string
	if cmdBytes, err := readProcFileBounded(filepath.Join(procDir, "cmdline"), maxProcMediumFileBytes); err == nil {
		cmdline = parseNulSeparatedStrings(cmdBytes)
	} else {
		cmdline = []string{}
	}

	var env map[string]string
	if opts.IncludeEnv {
		if envBytes, err := readProcFileBounded(filepath.Join(procDir, "environ"), maxProcMediumFileBytes); err == nil {
			env = parseNulSeparatedEnv(envBytes)
		}
	}

	name := guardDiscoveryName(cmdline, stat.Name)

	return &ProcessInfo{
		PID:             pid,
		PPID:            stat.PPID,
		Name:            name,
		User:            userPtr,
		CPUPercent:      cpuPercent,
		MemoryKB:        memoryKB,
		ElapsedSeconds:  elapsedSeconds,
		StartTimeUnixMS: startTimeUnixMS,
		ExePath:         exePath,
		State:           stat.State,
		Cmdline:         cmdline,
		Env:             env,
		ThreadCount:     threadCount,
	}, nil
}

func getProcessImpl(pid uint32, opts ProcessOptions) (*ProcessInfo, error) {
	return readProcessInfoLinux(pid, opts)
}

func snapshotAllImpl(opts ProcessOptions) ([]ProcessInfo, error) {
	entries, err := os.ReadDir("/proc")
	if err != nil {
		return nil, syserror.System("failed to read /proc: "+err.Error(), 0)
	}
	procs := make([]ProcessInfo, 0, len(entries))
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		pidVal, err := strconv.ParseUint(entry.Name(), 10, 32)
		if err != nil || pidVal == 0 || uint32(pidVal) > MaxSafePID { // #nosec G115 -- ParseUint bitSize 32
			continue
		}
		if info, err := readProcessInfoLinux(uint32(pidVal), opts); err == nil && info != nil { // #nosec G115 -- ParseUint bitSize 32
			procs = append(procs, *info)
		}
	}
	return procs, nil
}

func livenessImpl(pid uint32) Liveness {
	statBytes, err := readProcFileBounded(fmt.Sprintf("/proc/%d/stat", pid), maxProcSmallFileBytes)
	if err != nil {
		if errors.Is(err, os.ErrPermission) || errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return Liveness{Live: true, TablePresent: true}
		}
		return Liveness{Live: false, TablePresent: false}
	}
	stat, err := parseLinuxStat(string(statBytes))
	if err != nil {
		return Liveness{Live: true, TablePresent: true}
	}
	if stat.State == StateZombie {
		return Liveness{Live: false, TablePresent: true}
	}
	return Liveness{Live: true, TablePresent: true}
}

func waitPIDImpl(pid uint32, timeoutMS uint64) (*WaitPidResult, error) {
	p, err := readProcessInfoLinux(pid, ProcessOptions{})
	if err != nil {
		return nil, err
	}
	if p.State == StateZombie {
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
		cur, err := readProcessInfoLinux(pid, ProcessOptions{})
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
		if cur.State == StateZombie {
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
	statBytes, err := readProcFileBounded(fmt.Sprintf("/proc/%d/stat", pid), maxProcSmallFileBytes)
	if err != nil {
		return 0, mapReadErrorLinux(pid, err, "read /proc/pid/stat")
	}
	stat, err := parseLinuxStat(string(statBytes))
	if err != nil {
		return 0, err
	}
	nsPerTick := 1_000_000_000 / linuxClkTck
	return (stat.UTimeTicks + stat.STimeTicks) * nsPerTick, nil
}

func listFdsImpl(pid uint32, filter *FdFilter) (*FdSnapshot, error) {
	procDir := fmt.Sprintf("/proc/%d", pid)
	if _, err := os.Stat(procDir); err != nil {
		return nil, mapReadErrorLinux(pid, err, "stat /proc/pid")
	}
	fdDir := filepath.Join(procDir, "fd")
	entries, err := os.ReadDir(fdDir)
	if err != nil {
		return nil, mapReadErrorLinux(pid, err, "read /proc/pid/fd")
	}

	fds := make([]FdInfo, 0, len(entries))
	permissionDenied := 0
	readErrors := 0

	for _, entry := range entries {
		fdVal, err := strconv.ParseUint(entry.Name(), 10, 32)
		if err != nil {
			continue
		}
		target, err := os.Readlink(filepath.Join(fdDir, entry.Name()))
		if err != nil {
			if errors.Is(err, os.ErrPermission) || errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
				permissionDenied++
			} else if !errors.Is(err, os.ErrNotExist) {
				readErrors++
			}
			continue
		}

		var kind FdKind
		var pathPtr *string
		switch {
		case strings.HasPrefix(target, "socket:[") || strings.HasPrefix(target, "[0000]:"):
			kind = FdKindSocket
		case strings.HasPrefix(target, "pipe:["):
			kind = FdKindPipe
		case strings.HasPrefix(target, "/"):
			kind = FdKindFile
			t := target
			pathPtr = &t
		default:
			kind = FdKindUnknown
		}

		info := FdInfo{
			Fd:   uint32(fdVal), // #nosec G115 -- ParseUint bitSize 32
			Kind: kind,
			Path: pathPtr,
		}
		if filter == nil || filter.Matches(&info) {
			fds = append(fds, info)
		}
	}

	sort.Slice(fds, func(i, j int) bool {
		return fds[i].Fd < fds[j].Fd
	})

	warnings := []string{}
	if w := aggregatePermissionWarning(permissionDenied, "fd entries"); w != "" {
		warnings = append(warnings, w)
	}
	if w := aggregateErrorWarning(readErrors, "fd entries"); w != "" {
		warnings = append(warnings, w)
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

func listeningPortsImpl(filter *PortFilter) (*PortBindingsSnapshot, error) {
	type netSource struct {
		path  string
		proto Protocol
		ipv6  bool
	}
	sources := []netSource{
		{path: "/proc/net/tcp", proto: ProtocolTCP, ipv6: false},
		{path: "/proc/net/tcp6", proto: ProtocolTCP, ipv6: true},
		{path: "/proc/net/udp", proto: ProtocolUDP, ipv6: false},
		{path: "/proc/net/udp6", proto: ProtocolUDP, ipv6: true},
	}

	var netEntries []procNetEntry
	tableReadErrors := 0

	for _, src := range sources {
		if filter != nil && filter.Protocol != nil && *filter.Protocol != src.proto {
			continue
		}
		data, err := readProcFileBounded(src.path, maxProcNetFileBytes)
		if err != nil {
			if !errors.Is(err, os.ErrNotExist) {
				tableReadErrors++
			}
			continue
		}
		parsed, errs := parseProcNetTable(string(data), src.proto, src.ipv6)
		tableReadErrors += errs
		for _, e := range parsed {
			if filter != nil && filter.LocalPort != nil && e.LocalPort != *filter.LocalPort {
				continue
			}
			netEntries = append(netEntries, e)
		}
	}

	// Build inode -> PID map from /proc/<pid>/fd
	inodeToPID := make(map[uint64]uint32)
	procPermDenied := 0
	procReadErrors := 0

	if len(netEntries) > 0 {
		procEntries, err := os.ReadDir("/proc")
		if err == nil {
			for _, pe := range procEntries {
				if !pe.IsDir() {
					continue
				}
				pidVal, err := strconv.ParseUint(pe.Name(), 10, 32)
				if err != nil || pidVal == 0 || uint32(pidVal) > MaxSafePID { // #nosec G115 -- ParseUint bitSize 32
					continue
				}
				pid := uint32(pidVal) // #nosec G115 -- ParseUint bitSize 32
				fdDir := fmt.Sprintf("/proc/%d/fd", pid)
				fds, err := os.ReadDir(fdDir)
				if err != nil {
					if errors.Is(err, os.ErrPermission) || errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
						procPermDenied++
					} else if !errors.Is(err, os.ErrNotExist) {
						procReadErrors++
					}
					continue
				}
				for _, fdEntry := range fds {
					link, err := os.Readlink(filepath.Join(fdDir, fdEntry.Name()))
					if err != nil {
						continue
					}
					if ino, ok := parseLinuxSocketInode(link); ok && ino > 0 {
						if _, exists := inodeToPID[ino]; !exists {
							inodeToPID[ino] = pid
						}
					}
				}
			}
		}
	}

	procCache := make(map[uint32]*ProcessInfo)
	bindings := make([]PortBinding, 0, len(netEntries))

	for _, e := range netEntries {
		var pidPtr *uint32
		var procInfo *ProcessInfo
		if pid, ok := inodeToPID[e.Inode]; ok {
			pidCopy := pid
			pidPtr = &pidCopy
			if cached, found := procCache[pid]; found {
				procInfo = cached
			} else if info, err := readProcessInfoLinux(pid, ProcessOptions{}); err == nil {
				procCache[pid] = info
				procInfo = info
			}
		}
		b := PortBinding{
			Protocol:  e.Protocol,
			LocalAddr: e.LocalAddr,
			LocalPort: e.LocalPort,
			State:     e.State,
			PID:       pidPtr,
			Process:   procInfo,
		}
		if filter == nil || filter.Matches(&b) {
			bindings = append(bindings, b)
		}
	}

	sort.Slice(bindings, func(i, j int) bool {
		if bindings[i].LocalPort != bindings[j].LocalPort {
			return bindings[i].LocalPort < bindings[j].LocalPort
		}
		return bindings[i].Protocol < bindings[j].Protocol
	})

	warnings := []string{}
	if w := aggregateErrorWarning(tableReadErrors, "/proc/net entries"); w != "" {
		warnings = append(warnings, w)
	}
	if w := aggregatePermissionWarning(procPermDenied, "/proc/<pid>/fd directories"); w != "" {
		warnings = append(warnings, w)
	}
	if w := aggregateErrorWarning(procReadErrors, "/proc/<pid>/fd directories"); w != "" {
		warnings = append(warnings, w)
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
