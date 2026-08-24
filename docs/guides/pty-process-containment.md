# PTY Process Containment

PTY ownership and process-tree containment are separate responsibilities.
The PTY library owns terminal handles and byte streams; sysprims owns the
process-group or Job Object capability used for tree termination.

## Unix Composition

`portable-pty` creates its child as a session and process-group leader on Unix.
Keep that child handle and move it into a small adapter for
`adopt_contained`. The guard verifies the live PID, executable, start time,
process group, and session before it can signal the group.

```rust,ignore
use sysprims_timeout::{
    adopt_contained, ContainmentChild, TerminateTreeConfig,
};

struct PtyChild(Box<dyn portable_pty::Child + Send + Sync>);

impl ContainmentChild for PtyChild {
    fn process_id(&self) -> Option<u32> {
        self.0.process_id()
    }

    fn try_wait(&mut self) -> std::io::Result<bool> {
        self.0.try_wait().map(|status| status.is_some())
    }
}

let pty_child = pair.slave.spawn_command(command)?;
let mut containment = adopt_contained(PtyChild(pty_child))?;

// Retain the PTY master for I/O. Do not spawn the command a second time.
loop {
    if let Some(outcome) = containment.try_complete(TerminateTreeConfig::default())? {
        break outcome;
    }
    std::thread::sleep(std::time::Duration::from_millis(10));
}
```

Adoption is reported as `unproven`, not `guaranteed`, because it occurs after
the child starts. Once acquired, the guard owns the unreaped child and bound
group identity. Termination verifies that evidence before the first signal,
then escalates the bound group even if the leader exits before the grace period
ends. If identity or group evidence mismatches, termination fails closed rather
than reconstructing a group from a raw PID.

Acquisition fails if the child is not a process-group leader or shares the
caller's group. On failure, `ContainmentAdoptionError` returns child ownership
so the caller can still reap or terminate it safely.

Do not call the adapter's reaping `try_wait` while the guard is active.
`try_complete` observes leader exit without reaping, cleans any remaining
descendants, and only then reaps. After successful completion or termination,
`into_child` returns the adapter so it can inspect any retained exit status.
Dropping an active guard kills the contained tree and makes a bounded reap
attempt on both Unix and Windows.

On macOS, signalling a process group whose only remaining member is an exited,
unreaped leader can report that no members are signalable. The guard records
that condition and uses libproc group enumeration to confirm there are no live
descendants before reaping the retained leader. Permission failures while the
leader is live, or while a live descendant remains, still fail closed.

## Owned Standard-Library Spawn

For commands that do not need a PTY-owned spawn, pass a configured
`std::process::Command` to `spawn_contained`. On Unix, sysprims establishes the
group before exec and reports `guaranteed`. On Windows, this API fails closed
until a create-suspended Job assignment path is available; callers may instead
use explicitly `unproven` post-spawn adoption.

## Windows Status

Windows adoption assigns an already-running process to an owned Job Object.
The guard retains both the process capability and Job handle, but reports
`unproven` because descendants can escape between process creation and Job
assignment. Dropping the owned Job handle applies `KILL_ON_JOB_CLOSE` to
processes that were successfully assigned.

A `guaranteed` Windows path requires create-suspended, assign-to-Job, then
resume. `portable-pty` 0.9 does not expose that ConPTY preparation hook, so
ConPTY adoption must not be presented as guaranteed. sysprims does not emulate
ConPTY or take ownership of terminal handles.

Owned Windows termination calls `TerminateJobObject` immediately. POSIX signal
and grace-period fields in `TerminateTreeConfig` do not apply; the outcome
therefore reports no graceful signal or escalation signal.

## Legacy PID APIs

`terminate_tree(pid, ...)` remains a best-effort compatibility API. It does
not rediscover Job Objects or containment guards from a PID. Use the owned
guard whenever PID reuse safety and leader-exits-first group cleanup matter.

The PID-returning `spawn_in_group` API cannot retain a Windows Job capability,
so it fails closed on Windows. New Rust callers should use `spawn_contained`
instead.
