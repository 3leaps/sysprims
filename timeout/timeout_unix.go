// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build unix

package timeout

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"sync"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/3leaps/sysprims/session"
	"github.com/3leaps/sysprims/signal"
	"github.com/3leaps/sysprims/syserror"
	"golang.org/x/sys/unix"
)

func errnoCode(err error) int32 {
	var errno syscall.Errno
	if errors.As(err, &errno) {
		return int32(errno) // #nosec G115 -- POSIX errno fits in int32
	}
	return 0
}

func mapSpawnError(command string, err error) error {
	if errors.Is(err, exec.ErrNotFound) || errors.Is(err, os.ErrNotExist) || errors.Is(err, unix.ENOENT) {
		return syserror.NotFoundCommand(command)
	}
	if errors.Is(err, os.ErrPermission) || errors.Is(err, unix.EACCES) || errors.Is(err, unix.EPERM) {
		return syserror.PermissionDeniedCommand(command)
	}
	return syserror.SpawnFailed(err.Error())
}

type unixContainmentGuard struct {
	cmd         *exec.Cmd
	id          ContainmentIdentity
	reliability string
	boundary    string
	finalized   bool
	pgid        uint32
	sessionID   uint32
	receipt     *session.UnixSessionReceipt
	reapOnce    sync.Once
	reaped      bool
	waitErr     error
	exitState   *os.ProcessState
}

func (g *unixContainmentGuard) identity() ContainmentIdentity {
	return g.id
}

func (g *unixContainmentGuard) treeKillReliability() string {
	return g.reliability
}

func (g *unixContainmentGuard) boundaryStrength() string {
	return g.boundary
}

func cleanupFailedSpawnChild(cmd *exec.Cmd) {
	if cmd == nil || cmd.Process == nil {
		return
	}
	if pid := cmd.Process.Pid; pid > 0 && int64(pid) <= int64(MaxSafePID) {
		_ = signal.KillGroup(uint32(pid), signal.SIGKILL) // #nosec G115 -- validated 1 <= pid <= MaxSafePID
	}
	_ = cmd.Process.Kill()
	_, _ = cmd.Process.Wait()
}

func newUnixContainmentGuard(cmd *exec.Cmd, receipt *session.UnixSessionReceipt) (*unixContainmentGuard, error) {
	if cmd == nil || cmd.Process == nil {
		return nil, syserror.InvalidArgument("child process id is unavailable")
	}
	rawPID := cmd.Process.Pid
	if rawPID <= 0 || int64(rawPID) > int64(MaxSafePID) {
		return nil, syserror.InvalidArgument("child process id is unavailable")
	}
	pid := uint32(rawPID) // #nosec G115 -- validated 1 <= rawPID <= MaxSafePID

	if receipt == nil || !receipt.IsSealed() ||
		receipt.ChildPID() != pid ||
		receipt.ProcessGroupID() != pid ||
		receipt.SessionID() != pid ||
		receipt.SessionKind() != session.SessionKindNewSession ||
		receipt.IdentifierProvenance() != session.ProvenanceSetsidStructuralChildPID {
		return nil, syserror.InvalidArgument("session acquisition receipt does not match the owned child")
	}

	childWasExited, err := verifyOwnedChildSlot(pid)
	if err != nil {
		return nil, err
	}
	id, idErr := captureContainmentIdentity(pid, true)
	if idErr != nil {
		exitedUnreaped, uErr := ownedChildIsExitedUnreaped(pid)
		if uErr != nil {
			return nil, uErr
		}
		if childWasExited || exitedUnreaped {
			id = ContainmentIdentity{
				PID:             pid,
				StartTimeUnixMS: 0,
				ExePath:         "",
			}
		} else {
			return nil, idErr
		}
	}

	pgid, pgidErr := unix.Getpgid(int(pid))
	sid, sidErr := unix.Getsid(int(pid))
	if pgidErr != nil || sidErr != nil || pgid != int(pid) || sid != int(pid) {
		exitedUnreaped, uErr := ownedChildIsExitedUnreaped(pid)
		if uErr != nil {
			return nil, uErr
		}
		if !childWasExited && !exitedUnreaped {
			return nil, syserror.InvalidArgument("spawn-time session acquisition no longer matches the owned child")
		}
	} else {
		if callerPGID, cErr := unix.Getpgid(0); cErr == nil && callerPGID == pgid {
			return nil, syserror.InvalidArgument("acquired child shares the caller's process group")
		}
	}

	return &unixContainmentGuard{
		cmd:         cmd,
		id:          id,
		reliability: ReliabilityGuaranteed,
		boundary:    BoundaryCooperativeGroup,
		pgid:        receipt.ProcessGroupID(),
		sessionID:   receipt.SessionID(),
		receipt:     receipt,
	}, nil
}

