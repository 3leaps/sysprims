// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build unix

package session

import (
	"errors"
	"os"
	"os/exec"
	ossignal "os/signal"
	"path/filepath"
	"sync"
	"syscall"

	"github.com/3leaps/sysprims/syserror"
	"golang.org/x/sys/unix"
)

var nohupSpawnMu sync.Mutex

func errnoCode(err error) int32 {
	var errno syscall.Errno
	if errors.As(err, &errno) {
		return int32(errno) // #nosec G115 -- POSIX errno fits in int32
	}
	return 0
}

func setsidImpl() (uint32, error) {
	sid, err := unix.Setsid()
	if err != nil {
		return 0, syserror.System("setsid failed", errnoCode(err))
	}
	if sid <= 0 || int64(sid) > int64(MaxSafePID) {
		return 0, syserror.System("setsid returned invalid session id", 0)
	}
	return uint32(sid), nil // #nosec G115 -- validated 1 <= sid <= MaxSafePID
}

func getsidImpl(pid uint32) (uint32, error) {
	sid, err := unix.Getsid(int(pid))
	if err != nil {
		if errors.Is(err, unix.ESRCH) {
			return 0, syserror.NotFound(pid)
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return 0, syserror.PermissionDenied(pid, "getsid")
		}
		return 0, syserror.System("getsid failed", errnoCode(err))
	}
	if sid <= 0 || int64(sid) > int64(MaxSafePID) {
		return 0, syserror.System("getsid returned invalid session id", 0)
	}
	return uint32(sid), nil // #nosec G115 -- validated 1 <= sid <= MaxSafePID
}

func getpgidImpl(pid uint32) (uint32, error) {
	pgid, err := unix.Getpgid(int(pid))
	if err != nil {
		if errors.Is(err, unix.ESRCH) {
			return 0, syserror.NotFound(pid)
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return 0, syserror.PermissionDenied(pid, "getpgid")
		}
		return 0, syserror.System("getpgid failed", errnoCode(err))
	}
	if pgid <= 0 || int64(pgid) > int64(MaxSafePID) {
		return 0, syserror.System("getpgid returned invalid process group id", 0)
	}
	return uint32(pgid), nil // #nosec G115 -- validated 1 <= pgid <= MaxSafePID
}

func setpgidImpl(pid, pgid uint32) error {
	if err := unix.Setpgid(int(pid), int(pgid)); err != nil {
		if errors.Is(err, unix.ESRCH) {
			return syserror.NotFound(pid)
		}
		if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
			return syserror.PermissionDenied(pid, "setpgid")
		}
		return syserror.System("setpgid failed", errnoCode(err))
	}
	return nil
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

func extractExitCodeAndSignal(waitErr error, state *os.ProcessState) (*int32, *int32, error) {
	if waitErr != nil {
		var exitErr *exec.ExitError
		if !errors.As(waitErr, &exitErr) {
			return nil, nil, syserror.System("wait failed: "+waitErr.Error(), errnoCode(waitErr))
		}
		state = exitErr.ProcessState
	}
	if state == nil {
		return nil, nil, syserror.System("wait completed without process state", 0)
	}
	ws, ok := state.Sys().(syscall.WaitStatus)
	if !ok {
		code := int32(state.ExitCode()) // #nosec G115 -- exit code fits in int32
		return &code, nil, nil
	}
	if ws.Signaled() {
		sig := int32(ws.Signal()) // #nosec G115 -- POSIX signal number fits in int32
		return nil, &sig, nil
	}
	code := int32(ws.ExitStatus()) // #nosec G115 -- 8-bit POSIX exit status fits in int32
	return &code, nil, nil
}

func runSetsidImpl(command string, args []string, cfg RunSetsidConfig, detachStdio bool) (*SessionSpawnResult, error) {
	cmd := exec.Command(command, args...) // #nosec G204 -- session spawn primitive intentionally executes caller-supplied argv without a shell
	if cfg.Cwd != nil && *cfg.Cwd != "" {
		cmd.Dir = *cfg.Cwd
	}
	if len(cfg.Env) > 0 {
		cmd.Env = mergeEnv(os.Environ(), cfg.Env)
	}
	cmd.SysProcAttr = &syscall.SysProcAttr{
		Setsid: true,
	}

	var devNull *os.File
	if detachStdio && !cfg.Wait {
		f, err := os.OpenFile(os.DevNull, os.O_RDWR, 0)
		if err != nil {
			return nil, syserror.System("failed to open /dev/null: "+err.Error(), errnoCode(err))
		}
		devNull = f
		defer func() { _ = devNull.Close() }()
		cmd.Stdin = devNull
		cmd.Stdout = devNull
		cmd.Stderr = devNull
	}

	if err := cmd.Start(); err != nil {
		return nil, mapSpawnError(command, err)
	}

	rawPID := cmd.Process.Pid
	if rawPID <= 0 || int64(rawPID) > int64(MaxSafePID) {
		_ = cmd.Process.Kill()
		_, _ = cmd.Process.Wait()
		return nil, syserror.SpawnFailed("session spawn: child_pid is outside [1, MaxSafePID]")
	}
	childPID := uint32(rawPID) // #nosec G115 -- validated 1 <= rawPID <= MaxSafePID
	if err := validateOutputPID(childPID, "child_pid"); err != nil {
		return nil, err
	}

	sid := childPID
	pgid := childPID
	res := &SessionSpawnResult{
		SchemaID:             SessionSpawnResultSchemaID,
		Timestamp:            nowRFC3339(),
		Platform:             currentPlatform(),
		Verb:                 "setsid",
		PID:                  &childPID,
		SID:                  &sid,
		PGID:                 &pgid,
		SessionKind:          SessionKindNewSession,
		IdentifierProvenance: ProvenanceSetsidStructuralChildPID,
		Warnings:             []string{},
	}

	if cfg.Wait {
		waitErr := cmd.Wait()
		exitCode, sig, err := extractExitCodeAndSignal(waitErr, cmd.ProcessState)
		if err != nil {
			return nil, err
		}
		res.Status = "completed"
		res.ExitCode = exitCode
		res.Signal = sig
		return res, nil
	}

	_ = cmd.Process.Release()
	res.Status = "spawned"
	return res, nil
}

