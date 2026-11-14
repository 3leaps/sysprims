// SPDX-License-Identifier: MIT OR Apache-2.0

package timeout

import (
	"bytes"
	"io"
	"math"
	"os"
	"os/exec"
	"runtime"
	"sort"
	"sync"
	"sync/atomic"
	"time"

	"github.com/3leaps/sysprims/signal"
	"github.com/3leaps/sysprims/syserror"
)

type resolvedTerminateConfig struct {
	graceTimeoutMS uint64
	killTimeoutMS  uint64
	signal         int32
	killSignal     int32
}

func defaultResolvedTerminateConfig() resolvedTerminateConfig {
	return resolvedTerminateConfig{
		graceTimeoutMS: 10_000,
		killTimeoutMS:  2_000,
		signal:         int32(signal.SIGTERM),
		killSignal:     int32(signal.SIGKILL),
	}
}

func platformMaxSignal() int32 {
	if runtime.GOOS == "linux" {
		return 64
	}
	return 31
}

func validatePlatformSignal(sig int32, name string) error {
	maxSig := platformMaxSignal()
	if sig <= 0 || sig > maxSig {
		return syserror.InvalidArgumentf("%s must be between 1 and %d", name, maxSig)
	}
	return nil
}

func resolveTerminateTreeConfig(cfg TerminateTreeConfig) (resolvedTerminateConfig, error) {
	out := defaultResolvedTerminateConfig()
	if cfg.GraceTimeoutMS != nil {
		if *cfg.GraceTimeoutMS > MaxDurationMS {
			return out, syserror.InvalidArgumentf("grace_timeout_ms exceeds maximum of %d", MaxDurationMS)
		}
		out.graceTimeoutMS = *cfg.GraceTimeoutMS
	}
	if cfg.KillTimeoutMS != nil {
		if *cfg.KillTimeoutMS > MaxDurationMS {
			return out, syserror.InvalidArgumentf("kill_timeout_ms exceeds maximum of %d", MaxDurationMS)
		}
		out.killTimeoutMS = *cfg.KillTimeoutMS
	}
	if cfg.Signal != nil {
		if err := validatePlatformSignal(*cfg.Signal, "signal"); err != nil {
			return out, err
		}
		out.signal = *cfg.Signal
	}
	if cfg.KillSignal != nil {
		if err := validatePlatformSignal(*cfg.KillSignal, "kill_signal"); err != nil {
			return out, err
		}
		out.killSignal = *cfg.KillSignal
	}
	return out, nil
}

func validateSpawnRequest(req SpawnContainedConfig) (resolvedTerminateConfig, error) {
	var zero resolvedTerminateConfig
	if len(req.Argv) == 0 {
		return zero, syserror.InvalidArgument("argv must not be empty")
	}
	if len(req.Argv) > MaxArgvEntries {
		return zero, syserror.InvalidArgumentf("argv exceeds maximum of %d entries", MaxArgvEntries)
	}
	argvBytes := 0
	for i, arg := range req.Argv {
		if arg == "" {
			return zero, syserror.InvalidArgumentf("argv[%d] must not be empty", i)
		}
		if indexByteZero(arg) {
			return zero, syserror.InvalidArgumentf("argv[%d] must not contain NUL bytes", i)
		}
		if len(arg) > MaxArgvEntryBytes {
			return zero, syserror.InvalidArgumentf("argv[%d] exceeds maximum of %d bytes", i, MaxArgvEntryBytes)
		}
		argvBytes += len(arg)
		if argvBytes > MaxArgvTotalBytes {
			return zero, syserror.InvalidArgumentf("argv exceeds maximum of %d bytes", MaxArgvTotalBytes)
		}
	}

	if req.Cwd != nil {
		if *req.Cwd == "" {
			return zero, syserror.InvalidArgument("cwd must not be empty")
		}
		if indexByteZero(*req.Cwd) {
			return zero, syserror.InvalidArgument("cwd must not contain NUL bytes")
		}
		if len(*req.Cwd) > MaxCwdBytes {
			return zero, syserror.InvalidArgumentf("cwd exceeds maximum of %d bytes", MaxCwdBytes)
		}
	}

	if req.Env != nil {
		if len(req.Env) > MaxEnvEntries {
			return zero, syserror.InvalidArgumentf("env exceeds maximum of %d entries", MaxEnvEntries)
		}
		envBytes := 0
		for k, v := range req.Env {
			if k == "" {
				return zero, syserror.InvalidArgument("env keys must not be empty")
			}
			if indexByteZero(k) || indexByteZero(v) {
				return zero, syserror.InvalidArgument("env keys and values must not contain NUL bytes")
			}
			if len(k) > MaxEnvKeyBytes {
				return zero, syserror.InvalidArgumentf("env key exceeds maximum of %d bytes", MaxEnvKeyBytes)
			}
			if len(v) > MaxEnvValueBytes {
				return zero, syserror.InvalidArgumentf("env value exceeds maximum of %d bytes", MaxEnvValueBytes)
			}
			envBytes += len(k) + len(v)
			if envBytes > MaxEnvTotalBytes {
				return zero, syserror.InvalidArgumentf("env exceeds maximum of %d bytes", MaxEnvTotalBytes)
			}
		}
	}

	if req.ExecutionTimeoutMS != nil {
		if *req.ExecutionTimeoutMS == 0 || *req.ExecutionTimeoutMS > MaxDurationMS {
			return zero, syserror.InvalidArgumentf("execution_timeout_ms must be between 1 and %d", MaxDurationMS)
		}
	}

	return resolveTerminateTreeConfig(TerminateTreeConfig{
		GraceTimeoutMS: req.GraceTimeoutMS,
		KillTimeoutMS:  req.KillTimeoutMS,
		Signal:         req.Signal,
		KillSignal:     req.KillSignal,
	})
}

