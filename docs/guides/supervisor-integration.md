# Supervisor Integration Guide

This guide covers integrating sysprims v0.1.6+ primitives into long-running supervisors and job managers.

## Audience

Teams building:
- Job schedulers and supervisors (gonimbus, rampart lifecycle)
- Test harnesses with process lifecycle management (gauntlet)
- Container runtimes or orchestrators
- Any system that spawns and manages child processes

## Principles

1. **Thin adapters**: Keep sysprims calls behind a small interface for fallback capability
2. **Safety first**: PID validation and reuse guards reduce blast radius
3. **Platform-aware**: Windows termination semantics differ from Unix signals
4. **Incremental adoption**: No "big bang" switch required

## Recommended Integration Order

### Phase 1: Add PID Identity Checks

**Goal**: Detect PID reuse before signaling stale processes.

**Implementation**:

```go
// At job creation, store identity
info, _ := sysprims.ProcessGet(pid)
jobRecord := JobRecord{
    PID:         info.PID,
    StartTimeMs: info.StartTimeUnixMS,  // *uint64 (may be nil)
    ExePath:     info.ExePath,          // *string (may be nil)
    Cmdline:     info.Cmdline,
}

// Before signaling, verify identity
func (j *Job) VerifyIdentity() (bool, error) {
    current, err := sysprims.ProcessGet(j.PID)
    if err != nil {
        if errors.Is(err, sysprims.ErrNotFound) {
            return false, nil // Process already dead
        }
        return false, err
    }

    // Primary check: start time
    if j.StartTimeMs != nil && current.StartTimeUnixMs != nil {
        if *current.StartTimeUnixMs != *j.StartTimeMs {
            log.Warnf("PID %d reused: start time mismatch", j.PID)
            return false, nil
        }
    }

    // Fallback check: executable path or cmdline
    if j.ExePath != nil && current.ExePath != nil {
        if *current.ExePath != *j.ExePath {
            log.Warnf("PID %d reused: exe path mismatch", j.PID)
            return false, nil
        }
    }

    return true, nil
}
```

**Fallback behavior**:
- If identity fields unavailable, compare `cmdline`/`name`
- If ambiguous, require manual intervention or treat as "unknown"

### Phase 2: Replace Stop Flows with TerminateTree

**Goal**: Single primitive for graceful-then-kill termination.

Most supervisors implement:
1. Send graceful termination signal
2. Wait up to N seconds
3. Escalate to force kill

Replace with:

```go
grace := uint64(5000)
kill := uint64(2000)
config := sysprims.TerminateTreeConfig{
    GraceTimeoutMS: &grace,  // 5 seconds grace period
    KillTimeoutMS:  &kill,   // 2 seconds after kill
}

outcome, err := sysprims.TerminateTree(job.PID, config)
if err != nil {
    // Handle error (NotFound, PermissionDenied, etc.)
    return j.legacyStop()
}

// Log outcome for observability
log.Infof("Terminated PID %d: escalated=%v reliability=%s",
    job.PID, outcome.Escalated, outcome.TreeKillReliability)

for _, warning := range outcome.Warnings {
    log.Warnf("Termination warning: %s", warning)
}
```

**Observability fields to persist**:
- `tree_kill_reliability`: "guaranteed" or "best_effort"
- `escalated`: whether SIGKILL was required
- `warnings`: platform-specific issues

**Fallback behavior**:
- If sysprims returns `PermissionDenied`, fall back to per-OS logic

### Phase 3: Adopt SpawnInGroup

**Goal**: All jobs are "kill-tree safe" by construction.

```go
config := sysprims.SpawnInGroupConfig{
    Argv: []string{"./worker", "--job-id", jobID},
    Cwd:  workDir,
    Env:  map[string]string{"LOG_LEVEL": "info"},
}

result, err := sysprims.SpawnInGroup(config)
if err != nil {
    return j.legacySpawn(config.Argv)
}

job := &Job{
    PID:                 result.PID,
    PGID:                result.PGID,  // nil on Windows
    TreeKillReliability: result.TreeKillReliability,
}

if result.TreeKillReliability != "guaranteed" {
    log.Warnf("Job %s spawned with degraded tree-kill: %s", jobID, result.TreeKillReliability)
}
```

**Platform notes**:
- **Unix**: `PGID` contains the process group ID
- **Windows**: `PGID` is nil; Job Object handles tree-kill internally
- **Degradation**: If Job Object creation fails, reliability is "best_effort"

## Adapter Pattern

Define a clean interface for testability and fallback:

```go
type ProcessAdapter interface {
    GetProcess(pid uint32) (*ProcessInfo, error)
    SpawnInGroup(config SpawnInGroupConfig) (*SpawnInGroupResult, error)
    TerminateTree(pid uint32, config TerminateTreeConfig) (*TerminateTreeResult, error)
}

// Production: uses sysprims
type SysprimsAdapter struct{}

// Testing/fallback: uses os/exec + per-OS behavior
type LegacyAdapter struct{}
```

**Rollout strategy**:
1. Default to sysprims behind a feature flag
2. Use legacy adapter as fallback for safe-to-degrade error classes
3. Monitor metrics: tree_kill_reliability, escalation rate, warnings

## Platform Considerations

### Unix (Linux/macOS)

- Process groups via `setpgid(0, 0)`
- Tree kill via `killpg(pgid, signal)`
- Signals: SIGTERM for graceful, SIGKILL for force

### Windows

- Job Objects with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
- Nested Job Objects may prevent grouping (common in containers)
- No true signal semantics; `TerminateProcess` is immediate

**Note:** On Windows, sysprims uses Job Objects where possible. v0.1.6 does not expose a stable Job token in the API surface yet; tree kill without a Job is best-effort.

### Containers

- `PermissionDenied` is expected in some hardened environments
- Nested Job Objects common in Windows containers
- Ensure UX surfaces warnings clearly

## Operational Checklist

- [ ] Store `start_time_unix_ms` and `exe_path` in job records
- [ ] Verify identity before any signal operation
- [ ] Log `tree_kill_reliability` for monitoring
- [ ] Handle `PermissionDenied` gracefully with fallback
- [ ] Never use PID 0, 1, or u32::MAX (see ADR-0011)
- [ ] Test degraded reliability scenarios

## Error Handling

| Error | Meaning | Recommended Action |
|-------|---------|-------------------|
| `NotFound` | Process does not exist | Job already dead, update state |
| `PermissionDenied` | Cannot access process | Fall back to legacy or alert |
| `InvalidArgument` | Invalid PID (0, etc.) | Bug in caller; fix immediately |

**Note on grouping failures:** When Job Object creation fails (Windows) or `setpgid` fails (Unix), sysprims typically degrades gracefully rather than returning an error. Check `tree_kill_reliability` in the result and `warnings` array for degradation details.

## References

- [ADR-0003: Group-by-Default](../architecture/adr/0003-group-by-default.md)
- [ADR-0011: PID Validation Safety](../architecture/adr/0011-pid-validation-safety.md)
- [Timeout Spec](../design/sysprims-timeout/timeout-spec.md)
- [Proc Spec](../design/sysprims-proc/proc-spec.md)
