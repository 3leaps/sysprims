// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build windows

package timeout

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"time"
	"unsafe"

	"github.com/3leaps/sysprims/signal"
	"github.com/3leaps/sysprims/syserror"
	"golang.org/x/sys/windows"
)

func errnoCode(err error) int32 {
	var errno windows.Errno
	if errors.As(err, &errno) {
		return int32(errno) // #nosec G115 -- Windows error code fits in int32
	}
	return 0
}

func mapSpawnError(command string, err error) error {
	if errors.Is(err, exec.ErrNotFound) || errors.Is(err, os.ErrNotExist) {
		return syserror.NotFoundCommand(command)
	}
	if errors.Is(err, os.ErrPermission) {
		return syserror.PermissionDeniedCommand(command)
	}
	return syserror.SpawnFailed(err.Error())
}

func managedSpawn(req SpawnContainedConfig) (uint64, error) {
	if _, err := validateSpawnRequest(req); err != nil {
		return 0, err
	}
	return 0, syserror.NotSupported("managed containment spawn without create-suspended Job assignment", "windows")
}

func createKillOnCloseJob() (windows.Handle, error) {
	job, err := windows.CreateJobObject(nil, nil)
	if err != nil {
		return 0, syserror.GroupCreationFailed("CreateJobObjectW failed: " + err.Error())
	}
	var info windows.JOBOBJECT_EXTENDED_LIMIT_INFORMATION
	info.BasicLimitInformation.LimitFlags = windows.JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
	_, err = windows.SetInformationJobObject(
		job,
		windows.JobObjectExtendedLimitInformation,
		uintptr(unsafe.Pointer(&info)), // #nosec G103 -- required by SetInformationJobObject syscall signature
		uint32(unsafe.Sizeof(info)),
	)
	if err != nil {
		_ = windows.CloseHandle(job)
		return 0, syserror.GroupCreationFailed("SetInformationJobObject failed: " + err.Error())
	}
	return job, nil
}

func assignProcessToJob(job windows.Handle, pid uint32) error {
	const access = windows.PROCESS_SET_QUOTA | windows.PROCESS_TERMINATE
	proc, err := windows.OpenProcess(access, false, pid)
	if err != nil {
		return err
	}
	defer func() { _ = windows.CloseHandle(proc) }()
	return windows.AssignProcessToJobObject(job, proc)
}

func runWithTimeoutImpl(ctx context.Context, command string, args []string, timeout time.Duration, config TimeoutConfig) (*TimeoutResult, error) {
	useJobObject := config.Grouping == GroupByDefault
	reliability := ReliabilityUnproven
	var jobHandle windows.Handle
	hasJob := false

	if useJobObject {
		if job, err := createKillOnCloseJob(); err == nil {
			jobHandle = job
			hasJob = true
		} else {
			reliability = ReliabilityBestEffort
		}
	} else {
		reliability = ReliabilityBestEffort
	}
	defer func() {
		if hasJob {
			_ = windows.CloseHandle(jobHandle)
		}
	}()

	cmd := exec.Command(command, args...) // #nosec G204 -- timeout runner intentionally executes caller-supplied command and args without a shell
	if config.Cwd != "" {
		cmd.Dir = config.Cwd
	}
	if len(config.Env) > 0 {
		cmd.Env = mergeEnv(parentEnv(), config.Env)
	}

	pipes, err := attachBoundedPipes(cmd, config)
	if err != nil {
		return nil, err
	}

	if err := cmd.Start(); err != nil {
		pipes.closeAll()
		return nil, mapSpawnError(command, err)
	}
	pipes.startCollectors(config)

	rawPID := cmd.Process.Pid
	if rawPID <= 0 || int64(rawPID) > int64(MaxSafePID) {
		_ = cmd.Process.Kill()
		_, _ = cmd.Process.Wait()
		pipes.abortCollectors()
		return nil, syserror.SpawnFailed("child_pid is outside [1, MaxSafePID]")
	}
	childPID := uint32(rawPID) // #nosec G115 -- validated 1 <= rawPID <= MaxSafePID

	if hasJob {
		if err := assignProcessToJob(jobHandle, childPID); err != nil {
			reliability = ReliabilityBestEffort
			_ = windows.CloseHandle(jobHandle)
			hasJob = false
		}
	}

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

	if hasJob {
		_ = windows.TerminateJobObject(jobHandle, 1)
		_ = windows.CloseHandle(jobHandle)
		hasJob = false
	} else {
		_ = cmd.Process.Kill()
	}
	<-waitDone

	sigSent := config.Signal
	escalated := false
	res := &TimeoutResult{
		SchemaID:            TimeoutResultSchemaID,
		Status:              "timed_out",
		SignalSent:          &sigSent,
		Escalated:           &escalated,
		TreeKillReliability: &reliability,
	}
	pipes.drainInto(res, config.PipeDrainTimeout)
	return res, nil
}

func isProcessLive(pid uint32) (bool, error) {
	if pid == 0 || pid > MaxSafePID {
		return false, syserror.InvalidArgument("pid must be in 1..=2147483647")
	}
	const access = windows.PROCESS_QUERY_LIMITED_INFORMATION | windows.SYNCHRONIZE
	h, err := windows.OpenProcess(access, false, pid)
	if err != nil {
		if errors.Is(err, windows.ERROR_INVALID_PARAMETER) {
			return false, nil
		}
		if errors.Is(err, windows.ERROR_ACCESS_DENIED) {
			return false, syserror.PermissionDenied(pid, "liveness probe")
		}
		return false, syserror.System("OpenProcess failed: "+err.Error(), errnoCode(err))
	}
	defer func() { _ = windows.CloseHandle(h) }()

	ev, wErr := windows.WaitForSingleObject(h, 0)
	if wErr != nil {
		return false, syserror.System("WaitForSingleObject failed: "+wErr.Error(), errnoCode(wErr))
	}
	switch ev {
	case windows.WAIT_OBJECT_0:
		return false, nil
	case uint32(windows.WAIT_TIMEOUT):
		return true, nil
	default:
		return false, syserror.System("WaitForSingleObject failed", 0)
	}
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
		"PID-only termination has no Job handle; terminating the target process only",
	}

	if err := signal.Kill(pid, int(cfg.signal)); err != nil {
		return nil, err
	}

	waitTimeout := time.Duration(cfg.graceTimeoutMS+cfg.killTimeoutMS) * time.Millisecond // #nosec G115 -- graceTimeoutMS and killTimeoutMS bounded by MaxDurationMS
	exited, err := waitForPIDExit(pid, waitTimeout)
	if err != nil {
		return nil, err
	}
	if !exited {
		warnings = append(warnings, "Timed out waiting for process to exit after TerminateProcess")
	}

	return &TerminateTreeResult{
		SchemaID:            TerminateTreeResultSchemaID,
		Timestamp:           nowRFC3339(),
		Platform:            currentPlatform(),
		PID:                 pid,
		SignalSent:          cfg.signal,
		Escalated:           false,
		Exited:              exited,
		TimedOut:            !exited,
		TreeKillReliability: ReliabilityBestEffort,
		Warnings:            warnings,
	}, nil
}
