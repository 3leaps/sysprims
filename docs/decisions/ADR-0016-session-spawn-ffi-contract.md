# ADR-0016: Session Spawn FFI Contract

> **Status**: Accepted
> **Date**: 2026-07-05
> **Authors**: entarch

## Context

sysprims already provides Rust session-spawn primitives for starting commands
with `setsid` and `nohup` semantics. The C-ABI and language bindings need to
expose those primitives without creating a second, incompatible process-spawn
contract.

The existing FFI architecture uses JSON request strings, JSON response strings,
embedded `schema_id` values, caller-owned strings freed with
`sysprims_free_string()`, and typed `SysprimsErrorCode` failures. Session-spawn
FFI must preserve that pattern and must stay additive to the existing signal,
PID, timeout, and process-tree behavior.

The session-spawn surface also needs to distinguish two different POSIX facts:

- `setsid` creates a new session and process group for the child.
- `nohup` ignores `SIGHUP` but does not create a new session.

Conflating those facts would make binding consumers supervise detached children
with incorrect identifiers.

## Decision

### 1. C-ABI exports

Add two C-ABI exports:

```c
SysprimsErrorCode sysprims_run_setsid(const char *config_json, char **result_json_out);
SysprimsErrorCode sysprims_run_nohup(const char *config_json, char **result_json_out);
```

Both functions follow ADR-0004:

- `config_json` is a UTF-8 JSON string.
- `result_json_out` receives an owned UTF-8 JSON string on success.
- callers free successful result strings with `sysprims_free_string()`.
- failures return a typed `SysprimsErrorCode` and set thread-local error detail.
- no complex C structs are added to the ABI.

These exports are additive. They do not change existing signal dispatch, PID
validation, timeout behavior, process-tree termination, or ABI enum values.

### 2. Request schemas

The request schemas are:

- `https://schemas.3leaps.dev/sysprims/session/v1.0.0/run-setsid-config.schema.json`
- `https://schemas.3leaps.dev/sysprims/session/v1.0.0/run-nohup-config.schema.json`

Both request objects use `additionalProperties: false` and require:

| Field       | Type     | Meaning                                      |
| ----------- | -------- | -------------------------------------------- |
| `schema_id` | string   | Exact schema identifier for the request      |
| `argv`      | string[] | Command plus arguments; `argv[0]` is command |

`argv` is the only command representation. The FFI and bindings must not add a
shell string, command-line string, or convenience field that concatenates
arguments through a shell.

Both request schemas allow:

| Field | Type          | Meaning                                                             |
| ----- | ------------- | ------------------------------------------------------------------- |
| `cwd` | string or null | Working directory for the child; `null` or omitted means inherited |
| `env` | object or null | Environment overrides merged into the inherited environment        |
| `wait` | boolean      | `false` returns after spawn; `true` waits for child completion      |

`env` is caller-trusted input. When provided, keys and values are passed to a
process that may outlive the caller. `null` or omitted means the child inherits
the caller environment unchanged. The v1.0.0 contract does not define a
replace-or-clear environment mode; adding one requires an additive schema
revision.

The Rust session crate remains the behavioral source of truth. `cwd` and `env`
are part of the v1.0.0 contract, so the Rust session configuration
(`SetsidConfig` / `NohupConfig`) is extended to accept them, with tests, as part
of this workstream — the FFI and native binding layers must not duplicate
session-spawn logic to honor them. Every request field declared in this contract
is honored by the implementation: a request field that cannot be honored is
rejected with `SYSPRIMS_ERR_INVALID_ARGUMENT`, never silently ignored.

`run-nohup-config` also allows:

| Field         | Type          | Meaning                                                        |
| ------------- | ------------- | -------------------------------------------------------------- |
| `output_file` | string or null | Optional append target for nohup stdout/stderr redirection     |