func ownedChildIsExitedUnreaped(pid uint32) (bool, error) {
	for attempt := 0; attempt <= 5; attempt++ {
		exited, err := verifyOwnedChildSlot(pid)
		if err != nil {
			return false, err
		}
		if exited {
			return true, nil
		}
		if attempt < 5 {
			time.Sleep(pollInterval)
		}
	}
	return false, nil
}

func (g *unixContainmentGuard) validateBeforeSignal() (bool, error) {
	if g.cmd == nil || g.cmd.Process == nil || uint32(g.cmd.Process.Pid) != g.id.PID { // #nosec G115 -- PID range verified at guard creation
		return false, syserror.InvalidArgument("owned child identity changed; refusing containment operation")
	}
	if g.receipt != nil {
		if !g.receipt.IsSealed() ||
			g.receipt.ChildPID() != g.id.PID ||
			g.receipt.ProcessGroupID() != g.pgid ||
			g.receipt.SessionID() != g.sessionID ||
			g.receipt.SessionKind() != session.SessionKindNewSession ||
			g.receipt.IdentifierProvenance() != session.ProvenanceSetsidStructuralChildPID {
			return false, syserror.InvalidArgument("sealed session receipt no longer matches the containment guard")
		}
	}

	var ownedChildExited bool
	if g.receipt != nil {
		exited, err := verifyOwnedChildSlot(g.id.PID)
		if err != nil {
			return false, err
		}
		ownedChildExited = exited
	}

	receiptOnlyFastExit := g.receipt != nil && g.id.StartTimeUnixMS == 0 && g.id.ExePath == ""
	var identityMetadataVerified bool
	if !receiptOnlyFastExit {
		verified, err := verifyContainmentIdentity(g.id, g.receipt != nil)
		if err != nil {
			return false, err
		}
		identityMetadataVerified = verified
	}

	exitedOwnershipVerified := ownedChildExited
	if !exitedOwnershipVerified && g.receipt != nil && !identityMetadataVerified {
		exited, err := ownedChildIsExitedUnreaped(g.id.PID)
		if err != nil {
			return false, err
		}
		exitedOwnershipVerified = exited
	}
	if g.receipt != nil && !identityMetadataVerified && !exitedOwnershipVerified {
		return false, syserror.InvalidArgument("guaranteed child identity and unreaped ownership are unavailable; refusing group signal")
	}

	if identityMetadataVerified {
		curPGID, pgidErr := unix.Getpgid(int(g.id.PID))
		curSID, sidErr := unix.Getsid(int(g.id.PID))
		if pgidErr == nil && sidErr == nil && curPGID == int(g.pgid) && curSID == int(g.sessionID) {
			identityMetadataVerified = true
		} else if g.receipt != nil {
			exited, err := ownedChildIsExitedUnreaped(g.id.PID)
			if err != nil {
				return false, err
			}
			if !exited {
				return false, syserror.InvalidArgument("process group identity changed; refusing containment operation")
			}
			identityMetadataVerified = false
		} else {
			return false, syserror.InvalidArgument("process group identity changed; refusing containment operation")
		}
	}

	if callerPGID, err := unix.Getpgid(0); err == nil && callerPGID == int(g.pgid) {
		return false, syserror.InvalidArgument("contained group matches the caller's group; refusing group signal")
	}

	return identityMetadataVerified, nil
}

func verifyContainmentIdentity(expected ContainmentIdentity, receiptBound bool) (bool, error) {
	actualStartMS, actualExe, present, err := observeProcessIdentity(expected.PID)
	if err != nil {
		if syserror.IsCode(err, syserror.ErrNotFound) || syserror.IsCode(err, syserror.ErrPermissionDenied) {
			return false, nil
		}
		return false, err
	}
	if !present {
		return false, nil
	}
	if actualStartMS != expected.StartTimeUnixMS {
		return false, syserror.InvalidArgument("process identity changed; refusing containment operation")
	}
	if !receiptBound {
		if actualExe != "" && actualExe != expected.ExePath {
			return false, syserror.InvalidArgument("process identity changed; refusing containment operation")
		}
		return actualExe != "", nil
	}
	return true, nil
}

