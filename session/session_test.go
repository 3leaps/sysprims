// SPDX-License-Identifier: MIT OR Apache-2.0

package session_test

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/3leaps/sysprims/session"
	"github.com/3leaps/sysprims/signal"
	"github.com/3leaps/sysprims/syserror"
)

func TestSelfPGIDAndSelfSID(t *testing.T) {
	pgid, err := session.SelfPGID()
	if runtime.GOOS == "windows" {
		if !syserror.IsCode(err, syserror.ErrNotSupported) {
			t.Fatalf("expected ErrNotSupported on Windows, got %v", err)
		}
		return
	}
	if err != nil {
		t.Fatalf("SelfPGID failed: %v", err)
	}
	if pgid == 0 || pgid > session.MaxSafePID {
		t.Fatalf("unexpected SelfPGID: %d", pgid)
	}

	sid, err := session.SelfSID()
	if err != nil {
		t.Fatalf("SelfSID failed: %v", err)
	}
	if sid == 0 || sid > session.MaxSafePID {
		t.Fatalf("unexpected SelfSID: %d", sid)
	}

	// Getpgid(os.Getpid()) and Getsid(os.Getpid()) must match SelfPGID/SelfSID.
	selfPID := uint32(os.Getpid()) // #nosec G115
	gotPGID, err := session.Getpgid(selfPID)
	if err != nil || gotPGID != pgid {
		t.Fatalf("Getpgid(self) = (%d, %v), want %d", gotPGID, err, pgid)
	}
	gotSID, err := session.Getsid(selfPID)
	if err != nil || gotSID != sid {
		t.Fatalf("Getsid(self) = (%d, %v), want %d", gotSID, err, sid)
	}
}

func TestPIDBoundsValidation(t *testing.T) {
	if _, err := session.Getpgid(session.MaxSafePID + 1); !syserror.IsCode(err, syserror.ErrInvalidArgument) {
		t.Fatalf("expected ErrInvalidArgument for Getpgid(MaxSafePID+1), got %v", err)
	}
	if _, err := session.Getsid(session.MaxSafePID + 1); !syserror.IsCode(err, syserror.ErrInvalidArgument) {
		t.Fatalf("expected ErrInvalidArgument for Getsid(MaxSafePID+1), got %v", err)
	}
	if err := session.Setpgid(session.MaxSafePID+1, 0); !syserror.IsCode(err, syserror.ErrInvalidArgument) {
		t.Fatalf("expected ErrInvalidArgument for Setpgid(MaxSafePID+1, 0), got %v", err)
	}
	if err := session.Setpgid(0, session.MaxSafePID+1); !syserror.IsCode(err, syserror.ErrInvalidArgument) {
		t.Fatalf("expected ErrInvalidArgument for Setpgid(0, MaxSafePID+1), got %v", err)
	}
}

func TestConfigValidation(t *testing.T) {
	if _, err := session.RunSetsid(session.RunSetsidConfig{
		SchemaID: "https://schemas.3leaps.dev/sysprims/session/v9.9.9/wrong.schema.json",
		Argv:     []string{"true"},
	}); !syserror.IsCode(err, syserror.ErrInvalidArgument) {
		t.Fatalf("expected ErrInvalidArgument on wrong setsid schema_id, got %v", err)
	}
	if _, err := session.RunSetsid(session.RunSetsidConfig{}); !syserror.IsCode(err, syserror.ErrInvalidArgument) {
		t.Fatalf("expected ErrInvalidArgument on empty argv, got %v", err)
	}
	if _, err := session.RunSetsid(session.RunSetsidConfig{Argv: []string{""}}); !syserror.IsCode(err, syserror.ErrInvalidArgument) {
		t.Fatalf("expected ErrInvalidArgument on empty argv[0], got %v", err)
	}
	if _, err := session.RunNohup(session.RunNohupConfig{
		SchemaID: "https://schemas.3leaps.dev/sysprims/session/v9.9.9/wrong.schema.json",
		Argv:     []string{"true"},
	}); !syserror.IsCode(err, syserror.ErrInvalidArgument) {
		t.Fatalf("expected ErrInvalidArgument on wrong nohup schema_id, got %v", err)
	}
	if _, err := session.SpawnInGroup(session.SpawnInGroupConfig{}); !syserror.IsCode(err, syserror.ErrInvalidArgument) {
		t.Fatalf("expected ErrInvalidArgument on empty SpawnInGroup argv, got %v", err)
	}
}

