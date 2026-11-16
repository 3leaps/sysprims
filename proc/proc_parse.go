package proc

import (
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"net/netip"
	"strconv"
	"strings"

	"github.com/3leaps/sysprims/syserror"
)

const (
	maxEnvEntries    = 1024
	maxEnvKeyBytes   = 1024
	maxEnvValueBytes = 4096
	maxEnvTotalBytes = 1048576
)

func guardDiscoveryName(cmdline []string, fallbackName string) string {
	if len(cmdline) == 0 {
		return fallbackName
	}
	bin := cmdline[0]
	if idx := strings.LastIndexByte(bin, '/'); idx >= 0 {
		bin = bin[idx+1:]
	}
	if bin == "" {
		bin = fallbackName
	}
	if bin != "sysprims" && !strings.HasPrefix(bin, "sysprims-") {
		return fallbackName
	}
	guardIdx := -1
	for i, arg := range cmdline {
		if arg == "guard" {
			guardIdx = i
			break
		}
	}
	if guardIdx < 0 || guardIdx+1 >= len(cmdline) {
		return fallbackName
	}
	rootPIDStr := cmdline[guardIdx+1]
	rootPID, err := strconv.ParseUint(rootPIDStr, 10, 32)
	if err != nil || rootPID == 0 {
		return fallbackName
	}
	return fmt.Sprintf("sysprims-guard:%d", rootPID)
}

func aggregatePermissionWarning(skipped int, label string) string {
	if skipped <= 0 {
		return ""
	}
	return fmt.Sprintf("Skipped %d %s due to permission errors", skipped, label)
}

func aggregateErrorWarning(skipped int, label string) string {
	if skipped <= 0 {
		return ""
	}
	return fmt.Sprintf("Skipped %d %s due to read errors", skipped, label)
}

// linuxStatFields holds parsed fields from /proc/[pid]/stat.
type linuxStatFields struct {
	Name       string
	State      ProcessState
	PPID       uint32
	UTimeTicks uint64
	STimeTicks uint64
	StartTicks uint64
}

// parseLinuxStat parses /proc/[pid]/stat using the last ')' delimiter for comm.
func parseLinuxStat(content string) (linuxStatFields, error) {
	openParen := strings.IndexByte(content, '(')
	closeParen := strings.LastIndexByte(content, ')')
	if openParen < 0 || closeParen <= openParen {
		return linuxStatFields{}, syserror.Internal("malformed /proc/[pid]/stat: missing comm parens")
	}
	name := content[openParen+1 : closeParen]
	rest := strings.Fields(strings.TrimSpace(content[closeParen+1:]))
	// rest[0] = state (field 3)
	// rest[1] = ppid (field 4)
	// rest[11] = utime (field 14)
	// rest[12] = stime (field 15)
	// rest[19] = starttime (field 22)
	if len(rest) < 20 {
		return linuxStatFields{}, syserror.Internal("malformed /proc/[pid]/stat: too few fields")
	}

	var state ProcessState
	if len(rest[0]) > 0 {
		switch rest[0][0] {
		case 'R':
			state = StateRunning
		case 'S', 'D', 'I':
			state = StateSleeping
		case 'T', 't':
			state = StateStopped
		case 'Z':
			state = StateZombie
		default:
			state = StateUnknown
		}
	} else {
		state = StateUnknown
	}

	ppid, _ := strconv.ParseUint(rest[1], 10, 32)
	utime, _ := strconv.ParseUint(rest[11], 10, 64)
	stime, _ := strconv.ParseUint(rest[12], 10, 64)
	starttime, _ := strconv.ParseUint(rest[19], 10, 64)

	return linuxStatFields{
		Name:       name,
		State:      state,
		PPID:       uint32(ppid),
		UTimeTicks: utime,
		STimeTicks: stime,
		StartTicks: starttime,
	}, nil
}

// parseLinuxStatus extracts real UID and optional thread count from /proc/[pid]/status.
func parseLinuxStatus(content string) (uid uint32, hasUID bool, threads uint32, hasThreads bool) {
	for _, line := range strings.Split(content, "\n") {
		if strings.HasPrefix(line, "Uid:") {
			fields := strings.Fields(line[len("Uid:"):])
			if len(fields) >= 1 {
				if v, err := strconv.ParseUint(fields[0], 10, 32); err == nil {
					uid = uint32(v)
					hasUID = true
				}
			}
		} else if strings.HasPrefix(line, "Threads:") {
			fields := strings.Fields(line[len("Threads:"):])
			if len(fields) >= 1 {
				if v, err := strconv.ParseUint(fields[0], 10, 32); err == nil && v > 0 {
					threads = uint32(v)
					hasThreads = true
				}
			}
		}
	}
	return uid, hasUID, threads, hasThreads
}

