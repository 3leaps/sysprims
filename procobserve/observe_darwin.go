// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build darwin

package procobserve

import (
	"bytes"
	"encoding/binary"
	"errors"
	"fmt"
	"math"
	"strconv"
	"strings"
	"time"

	"github.com/3leaps/sysprims/evidence"
	"golang.org/x/sys/unix"
)

const (
	kinfoProcSize64   = 648
	offPStarttimeSec  = 0
	offPStarttimeUsec = 8
	offPStat          = 36
	offPPID           = 40
	offPRuid          = 392
	offPRgid          = 400
	offCrUID          = 420
	offCrNgroups      = 424
	offCrGroups0      = 428

	darwinSIDL   byte = 1
	darwinSRUN   byte = 2
	darwinSSLEEP byte = 3
	darwinSSTOP  byte = 4
	darwinSZOMB  byte = 5

	macosUnsupportedPIDNSReason  = "Darwin kernel has a single boot-scoped process table without PID namespaces."
	macosUnsupportedTimeNSReason = "Darwin kernel has a single boot clock domain without time namespaces."
	macosUnsupportedUserNSReason = "Darwin kernel has a single credential numbering domain without user namespaces."
	macosUnsupportedNSCtxReason  = "Darwin kernel does not implement Linux-style process namespaces."
	macosDeferredSecCtxReason    = "macOS security-context derivation is deferred in native-process-observation:v0."
)

type platformHandle struct {
	kqFD   int
	exited bool
}

func (p *platformHandle) closeHandle() error {
	if p.kqFD < 0 {
		return nil
	}
	fd := p.kqFD
	p.kqFD = -1
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
		var ts unix.Timespec
		if hasDeadline {
			rem := time.Until(deadline)
			if rem > 0 {
				ts = unix.NsecToTimespec(rem.Nanoseconds())
			}
		}

		var evs [1]unix.Kevent_t
		n, err := unix.Kevent(p.kqFD, nil, evs[:], &ts)
		if err != nil {
			if errors.Is(err, unix.EINTR) {
				if !hasDeadline || time.Now().After(deadline) {
					return false, nil
				}
				continue
			}
			return false, newErrorRefusal("kqueue-wait-failed", fmt.Sprintf("kevent wait failed: %v", err))
		}
		if n > 0 {
			ev := evs[0]
			if (ev.Flags & unix.EV_ERROR) != 0 {
				return false, newErrorRefusal("kqueue-wait-error", fmt.Sprintf("kevent reported EV_ERROR data=%d", ev.Data))
			}
			if (ev.Fflags & unix.NOTE_EXIT) != 0 {
				p.exited = true
				return true, nil
			}
		}
		return false, nil
	}
}

type kinfoSnapshot struct {
	startIdentity   string
	state           string
	principalValue  string
	principalErrStr string
}

type kinfoOutcome struct {
	status   string
	reason   string
	snapshot kinfoSnapshot
}

func readBootSessionUUID() (string, error) {
	raw, err := unix.SysctlRaw("kern.bootsessionuuid")
	if err != nil || len(raw) == 0 {
		return "", newErrorRefusal("boot-id-unavailable", "sysctl(kern.bootsessionuuid) failed")
	}
	if nul := bytes.IndexByte(raw, 0); nul >= 0 {
		raw = raw[:nul]
	}
	normalized := strings.ToLower(strings.TrimSpace(string(raw)))
	if !isLowercaseUUID(normalized) {
		return "", newErrorRefusal("invalid-boot-id", "kern.bootsessionuuid failed lowercase UUID validation")
	}
	return normalized, nil
}

