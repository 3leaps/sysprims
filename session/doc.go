// SPDX-License-Identifier: MIT OR Apache-2.0

// Package session provides cross-platform process session and process-group
// introspection and spawn primitives for Go without cgo (CGO_ENABLED=0).
//
// This package is the pure-Go twin of crates/sysprims-session (plus the
// PID-returning SpawnInGroup compatibility surface). It enforces ADR-0011
// PID/PGID safety invariants at the API boundary and emits JSON-schema-aligned
// SessionSpawnResult and SpawnInGroupResult receipts matching Rust, Python,
// and TypeScript.
package session

import (
	"math"
	"os/exec"
	"runtime"
	"sort"
	"strings"
	"time"

	"github.com/3leaps/sysprims/syserror"
)

// MaxSafePID is the maximum safe POSIX PID/PGID value (2147483647, math.MaxInt32).
const MaxSafePID uint32 = math.MaxInt32

// Schema URI constants matching crates/sysprims-core/src/schema.rs.
const (
	RunSetsidConfigSchemaID    = "https://schemas.3leaps.dev/sysprims/session/v1.0.0/run-setsid-config.schema.json"
	RunNohupConfigSchemaID     = "https://schemas.3leaps.dev/sysprims/session/v1.0.0/run-nohup-config.schema.json"
	SessionSpawnResultSchemaID = "https://schemas.3leaps.dev/sysprims/session/v1.0.0/session-spawn-result.schema.json"
	SpawnInGroupConfigSchemaID = "https://schemas.3leaps.dev/sysprims/process/v1.0.0/spawn-in-group-config.schema.json"
	SpawnInGroupResultSchemaID = "https://schemas.3leaps.dev/sysprims/process/v1.0.0/spawn-in-group-result.schema.json"
)

// SessionKind identifies whether a session spawn created a new session or
// inherited the caller's session.
type SessionKind string

const (
	SessionKindNewSession       SessionKind = "new_session"
	SessionKindInheritedSession SessionKind = "inherited_session"
)

// IdentifierProvenance records how SID and PGID on SessionSpawnResult were
// determined.
type IdentifierProvenance string

const (
	ProvenanceSetsidStructuralChildPID IdentifierProvenance = "setsid_structural_child_pid"
	ProvenanceCallerContextBeforeSpawn IdentifierProvenance = "caller_context_before_spawn"
)

// RunSetsidConfig configures a new-session spawn (RunSetsid / SpawnDetached).
type RunSetsidConfig struct {
	SchemaID string            `json:"schema_id,omitempty"`
	Argv     []string          `json:"argv"`
	Cwd      *string           `json:"cwd,omitempty"`
	Env      map[string]string `json:"env,omitempty"`
	Wait     bool              `json:"wait,omitempty"`
}

// RunNohupConfig configures a SIGHUP-immune spawn (RunNohup).
type RunNohupConfig struct {
	SchemaID   string            `json:"schema_id,omitempty"`
	Argv       []string          `json:"argv"`
	Cwd        *string           `json:"cwd,omitempty"`
	Env        map[string]string `json:"env,omitempty"`
	Wait       bool              `json:"wait,omitempty"`
	OutputFile *string           `json:"output_file,omitempty"`
}

// SessionSpawnResult is the schema-aligned receipt returned by RunSetsid,
// SpawnDetached, and RunNohup (`session-spawn-result.schema.json`).
type SessionSpawnResult struct {
	SchemaID             string               `json:"schema_id"`
	Timestamp            string               `json:"timestamp"`
	Platform             string               `json:"platform"`
	Verb                 string               `json:"verb"`
	Status               string               `json:"status"`
	PID                  *uint32              `json:"pid"`
	SID                  *uint32              `json:"sid"`
	PGID                 *uint32              `json:"pgid"`
	SessionKind          SessionKind          `json:"session_kind"`
	IdentifierProvenance IdentifierProvenance `json:"identifier_provenance"`
	ExitCode             *int32               `json:"exit_code"`
	Signal               *int32               `json:"signal"`
	OutputFile           *string              `json:"output_file"`
	Warnings             []string             `json:"warnings"`
}

// SpawnInGroupConfig configures a process-group spawn (SpawnInGroup).
type SpawnInGroupConfig struct {
	SchemaID string            `json:"schema_id,omitempty"`
	Argv     []string          `json:"argv"`
	Cwd      *string           `json:"cwd,omitempty"`
	Env      map[string]string `json:"env,omitempty"`
}

