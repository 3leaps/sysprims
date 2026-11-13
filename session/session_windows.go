// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build windows

package session

import (
	"os/exec"

	"github.com/3leaps/sysprims/syserror"
)

func setsidImpl() (uint32, error) {
	return 0, syserror.NotSupported("setsid", "windows")
}

func getsidImpl(pid uint32) (uint32, error) {
	_ = pid
	return 0, syserror.NotSupported("getsid", "windows")
}

func getpgidImpl(pid uint32) (uint32, error) {
	_ = pid
	return 0, syserror.NotSupported("getpgid", "windows")
}

func setpgidImpl(pid, pgid uint32) error {
	_, _ = pid, pgid
	return syserror.NotSupported("setpgid", "windows")
}

func runSetsidImpl(command string, args []string, cfg RunSetsidConfig, detachStdio bool) (*SessionSpawnResult, error) {
	_, _, _, _ = command, args, cfg, detachStdio
	return nil, syserror.NotSupported("setsid", "windows")
}

func runNohupImpl(command string, args []string, cfg RunNohupConfig) (*SessionSpawnResult, error) {
	_, _, _ = command, args, cfg
	return nil, syserror.NotSupported("nohup", "windows")
}

func spawnInGroupImpl(cfg SpawnInGroupConfig) (*SpawnInGroupResult, error) {
	_ = cfg
	return nil, syserror.NotSupported(
		"spawn_in_group without an owned containment guard",
		"windows; use spawn_contained",
	)
}

func spawnWithSessionReceiptImpl(cmd *exec.Cmd) (*UnixSessionReceipt, error) {
	_ = cmd
	return nil, syserror.NotSupported("SpawnWithSessionReceipt", "windows")
}