func readKinfoProc(pid uint32) kinfoOutcome {
	if pid == 0 || pid > MaxObservablePID {
		return kinfoOutcome{status: "error", reason: "invalid-pid-range"}
	}
	buf, err := unix.SysctlRaw("kern.proc.pid", int(pid))
	if err != nil {
		if errors.Is(err, unix.ESRCH) {
			return kinfoOutcome{status: "not-present", reason: "target-pid-absent"}
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return kinfoOutcome{status: "inaccessible", reason: "sysctl-kern-proc-pid-denied"}
		}
		return kinfoOutcome{status: "error", reason: "sysctl-kern-proc-pid-failed"}
	}
	if len(buf) == 0 {
		return kinfoOutcome{status: "not-present", reason: "target-pid-absent"}
	}
	if len(buf) != kinfoProcSize64 {
		return kinfoOutcome{status: "error", reason: "unexpected-kinfo-proc-size"}
	}

	tvSecBits := binary.LittleEndian.Uint64(buf[offPStarttimeSec : offPStarttimeSec+8])
	tvUsecBits := binary.LittleEndian.Uint32(buf[offPStarttimeUsec : offPStarttimeUsec+4])
	pStat := buf[offPStat]
	pPIDBits := binary.LittleEndian.Uint32(buf[offPPID : offPPID+4])
	pRuid := binary.LittleEndian.Uint32(buf[offPRuid : offPRuid+4])
	pRgid := binary.LittleEndian.Uint32(buf[offPRgid : offPRgid+4])
	crUID := binary.LittleEndian.Uint32(buf[offCrUID : offCrUID+4])
	crNgroupsBits := binary.LittleEndian.Uint16(buf[offCrNgroups : offCrNgroups+2])
	crGID0 := binary.LittleEndian.Uint32(buf[offCrGroups0 : offCrGroups0+4])

	if pPIDBits != pid {
		return kinfoOutcome{status: "error", reason: "kinfo-pid-mismatch"}
	}
	if tvSecBits > math.MaxInt64 || tvUsecBits > 999_999 {
		return kinfoOutcome{status: "error", reason: "invalid-kinfo-starttime"}
	}

	var state string
	switch pStat {
	case darwinSZOMB:
		state = "zombie"
	case darwinSRUN, darwinSSLEEP, darwinSSTOP:
		state = "live"
	case darwinSIDL:
		return kinfoOutcome{status: "error", reason: "embryonic-process-sidl"}
	default:
		return kinfoOutcome{status: "error", reason: "unexpected-proc-stat"}
	}

	snap := kinfoSnapshot{
		startIdentity: fmt.Sprintf("%s.%06d", strconv.FormatUint(tvSecBits, 10), tvUsecBits),
		state:         state,
	}
	if crNgroupsBits == 0 || crNgroupsBits > 0x7fff {
		snap.principalErrStr = "empty-kinfo-groups"
	} else {
		snap.principalValue = fmt.Sprintf("ruid=%d;euid=%d;rgid=%d;egid=%d", pRuid, crUID, pRgid, crGID0)
	}
	return kinfoOutcome{status: "observed", snapshot: snap}
}

func parseProcargs2Argv(buf []byte) ([][]byte, string) {
	if len(buf) < 4 {
		return nil, "malformed-kern-procargs2"
	}
	argcBits := binary.LittleEndian.Uint32(buf[0:4])
	if argcBits > 262_144 {
		return nil, "arguments-exceed-cap"
	}
	argc := int(argcBits)

	pos := 4
	for pos < len(buf) && buf[pos] != 0 {
		pos++
	}
	if pos == 4 || pos >= len(buf) {
		return nil, "malformed-kern-procargs2-execpath"
	}
	for pos < len(buf) && buf[pos] == 0 {
		pos++
	}
	if argc == 0 {
		return [][]byte{}, ""
	}
	argv := make([][]byte, 0, min(argc, 256))
	for i := 0; i < argc; i++ {
		if pos >= len(buf) {
			return nil, "truncated-kern-procargs2-argv"
		}
		start := pos
		for pos < len(buf) && buf[pos] != 0 {
			pos++
		}
		if pos >= len(buf) {
			return nil, "unterminated-kern-procargs2-arg"
		}
		argv = append(argv, buf[start:pos])
		pos++
	}
	return argv, ""
}

func probeMacOSArgumentsDigest(pid uint32, isZombie bool) map[string]any {
	buf, err := unix.SysctlRaw("kern.procargs2", int(pid))
	if err != nil {
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return factNonObserved("inaccessible", "kern-procargs2-denied")
		}
		if errors.Is(err, unix.ESRCH) {
			return factNonObserved("not-present", "target-pid-absent")
		}
		if isZombie && errors.Is(err, unix.EINVAL) {
			return factNonObserved("not-present", "zombie-process-image-released")
		}
		if errors.Is(err, unix.ENOMEM) {
			return factNonObserved("error", "arguments-exceed-cap")
		}
		return factNonObserved("error", "kern-procargs2-failed")
	}
	if len(buf) == 0 {
		if isZombie {
			return factNonObserved("not-present", "zombie-process-image-released")
		}
		return factNonObserved("error", "empty-kern-procargs2")
	}
	if len(buf) > MaxArgumentsRawBytes {
		return factNonObserved("error", "arguments-exceed-cap")
	}
	argv, reason := parseProcargs2Argv(buf)
	if reason != "" {
		return factNonObserved("error", reason)
	}
	return digestObserved(ArgvDigestDerivationID, evidence.DeriveArgumentsDigest(argv))
}

