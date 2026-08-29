# PTY Process Containment

PTY ownership and process-tree containment are separate responsibilities.
The PTY library owns terminal handles and byte streams; sysprims owns the
process-group or Job Object capability used for tree termination.

## Unix Composition

Guaranteed acquisition requires a prepared-spawn slot that installs the
sysprims session hook **instead of** the PTY library's own `setsid`. There is
exactly one session/group acquirer for the child. The PTY library retains its
terminal handles and may perform controlling-terminal setup after the hook.

The parent prepares the hook and acknowledgement channel before fork. The child
hook performs `setsid` and writes one fixed-size, nonblocking acknowledgement.
After spawn, the parent validates that acknowledgement and consumes the opaque
receipt with the owned child at an explicit unsafe ownership boundary:

```rust,ignore
use std::os::unix::process::CommandExt;
use std::process::Command;
use sysprims_session::prepare_session_acquisition;
use sysprims_timeout::{contain_acquired_session, TerminateTreeConfig};

let (hook, pending) = prepare_session_acquisition()?;
let mut command = Command::new("worker");

// A PTY prepared-spawn adapter installs this callback in place of its built-in
// setsid. sysprims never takes PTY file descriptors or terminal ownership.
unsafe {
    command.pre_exec(move || hook.acquire());
}

let child = command.spawn()?;
let receipt = pending.into_receipt(child.id())?;
// SAFETY: this is the same child that emitted the receipt, and its adapter
// retains exclusive unreaped ownership through guard finalization.
let mut containment = unsafe { contain_acquired_session(child, receipt)? };
```

The hook is async-signal-safe by contract: its child-side path performs no
allocation, locking, formatting, blocking, or panic. Missing, duplicate, or
malformed acknowledgement; a second session acquirer; and child/receipt
mismatch all fail closed without a reusable receipt.

The current `portable-pty` release does not expose this replacement slot.
Until a tagged companion does, keep its child handle and move it into a small
adapter for `adopt_contained`. The guard verifies the live PID, executable,
start time, process group, and session before it can signal the group.

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

Post-spawn adoption is reported as `unproven`, not `guaranteed`, because it occurs after
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
Receipt-bound Unix guards also verify with `waitid(WNOWAIT)` that the parent
still owns the unreaped child slot before every guaranteed group signal. An
adapter that externally reaps or changes its reported child PID fails closed.
The generic external-child constructor is therefore `unsafe`: the caller must
also guarantee that the adapter is the same spawn that emitted the receipt and
retains exclusive unreaped ownership. Violating that precondition can let PID
reuse redirect a signal; runtime checks remain defense-in-depth.
Dropping an active guard signals its containment and makes a bounded reap
attempt on both Unix and Windows.

Each `ContainmentOutcome` includes structured `completion` evidence collected
after the final cleanup action and before the leader is reaped or the owned Job
handle is released:

- `Empty` means a complete supported observation found no live members.
- `Survivors` includes a sorted, unique list of live member PIDs and its exact
  count.
- `Unknown` means visibility, capacity, allocation, stabilization, or platform
  support was insufficient for a trustworthy answer.

Completion evidence is a point-in-time receipt, not a permanent guarantee.
It is independent of leader `exited` state and `tree_kill_reliability`; an empty
sample does not upgrade `unproven` acquisition. Survivor PIDs are evidence only.
PID reuse makes them unsafe to pass to signaling APIs or use as a suggested
target list.

The guard and outcome also report `boundary_strength` independently:

- `kernel_enforced_job` means the exact Windows child was assigned before first
  execution to an owned Job with neither breakaway mode enabled.
- `cooperative_group` means a Unix session/process group whose descendants may
  later leave.
- `unknown` means the stronger boundary property was not proven.

Boundary strength never upgrades acquisition reliability, completion, or
leader-reap evidence.

Linux completion uses a bounded `/proc` process-group/session scan. macOS uses a
bounded libproc process-group scan with explicit sizing-race detection. Partial,
unstable, or permission-limited observations report `Unknown`, never `Empty`.
On macOS, signalling a process group whose only remaining member is an exited,
unreaped leader can report that no members are signalable; only a trustworthy
empty libproc observation allows the guard to proceed to reap in that case.

## Owned Standard-Library Spawn

For commands that do not need a PTY-owned spawn, pass a configured
`std::process::Command` to `spawn_contained`. On Unix, sysprims uses the same
`setsid` hook and sealed same-spawn receipt before reporting `guaranteed`. Here,
`guaranteed` means race-free acquisition and group-signaling eligibility. It
does not mean that a Unix descendant is unable to create or join a different
session or group later. On Windows, this API fails closed until a
create-suspended Job assignment path is available; callers may instead use
explicitly `unproven` post-spawn adoption.

## Windows Status

Windows adoption assigns an already-running process to an owned Job Object.
The guard retains both the process capability and Job handle, but reports
`unproven` because descendants can escape between process creation and Job
assignment. Dropping the owned Job handle applies `KILL_ON_JOB_CLOSE` to
processes that were successfully assigned.

A `guaranteed` Windows path requires create-suspended, assign-to-Job, then
resume. A ConPTY adapter can prepare a `PreparedWindowsJob` before creation,
call `assign_process` with the exact suspended process handle, and consume the
sealed receipt with `contain_acquired_windows_job`. Receipt consumption verifies
the same process and Job membership again, then transfers the Job and sole
wait/reap authority into the guard. The adapter retains its pseudoconsole and
presentation-pipe handles and resumes its separately owned primary thread
exactly once. Assignment, proof, adoption, or resume failure must resolve the
still-owned child without making it runnable.

The standard-library `spawn_contained(Command)` path remains unchanged and
fails closed on Windows because it does not own the primary-thread resume seam.
Post-spawn ConPTY adoption must not be presented as guaranteed. sysprims does
not emulate ConPTY or take ownership of terminal handles.

Owned Windows termination calls `TerminateJobObject` immediately. POSIX signal
and grace-period fields in `TerminateTreeConfig` do not apply; the outcome
therefore reports no graceful signal or escalation signal. Completion evidence
comes from a bounded Job process-ID-list query while the guard still owns the
Job handle. An incomplete or failed query reports `Unknown`.

## Legacy PID APIs

`terminate_tree(pid, ...)` remains a best-effort compatibility API. It does
not rediscover Job Objects or containment guards from a PID. Use the owned
guard whenever PID reuse safety and leader-exits-first group cleanup matter.

The PID-returning `spawn_in_group` API reports `best_effort` on Unix because it
cannot retain the owned child and sealed receipt, and it fails closed on Windows
because it cannot retain a Job capability. New Rust callers should use
`spawn_contained` instead.
