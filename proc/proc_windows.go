//go:build windows

package proc

import (
	"encoding/binary"
	"errors"
	"math"
	"net/netip"
	"runtime"
	"sort"
	"time"
	"unsafe"

	"github.com/3leaps/sysprims/syserror"
	"golang.org/x/sys/windows"
)

const (
	winStillActive           uint32  = 259
	winWindowsEpochDiff100Ns uint64  = 116444736000000000
	tcpTableOwnerPIDListener uint32  = 3
	udpTableOwnerPID         uint32  = 1
	mibTCPStateListen        uint32  = 2
	winAfInet                uint32  = 2
	winAfInet6               uint32  = 23
	errorInsufficientBuffer  uintptr = 122
)

var (
	modpsapi                 = windows.NewLazySystemDLL("psapi.dll")
	procGetProcessMemoryInfo = modpsapi.NewProc("GetProcessMemoryInfo")

	modiphlpapi             = windows.NewLazySystemDLL("iphlpapi.dll")
	procGetExtendedTcpTable = modiphlpapi.NewProc("GetExtendedTcpTable")
	procGetExtendedUdpTable = modiphlpapi.NewProc("GetExtendedUdpTable")
)

type processMemoryCounters struct {
	CB                         uint32
	PageFaultCount             uint32
	PeakWorkingSetSize         uintptr
	WorkingSetSize             uintptr
	QuotaPeakPagedPoolUsage    uintptr
	QuotaPagedPoolUsage        uintptr
	QuotaPeakNonPagedPoolUsage uintptr
	QuotaNonPagedPoolUsage     uintptr
	PagefileUsage              uintptr
	PeakPagefileUsage          uintptr
}

func filetimeTo100Ns(ft windows.Filetime) uint64 {
	return (uint64(ft.HighDateTime) << 32) | uint64(ft.LowDateTime)
}

func enrichProcessWindows(info *ProcessInfo) {
	h, err := windows.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION, false, info.PID)
	if err != nil {
		return
	}
	defer func() { _ = windows.CloseHandle(h) }()

	var creation, exit, kernel, user windows.Filetime
	if err := windows.GetProcessTimes(h, &creation, &exit, &kernel, &user); err == nil {
		create100Ns := filetimeTo100Ns(creation)
		if create100Ns > winWindowsEpochDiff100Ns {
			unix100Ns := create100Ns - winWindowsEpochDiff100Ns
			startMS := unix100Ns / 10000
			info.StartTimeUnixMS = &startMS
			startSec := startMS / 1000
			nowSec := uint64(time.Now().Unix()) // #nosec G115 -- Unix epoch seconds > 0
			if nowSec > startSec {
				info.ElapsedSeconds = nowSec - startSec
			}
		}

		if info.ElapsedSeconds > 0 {
			total100Ns := filetimeTo100Ns(kernel) + filetimeTo100Ns(user)
			cpuSecs := float64(total100Ns) / 1e7
			numCPUs := runtime.NumCPU()
			if numCPUs < 1 {
				numCPUs = 1
			}
			pct := (cpuSecs / float64(info.ElapsedSeconds)) * 100.0 / float64(numCPUs)
			if pct < 0.0 {
				pct = 0.0
			}
			if pct > 100.0 {
				pct = 100.0
			}
			info.CPUPercent = math.Round(pct*100.0) / 100.0
		}
	}

	var pmc processMemoryCounters
	pmc.CB = uint32(unsafe.Sizeof(pmc))
	r1, _, _ := procGetProcessMemoryInfo.Call(
		uintptr(h),
		uintptr(unsafe.Pointer(&pmc)), // #nosec G103 -- K32GetProcessMemoryInfo struct pointer
		uintptr(pmc.CB),
	)
	if r1 != 0 {
		info.MemoryKB = uint64(pmc.WorkingSetSize) / 1024
	}

	var pathBuf [windows.MAX_PATH * 2]uint16
	pathLen := uint32(len(pathBuf))
	if err := windows.QueryFullProcessImageName(h, 0, &pathBuf[0], &pathLen); err == nil && pathLen > 0 {
		s := windows.UTF16ToString(pathBuf[:pathLen])
		if s != "" {
			info.ExePath = &s
			if len(info.Cmdline) == 0 {
				info.Cmdline = []string{s}
			}
		}
	}
}