func reconcileGroupSignalFailure(err error, observe func() ContainmentCompletion, emptyWarning string) (string, error) {
	reconcilable := syserror.IsCode(err, syserror.ErrNotFound) ||
		(currentPlatform() == "macos" && syserror.IsCode(err, syserror.ErrPermissionDenied))
	if !reconcilable {
		return "", err
	}
	comp := observe()
	if comp.Status != "empty" {
		return "", err
	}
	return emptyWarning, nil
}

func (g *unixContainmentGuard) tryComplete(cfg resolvedTerminateConfig) (*containmentOutcome, error) {
	if g.finalized {
		return nil, syserror.InvalidArgument("containment guard has already been finalized")
	}
	if g.cmd == nil || g.cmd.Process == nil || uint32(g.cmd.Process.Pid) != g.id.PID { // #nosec G115 -- PID range verified at guard creation
		return nil, syserror.InvalidArgument("owned child identity changed; refusing containment operation")
	}
	if g.receipt != nil {
		if g.receipt.ChildPID() != g.id.PID ||
			g.receipt.ProcessGroupID() != g.pgid ||
			g.receipt.SessionID() != g.sessionID {
			return nil, syserror.InvalidArgument("sealed session receipt no longer matches the containment guard")
		}
	}
	live, err := isProcessLive(g.id.PID)
	if err != nil {
		return nil, err
	}
	if live {
		return nil, nil
	}
	return g.terminate(cfg)
}

func (g *unixContainmentGuard) terminate(cfg resolvedTerminateConfig) (*containmentOutcome, error) {
	if g.finalized {
		return nil, syserror.InvalidArgument("containment guard has already been finalized")
	}
	if err := validatePlatformSignal(cfg.signal, "signal"); err != nil {
		return nil, err
	}
	if err := validatePlatformSignal(cfg.killSignal, "kill_signal"); err != nil {
		return nil, err
	}

	identityMetadataVerified, err := g.validateBeforeSignal()
	if err != nil {
		return nil, err
	}

	warnings := make([]string, 0)
	if !identityMetadataVerified {
		if g.receipt != nil {
			warnings = append(warnings, "Leader exited unreaped; same-spawn receipt and waitid ownership preserve group identity")
		} else {
			warnings = append(warnings, "Live identity metadata unavailable; proceeding with owned child and bound group")
		}
	}

	gracefulSent := false
	if kErr := signal.KillGroup(g.pgid, int(cfg.signal)); kErr == nil {
		gracefulSent = true
	} else {
		warn, rErr := reconcileGroupSignalFailure(
			kErr,
			func() ContainmentCompletion { return observeContainmentCompletion(g.pgid, g.sessionID) },
			"Process group became empty before cleanup signal; proceeding to reap",
		)
		if rErr != nil {
			return nil, rErr
		}
		warnings = append(warnings, warn)
	}

	if gracefulSent && cfg.graceTimeoutMS > 0 {
		time.Sleep(time.Duration(cfg.graceTimeoutMS) * time.Millisecond) // #nosec G115 -- graceTimeoutMS bounded by MaxDurationMS
	}

	escalated := false
	if gracefulSent {
		if _, vErr := g.validateBeforeSignal(); vErr != nil {
			return nil, vErr
		}
		if kErr := signal.KillGroup(g.pgid, int(cfg.killSignal)); kErr == nil {
			escalated = true
		} else {
			warn, rErr := reconcileGroupSignalFailure(
				kErr,
				func() ContainmentCompletion { return observeContainmentCompletion(g.pgid, g.sessionID) },
				"Process group became empty before escalation; proceeding to reap",
			)
			if rErr != nil {
				return nil, rErr
			}
			warnings = append(warnings, warn)
		}
	}

	_ = waitForContainedChildExit(g.id.PID, time.Duration(cfg.killTimeoutMS)*time.Millisecond) // #nosec G115 -- killTimeoutMS bounded by MaxDurationMS
	completion := observeContainmentCompletion(g.pgid, g.sessionID)
	exited, rErr := g.reapContainedChild()
	if rErr != nil {
		return nil, rErr
	}
	if !exited {
		warnings = append(warnings, "Timed out waiting to reap contained child")
	} else {
		g.finalized = true
	}

	pgid := g.pgid
	var sigSent *int32
	if gracefulSent {
		s := cfg.signal
		sigSent = &s
	}
	var killSig *int32
	if escalated {
		k := cfg.killSignal
		killSig = &k
	}

	return &containmentOutcome{
		identity:            g.id,
		pgid:                &pgid,
		signalSent:          sigSent,
		killSignal:          killSig,
		escalated:           escalated,
		exited:              exited,
		timedOut:            !exited,
		treeKillReliability: g.reliability,
		boundaryStrength:    g.boundary,
		completion:          completion,
		warnings:            warnings,
	}, nil
}