// parseLinuxStatmRSSPages extracts resident set size in pages (second field) from /proc/[pid]/statm.
func parseLinuxStatmRSSPages(content string) uint64 {
	fields := strings.Fields(content)
	if len(fields) < 2 {
		return 0
	}
	rssPages, err := strconv.ParseUint(fields[1], 10, 64)
	if err != nil {
		return 0
	}
	return rssPages
}

// parseLinuxBootTimeSecs extracts btime (Unix epoch seconds at boot) from /proc/stat.
func parseLinuxBootTimeSecs(content string) (uint64, bool) {
	for _, line := range strings.Split(content, "\n") {
		if strings.HasPrefix(line, "btime ") {
			fields := strings.Fields(line)
			if len(fields) >= 2 {
				if v, err := strconv.ParseUint(fields[1], 10, 64); err == nil {
					return v, true
				}
			}
		}
	}
	return 0, false
}

// parseNulSeparatedStrings parses a NUL-delimited byte slice (e.g. /proc/[pid]/cmdline).
func parseNulSeparatedStrings(buf []byte) []string {
	if len(buf) == 0 {
		return []string{}
	}
	parts := strings.Split(string(buf), "\x00")
	out := make([]string, 0, len(parts))
	for _, p := range parts {
		if p != "" {
			out = append(out, p)
		}
	}
	return out
}

// parseNulSeparatedEnv parses a NUL-delimited KEY=VALUE environment block with safety caps.
func parseNulSeparatedEnv(buf []byte) map[string]string {
	if len(buf) == 0 || len(buf) > maxEnvTotalBytes {
		return nil
	}
	env := make(map[string]string)
	totalBytes := 0
	for _, raw := range strings.Split(string(buf), "\x00") {
		if raw == "" {
			continue
		}
		if len(env) >= maxEnvEntries {
			break
		}
		key, val, ok := strings.Cut(raw, "=")
		if !ok || key == "" {
			continue
		}
		if len(key) > maxEnvKeyBytes || len(val) > maxEnvValueBytes {
			continue
		}
		totalBytes += len(key) + len(val)
		if totalBytes > maxEnvTotalBytes {
			return nil
		}
		env[key] = val
	}
	return env
}

// parseLinuxSocketInode parses a readlink target like "socket:[12345]" or "[0000]:12345".
func parseLinuxSocketInode(target string) (uint64, bool) {
	if strings.HasPrefix(target, "socket:[") && strings.HasSuffix(target, "]") {
		numStr := target[len("socket:[") : len(target)-1]
		if ino, err := strconv.ParseUint(numStr, 10, 64); err == nil {
			return ino, true
		}
	}
	if strings.HasPrefix(target, "[0000]:") {
		numStr := target[len("[0000]:"):]
		if ino, err := strconv.ParseUint(numStr, 10, 64); err == nil {
			return ino, true
		}
	}
	return 0, false
}

type procNetEntry struct {
	Protocol  Protocol
	LocalAddr *string
	LocalPort uint16
	State     *string
	Inode     uint64
}

// parseHexEndpoint parses a /proc/net/{tcp,tcp6,udp,udp6} local_address token ("0100007F:1F90").
func parseHexEndpoint(token string, ipv6 bool) (addrStr *string, port uint16, err error) {
	hostHex, portHex, ok := strings.Cut(token, ":")
	if !ok {
		return nil, 0, syserror.Internal("malformed /proc/net endpoint")
	}
	portVal, err := strconv.ParseUint(portHex, 16, 16)
	if err != nil {
		return nil, 0, syserror.Internal("malformed /proc/net port")
	}
	port = uint16(portVal)

	if !ipv6 {
		if len(hostHex) != 8 {
			return nil, port, nil
		}
		raw, err := hex.DecodeString(hostHex)
		if err != nil || len(raw) != 4 {
			return nil, port, nil
		}
		// Linux /proc/net/tcp stores IPv4 in host (little-endian on x86/arm) u32 order.
		ip := netip.AddrFrom4([4]byte{raw[3], raw[2], raw[1], raw[0]})
		s := ip.String()
		return &s, port, nil
	}

	if len(hostHex) != 32 {
		return nil, port, nil
	}
	raw, err := hex.DecodeString(hostHex)
	if err != nil || len(raw) != 16 {
		return nil, port, nil
	}
	var b16 [16]byte
	for word := 0; word < 4; word++ {
		base := word * 4
		b16[base+0] = raw[base+3]
		b16[base+1] = raw[base+2]
		b16[base+2] = raw[base+1]
		b16[base+3] = raw[base+0]
	}
	ip := netip.AddrFrom16(b16)
	s := ip.String()
	return &s, port, nil
}