func entryToProcessInfoWindows(pe *windows.ProcessEntry32, opts ProcessOptions) ProcessInfo {
	name := windows.UTF16ToString(pe.ExeFile[:])
	var threadCount *uint32
	if opts.IncludeThreads && pe.Threads > 0 {
		tc := pe.Threads
		threadCount = &tc
	}
	cmdline := []string{}
	if name != "" {
		cmdline = []string{name}
	}
	info := ProcessInfo{
		PID:         pe.ProcessID,
		PPID:        pe.ParentProcessID,
		Name:        name,
		State:       StateRunning,
		Cmdline:     cmdline,
		ThreadCount: threadCount,
	}
	enrichProcessWindows(&info)
	return info
}

func snapshotAllImpl(opts ProcessOptions) ([]ProcessInfo, error) {
	snap, err := windows.CreateToolhelp32Snapshot(windows.TH32CS_SNAPPROCESS, 0)
	if err != nil {
		return nil, syserror.System("CreateToolhelp32Snapshot failed: "+err.Error(), 0)
	}
	defer func() { _ = windows.CloseHandle(snap) }()

	var pe windows.ProcessEntry32
	pe.Size = uint32(unsafe.Sizeof(pe))
	if err := windows.Process32First(snap, &pe); err != nil {
		return []ProcessInfo{}, nil
	}

	var procs []ProcessInfo
	for {
		if pe.ProcessID > 0 && pe.ProcessID <= MaxSafePID {
			procs = append(procs, entryToProcessInfoWindows(&pe, opts))
		}
		pe.Size = uint32(unsafe.Sizeof(pe))
		if err := windows.Process32Next(snap, &pe); err != nil {
			break
		}
	}
	return procs, nil
}

func getProcessImpl(pid uint32, opts ProcessOptions) (*ProcessInfo, error) {
	snap, err := windows.CreateToolhelp32Snapshot(windows.TH32CS_SNAPPROCESS, 0)
	if err != nil {
		return nil, syserror.System("CreateToolhelp32Snapshot failed: "+err.Error(), 0)
	}
	defer func() { _ = windows.CloseHandle(snap) }()

	var pe windows.ProcessEntry32
	pe.Size = uint32(unsafe.Sizeof(pe))
	if err := windows.Process32First(snap, &pe); err != nil {
		return nil, syserror.NotFound(pid)
	}

	for {
		if pe.ProcessID == pid {
			info := entryToProcessInfoWindows(&pe, opts)
			return &info, nil
		}
		pe.Size = uint32(unsafe.Sizeof(pe))
		if err := windows.Process32Next(snap, &pe); err != nil {
			break
		}
	}
	return nil, syserror.NotFound(pid)
}

func livenessImpl(pid uint32) Liveness {
	h, err := windows.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
	if err != nil {
		if errors.Is(err, windows.ERROR_ACCESS_DENIED) {
			return Liveness{Live: true, TablePresent: true}
		}
		return Liveness{Live: false, TablePresent: false}
	}
	defer func() { _ = windows.CloseHandle(h) }()

	var code uint32
	if err := windows.GetExitCodeProcess(h, &code); err == nil {
		if code == winStillActive {
			return Liveness{Live: true, TablePresent: true}
		}
		return Liveness{Live: false, TablePresent: false}
	}
	return Liveness{Live: true, TablePresent: true}
}