func (g *unixContainmentGuard) reapContainedChild() (bool, error) {
	if g.reaped {
		return true, nil
	}
	live, err := isProcessLive(g.id.PID)
	if err != nil && !syserror.IsCode(err, syserror.ErrNotFound) {
		return false, err
	}
	if live {
		return false, nil
	}
	g.reapOnce.Do(func() {
		g.waitErr = g.cmd.Wait()
		g.exitState = g.cmd.ProcessState
		g.reaped = true
	})
	if g.waitErr != nil {
		var exitErr *exec.ExitError
		if !errors.As(g.waitErr, &exitErr) {
			return false, syserror.System("failed to reap contained child: "+g.waitErr.Error(), errnoCode(g.waitErr))
		}
		g.exitState = exitErr.ProcessState
	}
	return g.reaped, nil
}

func (g *unixContainmentGuard) dropGuard() {
	if g.finalized || g.cmd == nil || g.cmd.Process == nil {
		return
	}
	if _, err := g.validateBeforeSignal(); err != nil {
		return
	}
	_ = signal.KillGroup(g.pgid, signal.SIGKILL)
	if waitForContainedChildExit(g.id.PID, 2*time.Second) {
		if exited, err := g.reapContainedChild(); err == nil && exited {
			g.finalized = true
		}
	}
}

func waitForContainedChildExit(pid uint32, timeout time.Duration) bool {
	deadline := time.Now().Add(timeout)
	for {
		live, err := isProcessLive(pid)
		if (err == nil && !live) || syserror.IsCode(err, syserror.ErrNotFound) {
			return true
		}
		if time.Now().Before(deadline) {
			time.Sleep(pollInterval)
			continue
		}
		return false
	}
}

func managedSpawn(req SpawnContainedConfig) (uint64, error) {
	terminateCfg, err := validateSpawnRequest(req)
	if err != nil {
		return 0, err
	}

	slotIndex, generation, err := allocateSlot()
	if err != nil {
		return 0, err
	}
	token := packToken(slotIndex, generation)

	cmd := exec.Command(req.Argv[0], req.Argv[1:]...) // #nosec G204 -- contained spawn primitive intentionally executes caller-supplied argv without a shell
	if req.Cwd != nil && *req.Cwd != "" {
		cmd.Dir = *req.Cwd
	}
	if len(req.Env) > 0 {
		cmd.Env = mergeEnv(parentEnv(), req.Env)
	}

	receipt, err := session.SpawnWithSessionReceipt(cmd)
	if err != nil {
		recycleUnusedSlot(slotIndex, generation)
		return 0, err
	}

	guard, err := newUnixContainmentGuard(cmd, receipt)
	if err != nil {
		cleanupFailedSpawnChild(cmd)
		recycleUnusedSlot(slotIndex, generation)
		return 0, err
	}

	var deadlineAt time.Time
	cancel := &atomic.Bool{}
	if req.ExecutionTimeoutMS != nil {
		deadlineAt = time.Now().Add(time.Duration(*req.ExecutionTimeoutMS) * time.Millisecond) // #nosec G115 -- ExecutionTimeoutMS bounded by MaxDurationMS
	}

	slot := &getRegistry().slots[slotIndex]
	slot.mu.Lock()
	if slot.data.generation != generation || slot.data.state != slotEmpty {
		slot.mu.Unlock()
		guard.dropGuard()
		return 0, syserror.Internal("containment slot changed before spawn insert")
	}
	idCopy := guard.identity()
	slot.data.identity = &idCopy
	slot.data.reliability = guard.treeKillReliability()
	slot.data.boundary = guard.boundaryStrength()
	slot.data.terminateConfig = terminateCfg
	slot.data.leaderStatus = leaderRunning
	slot.data.outcome = nil
	slot.data.deadlineCancel = cancel
	slot.data.deadlineAt = deadlineAt
	slot.data.guard = guard
	slot.data.state = slotActive
	slot.mu.Unlock()

	if !deadlineAt.IsZero() {
		startDeadlineMonitor(token, deadlineAt, cancel)
	}

	return token, nil
}