func probeMacOSExecutableDigest(pid uint32, isZombie bool) map[string]any {
	buf, err := unix.SysctlRaw("kern.procargs2", int(pid))
	if err != nil {
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return factNonObserved("inaccessible", "kern-procargs2-denied")
		}
		if errors.Is(err, unix.ESRCH) {
			return factNonObserved("not-present", "target-pid-absent")
		}
		if isZombie && errors.Is(err, unix.EINVAL) {
			return factNonObserved("not-present", "zombie-process-image-released")
		}
		return factNonObserved("error", "exec-path-resolution-failed")
	}
	if len(buf) <= 4 {
		if isZombie {
			return factNonObserved("not-present", "zombie-process-image-released")
		}
		return factNonObserved("error", "malformed-kern-procargs2")
	}
	if len(buf) > 1_048_576 {
		return factNonObserved("error", "exec-path-resolution-failed")
	}
	nulOff := bytes.IndexByte(buf[4:], 0)
	if nulOff <= 0 {
		return factNonObserved("error", "malformed-kern-procargs2-execpath")
	}
	execBytes := buf[4 : 4+nulOff]
	if execBytes[0] != '/' {
		return factNonObserved("error", "relative-exec-path-unsupported")
	}
	execPath := string(execBytes)

	var fd int
	for {
		fd, err = unix.Open(execPath, unix.O_RDONLY|unix.O_CLOEXEC, 0)
		if err == nil {
			break
		}
		if errors.Is(err, unix.EINTR) {
			continue
		}
		return errnoToDigestFact(err, "executable-open-failed")
	}
	defer func() {
		_ = unix.Close(fd)
	}()
	return streamExecutableDigestFromFD(fd)
}

func assembleMacOSPayload(
	pid uint32,
	hostScopeMap map[string]any,
	bootID string,
	processStateFact map[string]any,
	startIdentityFact map[string]any,
	securityPrincipalFact map[string]any,
	executableDigestFact map[string]any,
	argumentsDigestFact map[string]any,
	opts ObservationOptions,
) map[string]any {
	var nsCtxFact map[string]any
	if opts.IncludeNamespaceContextDigest {
		nsCtxFact = factNonObserved("unsupported", macosUnsupportedNSCtxReason)
	} else {
		nsCtxFact = factNonObserved("not-probed", defaultNotProbedReason)
	}
	var secCtxFact map[string]any
	if opts.IncludeSecurityContextDigest {
		secCtxFact = factNonObserved("unsupported", macosDeferredSecCtxReason)
	} else {
		secCtxFact = factNonObserved("not-probed", defaultNotProbedReason)
	}

	return map[string]any{
		"profile": map[string]any{
			"id":       evidence.NativeProcessObservationProfileID,
			"revision": evidence.NativeProcessObservationProfileRevision,
		},
		"observed_at": opts.ObservedAt,
		"observer": map[string]any{
			"platform_family": "macos",
			"architecture":    observerArchitecture(),
			"bracketed":       true,
			"time_namespace":  factNonObserved("unsupported", macosUnsupportedTimeNSReason),
			"user_namespace":  factNonObserved("unsupported", macosUnsupportedUserNSReason),
		},
		"process": map[string]any{
			"subject_ref":             opts.SubjectRef,
			"host_scope":              cloneMap(hostScopeMap),
			"boot_id":                 factObserved(bootID),
			"pid_numbering_namespace": factNonObserved("unsupported", macosUnsupportedPIDNSReason),
			"target_pid_namespace":    factNonObserved("unsupported", macosUnsupportedPIDNSReason),
			"pid":                     strconv.FormatUint(uint64(pid), 10),
			"process_state":           processStateFact,
			"start_identity":          startIdentityFact,
			"execution_context": map[string]any{
				"security_principal":       securityPrincipalFact,
				"namespace_context_digest": nsCtxFact,
				"executable_digest":        executableDigestFact,
				"arguments_digest":         argumentsDigestFact,
				"security_context_digest":  secCtxFact,
			},
		},
	}
}