func waitPIDImpl(pid uint32, timeoutMS uint64) (*WaitPidResult, error) {
	h, err := windows.OpenProcess(windows.SYNCHRONIZE|windows.PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
	if err != nil {
		if errors.Is(err, windows.ERROR_INVALID_PARAMETER) {
			return nil, syserror.NotFound(pid)
		}
		if errors.Is(err, windows.ERROR_ACCESS_DENIED) {
			return nil, syserror.PermissionDenied(pid, "OpenProcess for wait_pid")
		}
		return nil, syserror.System(err.Error(), 0)
	}
	defer func() { _ = windows.CloseHandle(h) }()

	if timeoutMS > math.MaxUint32-1 {
		timeoutMS = math.MaxUint32 - 1
	}
	waitMS := uint32(timeoutMS) // #nosec G115 -- clamped to math.MaxUint32-1 above
	event, err := windows.WaitForSingleObject(h, waitMS)
	if err != nil {
		return nil, syserror.System("WaitForSingleObject failed: "+err.Error(), 0)
	}

	if event == windows.WAIT_OBJECT_0 {
		var exitCodePtr *int32
		var code uint32
		if err := windows.GetExitCodeProcess(h, &code); err == nil && code != winStillActive {
			c := int32(code) // #nosec G115 -- Windows process exit code
			exitCodePtr = &c
		}
		return &WaitPidResult{
			SchemaID:  WaitPidResultSchemaID,
			Timestamp: nowRFC3339(),
			Platform:  CurrentPlatform(),
			PID:       pid,
			Exited:    true,
			TimedOut:  false,
			ExitCode:  exitCodePtr,
			Warnings:  []string{},
		}, nil
	}

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

func cpuTotalTimeNsImpl(pid uint32) (uint64, error) {
	h, err := windows.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
	if err != nil {
		if errors.Is(err, windows.ERROR_INVALID_PARAMETER) {
			return 0, syserror.NotFound(pid)
		}
		if errors.Is(err, windows.ERROR_ACCESS_DENIED) {
			return 0, syserror.PermissionDenied(pid, "OpenProcess for cpu_total_time_ns")
		}
		return 0, syserror.System(err.Error(), 0)
	}
	defer func() { _ = windows.CloseHandle(h) }()

	var creation, exit, kernel, user windows.Filetime
	if err := windows.GetProcessTimes(h, &creation, &exit, &kernel, &user); err != nil {
		return 0, syserror.System("GetProcessTimes failed: "+err.Error(), 0)
	}
	total100Ns := filetimeTo100Ns(kernel) + filetimeTo100Ns(user)
	return total100Ns * 100, nil
}

func listFdsImpl(pid uint32, filter *FdFilter) (*FdSnapshot, error) {
	_ = pid
	_ = filter
	return nil, syserror.NotSupported("list_fds", "windows")
}

func fetchExtendedTableWindows(proc *windows.LazyProc, family uint32, tableClass uint32) ([]byte, error) {
	var size uint32
	r1, _, _ := proc.Call(0, uintptr(unsafe.Pointer(&size)), 0, uintptr(family), uintptr(tableClass), 0) // #nosec G103 -- iphlpapi size pointer
	if r1 != errorInsufficientBuffer || size == 0 {
		return nil, syserror.Internal("iphlpapi table size query failed")
	}
	for attempt := 0; attempt < 4; attempt++ {
		buf := make([]byte, size)
		r1, _, _ = proc.Call(
			uintptr(unsafe.Pointer(&buf[0])), // #nosec G103 -- iphlpapi table buffer
			uintptr(unsafe.Pointer(&size)),   // #nosec G103 -- iphlpapi size pointer
			0,
			uintptr(family),
			uintptr(tableClass),
			0,
		)
		if r1 == 0 {
			return buf[:size], nil
		}
		if r1 != errorInsufficientBuffer {
			return nil, syserror.Internal("iphlpapi table read failed")
		}
	}
	return nil, syserror.Internal("iphlpapi table read exceeded retries")
}

func parsePortNetOrderWindows(val uint32) uint16 {
	return binary.BigEndian.Uint16([]byte{byte(val), byte(val >> 8)}) // #nosec G115 -- network-order port in low 16 bits
}

func listeningPortsImpl(filter *PortFilter) (*PortBindingsSnapshot, error) {
	var bindings []PortBinding
	procCache := make(map[uint32]*ProcessInfo)

	lookupProc := func(pid uint32) (*uint32, *ProcessInfo) {
		if pid == 0 || pid > MaxSafePID {
			return nil, nil
		}
		pidCopy := pid
		if cached, ok := procCache[pid]; ok {
			return &pidCopy, cached
		}
		if info, err := getProcessImpl(pid, ProcessOptions{}); err == nil {
			procCache[pid] = info
			return &pidCopy, info
		}
		return &pidCopy, nil
	}

	wantTCP := filter == nil || filter.Protocol == nil || *filter.Protocol == ProtocolTCP
	wantUDP := filter == nil || filter.Protocol == nil || *filter.Protocol == ProtocolUDP

	if wantTCP {
		// IPv4 TCP listeners (MIB_TCPROW_OWNER_PID: 24 bytes per row after 4-byte dwNumEntries)
		if buf, err := fetchExtendedTableWindows(procGetExtendedTcpTable, winAfInet, tcpTableOwnerPIDListener); err == nil && len(buf) >= 4 {
			count := int(binary.LittleEndian.Uint32(buf[0:4]))
			off := 4
			for i := 0; i < count && off+24 <= len(buf); i++ {
				state := binary.LittleEndian.Uint32(buf[off : off+4])
				localAddrBytes := buf[off+4 : off+8]
				localPortRaw := binary.LittleEndian.Uint32(buf[off+8 : off+12])
				owningPID := binary.LittleEndian.Uint32(buf[off+20 : off+24])
				off += 24
				if state != mibTCPStateListen {
					continue
				}
				port := parsePortNetOrderWindows(localPortRaw)
				if port == 0 {
					continue
				}
				ip := netip.AddrFrom4([4]byte{localAddrBytes[0], localAddrBytes[1], localAddrBytes[2], localAddrBytes[3]}).String()
				st := "listen"
				pidPtr, procInfo := lookupProc(owningPID)
				b := PortBinding{
					Protocol:  ProtocolTCP,
					LocalAddr: &ip,
					LocalPort: port,
					State:     &st,
					PID:       pidPtr,
					Process:   procInfo,
				}
				if filter == nil || filter.Matches(&b) {
					bindings = append(bindings, b)
				}
			}
		}

		// IPv6 TCP listeners (MIB_TCP6ROW_OWNER_PID: 56 bytes per row after 4-byte dwNumEntries)
		if buf, err := fetchExtendedTableWindows(procGetExtendedTcpTable, winAfInet6, tcpTableOwnerPIDListener); err == nil && len(buf) >= 4 {
			count := int(binary.LittleEndian.Uint32(buf[0:4]))
			off := 4
			for i := 0; i < count && off+56 <= len(buf); i++ {
				var b16 [16]byte
				copy(b16[:], buf[off:off+16])
				localPortRaw := binary.LittleEndian.Uint32(buf[off+20 : off+24])
				state := binary.LittleEndian.Uint32(buf[off+48 : off+52])
				owningPID := binary.LittleEndian.Uint32(buf[off+52 : off+56])
				off += 56
				if state != mibTCPStateListen {
					continue
				}
				port := parsePortNetOrderWindows(localPortRaw)
				if port == 0 {
					continue
				}
				ip := netip.AddrFrom16(b16).String()
				st := "listen"
				pidPtr, procInfo := lookupProc(owningPID)
				b := PortBinding{
					Protocol:  ProtocolTCP,
					LocalAddr: &ip,
					LocalPort: port,
					State:     &st,
					PID:       pidPtr,
					Process:   procInfo,
				}
				if filter == nil || filter.Matches(&b) {
					bindings = append(bindings, b)
				}
			}
		}
	}

	if wantUDP {
		// IPv4 UDP endpoints (MIB_UDPROW_OWNER_PID: 12 bytes per row after 4-byte dwNumEntries)
		if buf, err := fetchExtendedTableWindows(procGetExtendedUdpTable, winAfInet, udpTableOwnerPID); err == nil && len(buf) >= 4 {
			count := int(binary.LittleEndian.Uint32(buf[0:4]))
			off := 4
			for i := 0; i < count && off+12 <= len(buf); i++ {
				localAddrBytes := buf[off : off+4]
				localPortRaw := binary.LittleEndian.Uint32(buf[off+4 : off+8])
				owningPID := binary.LittleEndian.Uint32(buf[off+8 : off+12])
				off += 12
				port := parsePortNetOrderWindows(localPortRaw)
				if port == 0 {
					continue
				}
				ip := netip.AddrFrom4([4]byte{localAddrBytes[0], localAddrBytes[1], localAddrBytes[2], localAddrBytes[3]}).String()
				pidPtr, procInfo := lookupProc(owningPID)
				b := PortBinding{
					Protocol:  ProtocolUDP,
					LocalAddr: &ip,
					LocalPort: port,
					PID:       pidPtr,
					Process:   procInfo,
				}
				if filter == nil || filter.Matches(&b) {
					bindings = append(bindings, b)
				}
			}
		}

		// IPv6 UDP endpoints (MIB_UDP6ROW_OWNER_PID: 28 bytes per row after 4-byte dwNumEntries)
		if buf, err := fetchExtendedTableWindows(procGetExtendedUdpTable, winAfInet6, udpTableOwnerPID); err == nil && len(buf) >= 4 {
			count := int(binary.LittleEndian.Uint32(buf[0:4]))
			off := 4
			for i := 0; i < count && off+28 <= len(buf); i++ {
				var b16 [16]byte
				copy(b16[:], buf[off:off+16])
				localPortRaw := binary.LittleEndian.Uint32(buf[off+20 : off+24])
				owningPID := binary.LittleEndian.Uint32(buf[off+24 : off+28])
				off += 28
				port := parsePortNetOrderWindows(localPortRaw)
				if port == 0 {
					continue
				}
				ip := netip.AddrFrom16(b16).String()
				pidPtr, procInfo := lookupProc(owningPID)
				b := PortBinding{
					Protocol:  ProtocolUDP,
					LocalAddr: &ip,
					LocalPort: port,
					PID:       pidPtr,
					Process:   procInfo,
				}
				if filter == nil || filter.Matches(&b) {
					bindings = append(bindings, b)
				}
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
