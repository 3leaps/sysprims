# ADR-0003: Group-by-Default Process Control

> **Status**: Accepted  
> **Date**: 2025-12-31  
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

All `sysprims-timeout` operations create process groups (Unix) or Job Objects (Windows) by default. The entire process tree is terminated on timeout.

### Unix Implementation

```rust
// Pre-spawn: configure child to become process group leader
command.pre_exec(|| {
    unsafe { libc::setpgid(0, 0) };
    Ok(())
});

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

// Assign spawned process to job
unsafe {
    AssignProcessToJobObject(job, child_handle);
}

// On timeout: closing job handle terminates all processes
unsafe {
    CloseHandle(job);
}
```

### Fallback Behavior

If group/job creation fails (e.g., nested Job Objects on Windows, permission issues):

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
  "tree_kill_reliability": "guaranteed" // or "best_effort"
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

Every platform CI job **must** include a "tree escape" test:

1. Spawn child that creates 10 grandchildren
2. Grandchildren attempt to detach/ignore signals
3. `sysprims-timeout` kills within deadline
4. Assert: no orphaned processes remain

This is non-negotiable; it's the core differentiator.

## Consequences

### Positive

- CI jobs actually terminate on timeout
- Containers can shut down cleanly
- Resource leaks from orphaned processes eliminated
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

**Considered for v0.2.1**: We'll add `--strict-tree-kill` flag for users who need guaranteed tree kill. For MVP, best-effort with observability is the pragmatic choice.

## References

- [GNU timeout source](https://github.com/coreutils/coreutils/blob/master/src/timeout.c)
- [Windows Job Objects](https://docs.microsoft.com/en-us/windows/win32/procthread/job-objects)
- [POSIX Process Groups](https://pubs.opengroup.org/onlinepubs/9699919799/functions/setpgid.html)