// parseProcNetTable parses /proc/net/{tcp,tcp6,udp,udp6} content into listening/bound entries.
func parseProcNetTable(content string, proto Protocol, ipv6 bool) ([]procNetEntry, int) {
	var entries []procNetEntry
	readErrors := 0

	lines := strings.Split(content, "\n")
	for i, line := range lines {
		if i == 0 {
			continue // Skip header line
		}
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		fields := strings.Fields(line)
		if len(fields) < 10 {
			readErrors++
			continue
		}
		localToken := fields[1]
		stateHex := fields[3]
		inodeStr := fields[9]

		stateCode, err := strconv.ParseUint(stateHex, 16, 8)
		if err != nil {
			readErrors++
			continue
		}

		var statePtr *string
		if proto == ProtocolTCP {
			// 0x0A = TCP_LISTEN
			if stateCode != 0x0A {
				continue
			}
			s := "listen"
			statePtr = &s
		}

		addrStr, port, err := parseHexEndpoint(localToken, ipv6)
		if err != nil {
			readErrors++
			continue
		}
		if port == 0 {
			continue
		}
		inode, err := strconv.ParseUint(inodeStr, 10, 64)
		if err != nil {
			readErrors++
			continue
		}
		entries = append(entries, procNetEntry{
			Protocol:  proto,
			LocalAddr: addrStr,
			LocalPort: port,
			State:     statePtr,
			Inode:     inode,
		})
	}
	return entries, readErrors
}

// macOS kern.procargs2 parsing helpers.
func darwinProcargsArgvStart(buf []byte) (int, int, bool) {
	if len(buf) < 4 {
		return 0, 0, false
	}
	const maxArgc = 4096
	argc := int(int32(binary.LittleEndian.Uint32(buf[:4]))) // #nosec G115 -- C int32 argc from kern.procargs2
	if argc <= 0 || argc > maxArgc {
		return 0, 0, false
	}
	pos := 4
	// Skip exec_path string
	for pos < len(buf) && buf[pos] != 0 {
		pos++
	}
	// Skip NUL padding after exec_path
	for pos < len(buf) && buf[pos] == 0 {
		pos++
	}
	return argc, pos, true
}

func parseDarwinProcargsCmdline(buf []byte) []string {
	argc, pos, ok := darwinProcargsArgvStart(buf)
	if !ok {
		return []string{}
	}
	args := make([]string, 0, argc)
	for i := 0; i < argc; i++ {
		if pos >= len(buf) {
			break
		}
		start := pos
		for pos < len(buf) && buf[pos] != 0 {
			pos++
		}
		if start != pos {
			args = append(args, string(buf[start:pos]))
		}
		if pos < len(buf) {
			pos++
		}
	}
	return args
}

func parseDarwinProcargsEnv(buf []byte) map[string]string {
	argc, pos, ok := darwinProcargsArgvStart(buf)
	if !ok {
		return nil
	}
	for i := 0; i < argc; i++ {
		if pos >= len(buf) {
			break
		}
		for pos < len(buf) && buf[pos] != 0 {
			pos++
		}
		if pos < len(buf) {
			pos++
		}
	}
	env := make(map[string]string)
	totalBytes := 0
	for pos < len(buf) {
		for pos < len(buf) && buf[pos] == 0 {
			pos++
		}
		if pos >= len(buf) {
			break
		}
		start := pos
		for pos < len(buf) && buf[pos] != 0 {
			pos++
		}
		if start == pos {
			continue
		}
		if len(env) >= maxEnvEntries {
			break
		}
		pair := string(buf[start:pos])
		key, val, hasEq := strings.Cut(pair, "=")
		if !hasEq || key == "" {
			continue
		}
		if len(key) > maxEnvKeyBytes || len(val) > maxEnvValueBytes {
			continue
		}
		totalBytes += len(key) + len(val)
		if totalBytes > maxEnvTotalBytes {
			return nil
		}
		env[key] = val
	}
	return env
}

// macOS socket_fdinfo binary layout parser.
const (
	darwinSockInfoIn     = 1
	darwinSockInfoTCP    = 2
	darwinIniIPv4        = 0x1
	darwinIniIPv6        = 0x2
	darwinTcpStateListen = 1
	darwinIPProtoTCP     = 6
	darwinIPProtoUDP     = 17
)

func computeDarwinSocketInfoOffsets(vinfoStatSize int) (protoOff, kindOff, protoUnionOff int) {
	const procFileInfoSize = 24
	const sockbufInfoSize = 24
	off := procFileInfoSize + vinfoStatSize
	off += 8 // soi_so
	off += 8 // soi_pcb
	off += 4 // soi_type
	protoOff = off
	off += 4 // soi_protocol
	off += 4 // soi_family
	off += 2 // soi_options
	off += 2 // soi_linger
	off += 2 // soi_state
	off += 2 // soi_qlen
	off += 2 // soi_incqlen
	off += 2 // soi_qlimit
	off += 2 // soi_timeo
	off += 2 // soi_error
	off += 4 // soi_oobmark
	off += sockbufInfoSize
	off += sockbufInfoSize
	kindOff = off
	off += 4 // soi_kind
	off += 4 // rfu_1
	protoUnionOff = off
	return protoOff, kindOff, protoUnionOff
}

