// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build linux

package procobserve

import (
	"bytes"
	"errors"
	"fmt"
	"math"
	"sort"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/3leaps/sysprims/evidence"
	"golang.org/x/sys/unix"
)

const (
	classBResolve = unix.RESOLVE_BENEATH |
		unix.RESOLVE_NO_XDEV |
		unix.RESOLVE_NO_SYMLINKS |
		unix.RESOLVE_NO_MAGICLINKS

	maxSmallProcFileBytes = 65_536
	maxMountinfoBytes     = 524_288
)

type platformHandle struct {
	pidfd  int
	exited bool
}

func (p *platformHandle) closeHandle() error {
	if p.pidfd < 0 {
		return nil
	}
	fd := p.pidfd
	p.pidfd = -1
	return unix.Close(fd)
}

func (p *platformHandle) waitExit(timeout time.Duration) (bool, error) {
	if p.exited {
		return true, nil
	}
	var deadline time.Time
	hasDeadline := timeout > 0
	if hasDeadline {
		deadline = time.Now().Add(timeout)
	}

	for {
		timeoutMS := 0
		if hasDeadline {
			rem := time.Until(deadline)
			if rem > 0 {
				ms := rem.Milliseconds()
				if ms == 0 {
					timeoutMS = 1
				} else if ms > math.MaxInt32 {
					timeoutMS = math.MaxInt32
				} else {
					timeoutMS = int(ms)
				}
			}
		}

		pfds := [1]unix.PollFd{{
			Fd:     int32(p.pidfd), // #nosec G115 -- valid Linux file descriptor fits in int32
			Events: unix.POLLIN,
		}}
		n, err := unix.Poll(pfds[:], timeoutMS)
		if err != nil {
			if errors.Is(err, unix.EINTR) {
				if !hasDeadline || time.Now().After(deadline) {
					return false, nil
				}
				continue
			}
			return false, newErrorRefusal("pidfd-poll-failed", fmt.Sprintf("poll(pidfd) failed: %v", err))
		}
		if n > 0 {
			rev := pfds[0].Revents
			if (rev & unix.POLLNVAL) != 0 {
				return false, newErrorRefusal("pidfd-invalid", "poll reported POLLNVAL on pidfd")
			}
			if (rev & (unix.POLLIN | unix.POLLHUP | unix.POLLERR)) != 0 {
				p.exited = true
				return true, nil
			}
			return false, nil
		}
		if !hasDeadline || time.Now().After(deadline) {
			return false, nil
		}
	}
}

type nsProbe struct {
	status string
	value  string
	reason string
}

func (n nsProbe) toFact() map[string]any {
	if n.status == "observed" {
		return factObserved(n.value)
	}
	return factNonObserved(n.status, n.reason)
}

type userNSIDMapping struct {
	uidExtents  []evidence.IDMapExtent
	gidExtents  []evidence.IDMapExtent
	overflowUID uint32
	overflowGID uint32
	errStatus   string
	errReason   string
}

type observerAnchor struct {
	bootID          string
	pidNumberingNS  string
	timeNS          nsProbe
	userNS          nsProbe
	idMapping       *userNSIDMapping
	hidepidConceals bool
}

type linuxIncarnation struct {
	bootID         string
	pidNumberingNS string
	timeNS         nsProbe
	userNS         nsProbe
	targetPIDNS    nsProbe
	tgid           uint32
	startTicks     string
	state          string
	hasUIDGID      bool
	uids           [4]uint32
	gids           [4]uint32
}

type targetSnapshot struct {
	kind        string
	reason      string
	anchor      observerAnchor
	incarnation linuxIncarnation
	pidDirFD    int
	statusText  string
}

func (t *targetSnapshot) closeFD() {
	if t.pidDirFD >= 0 {
		_ = unix.Close(t.pidDirFD)
		t.pidDirFD = -1
	}
}

func openVerifiedProcRoot() (int, uint32, uint32, error) {
	flags := unix.O_RDONLY | unix.O_DIRECTORY | unix.O_CLOEXEC | unix.O_NOFOLLOW
	var fd int
	var err error
	for {
		fd, err = unix.Open("/proc", flags, 0)
		if err == nil {
			break
		}
		if errors.Is(err, unix.EINTR) {
			continue
		}
		return -1, 0, 0, newErrorRefusal("procfs-root-unverified", fmt.Sprintf("open(/proc) failed: %v", err))
	}

	var stfs unix.Statfs_t
	if err := unix.Fstatfs(fd, &stfs); err != nil {
		_ = unix.Close(fd)
		return -1, 0, 0, newErrorRefusal("procfs-root-unverified", fmt.Sprintf("fstatfs(/proc) failed: %v", err))
	}
	if int64(stfs.Type) != int64(unix.PROC_SUPER_MAGIC) {
		_ = unix.Close(fd)
		return -1, 0, 0, newErrorRefusal("procfs-root-unverified", fmt.Sprintf("unexpected /proc fs magic 0x%x", stfs.Type))
	}

	var st unix.Stat_t
	if err := unix.Fstat(fd, &st); err != nil {
		_ = unix.Close(fd)
		return -1, 0, 0, newErrorRefusal("procfs-root-unverified", fmt.Sprintf("fstat(/proc) failed: %v", err))
	}
	devU64 := uint64(st.Dev)
	return fd, unix.Major(devU64), unix.Minor(devU64), nil
}

// Class A: readlinkat(proc_fd, "self") must equal canonical decimal getpid().
func verifySelfSymlink(procFD int) (string, error) {
	selfPID := unix.Getpid()
	if selfPID <= 0 {
		return "", newErrorRefusal("self-pid-mismatch", "getpid() returned non-positive PID")
	}
	expected := strconv.Itoa(selfPID)
	buf := make([]byte, 64)
	n, err := unix.Readlinkat(procFD, "self", buf)
	if err != nil || n <= 0 || n >= len(buf) {
		return "", newErrorRefusal("self-pid-mismatch", fmt.Sprintf("readlinkat(proc_fd, self) failed: %v", err))
	}
	linkText := string(buf[:n])
	if linkText != expected {
		return "", newErrorRefusal("self-pid-mismatch", fmt.Sprintf("/proc/self resolved to %q, expected %q", linkText, expected))
	}
	return expected, nil
}