type slotState uint8

const (
	slotEmpty slotState = iota
	slotActive
	slotFinalizing
	slotInert
)

type leaderStatus string

const (
	leaderRunning    leaderStatus = "running"
	leaderCompleted  leaderStatus = "completed"
	leaderTimedOut   leaderStatus = "timed_out"
	leaderTerminated leaderStatus = "terminated"
)

type containmentOutcome struct {
	identity            ContainmentIdentity
	pgid                *uint32
	signalSent          *int32
	killSignal          *int32
	escalated           bool
	exited              bool
	timedOut            bool
	treeKillReliability string
	boundaryStrength    string
	completion          ContainmentCompletion
	warnings            []string
}

type platformContainmentGuard interface {
	identity() ContainmentIdentity
	treeKillReliability() string
	boundaryStrength() string
	tryComplete(cfg resolvedTerminateConfig) (*containmentOutcome, error)
	terminate(cfg resolvedTerminateConfig) (*containmentOutcome, error)
	dropGuard()
}

type slotData struct {
	generation      uint32
	retired         bool
	state           slotState
	guard           platformContainmentGuard
	identity        *ContainmentIdentity
	reliability     string
	boundary        string
	terminateConfig resolvedTerminateConfig
	outcome         *containmentOutcome
	leaderStatus    leaderStatus
	deadlineCancel  *atomic.Bool
	deadlineAt      time.Time
}

type registrySlot struct {
	mu   sync.Mutex
	cond *sync.Cond
	data slotData
}

type containmentRegistry struct {
	slots [MaxRegistrySlots]registrySlot
	freeM sync.Mutex
	free  []uint32
}

var (
	globalRegistryOnce sync.Once
	globalRegistry     *containmentRegistry
)

func getRegistry() *containmentRegistry {
	globalRegistryOnce.Do(func() {
		reg := &containmentRegistry{
			free: make([]uint32, 0, MaxRegistrySlots),
		}
		for i := 0; i < MaxRegistrySlots; i++ {
			slot := &reg.slots[i]
			slot.cond = sync.NewCond(&slot.mu)
			slot.data = slotData{
				generation:      1,
				state:           slotEmpty,
				terminateConfig: defaultResolvedTerminateConfig(),
				leaderStatus:    leaderRunning,
				deadlineCancel:  &atomic.Bool{},
			}
			reg.free = append(reg.free, uint32(i)) // #nosec G115 -- i < MaxRegistrySlots
		}
		globalRegistry = reg
	})
	return globalRegistry
}