func isatty(fd int) bool {
	_, err := unix.IoctlGetTermios(fd, ioctlGetTermiosReq)
	return err == nil
}

func openNoFollowAppend(path string) (*os.File, error) {
	syscall.ForkLock.RLock()
	fd, err := unix.Open(path, unix.O_WRONLY|unix.O_CREAT|unix.O_APPEND|unix.O_NOFOLLOW|unix.O_CLOEXEC, 0o600)
	if err == nil {
		unix.CloseOnExec(fd)
	}
	syscall.ForkLock.RUnlock()
	if err != nil {
		if errors.Is(err, unix.ELOOP) || errors.Is(err, unix.EACCES) || errors.Is(err, unix.EPERM) || errors.Is(err, os.ErrPermission) {
			return nil, syserror.PermissionDeniedPath(path, "open nohup output_file")
		}
		return nil, syserror.System("cannot open "+path+": "+err.Error(), errnoCode(err))
	}
	return os.NewFile(uintptr(fd), path), nil
}

func determineNohupOutput(cfg RunNohupConfig, stdoutIsTTY bool) (*string, *os.File, error) {
	if cfg.OutputFile != nil {
		path := *cfg.OutputFile
		f, err := openNoFollowAppend(path)
		if err != nil {
			return nil, nil, err
		}
		return &path, f, nil
	}
	if !stdoutIsTTY {
		return nil, nil, nil
	}
	cwdPath := "nohup.out"
	if f, err := openNoFollowAppend(cwdPath); err == nil {
		return &cwdPath, f, nil
	}
	if home := os.Getenv("HOME"); home != "" {
		homePath := filepath.Join(home, "nohup.out")
		f, err := openNoFollowAppend(homePath)
		if err != nil {
			return nil, nil, err
		}
		return &homePath, f, nil
	}
	f, err := openNoFollowAppend(cwdPath)
	if err != nil {
		return nil, nil, err
	}
	return &cwdPath, f, nil
}

func runNohupImpl(command string, args []string, cfg RunNohupConfig) (*SessionSpawnResult, error) {
	callerSID, err := Getsid(0)
	if err != nil {
		return nil, err
	}
	callerPGID, err := Getpgid(0)
	if err != nil {
		return nil, err
	}
	if err := validateOutputPID(callerSID, "caller_sid"); err != nil {
		return nil, err
	}
	if err := validateOutputPID(callerPGID, "caller_pgid"); err != nil {
		return nil, err
	}

	stdoutIsTTY := isatty(unix.Stdout)
	stderrIsTTY := isatty(unix.Stderr)
	explicitOutputFile := cfg.OutputFile != nil

	outputFile, outFileHandle, err := determineNohupOutput(cfg, stdoutIsTTY)
	if err != nil {
		return nil, err
	}
	if outFileHandle != nil {
		defer func() { _ = outFileHandle.Close() }()
	}

	cmd := exec.Command(command, args...) // #nosec G204 -- nohup spawn primitive intentionally executes caller-supplied argv without a shell
	if cfg.Cwd != nil && *cfg.Cwd != "" {
		cmd.Dir = *cfg.Cwd
	}
	if len(cfg.Env) > 0 {
		cmd.Env = mergeEnv(os.Environ(), cfg.Env)
	}
	if outFileHandle != nil && (explicitOutputFile || stdoutIsTTY) {
		cmd.Stdout = outFileHandle
		if explicitOutputFile || stderrIsTTY {
			cmd.Stderr = outFileHandle
		}
	}

	// Ensure the child inherits SIG_IGN for SIGHUP across fork+exec.
	nohupSpawnMu.Lock()
	wasIgnored := ossignal.Ignored(syscall.SIGHUP)
	ossignal.Ignore(syscall.SIGHUP)
	startErr := cmd.Start()
	if !wasIgnored {
		ossignal.Reset(syscall.SIGHUP)
	}
	nohupSpawnMu.Unlock()

	if startErr != nil {
		return nil, mapSpawnError(command, startErr)
	}

	rawPID := cmd.Process.Pid
	if rawPID <= 0 || int64(rawPID) > int64(MaxSafePID) {
		_ = cmd.Process.Kill()
		_, _ = cmd.Process.Wait()
		return nil, syserror.SpawnFailed("session spawn: child_pid is outside [1, MaxSafePID]")
	}
	childPID := uint32(rawPID) // #nosec G115 -- validated 1 <= rawPID <= MaxSafePID
	if err := validateOutputPID(childPID, "child_pid"); err != nil {
		return nil, err
	}

	res := &SessionSpawnResult{
		SchemaID:             SessionSpawnResultSchemaID,
		Timestamp:            nowRFC3339(),
		Platform:             currentPlatform(),
		Verb:                 "nohup",
		PID:                  &childPID,
		SID:                  &callerSID,
		PGID:                 &callerPGID,
		SessionKind:          SessionKindInheritedSession,
		IdentifierProvenance: ProvenanceCallerContextBeforeSpawn,
		OutputFile:           outputFile,
		Warnings:             []string{},
	}

	if cfg.Wait {
		waitErr := cmd.Wait()
		exitCode, sig, err := extractExitCodeAndSignal(waitErr, cmd.ProcessState)
		if err != nil {
			return nil, err
		}
		res.Status = "completed"
		res.ExitCode = exitCode
		res.Signal = sig
		return res, nil
	}

	_ = cmd.Process.Release()
	res.Status = "spawned"
	return res, nil
}

