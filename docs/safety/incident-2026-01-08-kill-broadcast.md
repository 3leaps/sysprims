# Incident: `kill(-1, …)` broadcast from an out-of-range PID

> **Status**: Resolved — safeguards landed in [ADR-0011](../decisions/ADR-0011-pid-validation-safety.md)
> **Class**: Process-control safety (signal broadcast)

This is the sterilized technical writeup of the failure that motivated
[ADR-0011: PID Validation Safety](../decisions/ADR-0011-pid-validation-safety.md).
It records the failure mode and the safeguard so the reasoning survives in-tree.

## Symptom

Running the `sysprims-signal` test suite on macOS terminated the desktop session
(the Finder process restarted). The signal library was sending a real signal to
far more processes than any test intended.

## Failure mode

A test sent a signal to `u32::MAX` as a target PID. PIDs cross the FFI boundary as
`u32`, but the platform signal call takes a signed `pid_t`. The cast wraps:

```text
u32::MAX (4_294_967_295)  ->  pid as i32  ->  -1
libc::kill(-1, SIGTERM)
```

Under POSIX, `kill(-1, sig)` is not "signal PID -1" — it is a **broadcast**:

> If `pid` is `-1`, `sig` shall be sent to all processes (excluding an unspecified
> set of system processes) for which the calling process has permission to send
> that signal.
> — POSIX `kill(2)`

So the call delivered `SIGTERM` to every process the user could signal, including
interactive desktop processes. A secondary variant of the same class — a signal
aimed at PID `1` (the init/`launchd` process) — is less destructive but similarly
illegitimate for a library test.

The general hazard: **any unvalidated PID above `i32::MAX` becomes a
negative `pid_t`**, and negative `pid_t` values carry POSIX broadcast /
process-group semantics rather than "one specific process." (`i32::MAX` itself
is the last in-range value and stays positive; the safeguard rejects PIDs
strictly greater than it.)

## Safeguard

[ADR-0011](../decisions/ADR-0011-pid-validation-safety.md) established the rule now
enforced across the crates:

- Public PID-taking entrypoints reject PID `0` and any PID above `i32::MAX` with an
  `InvalidArgument` error **before** the value can reach a `pid as pid_t` cast, so a
  negative/broadcast `pid_t` is never constructed.
- Tests never target PID `0`, PID `1`, or `u32::MAX`; they use a spawned child, the
  test's own PID, or a high-but-in-range placeholder such as `99999`.
- Dangerous, isolation-dependent tests run in a container harness rather than
  against the host.

See also [`docs/safety/signal-dispatch.md`](signal-dispatch.md) and
[`REPOSITORY_SAFETY_PROTOCOLS.md`](../../REPOSITORY_SAFETY_PROTOCOLS.md).