func packToken(slot, generation uint32) uint64 {
	return (uint64(generation) << 32) | (uint64(slot) + 1)
}

func unpackToken(token uint64) (uint32, uint32, error) {
	if token == 0 {
		return 0, 0, syserror.InvalidArgument("containment handle is invalid")
	}
	slotPlusOne := token & 0xffff_ffff
	if slotPlusOne == 0 {
		return 0, 0, syserror.InvalidArgument("containment handle is invalid")
	}
	slot := slotPlusOne - 1
	if slot >= MaxRegistrySlots {
		return 0, 0, syserror.InvalidArgument("containment handle is invalid")
	}
	generation := uint32(token >> 32) // #nosec G115 -- upper 32 bits of uint64 fit in uint32
	if generation == 0 {
		return 0, 0, syserror.InvalidArgument("containment handle is invalid")
	}
	return uint32(slot), generation, nil // #nosec G115 -- validated slot < MaxRegistrySlots
}

func staleHandleErr() error {
	return syserror.InvalidArgument("containment handle is stale or closed")
}

func cleanupNotConfirmedErr() error {
	return syserror.InvalidArgument("containment cleanup did not confirm child reaped; retry")
}

func allocateSlot() (uint32, uint32, error) {
	reg := getRegistry()
	reg.freeM.Lock()
	defer reg.freeM.Unlock()

	for len(reg.free) > 0 {
		idx := reg.free[len(reg.free)-1]
		reg.free = reg.free[:len(reg.free)-1]
		slot := &reg.slots[idx]
		slot.mu.Lock()
		if slot.data.retired || slot.data.state != slotEmpty {
			slot.mu.Unlock()
			continue
		}
		gen := slot.data.generation
		slot.mu.Unlock()
		return idx, gen, nil
	}
	return 0, 0, syserror.InvalidArgument("containment registry has no free slots")
}

func pushFreeIndex(slotIndex uint32) {
	reg := getRegistry()
	reg.freeM.Lock()
	reg.free = append(reg.free, slotIndex)
	reg.freeM.Unlock()
}

func resetSlotForReuse(data *slotData) bool {
	data.guard = nil
	data.identity = nil
	data.reliability = ""
	data.boundary = ""
	data.outcome = nil
	data.leaderStatus = leaderRunning
	if data.deadlineCancel != nil {
		data.deadlineCancel.Store(true)
	}
	data.deadlineCancel = &atomic.Bool{}
	data.deadlineAt = time.Time{}
	data.state = slotEmpty
	if data.generation == math.MaxUint32 {
		data.retired = true
		return false
	}
	next := data.generation + 1
	if next == 0 || next == math.MaxUint32 {
		data.generation = math.MaxUint32
		data.retired = true
		return false
	}
	data.generation = next
	return true
}

func recycleAfterDataUnlock(slot *registrySlot, slotIndex uint32) {
	shouldPush := resetSlotForReuse(&slot.data)
	slot.cond.Broadcast()
	slot.mu.Unlock()
	if shouldPush {
		pushFreeIndex(slotIndex)
	}
}

func recycleUnusedSlot(slotIndex, generation uint32) {
	reg := getRegistry()
	slot := &reg.slots[slotIndex]
	slot.mu.Lock()
	if slot.data.generation == generation && slot.data.state == slotEmpty && !slot.data.retired {
		slot.mu.Unlock()
		pushFreeIndex(slotIndex)
		return
	}
	slot.mu.Unlock()
}

func lookupSlot(token uint64) (uint32, uint32, *registrySlot, error) {
	slotIndex, generation, err := unpackToken(token)
	if err != nil {
		return 0, 0, nil, err
	}
	slot := &getRegistry().slots[slotIndex]
	return slotIndex, generation, slot, nil
}