func spawnInGroupImpl(cfg SpawnInGroupConfig) (*SpawnInGroupResult, error) {
	command := cfg.Argv[0]
	args := cfg.Argv[1:]
	for i, arg := range cfg.Argv {
		if indexByteZero(arg) {
			return nil, syserror.InvalidArgumentf("argv[%d] contains NUL byte", i)
		}
	}

	cmd := exec.Command(command, args...) // #nosec G204 -- grouped spawn primitive intentionally executes caller-supplied argv without a shell
	if cfg.Cwd != nil && *cfg.Cwd != "" {
		cmd.Dir = *cfg.Cwd
	}
	if len(cfg.Env) > 0 {
		cmd.Env = mergeEnv(os.Environ(), cfg.Env)
	}
	cmd.SysProcAttr = &syscall.SysProcAttr{
		Setpgid: true,
		Pgid:    0,
	}

	if err := cmd.Start(); err != nil {
		return nil, mapSpawnError(command, err)
	}
	rawPID := cmd.Process.Pid
	if rawPID <= 0 || int64(rawPID) > int64(MaxSafePID) {
		_ = cmd.Process.Kill()
		_, _ = cmd.Process.Wait()
		return nil, syserror.SpawnFailed("grouped spawn: child_pid is outside [1, MaxSafePID]")
	}
	pid := uint32(rawPID) // #nosec G115 -- validated 1 <= rawPID <= MaxSafePID
	_ = cmd.Process.Release()

	pgid := pid
	return &SpawnInGroupResult{
		SchemaID:            SpawnInGroupResultSchemaID,
		Timestamp:           nowRFC3339(),
		Platform:            currentPlatform(),
		PID:                 pid,
		PGID:                &pgid,
		TreeKillReliability: "best_effort",
		Warnings: []string{
			"PID-only grouped spawn does not retain an owned child and sealed receipt",
		},
	}, nil
}

func spawnWithSessionReceiptImpl(cmd *exec.Cmd) (*UnixSessionReceipt, error) {
	if cmd.Process != nil {
		return nil, syserror.InvalidArgument("cmd has already been started")
	}
	if cmd.SysProcAttr == nil {
		cmd.SysProcAttr = &syscall.SysProcAttr{}
	}
	if cmd.SysProcAttr.Setpgid {
		return nil, syserror.InvalidArgument("SpawnWithSessionReceipt requires Setsid as the sole session/group acquirer")
	}
	cmd.SysProcAttr.Setsid = true

	if err := cmd.Start(); err != nil {
		return nil, mapSpawnError(cmd.Path, err)
	}
	rawPID := cmd.Process.Pid
	if rawPID <= 0 || int64(rawPID) > int64(MaxSafePID) {
		_ = cmd.Process.Kill()
		_, _ = cmd.Process.Wait()
		return nil, syserror.InvalidArgument("owned child pid is outside the safe Unix process range")
	}
	childPID := uint32(rawPID) // #nosec G115 -- validated 1 <= rawPID <= MaxSafePID

	return &UnixSessionReceipt{
		childPID:       childPID,
		processGroupID: childPID,
		sessionID:      childPID,
		sealed:         true,
	}, nil
}

func indexByteZero(s string) bool {
	for i := 0; i < len(s); i++ {
		if s[i] == 0 {
			return true
		}
	}
	return false
}
