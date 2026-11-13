// SPDX-License-Identifier: MIT OR Apache-2.0

// Package signal provides cross-platform process signal dispatch and
// process-bound exit/signal handles (ProcessHandle) for Go without cgo
// (CGO_ENABLED=0).
//
// This package is the pure-Go twin of crates/sysprims-signal. It enforces the
// ADR-0011 PID/PGID safety invariants at the API boundary:
//
//   - PID/PGID == 0 is rejected (kill(0, sig) would signal the caller's own
//     process group).
//   - PID/PGID > MaxSafePID (2147483647, math.MaxInt32) is rejected (values
//     above MaxInt32 overflow to negative pid_t on 32-bit signed POSIX interfaces,
//     where kill(-1, sig) broadcasts to every reachable process).
//
// For unattended escalation and timeout supervision, prefer OpenProcessHandle
// over raw numeric Kill/ForceKill: ProcessHandle brackets handle registration
// with native process start-identity checks and binds signal delivery to an OS
// descriptor/handle where supported (Linux pidfd, Windows HANDLE) or re-checks
// the kqueue exit watch and kinfo_proc start-identity immediately before
// numeric kill on macOS.
package signal

import (
	"fmt"
	"math"
	"runtime"
	"strings"
	"sync/atomic"

	"github.com/3leaps/sysprims/syserror"
)

// Error and ErrorCode re-export canonical sysprims error types from syserror.
type (
	Error     = syserror.Error
	ErrorCode = syserror.ErrorCode
)

const (
	ErrOK                  = syserror.ErrOK
	ErrInvalidArgument     = syserror.ErrInvalidArgument
	ErrSpawnFailed         = syserror.ErrSpawnFailed
	ErrTimeout             = syserror.ErrTimeout
	ErrPermissionDenied    = syserror.ErrPermissionDenied
	ErrNotFound            = syserror.ErrNotFound
	ErrNotSupported        = syserror.ErrNotSupported
	ErrGroupCreationFailed = syserror.ErrGroupCreationFailed
	ErrSystem              = syserror.ErrSystem
	ErrInternal            = syserror.ErrInternal
)

// MaxSafePID is the maximum valid PID/PGID value (math.MaxInt32, 2147483647).
//
// PIDs above this value would overflow to negative when cast to a signed 32-bit
// pid_t, which has broadcast/process-group semantics in POSIX kill(2).
const MaxSafePID uint32 = math.MaxInt32

func validatePIDParam(pid uint32, paramName string) error {
	if pid == 0 {
		return syserror.InvalidArgument(fmt.Sprintf("%s must be > 0", paramName))
	}
	if pid > MaxSafePID {
		return syserror.InvalidArgument(fmt.Sprintf(
			"%s %d exceeds maximum safe value %d; larger values overflow to negative PIDs with dangerous POSIX semantics (see docs/safety/signal-dispatch.md)",
			paramName, pid, MaxSafePID,
		))
	}
	return nil
}

// ValidatePID validates that pid is in [1, MaxSafePID] per ADR-0011.
func ValidatePID(pid uint32) error {
	return validatePIDParam(pid, "pid")
}

// ValidatePGID validates that pgid is in [1, MaxSafePID] per ADR-0011.
func ValidatePGID(pgid uint32) error {
	return validatePIDParam(pgid, "pgid")
}

func validatePIDList(pids []uint32, paramName string) error {
	if len(pids) == 0 {
		return syserror.InvalidArgument(fmt.Sprintf("%s must not be empty", paramName))
	}
	for _, pid := range pids {
		if err := validatePIDParam(pid, paramName); err != nil {
			return err
		}
	}
	return nil
}

// BatchKillFailure records a per-PID failure during KillMany / TerminateMany /
// ForceKillMany.
type BatchKillFailure struct {
	PID uint32 `json:"pid"`
	Err error  `json:"error"`
}

// BatchKillResult contains the succeeded and failed PIDs from a batch signal
// operation.
type BatchKillResult struct {
	Succeeded []uint32           `json:"succeeded"`
	Failed    []BatchKillFailure `json:"failed"`
}

// Kill sends a signal to a single process by PID after validating ADR-0011
// safety bounds.
func Kill(pid uint32, sig int) error {
	if err := ValidatePID(pid); err != nil {
		return err
	}
	return killImpl(pid, sig)
}

// KillByName resolves signalName via the sysprims signal catalog and sends it
// to pid.
func KillByName(pid uint32, signalName string) error {
	sig, err := ResolveSignalName(signalName)
	if err != nil {
		return err
	}
	return Kill(pid, sig)
}