func TestRunSetsidCompletedAndCwdEnv(t *testing.T) {
	if runtime.GOOS == "windows" {
		_, err := session.RunSetsid(session.RunSetsidConfig{Argv: []string{"cmd", "/C", "exit 0"}})
		if !syserror.IsCode(err, syserror.ErrNotSupported) {
			t.Fatalf("expected ErrNotSupported on Windows, got %v", err)
		}
		return
	}

	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "marker"), []byte("ok"), 0o600); err != nil {
		t.Fatal(err)
	}

	res, err := session.RunSetsid(session.RunSetsidConfig{
		Argv: []string{"sh", "-c", "test -f marker && test \"$SYSPRIMS_SESSION_GO_TEST\" = expected && exit 42"},
		Cwd:  &dir,
		Env:  map[string]string{"SYSPRIMS_SESSION_GO_TEST": "expected"},
		Wait: true,
	})
	if err != nil {
		t.Fatalf("RunSetsid failed: %v", err)
	}
	if res.SchemaID != session.SessionSpawnResultSchemaID {
		t.Fatalf("unexpected schema_id: %s", res.SchemaID)
	}
	if res.Verb != "setsid" || res.Status != "completed" {
		t.Fatalf("unexpected verb/status: %s/%s", res.Verb, res.Status)
	}
	if res.PID == nil || res.SID == nil || res.PGID == nil {
		t.Fatalf("expected non-nil pid/sid/pgid")
	}
	if *res.PID != *res.SID || *res.PID != *res.PGID {
		t.Fatalf("expected structural SID==PGID==PID, got pid=%d sid=%d pgid=%d", *res.PID, *res.SID, *res.PGID)
	}
	if res.SessionKind != session.SessionKindNewSession {
		t.Fatalf("unexpected session_kind: %s", res.SessionKind)
	}
	if res.IdentifierProvenance != session.ProvenanceSetsidStructuralChildPID {
		t.Fatalf("unexpected provenance: %s", res.IdentifierProvenance)
	}
	if res.ExitCode == nil || *res.ExitCode != 42 {
		t.Fatalf("expected exit_code 42, got %v", res.ExitCode)
	}
}

func TestRunSetsidNotFoundCommand(t *testing.T) {
	if runtime.GOOS == "windows" {
		_, err := session.RunSetsid(session.RunSetsidConfig{
			Argv: []string{"nonexistent_sysprims_command_xyz_12345"},
		})
		if !syserror.IsCode(err, syserror.ErrNotSupported) {
			t.Fatalf("expected ErrNotSupported on Windows, got %v", err)
		}
		return
	}
	_, err := session.RunSetsid(session.RunSetsidConfig{
		Argv: []string{"nonexistent_sysprims_command_xyz_12345"},
	})
	if !syserror.IsCode(err, syserror.ErrNotFound) {
		t.Fatalf("expected ErrNotFound for missing command, got %v", err)
	}
}