`output_file` is caller-trusted input. Implementations open it with create and
append semantics **and `O_NOFOLLOW` on the final path component: sysprims does
not follow a symlink when opening the append target**, consistent with the
repository's policy of not following symlinks in security-sensitive file
operations. A symlinked `output_file` target is rejected with a typed error
(`SYSPRIMS_ERR_PERMISSION_DENIED`) rather than followed, so a planted symlink
cannot redirect a detached child's output to an attacker-chosen file. Binding
docs must still warn consumers not to forward untrusted input into `output_file`.

### 3. Response schema

Both exports return:

```text
https://schemas.3leaps.dev/sysprims/session/v1.0.0/session-spawn-result.schema.json
```

The common result shape is:

| Field                   | Type            | Meaning                                                |
| ----------------------- | --------------- | ------------------------------------------------------ |
| `schema_id`             | string          | Exact result schema identifier                         |
| `timestamp`             | string          | Result timestamp                                       |
| `platform`              | string          | Runtime platform                                       |
| `verb`                  | string          | `setsid` or `nohup`                                    |
| `status`                | string          | `spawned` or `completed`                               |
| `pid`                   | integer or null | Child PID when known                                   |
| `sid`                   | integer or null | Session identifier, with provenance below              |
| `pgid`                  | integer or null | Process-group identifier, with provenance below        |
| `session_kind`          | string          | `new_session` or `inherited_session`                   |
| `identifier_provenance` | string          | How `sid` and `pgid` were obtained                     |
| `exit_code`             | integer or null | Exit code for `wait: true` completions when available  |
| `signal`                | integer or null | Terminating signal for `wait: true` completions, Unix  |
| `output_file`           | string or null  | nohup output target if one was selected                |
| `warnings`              | string[]        | Non-fatal caveats                                      |

All non-null PID-like identifiers must be in `[1, i32::MAX]`. Values outside
that range are invalid for this FFI surface. When `status == "spawned"`, `pid`
must be present and non-null: a spawned result without a supervisable PID is a
spawn failure, not a success.

### 4. Identifier provenance

`sysprims_run_setsid` with `wait: false` must derive identifiers
structurally:

- `pid` is the child PID returned by spawn.
- `sid == pid`.
- `pgid == pid`.
- `session_kind == "new_session"`.
- `identifier_provenance == "setsid_structural_child_pid"`.

Implementations must not call `getsid(pid)` or `getpgid(pid)` on the child after
spawn. A detached child can exit before a post-spawn query, and the PID can be
reused by another process. Structural derivation avoids that race.

`sysprims_run_nohup` must be honest about nohup semantics:

- `nohup` does not create a new session.
- `pid` is the child PID returned by spawn.
- `sid` and `pgid` describe the caller session/process-group context inherited
  by the child at spawn time.
- `session_kind == "inherited_session"`.
- `identifier_provenance == "caller_context_before_spawn"`.

Because the nohup child stays in the caller's session and process group, the
returned `sid`/`pgid` name the *caller's own* group — unlike `setsid`, where
`pgid == pid` names a group containing only the child. A consumer must therefore
supervise a nohup child by its `pid`, and must never process-group-signal the
returned `pgid` (e.g. `kill(-pgid, …)`): that would signal the caller and its
siblings — a self-inflicted broadcast adjacent to the ADR-0011 class. Binding
docs must carry this caution.

Implementations may query the caller's own session and process group before
spawning. They must not describe nohup results as a new session.

For `wait: true`, the result uses `status == "completed"` and includes exit
status fields. Binding docs must state that `wait: true` blocks the calling
thread; Node.js and Bun consumers should run it off the main event loop.

### 5. Error mapping

The FFI exports must preserve typed failures. At minimum:

| Failure                         | FFI code                         |
| ------------------------------- | -------------------------------- |
| malformed JSON or invalid shape | `SYSPRIMS_ERR_INVALID_ARGUMENT`  |
| schema mismatch                 | `SYSPRIMS_ERR_INVALID_ARGUMENT`  |
| unsupported platform            | `SYSPRIMS_ERR_NOT_SUPPORTED`     |
| command not found               | `SYSPRIMS_ERR_NOT_FOUND`         |
| permission denied               | `SYSPRIMS_ERR_PERMISSION_DENIED` |
| spawn or child setup failure    | `SYSPRIMS_ERR_SPAWN_FAILED`      |
| unexpected internal failure     | `SYSPRIMS_ERR_INTERNAL`          |