func runWithTimeoutImpl(ctx context.Context, command string, args []string, timeout time.Duration, config TimeoutConfig) (*TimeoutResult, error) {
	cmd := exec.Command(command, args...) // #nosec G204 -- timeout runner intentionally executes caller-supplied command and args without a shell
	if config.Cwd != "" {
		cmd.Dir = config.Cwd
	}
	if len(config.Env) > 0 {
		cmd.Env = mergeEnv(parentEnv(), config.Env)
	}

	if config.Grouping == GroupByDefault {
		return runContainedWithTimeout(ctx, cmd, command, timeout, config)
	}
	return runForegroundWithTimeout(ctx, cmd, command, timeout, config)
}

func runContainedWithTimeout(ctx context.Context, cmd *exec.Cmd, program string, timeout time.Duration, config TimeoutConfig) (*TimeoutResult, error) {
	pipes, err := attachBoundedPipes(cmd, config)
	if err != nil {
		return nil, err
	}

	receipt, err := session.SpawnWithSessionReceipt(cmd)
	if err != nil {
		pipes.closeAll()
		return nil, err
	}
	pipes.startCollectors(config)

	guard, err := newUnixContainmentGuard(cmd, receipt)
	if err != nil {
		cleanupFailedSpawnChild(cmd)
		pipes.abortCollectors()
		return nil, err
	}
	defer guard.dropGuard()

	graceMS := uint64(0)
	if config.KillAfter > 0 {
		graceMS = uint64(config.KillAfter.Milliseconds()) // #nosec G115 -- KillAfter validated > 0 and bounded by MaxDurationMS
	}
	termination := resolvedTerminateConfig{
		graceTimeoutMS: graceMS,
		killTimeoutMS:  2_000,
		signal:         int32(config.Signal), // #nosec G115 -- validated in RunWithTimeoutContext
		killSignal:     int32(signal.SIGKILL),
	}

	start := time.Now()
	for {
		outcome, cErr := guard.tryComplete(termination)
		if cErr != nil {
			pipes.abortCollectors()
			return nil, cErr
		}
		if outcome != nil {
			code := 0
			if guard.exitState != nil {
				ws, ok := guard.exitState.Sys().(syscall.WaitStatus)
				if ok && ws.Signaled() {
					code = 128 + int(ws.Signal())
				} else {
					code = guard.exitState.ExitCode()
				}
			}
			res := &TimeoutResult{
				SchemaID: TimeoutResultSchemaID,
				Status:   "completed",
				ExitCode: &code,
			}
			pipes.drainInto(res, config.PipeDrainTimeout)
			return res, nil
		}

		expired := time.Since(start) >= timeout
		if !expired {
			select {
			case <-ctx.Done():
				expired = true
			default:
			}
		}
		if expired {
			outcome, tErr := guard.terminate(termination)
			if tErr != nil {
				pipes.abortCollectors()
				return nil, tErr
			}
			escalated := outcome.escalated || outcome.signalSent != nil
			sigSent := config.Signal
			rel := outcome.treeKillReliability
			res := &TimeoutResult{
				SchemaID:            TimeoutResultSchemaID,
				Status:              "timed_out",
				SignalSent:          &sigSent,
				Escalated:           &escalated,
				TreeKillReliability: &rel,
			}
			pipes.drainInto(res, config.PipeDrainTimeout)
			return res, nil
		}

		time.Sleep(pollInterval)
	}
}

