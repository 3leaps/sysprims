# REPOSITORY SAFETY PROTOCOLS

> **MANDATORY READING FOR ALL AGENTS**
>
> This repository contains code that **sends signals** and **terminates processes**.
> Incorrect code or tests can crash your desktop, kill your terminal, or worse.
>
> **READ THIS ENTIRE DOCUMENT BEFORE WORKING ON SIGNAL-SENDING CODE.**

---

## What sysprims Does

sysprims is a **process control library**. It provides utilities to:

- **Kill processes** by PID (`sysprims-signal`)
- **Signal process groups** (all children of a process)
- **Timeout commands** and terminate entire process trees (`sysprims-timeout`)
- **Inspect processes** (read-only, safe - `sysprims-proc`)

**The danger is in signal-sending code**, not process inspection. `sysprims-proc` reads process info and cannot harm the system. But `sysprims-signal` and `sysprims-timeout` can terminate processes, and incorrect PIDs can terminate the wrong processes - including everything.

---

## Critical Safety Rules

### Rule 1: NEVER Use Dangerous PIDs in Tests

| PID Value            | What Happens                     | NEVER USE IN TESTS |
| -------------------- | -------------------------------- | ------------------ |
| `0`                  | Signals caller's process group   | **DANGEROUS**      |
| `1`                  | Signals init/launchd (system)    | **DANGEROUS**      |
| `-1` (or `u32::MAX`) | Signals ALL processes            | **CATASTROPHIC**   |
| `u32::MAX`           | Overflows to -1 when cast to i32 | **CATASTROPHIC**   |

**Incident Reference**: On 2026-01-08, a test using `u32::MAX` caused `kill(-1, SIGTERM)` which terminated Finder and hundreds of other processes. See [ADR-0011](docs/decisions/ADR-0011-pid-validation-safety.md).

### Rule 2: Safe PIDs for Testing

```rust
// SAFE: High PID unlikely to exist
kill(99999, SIGTERM);

// SAFE: Our own process (for error path testing)
let my_pid = std::process::id();
kill(my_pid, INVALID_SIGNAL);

// SAFE: Spawn a child process and kill it
let child = Command::new("sleep").arg("60").spawn()?;
kill(child.id(), SIGTERM);
```

### Rule 3: Understand Integer Overflow

sysprims uses `u32` for PIDs. POSIX uses `i32` (`pid_t`). Large `u32` values wrap to negative:

```
u32::MAX (4294967295) as i32 = -1    // BROADCASTS TO ALL PROCESSES
(i32::MAX + 1) as i32 = -2147483648  // Negative, special semantics
```

**The library validates this**, but tests that bypass the public API (calling `kill_impl` directly) must be careful.

### Rule 4: Never Test Signal Code on Host Without Review

Before running any test that sends signals:

1. **Read the test code** - what PID does it target?
2. **Trace the PID value** - could it overflow or be special?
3. **Consider consequences** - what if the PID exists and is important?

For dangerous tests, use the container harness:

```bash
make test-diabolical  # Runs in disposable container
```

---

## Mandatory Reading

Before working on signal, timeout, or process control code, read:

| Document                                                                            | Purpose                                         |
| ----------------------------------------------------------------------------------- | ----------------------------------------------- |
| [ADR-0011: PID Validation Safety](docs/decisions/ADR-0011-pid-validation-safety.md) | Why we validate PIDs, what values are dangerous |
| [docs/safety/signal-dispatch.md](docs/safety/signal-dispatch.md)                    | POSIX signal semantics, safe usage patterns     |
| [ADR-0003: Group-by-Default](docs/decisions/ADR-0003-group-by-default.md)           | Why sysprims kills process trees                |

---

## Pre-Flight Checklist for Signal-Sending Code

Before writing or modifying code in `sysprims-signal` or `sysprims-timeout`:

- [ ] I have read ADR-0011 (PID validation)
- [ ] I understand that `u32::MAX as i32 == -1`
- [ ] I understand that `kill(-1, sig)` broadcasts to all processes
- [ ] My tests use safe PIDs (99999, `std::process::id()`, spawned children)
- [ ] I am not sending signals to PID 0 or PID 1
- [ ] I am not bypassing the public API validation without good reason

---

## What To Do If You're Unsure

1. **Ask** - escalate to the maintainer before running dangerous tests
2. **Use containers** - `make test-diabolical` runs tests safely
3. **Review the ADR** - ADR-0011 explains the rationale in detail
4. **Check the incident log** - `.plans/tasks/cargo-crash/` has historical context

---

## Incident Log

| Date       | Summary                                                          | Resolution                     |
| ---------- | ---------------------------------------------------------------- | ------------------------------ |
| 2026-01-08 | Test with `u32::MAX` caused Finder crash via `kill(-1, SIGTERM)` | ADR-0011, PID validation added |

---

## Acknowledgment

By working on this repository, you acknowledge:

1. This library can terminate processes, including critical system processes
2. Incorrect tests can crash desktops, kill terminals, and disrupt systems
3. You have read and understood the safety protocols above
4. You will follow the pre-flight checklist for signal/process code

**When in doubt, ask. A question takes seconds. Recovering from `kill(-1, SIGTERM)` takes much longer.**