// Class B: openat2 with RESOLVE_BENEATH | RESOLVE_NO_XDEV | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS.
func openat2ClassB(dirFD int, relPath string, directory bool) (int, error) {
	invalidRel := relPath == "" ||
		strings.HasPrefix(relPath, "/") ||
		relPath == "self" ||
		strings.HasPrefix(relPath, "self/")
	if invalidRel {
		return -1, unix.EINVAL
	}
	flags := uint64(unix.O_RDONLY | unix.O_CLOEXEC)
	if directory {
		flags |= uint64(unix.O_DIRECTORY)
	}
	how := unix.OpenHow{
		Flags:   flags,
		Mode:    0,
		Resolve: classBResolve,
	}
	for {
		fd, err := unix.Openat2(dirFD, relPath, &how)
		if err == nil {
			return fd, nil
		}
		if errors.Is(err, unix.EINTR) {
			continue
		}
		return -1, err
	}
}

func mapClassBObserverErr(err error, context string) error {
	if errors.Is(err, unix.ENOSYS) {
		return newRefusal("unsupported", "openat2-unavailable", "Linux kernel does not support openat2(2); unconfined procfs fallback is forbidden")
	}
	if errors.Is(err, unix.EXDEV) || errors.Is(err, unix.ELOOP) {
		return newErrorRefusal("procfs-confinement-violation", fmt.Sprintf("openat2 confinement rejected %s: %v", context, err))
	}
	if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
		return newRefusal("inaccessible", "procfs-observer-denied", fmt.Sprintf("permission denied opening %s", context))
	}
	return newErrorRefusal("procfs-observer-error", fmt.Sprintf("openat2 failed on %s: %v", context, err))
}

// Class C: single-component openat(parent_fd, entry, O_RDONLY | O_CLOEXEC).
func openatClassCSingle(parentFD int, entry string) (int, error) {
	invalidEntry := entry == "" ||
		strings.Contains(entry, "/") ||
		entry == "." ||
		entry == ".." ||
		entry == "self"
	if invalidEntry {
		return -1, unix.EINVAL
	}
	for {
		fd, err := unix.Openat(parentFD, entry, unix.O_RDONLY|unix.O_CLOEXEC, 0)
		if err == nil {
			return fd, nil
		}
		if errors.Is(err, unix.EINTR) {
			continue
		}
		return -1, err
	}
}

func readNSMagicID(nsDirFD int, kind, prefix string) nsProbe {
	fd, err := openatClassCSingle(nsDirFD, kind)
	if err != nil {
		if errors.Is(err, unix.ENOENT) {
			return nsProbe{status: "not-present", reason: "namespace-entry-absent"}
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return nsProbe{status: "inaccessible", reason: "ptrace-may-access-denied"}
		}
		return nsProbe{status: "error", reason: "namespace-open-failed"}
	}
	defer func() {
		_ = unix.Close(fd)
	}()

	var stfs unix.Statfs_t
	if err := unix.Fstatfs(fd, &stfs); err != nil {
		return nsProbe{status: "error", reason: "namespace-fstatfs-failed"}
	}
	if int64(stfs.Type) != int64(unix.NSFS_MAGIC) {
		return nsProbe{status: "error", reason: "namespace-not-nsfs-magic"}
	}

	var st unix.Stat_t
	if err := unix.Fstat(fd, &st); err != nil {
		return nsProbe{status: "error", reason: "namespace-fstat-failed"}
	}
	devU64 := uint64(st.Dev)
	inoU64 := uint64(st.Ino)
	val := fmt.Sprintf("%s:%d:%d:%s", prefix, unix.Major(devU64), unix.Minor(devU64), strconv.FormatUint(inoU64, 10))
	return nsProbe{status: "observed", value: val}
}

func readFDBytesCapped(fd, maxBytes int) ([]byte, error, string) {
	var out []byte
	buf := make([]byte, 8192)
	for {
		n, err := unix.Read(fd, buf)
		if n > 0 {
			if len(out)+n > maxBytes {
				return nil, unix.E2BIG, "procfs-file-exceeds-cap"
			}
			out = append(out, buf[:n]...)
		}
		if err != nil {
			if errors.Is(err, unix.EINTR) {
				continue
			}
			return nil, err, "procfs-read-failed"
		}
		if n == 0 {
			break
		}
	}
	return out, nil, ""
}

func readClassBText(dirFD int, relPath string, maxBytes int) (string, error, string) {
	fd, err := openat2ClassB(dirFD, relPath, false)
	if err != nil {
		return "", err, "openat2-failed"
	}
	defer func() {
		_ = unix.Close(fd)
	}()
	raw, rErr, reason := readFDBytesCapped(fd, maxBytes)
	if rErr != nil {
		return "", rErr, reason
	}
	if !utf8.Valid(raw) {
		return "", unix.EINVAL, "procfs-invalid-utf8"
	}
	return string(raw), nil, ""
}

func parseCanonicalU32Str(s string) (uint32, bool) {
	if len(s) == 0 || (len(s) > 1 && s[0] == '0') {
		return 0, false
	}
	for i := 0; i < len(s); i++ {
		if s[i] < '0' || s[i] > '9' {
			return 0, false
		}
	}
	v, err := strconv.ParseUint(s, 10, 32)
	if err != nil {
		return 0, false
	}
	return uint32(v), true
}

func parseCanonicalU64Str(s string) (uint64, bool) {
	if len(s) == 0 || (len(s) > 1 && s[0] == '0') {
		return 0, false
	}
	for i := 0; i < len(s); i++ {
		if s[i] < '0' || s[i] > '9' {
			return 0, false
		}
	}
	v, err := strconv.ParseUint(s, 10, 64)
	return v, err == nil
}

func parseIDMapExtents(text string) ([]evidence.IDMapExtent, bool) {
	var extents []evidence.IDMapExtent
	for _, line := range strings.Split(text, "\n") {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			continue
		}
		toks := strings.Fields(trimmed)
		if len(toks) != 3 {
			return nil, false
		}
		first, err1 := strconv.ParseUint(toks[0], 10, 32)
		_, err2 := strconv.ParseUint(toks[1], 10, 32)
		count, err3 := strconv.ParseUint(toks[2], 10, 32)
		if err1 != nil || err2 != nil || err3 != nil {
			return nil, false
		}
		extents = append(extents, evidence.IDMapExtent{First: uint32(first), Count: uint32(count)})
	}
	return extents, true
}

