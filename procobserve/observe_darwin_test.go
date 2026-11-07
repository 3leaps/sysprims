// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build darwin

package procobserve_test

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"testing"

	"github.com/3leaps/sysprims/evidence"
	"github.com/3leaps/sysprims/procobserve"
)

func findSleepBinary(t *testing.T) string {
	t.Helper()
	for _, candidate := range []string{"/bin/sleep", "/usr/bin/sleep"} {
		if _, err := os.Stat(candidate); err == nil {
			return candidate
		}
	}
	t.Fatal("neither /bin/sleep nor /usr/bin/sleep exists")
	return ""
}

func TestMacOSSymlinkedExecutableAndLiveEmptyArgv0AllResidues(t *testing.T) {
	sleepPath := findSleepBinary(t)
	// #nosec G304 -- sleepPath is selected from constant ["/bin/sleep", "/usr/bin/sleep"]
	sleepBytes, err := os.ReadFile(sleepPath)
	if err != nil {
		t.Fatalf("read %s: %v", sleepPath, err)
	}
	wantExeDigest := evidence.DeriveExecutableDigest(sleepBytes)

	tmpDir := t.TempDir()
	hs := sampleHostScope()
	opts := procobserve.DefaultObservationOptions().WithAllContextFacts()
	opts.ObservedAt = "2026-11-06T12:00:00Z"

	// 1. Symlinked executable resolves cleanly and hashes target image under O_NOFOLLOW | O_NONBLOCK.
	symPath := filepath.Join(tmpDir, "sleep-symlink")
	if err := os.Symlink(sleepPath, symPath); err != nil {
		t.Fatalf("symlink: %v", err)
	}
	// #nosec G204 -- symPath points to constant sleepPath inside t.TempDir()
	symCmd := exec.Command(symPath, "30")
	if err := symCmd.Start(); err != nil {
		t.Fatalf("start symlink child: %v", err)
	}
	t.Cleanup(func() {
		_ = symCmd.Process.Kill()
		_, _ = symCmd.Process.Wait()
	})
	symPID := uint32(symCmd.Process.Pid) // #nosec G115 -- valid positive Darwin PID
	symObs, err := procobserve.ObserveProcess(symPID, hs, opts)
	if err != nil {
		t.Fatalf("ObserveProcess symlink child: %v", err)
	}
	symEC := mustMap(t, mustMap(t, symObs["process"], "process")["execution_context"], "execution_context")
	symExe := mustMap(t, symEC["executable_digest"], "executable_digest")
	if symExe["status"] != "observed" || symExe["digest"] != wantExeDigest {
		t.Fatalf("symlink executable_digest = %v, want %s", symExe, wantExeDigest)
	}
	wantSymArgs := evidence.DeriveArgumentsDigest([][]byte{[]byte(symPath), []byte("30")})
	symArgs := mustMap(t, symEC["arguments_digest"], "arguments_digest")
	if symArgs["status"] != "observed" || symArgs["digest"] != wantSymArgs {
		t.Fatalf("symlink arguments_digest = %v, want %s", symArgs, wantSymArgs)
	}

	// 2. Live kernel execve with empty argv[0] = "" across all 8 exec_path length residues
	// refuses arguments_digest fail-closed with ambiguous-kern-procargs2-padding without
	// consuming subsequent argv or SECRET_ENV bytes.
	for residue := range 8 {
		name := "s" + strings.Repeat("x", residue)
		linkPath := filepath.Join(tmpDir, name)
		// Adjust name length so len(linkPath)%8 == residue.
		extra := (8 + residue - (len(linkPath) % 8)) % 8
		linkPath += strings.Repeat("y", extra)
		if len(linkPath)%8 != residue {
			t.Fatalf("constructed linkPath %q residue %d != %d", linkPath, len(linkPath)%8, residue)
		}
		if err := os.Symlink(sleepPath, linkPath); err != nil {
			t.Fatalf("symlink residue %d: %v", residue, err)
		}

		rawPID, err := syscall.ForkExec(linkPath, []string{"", "30"}, &syscall.ProcAttr{
			Env: []string{"SECRET_ENV=NEVER_IN_ARGV"},
		})
		if err != nil {
			t.Fatalf("ForkExec residue %d: %v", residue, err)
		}
		reaped := false
		cleanupChild := func() {
			if !reaped {
				_ = syscall.Kill(rawPID, syscall.SIGKILL)
				var ws syscall.WaitStatus
				_, _ = syscall.Wait4(rawPID, &ws, 0, nil)
				reaped = true
			}
		}
		t.Cleanup(cleanupChild)

		obs, err := procobserve.ObserveProcess(uint32(rawPID), hs, opts) // #nosec G115 -- valid positive Darwin PID
		if err != nil {
			t.Fatalf("ObserveProcess empty-argv0 residue %d: %v", residue, err)
		}
		ec := mustMap(t, mustMap(t, obs["process"], "process")["execution_context"], "execution_context")
		argsFact := mustMap(t, ec["arguments_digest"], "arguments_digest")
		if argsFact["status"] != "error" || argsFact["reason"] != "ambiguous-kern-procargs2-padding" {
			t.Fatalf("residue %d empty-argv0 arguments_digest = %v, want status=error reason=ambiguous-kern-procargs2-padding", residue, argsFact)
		}
		exeFact := mustMap(t, ec["executable_digest"], "executable_digest")
		if exeFact["status"] != "observed" || exeFact["digest"] != wantExeDigest {
			t.Fatalf("residue %d executable_digest = %v, want %s", residue, exeFact, wantExeDigest)
		}
		cleanupChild()
	}
}