func nonObservedMacOSContext(status, reason string, opts ObservationOptions) (map[string]any, map[string]any, map[string]any) {
	sp := factNonObserved("not-probed", defaultNotProbedReason)
	if opts.probeSecurityPrincipal() {
		sp = factNonObserved(status, reason)
	}
	exe := factNonObserved("not-probed", defaultNotProbedReason)
	if opts.IncludeExecutableDigest {
		exe = factNonObserved(status, reason)
	}
	args := factNonObserved("not-probed", defaultNotProbedReason)
	if opts.IncludeArgumentsDigest {
		args = factNonObserved(status, reason)
	}
	return sp, exe, args
}

func observeProcessPlatform(pid uint32, hostScopeMap map[string]any, opts ObservationOptions) (map[string]any, error) {
	for attempt := 0; attempt < 2; attempt++ {
		boot0, err := readBootSessionUUID()
		if err != nil {
			return nil, err
		}
		k0 := readKinfoProc(pid)

		switch k0.status {
		case "not-present", "inaccessible":
			boot1, err := readBootSessionUUID()
			if err != nil {
				return nil, err
			}
			if boot0 != boot1 {
				if attempt == 0 {
					continue
				}
				return nil, newErrorRefusal("unbracketed-incarnation", "boot session UUID changed during observation")
			}
			sp, exe, args := nonObservedMacOSContext(k0.status, k0.reason, opts)
			return assembleMacOSPayload(
				pid,
				hostScopeMap,
				boot0,
				factNonObserved(k0.status, k0.reason),
				startIdentityNonObserved(MacOSStartIdentityProfileID, k0.status, k0.reason),
				sp,
				exe,
				args,
				opts,
			), nil

		case "error":
			if attempt == 0 {
				continue
			}
			return nil, newErrorRefusal(k0.reason, "macos kinfo_proc probe failed")

		case "observed":
			snap0 := k0.snapshot
			isZombie := snap0.state == "zombie"
			exeFact := factNonObserved("not-probed", defaultNotProbedReason)
			if opts.IncludeExecutableDigest {
				exeFact = probeMacOSExecutableDigest(pid, isZombie)
			}
			argsFact := factNonObserved("not-probed", defaultNotProbedReason)
			if opts.IncludeArgumentsDigest {
				argsFact = probeMacOSArgumentsDigest(pid, isZombie)
			}

			boot1, err := readBootSessionUUID()
			if err != nil {
				return nil, err
			}
			k1 := readKinfoProc(pid)
			if k1.status != "observed" || boot0 != boot1 || k1.snapshot != snap0 {
				if attempt == 0 {
					continue
				}
				return nil, newErrorRefusal("unbracketed-incarnation", "incarnation tuple changed between pre/post sysctl snapshots")
			}

			spFact := factNonObserved("not-probed", defaultNotProbedReason)
			if opts.probeSecurityPrincipal() {
				if snap0.principalErrStr != "" {
					spFact = factNonObserved("error", snap0.principalErrStr)
				} else {
					spFact = principalObserved(MacOSPrincipalDerivationID, "urn:3leaps:posix-credentials:macos-kinfo:v0", snap0.principalValue)
				}
			}

			return assembleMacOSPayload(
				pid,
				hostScopeMap,
				boot0,
				factObserved(snap0.state),
				startIdentityObserved(MacOSStartIdentityProfileID, snap0.startIdentity),
				spFact,
				exeFact,
				argsFact,
				opts,
			), nil
		}
	}

	return nil, newErrorRefusal("unbracketed-incarnation", "failed to bracket macOS process observation")
}