// KillGroup sends a signal to a process group by PGID after validating ADR-0011
// safety bounds. On Windows, always returns ErrNotSupported.
func KillGroup(pgid uint32, sig int) error {
	if err := ValidatePGID(pgid); err != nil {
		return err
	}
	return killGroupImpl(pgid, sig)
}

// KillGroupByName resolves signalName via the sysprims signal catalog and sends
// it to process group pgid.
func KillGroupByName(pgid uint32, signalName string) error {
	sig, err := ResolveSignalName(signalName)
	if err != nil {
		return err
	}
	return KillGroup(pgid, sig)
}

// KillMany validates the entire pids slice before sending sig to each PID,
// collecting per-PID results in BatchKillResult.
func KillMany(pids []uint32, sig int) (*BatchKillResult, error) {
	if err := validatePIDList(pids, "pids"); err != nil {
		return nil, err
	}
	res := &BatchKillResult{
		Succeeded: make([]uint32, 0, len(pids)),
		Failed:    make([]BatchKillFailure, 0),
	}
	for _, pid := range pids {
		if err := Kill(pid, sig); err != nil {
			res.Failed = append(res.Failed, BatchKillFailure{PID: pid, Err: err})
		} else {
			res.Succeeded = append(res.Succeeded, pid)
		}
	}
	return res, nil
}

// Terminate sends SIGTERM to pid.
func Terminate(pid uint32) error {
	return Kill(pid, SIGTERM)
}

// ForceKill sends SIGKILL to pid.
func ForceKill(pid uint32) error {
	return Kill(pid, SIGKILL)
}

// TerminateGroup sends SIGTERM to process group pgid.
func TerminateGroup(pgid uint32) error {
	return KillGroup(pgid, SIGTERM)
}

// ForceKillGroup sends SIGKILL to process group pgid.
func ForceKillGroup(pgid uint32) error {
	return KillGroup(pgid, SIGKILL)
}

// TerminateMany sends SIGTERM to every PID in pids after validating the slice.
func TerminateMany(pids []uint32) (*BatchKillResult, error) {
	return KillMany(pids, SIGTERM)
}

// ForceKillMany sends SIGKILL to every PID in pids after validating the slice.
func ForceKillMany(pids []uint32) (*BatchKillResult, error) {
	return KillMany(pids, SIGKILL)
}

// SignalBinding describes how tightly signal delivery on a ProcessHandle is
// bound to the verified process incarnation.
type SignalBinding string

const (
	// SignalBindingBound indicates kernel-bound signal delivery via an OS
	// descriptor/handle (pidfd on Linux >= 5.3, HANDLE on Windows).
	SignalBindingBound SignalBinding = "bound"

	// SignalBindingCheckedNumeric indicates that the exit watch and process
	// start-identity are re-verified immediately before a numeric kill(pid, sig)
	// call (macOS; Linux < 5.3 ENOSYS fallback).
	SignalBindingCheckedNumeric SignalBinding = "checked_numeric"
)

// ProcessHandle is a signal and exit-watch handle bound to a verified process
// incarnation.
//
// ProcessHandle embeds a non-serializable guard so json.Marshal / yaml.Marshal
// fail closed and raw OS descriptors/handles are never exposed.
type ProcessHandle struct {
	noCopy noCopy //nolint:unused

	pid           uint32
	startIdentity string
	binding       SignalBinding
	exited        atomic.Bool
	closed        atomic.Bool
	inner         *platformProcessHandle
}

type noCopy struct{}

func (*noCopy) Lock()   {}
func (*noCopy) Unlock() {}

// MarshalJSON fails closed so a live ProcessHandle cannot be serialized into
// logs or wire payloads.
func (*ProcessHandle) MarshalJSON() ([]byte, error) {
	return nil, syserror.InvalidArgument("ProcessHandle is not serializable")
}

// MarshalText fails closed so a live ProcessHandle cannot be marshaled as text.
func (*ProcessHandle) MarshalText() ([]byte, error) {
	return nil, syserror.InvalidArgument("ProcessHandle is not serializable")
}

// String returns a redacted diagnostic representation that omits raw OS
// file descriptors or handles.
func (h *ProcessHandle) String() string {
	if h == nil {
		return "ProcessHandle(<nil>)"
	}
	return fmt.Sprintf(
		"ProcessHandle{pid:%d, start_identity:%q, binding:%q, exited:%t, closed:%t}",
		h.pid,
		h.startIdentity,
		h.binding,
		h.exited.Load(),
		h.closed.Load(),
	)
}

