# `sysprims-session`: GPL-free provenance and behavioral-comparison testing

This note records the provenance discipline behind `sysprims-session` and the
rationale for its behavioral-comparison tests. It is the authoritative reference
cited from the crate's source documentation.

## Provenance policy

`sysprims-session` reimplements session and process-group primitives (`setsid`,
`nohup`) that are commonly provided by GPL-licensed tools. To keep the crate
GPL-free (see [ADR-0001: License Policy](../decisions/ADR-0001-license-policy.md)):

- **No GPL source code is consulted** during implementation.
- **POSIX specifications are the primary reference.**
- **BSD/MIT/ISC-licensed** implementations may be consulted for understanding.
- **Behavioral comparison** against system tools (via shell-out at test time) is
  permitted — it observes behavior, not source.

### `setsid`

- Primary: POSIX.1-2017 `setsid(2)`; FreeBSD and Apple/Darwin `setsid(2)` man pages.
- Consulted for approach (permissive licenses only): BSD-2-Clause and MIT
  reimplementations.
- **Not consulted:** `util-linux` (GPL-2.0), GNU coreutils (GPL-3.0).
- Shape from the spec: if the caller is a process-group leader, fork first (a child
  cannot be a group leader); call `setsid()`; exec the target.

### `nohup`

- Primary: POSIX.1-2017 `nohup` utility; FreeBSD `nohup(1)` man page.
- Consulted for approach: ISC-licensed (BSD-like) reimplementation.
- **Not consulted:** GNU coreutils `nohup.c` (GPL-3.0).
- Shape from the spec: set `SIGHUP` to `SIG_IGN`; redirect a terminal stdout to
  `nohup.out` (or `$HOME/nohup.out`); redirect a terminal stderr to stdout; exec.

## Why behavioral-comparison testing is permitted

The behavioral-comparison tests validate our implementation against the system
tool for the *same input*, comparing **observable behavior** (e.g. that a new
session/process group was established), not source:

- We shell out to the system tool (which may be GPL-licensed) purely to observe
  its behavior.
- We assert our output matches that observed behavior.
- This is black-box cleanroom verification — **not** code copying. The tests
  themselves are MIT/Apache-2.0 licensed.

Observing a program's runtime behavior and matching it is a standard cleanroom
technique and does not create a derivative work of that program's source. This is
why the `behavioral_comparison` tests may invoke GPL system tools without
affecting the crate's GPL-free status.