func snapshotFrom(data *slotData, handleState string, status leaderStatus) (*ContainmentSnapshot, error) {
	if data.identity == nil || data.reliability == "" || data.boundary == "" {
		return nil, syserror.Internal("containment metadata missing")
	}
	snap := &ContainmentSnapshot{
		SchemaID:            ContainmentSnapshotSchemaID,
		Timestamp:           nowRFC3339(),
		Platform:            currentPlatform(),
		HandleState:         handleState,
		LeaderStatus:        string(status),
		Identity:            *data.identity,
		TreeKillReliability: data.reliability,
		BoundaryStrength:    data.boundary,
		Warnings:            []string{},
	}
	if data.outcome != nil {
		snap.PGID = data.outcome.pgid
		snap.SignalSent = data.outcome.signalSent
		snap.KillSignal = data.outcome.killSignal
		esc := data.outcome.escalated
		ex := data.outcome.exited
		to := data.outcome.timedOut
		comp := data.outcome.completion
		snap.Escalated = &esc
		snap.Exited = &ex
		snap.TimedOut = &to
		snap.Completion = &comp
		if data.outcome.warnings != nil {
			snap.Warnings = append([]string(nil), data.outcome.warnings...)
		}
	}
	return snap, nil
}

func commitInert(slot *registrySlot, outcome *containmentOutcome, status leaderStatus) (*ContainmentSnapshot, error) {
	if slot.data.deadlineCancel != nil {
		slot.data.deadlineCancel.Store(true)
	}
	slot.data.outcome = outcome
	slot.data.leaderStatus = status
	slot.data.state = slotInert
	slot.cond.Broadcast()
	return snapshotFrom(&slot.data, "inert", status)
}

func startDeadlineMonitor(token uint64, deadline time.Time, cancel *atomic.Bool) {
	go func() {
		for {
			if cancel.Load() {
				return
			}
			now := time.Now()
			if !now.Before(deadline) {
				if _, err := terminateWithStatus(token, leaderTimedOut); err == nil {
					return
				}
				time.Sleep(pollInterval)
				continue
			}
			rem := deadline.Sub(now)
			if rem > pollInterval {
				rem = pollInterval
			}
			time.Sleep(rem)
		}
	}()
}

func restoreDeadlineMonitor(data *slotData, token uint64) {
	if data.deadlineAt.IsZero() {
		return
	}
	if data.deadlineCancel != nil && !data.deadlineCancel.Load() {
		return
	}
	cancel := &atomic.Bool{}
	data.deadlineCancel = cancel
	startDeadlineMonitor(token, data.deadlineAt, cancel)
}

func waitForNotFinalizing(slot *registrySlot, waitDeadline time.Time) {
	generation := slot.data.generation
	if !waitDeadline.IsZero() {
		done := make(chan struct{})
		defer close(done)
		go func() {
			timer := time.NewTimer(time.Until(waitDeadline))
			defer timer.Stop()
			select {
			case <-timer.C:
				slot.mu.Lock()
				slot.cond.Broadcast()
				slot.mu.Unlock()
			case <-done:
			}
		}()
	}
	for slot.data.state == slotFinalizing && slot.data.generation == generation {
		if !waitDeadline.IsZero() && !time.Now().Before(waitDeadline) {
			return
		}
		slot.cond.Wait()
	}
}

func managedIdentity(token uint64) (*ContainmentSnapshot, error) {
	_, generation, slot, err := lookupSlot(token)
	if err != nil {
		return nil, err
	}
	slot.mu.Lock()
	defer slot.mu.Unlock()

	if slot.data.generation != generation || slot.data.retired {
		return nil, staleHandleErr()
	}
	switch slot.data.state {
	case slotEmpty:
		return nil, staleHandleErr()
	case slotActive, slotFinalizing:
		return snapshotFrom(&slot.data, "active", leaderRunning)
	case slotInert:
		return snapshotFrom(&slot.data, "inert", slot.data.leaderStatus)
	default:
		return nil, staleHandleErr()
	}
}

func managedPoll(token uint64) (*ContainmentSnapshot, error) {
	return operateInner(token, leaderCompleted, time.Now(), func(guard platformContainmentGuard, cfg resolvedTerminateConfig) (*containmentOutcome, error) {
		return guard.tryComplete(cfg)
	})
}