func selectDarwinSocketInfoLayout(buf []byte) (protoOff, kindOff, protoUnionOff int, ok bool) {
	for _, statSize := range []int{136, 144} {
		pOff, kOff, uOff := computeDarwinSocketInfoOffsets(statSize)
		if len(buf) < kOff+4 {
			continue
		}
		kind := int32(binary.LittleEndian.Uint32(buf[kOff : kOff+4])) // #nosec G115 -- C int32 soi_kind
		if kind < 0 || kind > 7 {
			continue
		}
		proto := int32(binary.LittleEndian.Uint32(buf[pOff : pOff+4])) // #nosec G115 -- C int32 soi_protocol
		protoOK := proto == darwinIPProtoTCP || proto == darwinIPProtoUDP || proto == 0
		kindOK := kind == darwinSockInfoTCP || kind == darwinSockInfoIn || kind == 0
		if protoOK && kindOK {
			return pOff, kOff, uOff, true
		}
	}
	return 0, 0, 0, false
}

// parseDarwinSocketFdInfo parses a socket_fdinfo buffer returned by PROC_PIDFDSOCKETINFO.
func parseDarwinSocketFdInfo(buf []byte, pid uint32) (*PortBinding, error) {
	protoOff, kindOff, protoUnionOff, ok := selectDarwinSocketInfoLayout(buf)
	if !ok {
		return nil, syserror.Internal("unsupported socket_info layout")
	}
	kind := int32(binary.LittleEndian.Uint32(buf[kindOff : kindOff+4]))       // #nosec G115 -- C int32 soi_kind
	soiProto := int32(binary.LittleEndian.Uint32(buf[protoOff : protoOff+4])) // #nosec G115 -- C int32 soi_protocol

	var protocol Protocol
	switch soiProto {
	case darwinIPProtoTCP:
		protocol = ProtocolTCP
	case darwinIPProtoUDP:
		protocol = ProtocolUDP
	default:
		switch kind {
		case darwinSockInfoTCP:
			protocol = ProtocolTCP
		case darwinSockInfoIn:
			protocol = ProtocolUDP
		default:
			return nil, syserror.Internal("unsupported socket kind for port bindings")
		}
	}

	const inSockInfoSize = 80
	const tcpSockInfoMinSize = 84 // InSockInfo (80) + tcpsi_state (4)
	if len(buf) < protoUnionOff+inSockInfoSize {
		return nil, syserror.Internal("socket proto truncated")
	}
	inBuf := buf[protoUnionOff:]

	// InSockInfo layout:
	// 0..4: insi_fport (i32)
	// 4..8: insi_lport (i32, network byte order in low 16 bits)
	// 24: insi_vflag (u8)
	// 48..64: insi_laddr (16-byte union; IPv4 at 60..64, IPv6 at 48..64)
	lportRaw := uint16(binary.LittleEndian.Uint32(inBuf[4:8]))                        // #nosec G115 -- low 16 bits hold network-order port
	localPort := binary.BigEndian.Uint16([]byte{byte(lportRaw), byte(lportRaw >> 8)}) // #nosec G115 -- low/high byte swap
	if localPort == 0 {
		return nil, syserror.Internal("socket has no local port")
	}

	vflag := inBuf[24]
	var localAddr *string
	if vflag&darwinIniIPv4 != 0 {
		ip := netip.AddrFrom4([4]byte{inBuf[60], inBuf[61], inBuf[62], inBuf[63]})
		s := ip.String()
		localAddr = &s
	} else if vflag&darwinIniIPv6 != 0 {
		var b16 [16]byte
		copy(b16[:], inBuf[48:64])
		ip := netip.AddrFrom16(b16)
		s := ip.String()
		localAddr = &s
	}

	var state *string
	if protocol == ProtocolTCP {
		if kind != darwinSockInfoTCP || len(inBuf) < tcpSockInfoMinSize {
			return nil, syserror.Internal("tcp socket kind unsupported for listener detection")
		}
		tcpState := int32(binary.LittleEndian.Uint32(inBuf[80:84])) // #nosec G115 -- C int32 tcpsi_state
		if tcpState != darwinTcpStateListen {
			return nil, syserror.Internal("tcp socket not listening")
		}
		s := "listen"
		state = &s
	}

	pidCopy := pid
	return &PortBinding{
		Protocol:  protocol,
		LocalAddr: localAddr,
		LocalPort: localPort,
		State:     state,
		PID:       &pidCopy,
	}, nil
}