func mountinfoConcealsPIDs(mountinfoText string, procMaj, procMin uint32) bool {
	expectedDev := fmt.Sprintf("%d:%d", procMaj, procMin)
	matchedAny := false
	for _, line := range strings.Split(mountinfoText, "\n") {
		lhs, rhs, ok := strings.Cut(line, " - ")
		if !ok {
			continue
		}
		lfields := strings.Fields(lhs)
		rfields := strings.Fields(rhs)
		if len(lfields) < 5 || len(rfields) < 3 {
			continue
		}
		devID := lfields[2]
		mountPoint := lfields[4]
		fstype := rfields[0]
		if fstype != "proc" || (mountPoint != "/proc" && devID != expectedDev) {
			continue
		}
		matchedAny = true
		optsCombined := rfields[2]
		if len(lfields) > 5 {
			optsCombined = lfields[5] + "," + optsCombined
		}
		for _, opt := range strings.Split(optsCombined, ",") {
			if opt == "subset=pid" {
				return true
			}
			if val, has := strings.CutPrefix(opt, "hidepid="); has {
				if val != "0" && val != "off" && val != "1" && val != "noaccess" {
					return true
				}
			}
		}
	}
	return !matchedAny
}

func parseProcStat(statText string) (string, string, string) {
	closeIdx := strings.LastIndexByte(statText, ')')
	if closeIdx < 0 || closeIdx+2 >= len(statText) {
		return "", "", "malformed-procfs-stat-missing-comm"
	}
	after := statText[closeIdx+1:]
	rest, ok := strings.CutPrefix(after, " ")
	if !ok {
		return "", "", "malformed-procfs-stat-delimiter"
	}
	fields := strings.Split(strings.TrimRight(rest, "\n"), " ")
	if len(fields) <= 19 {
		return "", "", "malformed-procfs-stat-field-count"
	}
	stateTok := fields[0]
	if len(stateTok) != 1 {
		return "", "", "malformed-procfs-stat-state"
	}
	var state string
	switch stateTok[0] {
	case 'Z':
		state = "zombie"
	case 'R', 'S', 'D', 'T', 't', 'W', 'I', 'P':
		state = "live"
	default:
		return "", "", "transient-or-dead-procfs-state"
	}
	startTicksTok := fields[19]
	if _, ok := parseCanonicalU64Str(startTicksTok); !ok {
		return "", "", "invalid-procfs-starttime-ticks"
	}
	return state, startTicksTok, ""
}

func statusLineMap(statusText string) map[string]string {
	m := make(map[string]string)
	for _, line := range strings.Split(statusText, "\n") {
		if k, v, ok := strings.Cut(line, ":"); ok {
			kt := strings.TrimSpace(k)
			if _, exists := m[kt]; !exists {
				m[kt] = strings.TrimSpace(v)
			}
		}
	}
	return m
}

func parseQuadU32(val string) ([4]uint32, bool) {
	var out [4]uint32
	toks := strings.Fields(val)
	if len(toks) != 4 {
		return out, false
	}
	for i := 0; i < 4; i++ {
		u, err := strconv.ParseUint(toks[i], 10, 32)
		if err != nil {
			return out, false
		}
		out[i] = uint32(u)
	}
	return out, true
}

func loadUserNSIDMapping(procFD, selfDirFD int) (*userNSIDMapping, error) {
	readMap := func(rel string) ([]evidence.IDMapExtent, string, string, error) {
		txt, err, _ := readClassBText(selfDirFD, rel, maxSmallProcFileBytes)
		if err != nil {
			if errors.Is(err, unix.ENOSYS) || errors.Is(err, unix.EXDEV) || errors.Is(err, unix.ELOOP) {
				return nil, "", "", mapClassBObserverErr(err, rel)
			}
			if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
				return nil, "inaccessible", "userns-id-map-denied", nil
			}
			return nil, "error", "userns-id-map-read-failed", nil
		}
		ext, ok := parseIDMapExtents(txt)
		if !ok {
			return nil, "error", "malformed-userns-id-map", nil
		}
		return ext, "", "", nil
	}
	readOverflow := func(rel string) (uint32, string, string, error) {
		txt, err, _ := readClassBText(procFD, rel, 64)
		if err != nil {
			if errors.Is(err, unix.ENOSYS) || errors.Is(err, unix.EXDEV) || errors.Is(err, unix.ELOOP) {
				return 0, "", "", mapClassBObserverErr(err, rel)
			}
			if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
				return 0, "inaccessible", "overflow-id-denied", nil
			}
			return 0, "error", "overflow-id-read-failed", nil
		}
		v, pErr := strconv.ParseUint(strings.TrimSpace(txt), 10, 32)
		if pErr != nil {
			return 0, "error", "malformed-overflow-id", nil
		}
		return uint32(v), "", "", nil
	}

	uidExt, st, rsn, fatal := readMap("uid_map")
	if fatal != nil {
		return nil, fatal
	}
	if st != "" {
		return &userNSIDMapping{errStatus: st, errReason: rsn}, nil
	}
	gidExt, st, rsn, fatal := readMap("gid_map")
	if fatal != nil {
		return nil, fatal
	}
	if st != "" {
		return &userNSIDMapping{errStatus: st, errReason: rsn}, nil
	}
	ovUID, st, rsn, fatal := readOverflow("sys/kernel/overflowuid")
	if fatal != nil {
		return nil, fatal
	}
	if st != "" {
		return &userNSIDMapping{errStatus: st, errReason: rsn}, nil
	}
	ovGID, st, rsn, fatal := readOverflow("sys/kernel/overflowgid")
	if fatal != nil {
		return nil, fatal
	}
	if st != "" {
		return &userNSIDMapping{errStatus: st, errReason: rsn}, nil
	}

	return &userNSIDMapping{
		uidExtents:  uidExt,
		gidExtents:  gidExt,
		overflowUID: ovUID,
		overflowGID: ovGID,
	}, nil
}

