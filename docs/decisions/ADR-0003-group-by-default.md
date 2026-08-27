# ADR-0003: Group-by-Default Process Control

> **Status**: Accepted  
> **Date**: 2025-12-31  
> **Amended**: 2026-08-27
> **Authors**: Architecture Council

## Context

Process tree management is unreliable in standard tooling:

1. **GNU timeout**: Kills only the direct child; grandchildren may continue as orphans
2. **Standard spawn**: No automatic process group creation
3. **CI/CD impact**: Jobs "time out" but processes continue consuming resources
4. **Container impact**: Orphaned processes prevent clean container shutdown

This is sysprims's core reliability differentiator. We must get it right.

## Decision

### Default Behavior

`sysprims-timeout` acquires process groups (Unix) or Job Objects (Windows) by
default and targets that containment on timeout. `guaranteed` means race-free
spawn-time acquisition and group/job-signaling eligibility. A Unix process
group is cooperative containment: descendants may later change session or
group, so this is not an OS-enforced non-escape guarantee.

### Unix Implementation

```rust
// Parent, before fork: prepare a non-cloneable hook and private receipt channel.
let (hook, pending) = prepare_session_acquisition()?;

// Child, post-fork/pre-exec: exactly one async-signal-safe acquirer.
command.pre_exec(move || hook.acquire());

// Parent, after spawn: require positive same-spawn acknowledgement and bind it
// to the owned child before constructing a guaranteed guard.
let child = command.spawn()?;
let receipt = pending.into_receipt(child.id())?;
// SAFETY: this is the same child that emitted the receipt, and its adapter
// retains exclusive unreaped ownership through guard finalization.
let guard = unsafe { contain_acquired_session(child, receipt)? };

// On timeout: signal the entire group
unsafe {
    libc::killpg(child_pgid, libc::SIGTERM);
}
// Wait for kill_after duration
std::thread::sleep(kill_after);
// Escalate if still alive
unsafe {
    libc::killpg(child_pgid, libc::SIGKILL);
}
```

The sealed receipt records the structural `setsid` result
(`sid == pgid == child_pid`) with
`identifier_provenance = "setsid_structural_child_pid"`. It cannot be
constructed from a PID, boolean, enum, post-spawn observation, or another
library's testimony. The guard retains exclusive reap ownership and keeps the
leader unreaped through the final group signal, preventing PID/PGID reuse.
Legacy PID-returning group spawn paths may still use pre-exec `setpgid`; they do
not provide a receipt for an external owned child and therefore report
`best_effort`, never `guaranteed`.

### Windows Implementation

```rust
// Create Job Object with termination semantics
let job = unsafe { CreateJobObjectW(null_mut(), null()) };

let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
unsafe {
    SetInformationJobObject(
        job,
        JobObjectExtendedLimitInformation,
        &info as *const _ as *const c_void,
        size_of_val(&info) as u32,
    );
}

// Assign the owned process capability to the job. Post-spawn assignment is
// explicitly unproven because descendants may escape before this call.
unsafe {
    AssignProcessToJobObject(job, child_handle);
}

// On timeout: terminate the owned job, then reap the retained child
unsafe {
    TerminateJobObject(job, 1);
}
```

An owned Windows spawn is guaranteed only when the child is created suspended,
assigned to the Job, and then resumed. Until that seam exists, the owned spawn
API fails before spawning. The post-spawn adoption API returns an owned guard
with `tree_kill_reliability = "unproven"`. PID-returning spawn APIs cannot carry
the Job capability and therefore fail closed on Windows.

### Owned Guard Lifecycle

The containment guard retains both the child reap capability and the bound
process group or Job. Normal completion is observed without reaping the leader;
the guard first cleans remaining descendants, then reaps and permits the child
adapter to be released. Dropping an active guard kills the containment and makes
a bounded attempt to reap the child on Unix and Windows.

### Fallback Behavior

Legacy timeout operations may still degrade when group/job creation fails. They:

1. Log warning
2. Proceed with direct child process only
3. Set `tree_kill_reliability = "best_effort"` in output
4. Document degradation in result

### Observable Output

```json
{
  "status": "timeout",
  "grouping_requested": "group_by_default",
  "grouping_effective": "group_by_default",
  "tree_kill_reliability": "guaranteed" // or "unproven" / "best_effort"
}
```

### Opt-Out

Users can opt out with:

- CLI: `--foreground` flag
- Library: `GroupingMode::Foreground`

```rust
let config = TimeoutConfig {
    grouping: GroupingMode::Foreground,  // Don't create process group
    ..Default::default()
};
```

### Testing Requirement

Every platform CI job **must** include group-cleanup coverage:

1. Spawn child that creates 10 grandchildren
2. Cooperative grandchildren remain in the acquired group
3. `sysprims-timeout` signals the group within deadline
4. Assert: no in-group processes remain

Hostile detach/non-escape boundary cases run only in the disposable-container
`make test-diabolical` target. Those cases document that a descendant which
successfully leaves a Unix group is outside the group-signaling contract.

This is non-negotiable; it's the core differentiator.

## Consequences

### Positive

- CI jobs actually terminate on timeout
- Containers can shut down cleanly
- Resource leaks from cooperative in-group descendants are prevented
- Clear improvement over GNU timeout

### Negative

- Slightly more complex spawn path
- Windows Job Object limitations on older systems
- Users with specific process group needs must opt out

### Neutral

- JSON output includes reliability field
- Documentation must explain behavior clearly
- Benchmarks should measure overhead

## Alternatives Considered

### Alternative 1: Opt-In Tree Kill

Make tree kill a flag (`--kill-tree`) rather than default.

**Rejected**: The default should be the safe behavior. Most users expect timeout to actually stop everything.

### Alternative 2: SIGKILL Only

Skip SIGTERM and go straight to SIGKILL.

**Rejected**: Graceful shutdown should be attempted first. SIGTERM allows cleanup handlers.

### Alternative 3: Platform Parity via Abstraction Layer

Create unified abstraction hiding all platform differences.

**Rejected**: Platform differences are real and should be observable. Hiding them causes subtle bugs.

### Alternative 4: Fail on Job Object Error

If Windows Job Object creation fails, fail the entire operation.

**Partially adopted for owned guards**: APIs that promise an owned containment
capability fail closed if they cannot establish one. Legacy timeout operations
retain best-effort behavior with observable reliability.

## References

- [GNU timeout source](https://github.com/coreutils/coreutils/blob/master/src/timeout.c)
- [Windows Job Objects](https://docs.microsoft.com/en-us/windows/win32/procthread/job-objects)
- [POSIX Process Groups](https://pubs.opengroup.org/onlinepubs/9699919799/functions/setpgid.html)