func managedWait(token uint64, waitTimeoutMS uint64) (*ContainmentSnapshot, error) {
	if waitTimeoutMS > MaxDurationMS {
		return nil, syserror.InvalidArgumentf("wait timeout exceeds maximum of %d", MaxDurationMS)
	}
	var waitDeadline time.Time
	if waitTimeoutMS > 0 {
		waitDeadline = time.Now().Add(time.Duration(waitTimeoutMS) * time.Millisecond)
	}

	for {
		snap, err := operateInner(token, leaderCompleted, waitDeadline, func(guard platformContainmentGuard, cfg resolvedTerminateConfig) (*containmentOutcome, error) {
			return guard.tryComplete(cfg)
		})
		if err != nil {
			return nil, err
		}
		if snap.LeaderStatus != string(leaderRunning) {
			return snap, nil
		}
		if !waitDeadline.IsZero() && !time.Now().Before(waitDeadline) {
			return snap, nil
		}
		sleepDur := pollInterval
		if !waitDeadline.IsZero() {
			if rem := time.Until(waitDeadline); rem < sleepDur {
				if rem <= 0 {
					return snap, nil
				}
				sleepDur = rem
			}
		}
		time.Sleep(sleepDur)
	}
}

func managedTerminate(token uint64) (*ContainmentSnapshot, error) {
	return terminateWithStatus(token, leaderTerminated)
}

func terminateWithStatus(token uint64, status leaderStatus) (*ContainmentSnapshot, error) {
	return operateInner(token, status, time.Time{}, func(guard platformContainmentGuard, cfg resolvedTerminateConfig) (*containmentOutcome, error) {
		return guard.terminate(cfg)
	})
}

func managedClose(token uint64) error {
	slotIndex, generation, slot, err := lookupSlot(token)
	if err != nil {
		return err
	}
	slot.mu.Lock()
	if slot.data.generation != generation || slot.data.retired {
		slot.mu.Unlock()
		return staleHandleErr()
	}
	waitForNotFinalizing(slot, time.Now().Add(120*time.Second))
	if slot.data.generation != generation {
		slot.mu.Unlock()
		return staleHandleErr()
	}
	switch slot.data.state {
	case slotEmpty:
		slot.mu.Unlock()
		return staleHandleErr()
	case slotFinalizing:
		slot.mu.Unlock()
		return syserror.InvalidArgument("containment handle is busy; retry close")
	case slotInert:
		recycleAfterDataUnlock(slot, slotIndex)
		return nil
	case slotActive:
		guard := slot.data.guard
		slot.data.guard = nil
		cfg := slot.data.terminateConfig
		slot.data.state = slotFinalizing
		slot.mu.Unlock()

		outcome, termErr := guard.terminate(cfg)

		slot.mu.Lock()
		slot.data.guard = guard
		if termErr == nil && outcome != nil && outcome.exited {
			recycleAfterDataUnlock(slot, slotIndex)
			return nil
		}
		slot.data.state = slotActive
		restoreDeadlineMonitor(&slot.data, token)
		slot.cond.Broadcast()
		slot.mu.Unlock()
		if termErr != nil {
			return termErr
		}
		return cleanupNotConfirmedErr()
	default:
		slot.mu.Unlock()
		return staleHandleErr()
	}
}