func collectObserverAnchor(procFD int, procMaj, procMin uint32, needIDMaps bool) (observerAnchor, error) {
	var zero observerAnchor
	selfPIDStr, err := verifySelfSymlink(procFD)
	if err != nil {
		return zero, err
	}
	selfDirFD, err := openat2ClassB(procFD, selfPIDStr, true)
	if err != nil {
		return zero, mapClassBObserverErr(err, "self_dir")
	}
	defer func() {
		_ = unix.Close(selfDirFD)
	}()

	selfStatus, sErr, _ := readClassBText(selfDirFD, "status", maxSmallProcFileBytes)
	if sErr != nil {
		return zero, mapClassBObserverErr(sErr, "self_dir/status")
	}
	selfMap := statusLineMap(selfStatus)
	nspidRaw, hasNSPID := selfMap["NSpid"]
	if !hasNSPID {
		return zero, newErrorRefusal("missing-self-nspid", "self/status lacks NSpid line")
	}
	nspidToks := strings.Fields(nspidRaw)
	if len(nspidToks) != 1 || nspidToks[0] != selfPIDStr {
		return zero, newErrorRefusal("nested-or-mismatched-nspid", fmt.Sprintf("self/status NSpid=%q does not match single-level getpid()=%s", nspidRaw, selfPIDStr))
	}

	hidepidConceals := true
	if mi, miErr, _ := readClassBText(selfDirFD, "mountinfo", maxMountinfoBytes); miErr == nil {
		hidepidConceals = mountinfoConcealsPIDs(mi, procMaj, procMin)
	} else if errors.Is(miErr, unix.ENOSYS) || errors.Is(miErr, unix.EXDEV) || errors.Is(miErr, unix.ELOOP) {
		return zero, mapClassBObserverErr(miErr, "self_dir/mountinfo")
	}

	selfNSFD, err := openat2ClassB(selfDirFD, "ns", true)
	if err != nil {
		return zero, mapClassBObserverErr(err, "self_dir/ns")
	}
	defer func() {
		_ = unix.Close(selfNSFD)
	}()

	pidNumProbe := readNSMagicID(selfNSFD, "pid", "pidns")
	if pidNumProbe.status == "inaccessible" {
		return zero, newRefusal("inaccessible", "pid-numbering-namespace-denied", "unable to read observer self/ns/pid")
	}
	if pidNumProbe.status != "observed" {
		return zero, newErrorRefusal("pid-numbering-namespace-unavailable", "unable to verify observer self/ns/pid")
	}

	timeNS := readNSMagicID(selfNSFD, "time", "timens")
	if timeNS.status == "not-present" {
		timeNS.reason = "kernel-without-timens"
	}
	userNS := readNSMagicID(selfNSFD, "user", "userns")
	if userNS.status == "not-present" {
		userNS.reason = "kernel-without-userns"
	}

	var idMap *userNSIDMapping
	if needIDMaps && userNS.status == "observed" {
		m, mErr := loadUserNSIDMapping(procFD, selfDirFD)
		if mErr != nil {
			return zero, mErr
		}
		idMap = m
	}

	bootRaw, bErr, _ := readClassBText(procFD, "sys/kernel/random/boot_id", 128)
	if bErr != nil {
		return zero, mapClassBObserverErr(bErr, "sys/kernel/random/boot_id")
	}
	bootID := strings.TrimSuffix(bootRaw, "\n")
	if !isLowercaseUUID(bootID) {
		return zero, newErrorRefusal("invalid-boot-id", "Linux boot_id failed lowercase UUID validation")
	}

	return observerAnchor{
		bootID:          bootID,
		pidNumberingNS:  pidNumProbe.value,
		timeNS:          timeNS,
		userNS:          userNS,
		idMapping:       idMap,
		hidepidConceals: hidepidConceals,
	}, nil
}

func captureTargetSnapshot(procFD int, procMaj, procMin, pid uint32, needIDMaps bool) (*targetSnapshot, error) {
	anchor, err := collectObserverAnchor(procFD, procMaj, procMin, needIDMaps)
	if err != nil {
		return nil, err
	}
	pidStr := strconv.FormatUint(uint64(pid), 10)

	pidDirFD, err := openat2ClassB(procFD, pidStr, true)
	if err != nil {
		if errors.Is(err, unix.ENOSYS) || errors.Is(err, unix.EXDEV) || errors.Is(err, unix.ELOOP) {
			return nil, mapClassBObserverErr(err, "<pid>")
		}
		if errors.Is(err, unix.ENOENT) {
			if anchor.hidepidConceals {
				return &targetSnapshot{kind: "unknown", reason: "procfs-hidepid-concealment-possible", anchor: anchor, pidDirFD: -1}, nil
			}
			return &targetSnapshot{kind: "not-present", reason: "target-pid-absent", anchor: anchor, pidDirFD: -1}, nil
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return &targetSnapshot{kind: "inaccessible", reason: "procfs-pid-dir-denied", anchor: anchor, pidDirFD: -1}, nil
		}
		return nil, newErrorRefusal("procfs-pid-open-failed", fmt.Sprintf("openat2(<pid>) failed: %v", err))
	}

	keepDir := false
	defer func() {
		if !keepDir {
			_ = unix.Close(pidDirFD)
		}
	}()

	statusText, sErr, sReason := readClassBText(pidDirFD, "status", maxSmallProcFileBytes)
	if sErr != nil {
		if errors.Is(sErr, unix.ENOSYS) || errors.Is(sErr, unix.EXDEV) || errors.Is(sErr, unix.ELOOP) {
			return nil, mapClassBObserverErr(sErr, "<pid>/status")
		}
		if errors.Is(sErr, unix.ENOENT) || errors.Is(sErr, unix.ESRCH) {
			if anchor.hidepidConceals {
				return &targetSnapshot{kind: "unknown", reason: "procfs-hidepid-concealment-possible", anchor: anchor, pidDirFD: -1}, nil
			}
			return &targetSnapshot{kind: "not-present", reason: "target-pid-absent", anchor: anchor, pidDirFD: -1}, nil
		}
		if errors.Is(sErr, unix.EPERM) || errors.Is(sErr, unix.EACCES) {
			return &targetSnapshot{kind: "inaccessible", reason: "procfs-status-denied", anchor: anchor, pidDirFD: -1}, nil
		}
		return nil, newErrorRefusal(sReason, "failed reading <pid>/status")
	}

	smap := statusLineMap(statusText)
	tgidStr, hasTgid := smap["Tgid"]
	if !hasTgid {
		return nil, newErrorRefusal("missing-procfs-tgid", "<pid>/status missing Tgid line")
	}
	tgid, ok := parseCanonicalU32Str(tgidStr)
	if !ok {
		return nil, newErrorRefusal("invalid-procfs-tgid", "<pid>/status Tgid is not a canonical u32")
	}
	if tgid != pid {
		return &targetSnapshot{kind: "not-present", reason: "tid-not-thread-group-leader", anchor: anchor, pidDirFD: -1}, nil
	}

	statText, stErr, stReason := readClassBText(pidDirFD, "stat", maxSmallProcFileBytes)
	if stErr != nil {
		if errors.Is(stErr, unix.ENOSYS) || errors.Is(stErr, unix.EXDEV) || errors.Is(stErr, unix.ELOOP) {
			return nil, mapClassBObserverErr(stErr, "<pid>/stat")
		}
		if errors.Is(stErr, unix.ENOENT) || errors.Is(stErr, unix.ESRCH) {
			if anchor.hidepidConceals {
				return &targetSnapshot{kind: "unknown", reason: "procfs-hidepid-concealment-possible", anchor: anchor, pidDirFD: -1}, nil
			}
			return &targetSnapshot{kind: "not-present", reason: "target-pid-absent", anchor: anchor, pidDirFD: -1}, nil
		}
		if errors.Is(stErr, unix.EPERM) || errors.Is(stErr, unix.EACCES) {
			return &targetSnapshot{kind: "inaccessible", reason: "procfs-stat-denied", anchor: anchor, pidDirFD: -1}, nil
		}
		return nil, newErrorRefusal(stReason, "failed reading <pid>/stat")
	}

	state, startTicks, parseErrReason := parseProcStat(statText)
	if parseErrReason != "" {
		return nil, newErrorRefusal(parseErrReason, "invalid <pid>/stat")
	}

	var targetPIDNS nsProbe
	pidNSFD, nsErr := openat2ClassB(pidDirFD, "ns", true)
	if nsErr != nil {
		if errors.Is(nsErr, unix.ENOSYS) || errors.Is(nsErr, unix.EXDEV) || errors.Is(nsErr, unix.ELOOP) {
			return nil, mapClassBObserverErr(nsErr, "<pid>/ns")
		}
		if errors.Is(nsErr, unix.EPERM) || errors.Is(nsErr, unix.EACCES) {
			targetPIDNS = nsProbe{status: "inaccessible", reason: "ptrace-may-access-denied"}
		} else if errors.Is(nsErr, unix.ENOENT) {
			targetPIDNS = nsProbe{status: "not-present", reason: "namespace-dir-absent"}
		} else {
			targetPIDNS = nsProbe{status: "error", reason: "namespace-dir-open-failed"}
		}
	} else {
		targetPIDNS = readNSMagicID(pidNSFD, "pid", "pidns")
		_ = unix.Close(pidNSFD)
	}

	uids, okU := parseQuadU32(smap["Uid"])
	gids, okG := parseQuadU32(smap["Gid"])

	keepDir = true
	return &targetSnapshot{
		kind:   "observed",
		anchor: anchor,
		incarnation: linuxIncarnation{
			bootID:         anchor.bootID,
			pidNumberingNS: anchor.pidNumberingNS,
			timeNS:         anchor.timeNS,
			userNS:         anchor.userNS,
			targetPIDNS:    targetPIDNS,
			tgid:           tgid,
			startTicks:     startTicks,
			state:          state,
			hasUIDGID:      okU && okG,
			uids:           uids,
			gids:           gids,
		},
		pidDirFD:   pidDirFD,
		statusText: statusText,
	}, nil
}