func runForegroundWithTimeout(ctx context.Context, cmd *exec.Cmd, program string, timeout time.Duration, config TimeoutConfig) (*TimeoutResult, error) {
	pipes, err := attachBoundedPipes(cmd, config)
	if err != nil {
		return nil, err
	}
	if err := cmd.Start(); err != nil {
		pipes.closeAll()
		return nil, mapSpawnError(program, err)
	}
	pipes.startCollectors(config)

	rawPID := cmd.Process.Pid
	if rawPID <= 0 || int64(rawPID) > int64(MaxSafePID) {
		_ = cmd.Process.Kill()
		_, _ = cmd.Process.Wait()
		pipes.abortCollectors()
		return nil, syserror.SpawnFailed("foreground child_pid is outside [1, MaxSafePID]")
	}
	childPID := uint32(rawPID) // #nosec G115 -- validated 1 <= rawPID <= MaxSafePID

	waitDone := make(chan struct{})
	var waitErr error
	go func() {
		waitErr = cmd.Wait()
		close(waitDone)
	}()

	timer := time.NewTimer(timeout)
	defer timer.Stop()

	select {
	case <-waitDone:
		if waitErr != nil {
			var exitErr *exec.ExitError
			if !errors.As(waitErr, &exitErr) {
				pipes.abortCollectors()
				return nil, syserror.System("wait failed: "+waitErr.Error(), errnoCode(waitErr))
			}
		}
		code := 0
		if cmd.ProcessState != nil {
			code = cmd.ProcessState.ExitCode()
		}
		res := &TimeoutResult{
			SchemaID: TimeoutResultSchemaID,
			Status:   "completed",
			ExitCode: &code,
		}
		pipes.drainInto(res, config.PipeDrainTimeout)
		return res, nil

	case <-timer.C:
	case <-ctx.Done():
	}

	_ = signal.Kill(childPID, config.Signal)
	escalated := false
	if config.KillAfter > 0 {
		escTimer := time.NewTimer(config.KillAfter)
		select {
		case <-waitDone:
			escTimer.Stop()
		case <-escTimer.C:
			escalated = true
			_ = signal.ForceKill(childPID)
			<-waitDone
		}
	} else {
		escalated = true
		_ = signal.ForceKill(childPID)
		<-waitDone
	}

	sigSent := config.Signal
	rel := ReliabilityBestEffort
	res := &TimeoutResult{
		SchemaID:            TimeoutResultSchemaID,
		Status:              "timed_out",
		SignalSent:          &sigSent,
		Escalated:           &escalated,
		TreeKillReliability: &rel,
	}
	pipes.drainInto(res, config.PipeDrainTimeout)
	return res, nil
}

func waitForPIDExit(pid uint32, timeout time.Duration) (bool, error) {
	deadline := time.Now().Add(timeout)
	for {
		live, err := isProcessLive(pid)
		if err != nil {
			if syserror.IsCode(err, syserror.ErrNotFound) {
				return true, nil
			}
			return false, err
		}
		if !live {
			return true, nil
		}
		if !time.Now().Before(deadline) {
			return false, nil
		}
		rem := time.Until(deadline)
		if rem > pollInterval {
			rem = pollInterval
		}
		time.Sleep(rem)
	}
}

func terminateTreeImpl(pid uint32, cfg resolvedTerminateConfig) (*TerminateTreeResult, error) {
	warnings := []string{
		"PID-only termination has no sealed group capability; signaling the target PID only",
	}

	if err := signal.Kill(pid, int(cfg.signal)); err != nil {
		return nil, err
	}

	exited, err := waitForPIDExit(pid, time.Duration(cfg.graceTimeoutMS)*time.Millisecond) // #nosec G115 -- graceTimeoutMS bounded by MaxDurationMS
	if err != nil {
		return nil, err
	}
	if exited {
		return &TerminateTreeResult{
			SchemaID:            TerminateTreeResultSchemaID,
			Timestamp:           nowRFC3339(),
			Platform:            currentPlatform(),
			PID:                 pid,
			SignalSent:          cfg.signal,
			Escalated:           false,
			Exited:              true,
			TimedOut:            false,
			TreeKillReliability: ReliabilityBestEffort,
			Warnings:            warnings,
		}, nil
	}

	if err := signal.Kill(pid, int(cfg.killSignal)); err != nil {
		if !syserror.IsCode(err, syserror.ErrNotFound) {
			return nil, err
		}
	}

	exited, err = waitForPIDExit(pid, time.Duration(cfg.killTimeoutMS)*time.Millisecond) // #nosec G115 -- killTimeoutMS bounded by MaxDurationMS
	if err != nil {
		return nil, err
	}
	timedOut := !exited
	if timedOut {
		live, lErr := isProcessLive(pid)
		switch {
		case lErr == nil && !live, syserror.IsCode(lErr, syserror.ErrNotFound):
			exited = true
			timedOut = false
			warnings = append(warnings, "PID no longer found after timeout; treating as exited")
		case syserror.IsCode(lErr, syserror.ErrPermissionDenied):
			warnings = append(warnings, "Permission denied while confirming exit after timeout")
		case lErr != nil:
			warnings = append(warnings, "Failed to confirm exit after timeout: "+lErr.Error())
		}
	}

	killSig := cfg.killSignal
	return &TerminateTreeResult{
		SchemaID:            TerminateTreeResultSchemaID,
		Timestamp:           nowRFC3339(),
		Platform:            currentPlatform(),
		PID:                 pid,
		SignalSent:          cfg.signal,
		KillSignal:          &killSig,
		Escalated:           true,
		Exited:              exited,
		TimedOut:            timedOut,
		TreeKillReliability: ReliabilityBestEffort,
		Warnings:            warnings,
	}, nil
}