// OpenProcessHandle opens a ProcessHandle for pid, bracketing registration with
// native start-identity checks before and after acquiring the OS handle.
func OpenProcessHandle(pid uint32) (*ProcessHandle, error) {
	return openProcessHandleInner(pid, "")
}

// OpenProcessHandleVerified opens a ProcessHandle for pid and requires its
// bracketed native start-identity to equal expectedStartIdentity.
//
// If pid is currently held by a different process incarnation, returns
// ErrNotFound.
func OpenProcessHandleVerified(pid uint32, expectedStartIdentity string) (*ProcessHandle, error) {
	trimmed := strings.TrimSpace(expectedStartIdentity)
	if trimmed == "" {
		return nil, syserror.InvalidArgument("expected_start_identity must not be empty")
	}
	return openProcessHandleInner(pid, trimmed)
}

func openProcessHandleInner(pid uint32, expectedStartIdentity string) (*ProcessHandle, error) {
	if err := ValidatePID(pid); err != nil {
		return nil, err
	}
	startIdentity, binding, inner, err := openPlatformHandle(pid, expectedStartIdentity)
	if err != nil {
		return nil, err
	}
	h := &ProcessHandle{
		pid:           pid,
		startIdentity: startIdentity,
		binding:       binding,
		inner:         inner,
	}
	runtime.SetFinalizer(h, func(ph *ProcessHandle) {
		_ = ph.Close()
	})
	return h, nil
}

// PID returns the numeric PID verified at handle creation.
func (h *ProcessHandle) PID() uint32 {
	if h == nil {
		return 0
	}
	return h.pid
}

// StartIdentity returns the canonical native start-identity string captured at
// handle creation.
func (h *ProcessHandle) StartIdentity() string {
	if h == nil {
		return ""
	}
	return h.startIdentity
}

// Binding returns the SignalBinding strength for signals sent through this
// handle.
func (h *ProcessHandle) Binding() SignalBinding {
	if h == nil {
		return ""
	}
	return h.binding
}

// IsClosed reports whether Close has been called on this handle.
func (h *ProcessHandle) IsClosed() bool {
	if h == nil {
		return true
	}
	return h.closed.Load()
}

func (h *ProcessHandle) ensureOpen() error {
	if h == nil || h.closed.Load() {
		return syserror.InvalidArgument("process handle is closed")
	}
	return nil
}

// Exited reports whether whole-process exit has been observed for the bound
// process.
//
// Once true is observed, the result is latched on the handle and subsequent
// calls return (true, nil) even after the process is reaped or its numeric PID
// is reused.
func (h *ProcessHandle) Exited() (bool, error) {
	if err := h.ensureOpen(); err != nil {
		return false, err
	}
	if h.exited.Load() {
		return true, nil
	}
	nowExited, err := h.inner.checkExited(h.pid, h.startIdentity)
	if err != nil {
		return false, err
	}
	if nowExited {
		h.exited.Store(true)
	}
	return nowExited, nil
}

// Signal delivers sig to the bound process only, returning the SignalBinding
// strength used for delivery.
//
// Once exit is observed, no signal is sent and ErrNotFound is returned.
func (h *ProcessHandle) Signal(sig int) (SignalBinding, error) {
	if err := h.ensureOpen(); err != nil {
		return "", err
	}
	exited, err := h.Exited()
	if err != nil {
		return "", err
	}
	if exited {
		return "", syserror.NotFound(h.pid)
	}
	binding, err := h.inner.sendSignal(h.pid, h.startIdentity, sig)
	if err != nil {
		if syserror.IsCode(err, ErrNotFound) {
			h.exited.Store(true)
		}
		return "", err
	}
	return binding, nil
}

// SignalByName resolves signalName and delivers it to the bound process.
func (h *ProcessHandle) SignalByName(signalName string) (SignalBinding, error) {
	if err := h.ensureOpen(); err != nil {
		return "", err
	}
	sig, err := ResolveSignalName(signalName)
	if err != nil {
		return "", err
	}
	return h.Signal(sig)
}

// Terminate delivers SIGTERM to the bound process.
func (h *ProcessHandle) Terminate() (SignalBinding, error) {
	return h.Signal(SIGTERM)
}

// ForceKill delivers SIGKILL to the bound process.
func (h *ProcessHandle) ForceKill() (SignalBinding, error) {
	return h.Signal(SIGKILL)
}

// Close releases the underlying OS descriptor/handle (idempotent).
func (h *ProcessHandle) Close() error {
	if h == nil {
		return nil
	}
	if !h.closed.Swap(true) {
		runtime.SetFinalizer(h, nil)
		if h.inner != nil {
			h.inner.closeHandle()
		}
	}
	return nil
}