func idMappingEqual(a, b *userNSIDMapping) bool {
	if a == nil || b == nil {
		return a == nil && b == nil
	}
	if a.errStatus != b.errStatus || a.errReason != b.errReason || a.overflowUID != b.overflowUID || a.overflowGID != b.overflowGID {
		return false
	}
	if len(a.uidExtents) != len(b.uidExtents) || len(a.gidExtents) != len(b.gidExtents) {
		return false
	}
	for i := range a.uidExtents {
		if a.uidExtents[i] != b.uidExtents[i] {
			return false
		}
	}
	for i := range a.gidExtents {
		if a.gidExtents[i] != b.gidExtents[i] {
			return false
		}
	}
	return true
}

func evaluateLinuxPrincipal(anchor observerAnchor, inc linuxIncarnation) map[string]any {
	if !inc.hasUIDGID {
		return factNonObserved("error", "malformed-procfs-status-uidgid")
	}
	var usernsScope string
	switch anchor.userNS.status {
	case "observed":
		if anchor.idMapping == nil {
			return factNonObserved("error", "missing-userns-id-map")
		}
		if anchor.idMapping.errStatus != "" {
			return factNonObserved(anchor.idMapping.errStatus, anchor.idMapping.errReason)
		}
		for _, u := range inc.uids {
			if !evidence.LinuxIDIsUnambiguouslyMapped(u, anchor.idMapping.uidExtents, anchor.idMapping.overflowUID) {
				return factNonObserved("unknown", "unmapped-or-overflow-id")
			}
		}
		for _, g := range inc.gids {
			if !evidence.LinuxIDIsUnambiguouslyMapped(g, anchor.idMapping.gidExtents, anchor.idMapping.overflowGID) {
				return factNonObserved("unknown", "unmapped-or-overflow-id")
			}
		}
		usernsScope = anchor.userNS.value
	case "not-present":
		for _, u := range inc.uids {
			if u == math.MaxUint32 {
				return factNonObserved("unknown", "unmapped-or-overflow-id")
			}
		}
		for _, g := range inc.gids {
			if g == math.MaxUint32 {
				return factNonObserved("unknown", "unmapped-or-overflow-id")
			}
		}
		usernsScope = "userns:initial"
	case "inaccessible":
		return factNonObserved("inaccessible", anchor.userNS.reason)
	default:
		return factNonObserved("error", anchor.userNS.reason)
	}

	ns := "urn:3leaps:posix-credentials:linux-procfs-status:v0:" + usernsScope
	val := fmt.Sprintf(
		"ruid=%d;euid=%d;suid=%d;fsuid=%d;rgid=%d;egid=%d;sgid=%d;fsgid=%d",
		inc.uids[0], inc.uids[1], inc.uids[2], inc.uids[3],
		inc.gids[0], inc.gids[1], inc.gids[2], inc.gids[3],
	)
	return principalObserved(LinuxPrincipalDerivationID, ns, val)
}