func TestRunNohupCompletedAndSymlinkRejection(t *testing.T) {
	if runtime.GOOS == "windows" {
		_, err := session.RunNohup(session.RunNohupConfig{Argv: []string{"cmd", "/C", "exit 0"}})
		if !syserror.IsCode(err, syserror.ErrNotSupported) {
			t.Fatalf("expected ErrNotSupported on Windows, got %v", err)
		}
		return
	}

	callerSID, err := session.SelfSID()
	if err != nil {
		t.Fatal(err)
	}
	callerPGID, err := session.SelfPGID()
	if err != nil {
		t.Fatal(err)
	}

	dir := t.TempDir()
	outPath := filepath.Join(dir, "nohup.out")
	res, err := session.RunNohup(session.RunNohupConfig{
		Argv:       []string{"sh", "-c", "kill -HUP $$ && echo survived-hup"},
		Cwd:        &dir,
		OutputFile: &outPath,
		Wait:       true,
	})
	if err != nil {
		t.Fatalf("RunNohup failed: %v", err)
	}
	if res.Verb != "nohup" || res.Status != "completed" {
		t.Fatalf("unexpected verb/status: %s/%s", res.Verb, res.Status)
	}
	if res.SID == nil || *res.SID != callerSID || res.PGID == nil || *res.PGID != callerPGID {
		t.Fatalf("expected inherited caller sid/pgid (%d/%d), got (%v/%v)", callerSID, callerPGID, res.SID, res.PGID)
	}
	if res.SessionKind != session.SessionKindInheritedSession {
		t.Fatalf("unexpected session_kind: %s", res.SessionKind)
	}
	if res.IdentifierProvenance != session.ProvenanceCallerContextBeforeSpawn {
		t.Fatalf("unexpected provenance: %s", res.IdentifierProvenance)
	}
	if res.ExitCode == nil || *res.ExitCode != 0 {
		t.Fatalf("expected exit_code 0 (child survived SIGHUP), got exit=%v sig=%v", res.ExitCode, res.Signal)
	}
	data, err := os.ReadFile(outPath) // #nosec G304 -- test reads temp output file
	if err != nil {
		t.Fatalf("failed to read nohup output: %v", err)
	}
	if string(data) != "survived-hup\n" {
		t.Fatalf("unexpected nohup output: %q", string(data))
	}

	// Symlink output_file must be rejected with ErrPermissionDenied (O_NOFOLLOW).
	target := filepath.Join(dir, "target.log")
	link := filepath.Join(dir, "link.log")
	if err := os.WriteFile(target, []byte("existing"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(target, link); err != nil {
		t.Fatal(err)
	}
	_, err = session.RunNohup(session.RunNohupConfig{
		Argv:       []string{"sh", "-c", "exit 0"},
		OutputFile: &link,
		Wait:       true,
	})
	if !syserror.IsCode(err, syserror.ErrPermissionDenied) {
		t.Fatalf("expected ErrPermissionDenied for symlink output_file, got %v", err)
	}
}

func TestSpawnInGroupAndSessionReceipt(t *testing.T) {
	if runtime.GOOS == "windows" {
		_, err := session.SpawnInGroup(session.SpawnInGroupConfig{Argv: []string{"cmd", "/C", "exit 0"}})
		if !syserror.IsCode(err, syserror.ErrNotSupported) {
			t.Fatalf("expected ErrNotSupported on Windows, got %v", err)
		}
		return
	}

	res, err := session.SpawnInGroup(session.SpawnInGroupConfig{
		Argv: []string{"sleep", "30"},
	})
	if err != nil {
		t.Fatalf("SpawnInGroup failed: %v", err)
	}
	defer func() {
		_ = signal.ForceKill(res.PID)
	}()
	if res.PGID == nil || *res.PGID != res.PID {
		t.Fatalf("expected PGID == PID (%d), got %v", res.PID, res.PGID)
	}
	if res.TreeKillReliability != "best_effort" {
		t.Fatalf("expected best_effort, got %s", res.TreeKillReliability)
	}
	gotPGID, err := session.Getpgid(res.PID)
	if err != nil || gotPGID != res.PID {
		t.Fatalf("Getpgid(%d) = (%d, %v), want %d", res.PID, gotPGID, err, res.PID)
	}

	// SpawnWithSessionReceipt must mint a sealed receipt with SID == PGID == PID.
	cmd := exec.Command("sleep", "30")
	receipt, err := session.SpawnWithSessionReceipt(cmd)
	if err != nil {
		t.Fatalf("SpawnWithSessionReceipt failed: %v", err)
	}
	defer func() {
		_ = cmd.Process.Kill()
		_, _ = cmd.Process.Wait()
	}()
	if !receipt.IsSealed() {
		t.Fatalf("expected receipt to be sealed")
	}
	if receipt.ChildPID() == 0 || receipt.ProcessGroupID() != receipt.ChildPID() || receipt.SessionID() != receipt.ChildPID() {
		t.Fatalf("unexpected receipt IDs: pid=%d pgid=%d sid=%d", receipt.ChildPID(), receipt.ProcessGroupID(), receipt.SessionID())
	}
	gotSID, err := session.Getsid(receipt.ChildPID())
	if err != nil || gotSID != receipt.ChildPID() {
		t.Fatalf("Getsid(%d) = (%d, %v), want %d", receipt.ChildPID(), gotSID, err, receipt.ChildPID())
	}
}