A failed child setup, including a failed `setsid()` in child setup, is a spawn
failure. It must not produce a successful JSON result. Language bindings that
wrap FFI failures into exceptions or result objects must preserve the typed
code, not collapse detached-spawn failures into a generic failure.

### 6. Binding documentation

Binding docs for TypeScript, Go, and any future language binding must document:

- `argv` is an argument vector, not a shell command string.
- detached children can outlive the caller.
- default environment behavior inherits the caller environment.
- `env` values can carry secrets into parent-outliving processes.
- `output_file` is a caller-chosen append target for nohup redirection; sysprims
  opens it with `O_NOFOLLOW` and rejects a symlinked target (it does not follow
  symlinks to the append destination).
- `wait: true` blocks the calling thread.
- `setsid` returns structurally derived child session identifiers.
- `nohup` returns inherited caller session context, not a new session.
- for `nohup`, the returned `sid`/`pgid` are the caller's own group; supervise
  the child by `pid` and never process-group-signal the returned `pgid`.
- the detached child inherits the caller's entire environment, including
  secrets; v1.0.0 has no empty-env or replace mode, so scrub secrets from the
  caller environment before spawning if the child must not see them.

### 7. Room for a future detach helper

The session-spawn result shape is intentionally common across session-spawn
verbs. A future helper that prepares a `Command` to detach into a new session
must reuse the same vocabulary:

- `verb`
- `status`
- `pid`
- `sid`
- `pgid`
- `session_kind`
- `identifier_provenance`

That future helper must not overload `run_nohup` semantics and must not add a
parallel identifier vocabulary. If it needs new request fields or a new `verb`
value, it should use an additive schema revision under the `session` schema
topic.

## Consequences

### Positive

- The FFI surface follows the existing JSON-string and `schema_id` contract.
- Binding consumers can distinguish new-session and inherited-session results.
- `setsid` avoids post-spawn child identity queries and their PID-reuse race.
- `nohup` supervision data is explicit about inherited session context.
- Future detach work has a shared result vocabulary instead of a drifting
  second contract.

### Negative

- The result schema is more explicit than a minimal `pid` response.
- Binding documentation must carry security and availability cautions for
  `env`, `output_file`, and `wait: true`.

### Neutral

- This ADR does not add signal-sending behavior.
- This ADR does not change process-tree termination or timeout semantics.
- Windows remains `NotSupported` for POSIX session verbs unless a future ADR
  defines Windows-specific behavior.

## Alternatives Considered

### Alternative 1: Return only child PID

Rejected. Detached callers need enough context to supervise a parent-outliving
child, and a PID-only result would force each binding to invent its own session
lookup behavior.

### Alternative 2: Query child `sid` and `pgid` after spawn

Rejected. A detached child can exit before the query, and the PID can be reused.
For `setsid`, POSIX semantics already provide the correct identifiers:
`sid == pgid == pid`.

### Alternative 3: Use shell command strings for convenience

Rejected. Shell strings add quoting and injection ambiguity. The established
sysprims spawn contract uses `argv` arrays.

### Alternative 4: Separate unrelated result schemas per verb

Rejected. The verbs have different semantics, but they are part of one
session-spawn family. A common result vocabulary prevents drift and leaves room
for future detach helpers.

## References

- ADR-0004: FFI Design (`docs/decisions/ADR-0004-ffi-design.md`)
- ADR-0005: Schema Contracts (`docs/decisions/ADR-0005-schema-contracts.md`)
- ADR-0008: Error Handling (`docs/decisions/ADR-0008-error-handling.md`)
- ADR-0011: PID Validation Safety (`docs/decisions/ADR-0011-pid-validation-safety.md`)
- POSIX `setsid`: https://pubs.opengroup.org/onlinepubs/9699919799/functions/setsid.html
- POSIX `nohup`: https://pubs.opengroup.org/onlinepubs/9699919799/utilities/nohup.html