func probeLinuxArgumentsDigest(pidDirFD int) (map[string]any, error) {
	fd, err := openat2ClassB(pidDirFD, "cmdline", false)
	if err != nil {
		if errors.Is(err, unix.ENOSYS) || errors.Is(err, unix.EXDEV) || errors.Is(err, unix.ELOOP) {
			return nil, mapClassBObserverErr(err, "<pid>/cmdline")
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return factNonObserved("inaccessible", "procfs-cmdline-denied"), nil
		}
		if errors.Is(err, unix.ENOENT) || errors.Is(err, unix.ESRCH) {
			return factNonObserved("not-present", "empty-cmdline"), nil
		}
		return factNonObserved("error", "procfs-cmdline-open-failed"), nil
	}
	defer func() {
		_ = unix.Close(fd)
	}()

	raw, rErr, rReason := readFDBytesCapped(fd, MaxArgumentsRawBytes)
	if rErr != nil {
		if errors.Is(rErr, unix.E2BIG) {
			return factNonObserved("error", "arguments-exceed-cap"), nil
		}
		if errors.Is(rErr, unix.EPERM) || errors.Is(rErr, unix.EACCES) {
			return factNonObserved("inaccessible", "procfs-cmdline-denied"), nil
		}
		if errors.Is(rErr, unix.ESRCH) {
			return factNonObserved("not-present", "empty-cmdline"), nil
		}
		return factNonObserved("error", rReason), nil
	}
	if len(raw) == 0 {
		return factNonObserved("not-present", "empty-cmdline"), nil
	}
	trimmed := raw
	if raw[len(raw)-1] == 0 {
		trimmed = raw[:len(raw)-1]
	}
	argv := bytes.Split(trimmed, []byte{0})
	return digestObserved(ArgvDigestDerivationID, evidence.DeriveArgumentsDigest(argv)), nil
}

func probeLinuxExecutableDigest(pidDirFD int) map[string]any {
	exeFD, err := openatClassCSingle(pidDirFD, "exe")
	if err != nil {
		return errnoToDigestFact(err, "procfs-exe-open-failed")
	}
	defer func() {
		_ = unix.Close(exeFD)
	}()
	return streamExecutableDigestFromFD(exeFD)
}

func probeLinuxNamespaceContextDigest(pidDirFD int) (map[string]any, error) {
	pidNSFD, err := openat2ClassB(pidDirFD, "ns", true)
	if err != nil {
		if errors.Is(err, unix.ENOSYS) || errors.Is(err, unix.EXDEV) || errors.Is(err, unix.ELOOP) {
			return nil, mapClassBObserverErr(err, "<pid>/ns")
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return factNonObserved("inaccessible", "ptrace-may-access-denied"), nil
		}
		if errors.Is(err, unix.ENOENT) {
			return factNonObserved("unsupported", "namespace-dir-absent"), nil
		}
		return factNonObserved("error", "namespace-dir-open-failed"), nil
	}
	defer func() {
		_ = unix.Close(pidNSFD)
	}()

	nsMap := make(map[string]string, len(evidence.LinuxNamespaceKinds))
	for _, kind := range evidence.LinuxNamespaceKinds {
		pr := readNSMagicID(pidNSFD, kind, "ns")
		switch pr.status {
		case "observed":
			nsMap[kind] = pr.value
		case "inaccessible":
			return factNonObserved("inaccessible", pr.reason), nil
		case "not-present":
			return factNonObserved("unsupported", "kernel-missing-namespace-member"), nil
		default:
			return factNonObserved("error", pr.reason), nil
		}
	}
	digest, dErr := evidence.DeriveLinuxNamespaceContextDigest(nsMap)
	if dErr != nil {
		return factNonObserved("error", "invalid-linux-ns-digest-input"), nil
	}
	return digestObserved(LinuxNSDigestDerivationID, digest), nil
}

func probeLinuxSecurityContextDigest(anchor observerAnchor, statusText string) map[string]any {
	var usernsScope string
	switch anchor.userNS.status {
	case "observed":
		usernsScope = anchor.userNS.value
	case "not-present":
		usernsScope = "userns:initial"
	case "inaccessible":
		return factNonObserved("inaccessible", anchor.userNS.reason)
	default:
		return factNonObserved("error", anchor.userNS.reason)
	}

	smap := statusLineMap(statusText)
	reqKeys := [8]string{"Groups", "NoNewPrivs", "Seccomp", "CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb"}
	for _, k := range reqKeys {
		if _, ok := smap[k]; !ok {
			return factNonObserved("unsupported", "missing-procfs-status-secctx-field")
		}
	}

	var gids []uint32
	if smap["Groups"] != "" {
		for _, tok := range strings.Fields(smap["Groups"]) {
			g64, err := strconv.ParseUint(tok, 10, 32)
			if err != nil {
				return factNonObserved("error", "malformed-procfs-groups")
			}
			gid := uint32(g64)
			if anchor.userNS.status == "observed" {
				if anchor.idMapping == nil {
					return factNonObserved("error", "missing-userns-id-map")
				}
				if anchor.idMapping.errStatus != "" {
					return factNonObserved(anchor.idMapping.errStatus, anchor.idMapping.errReason)
				}
				if !evidence.LinuxIDIsUnambiguouslyMapped(gid, anchor.idMapping.gidExtents, anchor.idMapping.overflowGID) {
					return factNonObserved("unknown", "unmapped-or-overflow-gid")
				}
			} else if gid == math.MaxUint32 {
				return factNonObserved("unknown", "unmapped-or-overflow-gid")
			}
			gids = append(gids, gid)
		}
	}
	sort.Slice(gids, func(i, j int) bool { return gids[i] < gids[j] })
	var dedup []string
	for i, g := range gids {
		if i == 0 || g != gids[i-1] {
			dedup = append(dedup, strconv.FormatUint(uint64(g), 10))
		}
	}
	groupsCSV := strings.Join(dedup, ",")

	nnp := smap["NoNewPrivs"]
	if nnp != "0" && nnp != "1" {
		return factNonObserved("error", "malformed-procfs-nonewprivs")
	}
	seccompU64, err := strconv.ParseUint(smap["Seccomp"], 10, 32)
	if err != nil {
		return factNonObserved("error", "malformed-procfs-seccomp")
	}

	canonical := fmt.Sprintf(
		"observer_user_ns=%s;groups=%s;no_new_privs=%s;seccomp=%d;cap_inh=%s;cap_prm=%s;cap_eff=%s;cap_bnd=%s;cap_amb=%s",
		usernsScope,
		groupsCSV,
		nnp,
		uint32(seccompU64),
		smap["CapInh"],
		smap["CapPrm"],
		smap["CapEff"],
		smap["CapBnd"],
		smap["CapAmb"],
	)
	digest, dErr := evidence.DeriveLinuxSecurityContextDigest([]byte(canonical))
	if dErr != nil {
		return factNonObserved("error", "malformed-procfs-capabilities")
	}
	return digestObserved(LinuxSecCtxDigestDerivationID, digest)
}