// SpawnInGroupResult is the schema-aligned receipt returned by SpawnInGroup
// (`spawn-in-group-result.schema.json`).
type SpawnInGroupResult struct {
	SchemaID            string   `json:"schema_id"`
	Timestamp           string   `json:"timestamp"`
	Platform            string   `json:"platform"`
	PID                 uint32   `json:"pid"`
	PGID                *uint32  `json:"pgid,omitempty"`
	TreeKillReliability string   `json:"tree_kill_reliability"`
	Warnings            []string `json:"warnings"`
}

// UnixSessionReceipt is an opaque, single-spawn proof that a child process was
// spawned into a dedicated POSIX session and process group (`SID == PGID == PID`).
// It cannot be constructed from an arbitrary PID outside this package.
type UnixSessionReceipt struct {
	childPID       uint32
	processGroupID uint32
	sessionID      uint32
	sealed         bool
}

// ChildPID returns the child PID proved by the receipt.
func (r *UnixSessionReceipt) ChildPID() uint32 {
	if r == nil {
		return 0
	}
	return r.childPID
}

// ProcessGroupID returns the process group ID proved by the receipt.
func (r *UnixSessionReceipt) ProcessGroupID() uint32 {
	if r == nil {
		return 0
	}
	return r.processGroupID
}

// SessionID returns the session ID proved by the receipt.
func (r *UnixSessionReceipt) SessionID() uint32 {
	if r == nil {
		return 0
	}
	return r.sessionID
}

// SessionKind returns "new_session".
func (r *UnixSessionReceipt) SessionKind() SessionKind {
	return SessionKindNewSession
}

// IdentifierProvenance returns "setsid_structural_child_pid".
func (r *UnixSessionReceipt) IdentifierProvenance() IdentifierProvenance {
	return ProvenanceSetsidStructuralChildPID
}

// IsSealed reports whether this receipt was minted by SpawnWithSessionReceipt.
func (r *UnixSessionReceipt) IsSealed() bool {
	return r != nil && r.sealed && r.childPID > 0 && r.childPID <= MaxSafePID &&
		r.processGroupID == r.childPID && r.sessionID == r.childPID
}

// SelfPGID returns the caller's current process group ID (`getpgid(0)` on Unix).
// On Windows, returns ErrNotSupported.
func SelfPGID() (uint32, error) {
	return Getpgid(0)
}

// SelfSID returns the caller's current session ID (`getsid(0)` on Unix).
// On Windows, returns ErrNotSupported.
func SelfSID() (uint32, error) {
	return Getsid(0)
}

// Getpgid returns the process group ID for pid (`0` denotes the calling process).
// Values above MaxSafePID return ErrInvalidArgument. On Windows, returns ErrNotSupported.
func Getpgid(pid uint32) (uint32, error) {
	if pid > MaxSafePID {
		return 0, syserror.InvalidArgumentf("pid %d exceeds maximum safe value %d", pid, MaxSafePID)
	}
	return getpgidImpl(pid)
}

// Getsid returns the session ID for pid (`0` denotes the calling process).
// Values above MaxSafePID return ErrInvalidArgument. On Windows, returns ErrNotSupported.
func Getsid(pid uint32) (uint32, error) {
	if pid > MaxSafePID {
		return 0, syserror.InvalidArgumentf("pid %d exceeds maximum safe value %d", pid, MaxSafePID)
	}
	return getsidImpl(pid)
}

// Setpgid sets the process group ID for pid (`0` denotes the calling process;
// `pgid == 0` uses pid as pgid). Values above MaxSafePID return ErrInvalidArgument.
// On Windows, returns ErrNotSupported.
func Setpgid(pid, pgid uint32) error {
	if pid > MaxSafePID {
		return syserror.InvalidArgumentf("pid %d exceeds maximum safe value %d", pid, MaxSafePID)
	}
	if pgid > MaxSafePID {
		return syserror.InvalidArgumentf("pgid %d exceeds maximum safe value %d", pgid, MaxSafePID)
	}
	return setpgidImpl(pid, pgid)
}

// Setsid creates a new session for the calling process (`setsid(2)` on Unix).
// On Windows, returns ErrNotSupported.
func Setsid() (uint32, error) {
	return setsidImpl()
}

// RunSetsid spawns a command in a new POSIX session (`setsid(2)`).
//
// On Unix, the spawned child is session leader and process group leader
// (`SID == PGID == PID`), with `SessionKindNewSession` and
// `ProvenanceSetsidStructuralChildPID`. On Windows, returns ErrNotSupported.
func RunSetsid(cfg RunSetsidConfig) (*SessionSpawnResult, error) {
	if cfg.SchemaID != "" && cfg.SchemaID != RunSetsidConfigSchemaID {
		return nil, syserror.InvalidArgumentf("invalid schema_id (expected %s)", RunSetsidConfigSchemaID)
	}
	cmd, args, err := splitArgv(cfg.Argv)
	if err != nil {
		return nil, err
	}
	return runSetsidImpl(cmd, args, cfg, false)
}