func operateInner(
	token uint64,
	completeStatus leaderStatus,
	waitDeadline time.Time,
	op func(platformContainmentGuard, resolvedTerminateConfig) (*containmentOutcome, error),
) (*ContainmentSnapshot, error) {
	_, generation, slot, err := lookupSlot(token)
	if err != nil {
		return nil, err
	}
	slot.mu.Lock()
	if slot.data.generation != generation || slot.data.retired {
		slot.mu.Unlock()
		return nil, staleHandleErr()
	}
	waitedForOwner := slot.data.state == slotFinalizing
	waitForNotFinalizing(slot, waitDeadline)
	if slot.data.generation != generation {
		slot.mu.Unlock()
		return nil, staleHandleErr()
	}
	if waitedForOwner && slot.data.state == slotActive && !waitDeadline.IsZero() && !time.Now().Before(waitDeadline) {
		snap, sErr := snapshotFrom(&slot.data, "active", leaderRunning)
		slot.mu.Unlock()
		return snap, sErr
	}
	switch slot.data.state {
	case slotEmpty:
		slot.mu.Unlock()
		return nil, staleHandleErr()
	case slotFinalizing:
		snap, sErr := snapshotFrom(&slot.data, "active", leaderRunning)
		slot.mu.Unlock()
		return snap, sErr
	case slotInert:
		snap, sErr := snapshotFrom(&slot.data, "inert", slot.data.leaderStatus)
		slot.mu.Unlock()
		return snap, sErr
	case slotActive:
		guard := slot.data.guard
		slot.data.guard = nil
		cfg := slot.data.terminateConfig
		pendingStatus := slot.data.leaderStatus
		slot.data.state = slotFinalizing
		slot.mu.Unlock()

		var outcome *containmentOutcome
		var opErr error
		if completeStatus == leaderTimedOut && pendingStatus != leaderTimedOut {
			outcome, opErr = guard.tryComplete(cfg)
			if opErr == nil && outcome != nil {
				completeStatus = leaderCompleted
			} else if opErr == nil {
				pendingStatus = leaderTimedOut
				outcome, opErr = op(guard, cfg)
			}
		} else {
			outcome, opErr = op(guard, cfg)
		}

		slot.mu.Lock()
		slot.data.guard = guard
		slot.data.leaderStatus = pendingStatus
		if opErr != nil {
			slot.data.state = slotActive
			slot.cond.Broadcast()
			slot.mu.Unlock()
			return nil, opErr
		}
		if outcome == nil {
			slot.data.state = slotActive
			slot.cond.Broadcast()
			snap, sErr := snapshotFrom(&slot.data, "active", leaderRunning)
			slot.mu.Unlock()
			return snap, sErr
		}
		if !outcome.exited {
			slot.data.state = slotActive
			slot.cond.Broadcast()
			slot.mu.Unlock()
			return nil, cleanupNotConfirmedErr()
		}
		if slot.data.leaderStatus == leaderTimedOut {
			completeStatus = leaderTimedOut
		}
		snap, sErr := commitInert(slot, outcome, completeStatus)
		slot.mu.Unlock()
		return snap, sErr
	default:
		slot.mu.Unlock()
		return nil, staleHandleErr()
	}
}

func unknownCompletion(observation string) ContainmentCompletion {
	return ContainmentCompletion{
		Status:      "unknown",
		Observation: observation,
	}
}

func completionFromPIDs(observation string, pids []uint32) ContainmentCompletion {
	if len(pids) >= MaxCompletionObservedPIDs {
		return unknownCompletion(observation)
	}
	for _, pid := range pids {
		if pid == 0 || pid > MaxSafePID {
			return unknownCompletion(observation)
		}
	}
	observedCount := len(pids)
	sorted := append([]uint32(nil), pids...)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i] < sorted[j] })
	for i := 1; i < len(sorted); i++ {
		if sorted[i] == sorted[i-1] {
			return unknownCompletion(observation)
		}
	}
	if len(sorted) != observedCount {
		return unknownCompletion(observation)
	}
	if len(sorted) == 0 {
		return ContainmentCompletion{
			Status:      "empty",
			Observation: observation,
		}
	}
	count := uint32(len(sorted)) // #nosec G115 -- len(sorted) < MaxCompletionObservedPIDs (4096)
	return ContainmentCompletion{
		Status:        "survivors",
		Observation:   observation,
		ObservedCount: &count,
		SurvivorPIDs:  sorted,
	}
}

func stabilizeCompletion(observation string, snapshot func() ([]uint32, bool)) ContainmentCompletion {
	var previous []uint32
	hasPrevious := false
	for attempt := 0; attempt <= MaxCompletionObservationRetries; attempt++ {
		current, ok := snapshot()
		if !ok {
			return unknownCompletion(observation)
		}
		sort.Slice(current, func(i, j int) bool { return current[i] < current[j] })
		if hasPrevious && slicesEqualUint32(previous, current) {
			return completionFromPIDs(observation, current)
		}
		previous = current
		hasPrevious = true
		if attempt < MaxCompletionObservationRetries {
			time.Sleep(pollInterval)
		}
	}
	return unknownCompletion(observation)
}

func slicesEqualUint32(a, b []uint32) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

// boundedCollector captures up to maxBytes from a pipe reader and discards the
// remainder so the child process never blocks on a full pipe buffer.
type boundedCollector struct {
	maxBytes  int
	buf       bytes.Buffer
	truncated bool
	done      chan struct{}
}