func assembleLinuxPayload(
	pid uint32,
	hostScopeMap map[string]any,
	anchor observerAnchor,
	targetPIDNSFact map[string]any,
	processStateFact map[string]any,
	startIdentityFact map[string]any,
	securityPrincipalFact map[string]any,
	nsCtxFact map[string]any,
	exeFact map[string]any,
	argsFact map[string]any,
	secCtxFact map[string]any,
	opts ObservationOptions,
) map[string]any {
	return map[string]any{
		"profile": map[string]any{
			"id":       evidence.NativeProcessObservationProfileID,
			"revision": evidence.NativeProcessObservationProfileRevision,
		},
		"observed_at": opts.ObservedAt,
		"observer": map[string]any{
			"platform_family": "linux",
			"architecture":    observerArchitecture(),
			"bracketed":       true,
			"time_namespace":  anchor.timeNS.toFact(),
			"user_namespace":  anchor.userNS.toFact(),
		},
		"process": map[string]any{
			"subject_ref":             opts.SubjectRef,
			"host_scope":              cloneMap(hostScopeMap),
			"boot_id":                 factObserved(anchor.bootID),
			"pid_numbering_namespace": factObserved(anchor.pidNumberingNS),
			"target_pid_namespace":    targetPIDNSFact,
			"pid":                     strconv.FormatUint(uint64(pid), 10),
			"process_state":           processStateFact,
			"start_identity":          startIdentityFact,
			"execution_context": map[string]any{
				"security_principal":       securityPrincipalFact,
				"namespace_context_digest": nsCtxFact,
				"executable_digest":        exeFact,
				"arguments_digest":         argsFact,
				"security_context_digest":  secCtxFact,
			},
		},
	}
}

func nonObservedLinuxContext(status, reason string, opts ObservationOptions) (map[string]any, map[string]any, map[string]any, map[string]any, map[string]any) {
	sp := factNonObserved("not-probed", defaultNotProbedReason)
	if opts.probeSecurityPrincipal() {
		sp = factNonObserved(status, reason)
	}
	ns := factNonObserved("not-probed", defaultNotProbedReason)
	if opts.IncludeNamespaceContextDigest {
		ns = factNonObserved(status, reason)
	}
	exe := factNonObserved("not-probed", defaultNotProbedReason)
	if opts.IncludeExecutableDigest {
		exe = factNonObserved(status, reason)
	}
	args := factNonObserved("not-probed", defaultNotProbedReason)
	if opts.IncludeArgumentsDigest {
		args = factNonObserved(status, reason)
	}
	sec := factNonObserved("not-probed", defaultNotProbedReason)
	if opts.IncludeSecurityContextDigest {
		sec = factNonObserved(status, reason)
	}
	return sp, ns, exe, args, sec
}

func observeProcessPlatform(pid uint32, hostScopeMap map[string]any, opts ObservationOptions) (map[string]any, error) {
	procFD, procMaj, procMin, err := openVerifiedProcRoot()
	if err != nil {
		return nil, err
	}
	defer func() {
		_ = unix.Close(procFD)
	}()

	needIDMaps := opts.probeSecurityPrincipal() || opts.IncludeSecurityContextDigest

	for attempt := 0; attempt < 2; attempt++ {
		s0, err := captureTargetSnapshot(procFD, procMaj, procMin, pid, needIDMaps)
		if err != nil {
			return nil, err
		}

		switch s0.kind {
		case "not-present":
			s0.closeFD()
			a1, err := collectObserverAnchor(procFD, procMaj, procMin, false)
			if err != nil {
				return nil, err
			}
			anchorChanged := s0.anchor.bootID != a1.bootID ||
				s0.anchor.pidNumberingNS != a1.pidNumberingNS ||
				s0.anchor.timeNS != a1.timeNS ||
				s0.anchor.userNS != a1.userNS
			if anchorChanged {
				if attempt == 0 {
					continue
				}
				return nil, newErrorRefusal("unbracketed-incarnation", "observer anchor changed during absent-process observation")
			}
			sp, ns, exe, args, sec := nonObservedLinuxContext("not-present", s0.reason, opts)
			return assembleLinuxPayload(
				pid,
				hostScopeMap,
				s0.anchor,
				factNonObserved("not-present", s0.reason),
				factNonObserved("not-present", s0.reason),
				startIdentityNonObserved(LinuxStartIdentityProfileID, "not-present", s0.reason),
				sp, ns, exe, args, sec,
				opts,
			), nil

		case "unknown", "inaccessible":
			s0.closeFD()
			sp, ns, exe, args, sec := nonObservedLinuxContext(s0.kind, s0.reason, opts)
			return assembleLinuxPayload(
				pid,
				hostScopeMap,
				s0.anchor,
				factNonObserved(s0.kind, s0.reason),
				factNonObserved(s0.kind, s0.reason),
				startIdentityNonObserved(LinuxStartIdentityProfileID, s0.kind, s0.reason),
				sp, ns, exe, args, sec,
				opts,
			), nil

		case "observed":
			nsCtxFact := factNonObserved("not-probed", defaultNotProbedReason)
			if opts.IncludeNamespaceContextDigest {
				f, pErr := probeLinuxNamespaceContextDigest(s0.pidDirFD)
				if pErr != nil {
					s0.closeFD()
					return nil, pErr
				}
				nsCtxFact = f
			}
			exeFact := factNonObserved("not-probed", defaultNotProbedReason)
			if opts.IncludeExecutableDigest {
				exeFact = probeLinuxExecutableDigest(s0.pidDirFD)
			}
			argsFact := factNonObserved("not-probed", defaultNotProbedReason)
			if opts.IncludeArgumentsDigest {
				f, pErr := probeLinuxArgumentsDigest(s0.pidDirFD)
				if pErr != nil {
					s0.closeFD()
					return nil, pErr
				}
				argsFact = f
			}
			var secCtx0 map[string]any
			if opts.IncludeSecurityContextDigest {
				secCtx0 = probeLinuxSecurityContextDigest(s0.anchor, s0.statusText)
			}
			s0.closeFD()

			s1, err := captureTargetSnapshot(procFD, procMaj, procMin, pid, needIDMaps)
			if err != nil {
				return nil, err
			}
			s1.closeFD()

			if s1.kind != "observed" || s0.incarnation != s1.incarnation || !idMappingEqual(s0.anchor.idMapping, s1.anchor.idMapping) {
				if attempt == 0 {
					continue
				}
				return nil, newErrorRefusal("unbracketed-incarnation", "Linux incarnation or credentials changed between pre/post snapshots")
			}

			secCtxFact := factNonObserved("not-probed", defaultNotProbedReason)
			if secCtx0 != nil {
				secCtx1 := probeLinuxSecurityContextDigest(s1.anchor, s1.statusText)
				if !evidence.JSONEqual(secCtx0, secCtx1) {
					if attempt == 0 {
						continue
					}
					return nil, newErrorRefusal("unbracketed-incarnation", "security_context_digest changed between pre/post snapshots")
				}
				secCtxFact = secCtx0
			}

			spFact := factNonObserved("not-probed", defaultNotProbedReason)
			if opts.probeSecurityPrincipal() {
				spFact = evaluateLinuxPrincipal(s0.anchor, s0.incarnation)
			}

			return assembleLinuxPayload(
				pid,
				hostScopeMap,
				s0.anchor,
				s0.incarnation.targetPIDNS.toFact(),
				factObserved(s0.incarnation.state),
				startIdentityObserved(LinuxStartIdentityProfileID, s0.incarnation.startTicks),
				spFact,
				nsCtxFact,
				exeFact,
				argsFact,
				secCtxFact,
				opts,
			), nil
		}
	}

	return nil, newErrorRefusal("unbracketed-incarnation", "failed to bracket Linux process observation")
}