// SpawnDetached spawns a command in a new POSIX session with stdin/stdout/stderr
// detached to /dev/null when Wait is false.
func SpawnDetached(cfg RunSetsidConfig) (*SessionSpawnResult, error) {
	if cfg.SchemaID != "" && cfg.SchemaID != RunSetsidConfigSchemaID {
		return nil, syserror.InvalidArgumentf("invalid schema_id (expected %s)", RunSetsidConfigSchemaID)
	}
	cmd, args, err := splitArgv(cfg.Argv)
	if err != nil {
		return nil, err
	}
	return runSetsidImpl(cmd, args, cfg, true)
}

// RunNohup spawns a command with SIGHUP ignored.
//
// The child inherits the caller's session and process group (`SessionKindInheritedSession`,
// `ProvenanceCallerContextBeforeSpawn`). If OutputFile is set (or stdout is a terminal),
// output is opened append/create with O_NOFOLLOW (rejecting symlinks with
// ErrPermissionDenied). On Windows, returns ErrNotSupported.
func RunNohup(cfg RunNohupConfig) (*SessionSpawnResult, error) {
	if cfg.SchemaID != "" && cfg.SchemaID != RunNohupConfigSchemaID {
		return nil, syserror.InvalidArgumentf("invalid schema_id (expected %s)", RunNohupConfigSchemaID)
	}
	cmd, args, err := splitArgv(cfg.Argv)
	if err != nil {
		return nil, err
	}
	return runNohupImpl(cmd, args, cfg)
}

// SpawnInGroup spawns a process in a new process group on Unix (`setpgid(0, 0)`).
//
// Because this PID-returning compatibility API does not retain an owned child
// handle and sealed containment receipt, Unix reports `tree_kill_reliability: "best_effort"`
// and Windows fails closed with ErrNotSupported.
func SpawnInGroup(cfg SpawnInGroupConfig) (*SpawnInGroupResult, error) {
	if cfg.SchemaID != "" && cfg.SchemaID != SpawnInGroupConfigSchemaID {
		return nil, syserror.InvalidArgumentf("invalid schema_id (expected %s)", SpawnInGroupConfigSchemaID)
	}
	if len(cfg.Argv) == 0 {
		return nil, syserror.InvalidArgument("argv must not be empty")
	}
	if cfg.Argv[0] == "" {
		return nil, syserror.InvalidArgument("argv[0] (command) must not be empty")
	}
	return spawnInGroupImpl(cfg)
}

// SpawnWithSessionReceipt starts cmd in a new POSIX session (`Setsid: true`)
// and returns a sealed *UnixSessionReceipt proving that the spawned child
// acquired `SID == PGID == PID`.
func SpawnWithSessionReceipt(cmd *exec.Cmd) (*UnixSessionReceipt, error) {
	if cmd == nil {
		return nil, syserror.InvalidArgument("cmd must not be nil")
	}
	return spawnWithSessionReceiptImpl(cmd)
}

func splitArgv(argv []string) (string, []string, error) {
	if len(argv) == 0 {
		return "", nil, syserror.InvalidArgument("argv must not be empty")
	}
	if argv[0] == "" {
		return "", nil, syserror.InvalidArgument("argv[0] must not be empty")
	}
	for i, arg := range argv {
		if strings.IndexByte(arg, 0) >= 0 {
			return "", nil, syserror.InvalidArgumentf("argv[%d] contains NUL byte", i)
		}
	}
	return argv[0], argv[1:], nil
}

func validateOutputPID(value uint32, name string) error {
	if value == 0 || value > MaxSafePID {
		return syserror.SpawnFailed(
			"session spawn: " + name + " is outside [1, MaxSafePID]",
		)
	}
	return nil
}

func nowRFC3339() string {
	return time.Now().UTC().Format(time.RFC3339Nano)
}

func currentPlatform() string {
	switch runtime.GOOS {
	case "darwin":
		return "macos"
	default:
		return runtime.GOOS
	}
}

func mergeEnv(base []string, overrides map[string]string) []string {
	if len(overrides) == 0 {
		return base
	}
	out := make([]string, 0, len(base)+len(overrides))
	out = append(out, base...)
	keys := make([]string, 0, len(overrides))
	for k := range overrides {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	for _, k := range keys {
		out = append(out, k+"="+overrides[k])
	}
	return out
}