func openContinuityHandlePlatform(pid uint32, hostScopeMap map[string]any, opts ObservationOptions) (map[string]any, *platformHandle, error) {
	boot0, err := readBootSessionUUID()
	if err != nil {
		return nil, nil, err
	}
	k0 := readKinfoProc(pid)
	switch k0.status {
	case "not-present":
		return nil, nil, newRefusal("not-present", "target-not-present", "target PID does not exist")
	case "inaccessible":
		return nil, nil, newRefusal("inaccessible", k0.reason, "target PID is inaccessible")
	case "error":
		return nil, nil, newErrorRefusal(k0.reason, "failed to read target kinfo_proc")
	}
	snap0 := k0.snapshot
	if snap0.state != "live" {
		return nil, nil, newErrorRefusal("target-not-live", "continuity handle requires a live target process (not zombie)")
	}

	kq, err := unix.Kqueue()
	if err != nil {
		return nil, nil, newErrorRefusal("kqueue-create-failed", fmt.Sprintf("kqueue() failed: %v", err))
	}
	unix.CloseOnExec(kq)
	keepFD := false
	defer func() {
		if !keepFD {
			_ = unix.Close(kq)
		}
	}()

	changes := [1]unix.Kevent_t{{
		Ident:  uint64(pid),
		Filter: unix.EVFILT_PROC,
		Flags:  unix.EV_ADD | unix.EV_ENABLE | unix.EV_ONESHOT,
		Fflags: unix.NOTE_EXIT,
	}}
	var outEvs [1]unix.Kevent_t
	var zeroTS unix.Timespec
	n, err := unix.Kevent(kq, changes[:], outEvs[:], &zeroTS)
	if err != nil {
		if errors.Is(err, unix.ESRCH) {
			return nil, nil, newRefusal("not-present", "target-not-present", "target PID exited before kqueue registration")
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return nil, nil, newRefusal("inaccessible", "kqueue-proc-denied", "kernel denied EVFILT_PROC NOTE_EXIT registration")
		}
		return nil, nil, newErrorRefusal("kqueue-registration-failed", fmt.Sprintf("kevent registration failed: %v", err))
	}
	if n > 0 {
		outEv := outEvs[0]
		if (outEv.Flags & unix.EV_ERROR) != 0 {
			if outEv.Data == int64(unix.ESRCH) {
				return nil, nil, newRefusal("not-present", "target-not-present", "target PID exited before kqueue registration")
			}
			if outEv.Data == int64(unix.EPERM) || outEv.Data == int64(unix.EACCES) {
				return nil, nil, newRefusal("inaccessible", "kqueue-proc-denied", "kernel denied EVFILT_PROC NOTE_EXIT registration")
			}
			return nil, nil, newErrorRefusal("kqueue-registration-failed", fmt.Sprintf("kevent EV_ERROR data=%d", outEv.Data))
		}
		if (outEv.Fflags & unix.NOTE_EXIT) != 0 {
			return nil, nil, newErrorRefusal("target-not-live", "target process exited during kqueue registration")
		}
	}

	exeFact := factNonObserved("not-probed", defaultNotProbedReason)
	if opts.IncludeExecutableDigest {
		exeFact = probeMacOSExecutableDigest(pid, false)
	}
	argsFact := factNonObserved("not-probed", defaultNotProbedReason)
	if opts.IncludeArgumentsDigest {
		argsFact = probeMacOSArgumentsDigest(pid, false)
	}

	boot1, err := readBootSessionUUID()
	if err != nil {
		return nil, nil, err
	}
	k1 := readKinfoProc(pid)
	if k1.status != "observed" || boot0 != boot1 || k1.snapshot != snap0 || k1.snapshot.state != "live" {
		return nil, nil, newErrorRefusal("unbracketed-incarnation", "incarnation changed during continuity handle registration")
	}

	spFact := factNonObserved("not-probed", defaultNotProbedReason)
	if opts.probeSecurityPrincipal() {
		if snap0.principalErrStr != "" {
			spFact = factNonObserved("error", snap0.principalErrStr)
		} else {
			spFact = principalObserved(MacOSPrincipalDerivationID, "urn:3leaps:posix-credentials:macos-kinfo:v0", snap0.principalValue)
		}
	}

	baseline := assembleMacOSPayload(
		pid,
		hostScopeMap,
		boot0,
		factObserved("live"),
		startIdentityObserved(MacOSStartIdentityProfileID, snap0.startIdentity),
		spFact,
		exeFact,
		argsFact,
		opts,
	)

	keepFD = true
	return baseline, &platformHandle{kqFD: kq}, nil
}