func openContinuityHandlePlatform(pid uint32, hostScopeMap map[string]any, opts ObservationOptions) (map[string]any, *platformHandle, error) {
	procFD, procMaj, procMin, err := openVerifiedProcRoot()
	if err != nil {
		return nil, nil, err
	}
	defer func() {
		_ = unix.Close(procFD)
	}()

	needIDMaps := opts.probeSecurityPrincipal() || opts.IncludeSecurityContextDigest
	s0, err := captureTargetSnapshot(procFD, procMaj, procMin, pid, needIDMaps)
	if err != nil {
		return nil, nil, err
	}
	defer s0.closeFD()

	switch s0.kind {
	case "not-present":
		return nil, nil, newRefusal("not-present", "target-not-present", s0.reason)
	case "unknown":
		return nil, nil, newRefusal("unknown", "target-visibility-unknown", s0.reason)
	case "inaccessible":
		return nil, nil, newRefusal("inaccessible", s0.reason, "target PID is inaccessible")
	}
	if s0.incarnation.state != "live" {
		return nil, nil, newErrorRefusal("target-not-live", "continuity handle requires a live target process (not zombie)")
	}

	var pidfd int
	for {
		pidfd, err = unix.PidfdOpen(int(pid), 0)
		if err == nil {
			break
		}
		if errors.Is(err, unix.EINTR) {
			continue
		}
		if errors.Is(err, unix.ENOSYS) {
			return nil, nil, newRefusal("unsupported", "pidfd-unavailable", "Linux kernel does not support pidfd_open(2)")
		}
		if errors.Is(err, unix.ESRCH) {
			return nil, nil, newRefusal("not-present", "target-not-present", "target PID exited before pidfd_open")
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return nil, nil, newRefusal("inaccessible", "pidfd-open-denied", "kernel denied pidfd_open")
		}
		return nil, nil, newErrorRefusal("pidfd-open-failed", fmt.Sprintf("pidfd_open failed: %v", err))
	}
	unix.CloseOnExec(pidfd)
	keepPIDFD := false
	defer func() {
		if !keepPIDFD {
			_ = unix.Close(pidfd)
		}
	}()

	nsCtxFact := factNonObserved("not-probed", defaultNotProbedReason)
	if opts.IncludeNamespaceContextDigest {
		f, pErr := probeLinuxNamespaceContextDigest(s0.pidDirFD)
		if pErr != nil {
			return nil, nil, pErr
		}
		nsCtxFact = f
	}
	exeFact := factNonObserved("not-probed", defaultNotProbedReason)
	if opts.IncludeExecutableDigest {
		exeFact = probeLinuxExecutableDigest(s0.pidDirFD)
	}
	argsFact := factNonObserved("not-probed", defaultNotProbedReason)
	if opts.IncludeArgumentsDigest {
		f, pErr := probeLinuxArgumentsDigest(s0.pidDirFD)
		if pErr != nil {
			return nil, nil, pErr
		}
		argsFact = f
	}
	var secCtx0 map[string]any
	if opts.IncludeSecurityContextDigest {
		secCtx0 = probeLinuxSecurityContextDigest(s0.anchor, s0.statusText)
	}
	s0.closeFD()

	s1, err := captureTargetSnapshot(procFD, procMaj, procMin, pid, needIDMaps)
	if err != nil {
		return nil, nil, err
	}
	s1.closeFD()

	if s1.kind != "observed" || s0.incarnation != s1.incarnation || s1.incarnation.state != "live" || !idMappingEqual(s0.anchor.idMapping, s1.anchor.idMapping) {
		return nil, nil, newErrorRefusal("unbracketed-incarnation", "incarnation changed during pidfd registration")
	}

	secCtxFact := factNonObserved("not-probed", defaultNotProbedReason)
	if secCtx0 != nil {
		secCtx1 := probeLinuxSecurityContextDigest(s1.anchor, s1.statusText)
		if !evidence.JSONEqual(secCtx0, secCtx1) {
			return nil, nil, newErrorRefusal("unbracketed-incarnation", "security_context_digest changed during pidfd registration")
		}
		secCtxFact = secCtx0
	}

	spFact := factNonObserved("not-probed", defaultNotProbedReason)
	if opts.probeSecurityPrincipal() {
		spFact = evaluateLinuxPrincipal(s0.anchor, s0.incarnation)
	}

	baseline := assembleLinuxPayload(
		pid,
		hostScopeMap,
		s0.anchor,
		s0.incarnation.targetPIDNS.toFact(),
		factObserved("live"),
		startIdentityObserved(LinuxStartIdentityProfileID, s0.incarnation.startTicks),
		spFact,
		nsCtxFact,
		exeFact,
		argsFact,
		secCtxFact,
		opts,
	)

	keepPIDFD = true
	return baseline, &platformHandle{pidfd: pidfd}, nil
}