func startBoundedCollector(r io.ReadCloser, maxBytes int) *boundedCollector {
	c := &boundedCollector{
		maxBytes: maxBytes,
		done:     make(chan struct{}),
	}
	go func() {
		defer close(c.done)
		defer func() { _ = r.Close() }()
		chunk := make([]byte, 4096)
		for {
			n, err := r.Read(chunk)
			if n > 0 {
				rem := c.maxBytes - c.buf.Len()
				if rem > 0 {
					if n <= rem {
						_, _ = c.buf.Write(chunk[:n])
					} else {
						_, _ = c.buf.Write(chunk[:rem])
						c.truncated = true
					}
				} else {
					c.truncated = true
				}
			}
			if err != nil {
				return
			}
		}
	}()
	return c
}

func (c *boundedCollector) waitDrain(r io.Closer, timeout time.Duration) ([]byte, bool) {
	if c == nil {
		return nil, false
	}
	if timeout <= 0 {
		timeout = 200 * time.Millisecond
	}
	select {
	case <-c.done:
	case <-time.After(timeout):
		_ = r.Close()
		<-c.done
	}
	return c.buf.Bytes(), c.truncated
}

type boundedPipes struct {
	stdoutR *os.File
	stdoutW *os.File
	stderrR *os.File
	stderrW *os.File
	outCol  *boundedCollector
	errCol  *boundedCollector
}

func attachBoundedPipes(cmd *exec.Cmd, config TimeoutConfig) (*boundedPipes, error) {
	p := &boundedPipes{}
	if config.MaxStdoutBytes > 0 {
		r, w, err := os.Pipe()
		if err != nil {
			return nil, syserror.System("failed to create stdout pipe: "+err.Error(), errnoCode(err))
		}
		p.stdoutR = r
		p.stdoutW = w
		cmd.Stdout = w
	}
	if config.MaxStderrBytes > 0 {
		r, w, err := os.Pipe()
		if err != nil {
			if p.stdoutR != nil {
				_ = p.stdoutR.Close()
			}
			if p.stdoutW != nil {
				_ = p.stdoutW.Close()
			}
			return nil, syserror.System("failed to create stderr pipe: "+err.Error(), errnoCode(err))
		}
		p.stderrR = r
		p.stderrW = w
		cmd.Stderr = w
	}
	return p, nil
}

func (p *boundedPipes) closeAll() {
	if p == nil {
		return
	}
	if p.stdoutW != nil {
		_ = p.stdoutW.Close()
		p.stdoutW = nil
	}
	if p.stderrW != nil {
		_ = p.stderrW.Close()
		p.stderrW = nil
	}
	if p.stdoutR != nil {
		_ = p.stdoutR.Close()
		p.stdoutR = nil
	}
	if p.stderrR != nil {
		_ = p.stderrR.Close()
		p.stderrR = nil
	}
}

func (p *boundedPipes) startCollectors(config TimeoutConfig) {
	if p == nil {
		return
	}
	if p.stdoutW != nil {
		_ = p.stdoutW.Close()
		p.stdoutW = nil
	}
	if p.stderrW != nil {
		_ = p.stderrW.Close()
		p.stderrW = nil
	}
	if p.stdoutR != nil && config.MaxStdoutBytes > 0 {
		p.outCol = startBoundedCollector(p.stdoutR, config.MaxStdoutBytes)
	}
	if p.stderrR != nil && config.MaxStderrBytes > 0 {
		p.errCol = startBoundedCollector(p.stderrR, config.MaxStderrBytes)
	}
}

func (p *boundedPipes) abortCollectors() {
	if p == nil {
		return
	}
	if p.stdoutR != nil {
		_ = p.stdoutR.Close()
	}
	if p.stderrR != nil {
		_ = p.stderrR.Close()
	}
}

func (p *boundedPipes) drainInto(res *TimeoutResult, drainTimeout time.Duration) {
	if p == nil || res == nil {
		return
	}
	res.Stdout, res.StdoutTruncated = p.outCol.waitDrain(p.stdoutR, drainTimeout)
	res.Stderr, res.StderrTruncated = p.errCol.waitDrain(p.stderrR, drainTimeout)
}
