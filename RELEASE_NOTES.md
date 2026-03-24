# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

---

## v0.1.15 - Draft

**Status:** Guard Automation & Provenance Release

This release turns the recurring VSCodium runaway-plugin incident into a first-class sysprims
workflow. The release adds a reusable one-shot guard primitive, subtree-aware remediation, a
managed guard loop for long-lived watchdogs, a new ancestors surface for provenance, and
operational guard controls for background execution and discovery. The result is a cleaner path
from "find the hot offender" to "kill the whole offending subtree when needed" to "explain it" to
"run a watchdog that can keep the host healthy."

### Highlights

- **One-shot guard primitive**: `GuardStep` gives Rust, FFI, Go, and TypeScript consumers a shared
  per-tick remediation kernel instead of forcing each ecosystem to reimplement detection and safety
  logic.
- **Cascade remediation**: `kill-descendants --cascade` and guard actions can expand each matched
  offender to its subtree so cleanup does not leave child work running behind the hot process.
- **Managed watchdog loop**: `GuardRunner` extracts long-running guard behavior into a reusable
  library surface and now powers the CLI.
- **Provenance support**: New `ancestors` APIs make it easier to answer "what spawned this?" when
  diagnosing runaway helpers and plugin children.
- **Background guard operations**: `sysprims guard` now supports `--daemon`, `--pidfile`,
  `--status`, and `--stop` for production-style watchdog management on Unix hosts.
- **Self-discovery**: Running guards are discoverable through sysprims itself instead of requiring
  `ps | grep` workflows.
- **Shared runtime primitives**: `Tick`, `now_rfc3339()`, and `GuardSignals` standardize timing,
  timestamps, and signal-aware shutdown for long-running loops.

### GuardStep and `sysprims guard`

The original dogfood need was straightforward but painful: reliably find actively hot descendants,
preview the impact, then kill the offenders without taking down the parent editor process. This
release turns that workflow into a reusable contract.

`GuardStep` is the new one-shot primitive behind that workflow. It evaluates a guard rule, can
expand a matched descendant to its subtree with cascade targeting, applies an optional action only
when explicitly enabled, and emits a structured event suitable for logs and metrics.

On top of that, `sysprims guard` now acts as a thin orchestrator rather than owning bespoke loop
logic. It benefits from the shared guard surface while keeping the CLI behavior operators expect.

### `GuardRunner`: Managed Guard Loop

For long-running watchdog use cases, `GuardRunner` in `sysprims-proc` now provides the managed
loop:

- drift-free scheduling using `Tick`
- clean shutdown via `GuardSignals`
- cloneable programmatic stop handles
- max-iteration stop support
- stop-reason summaries (`Signal`, `Requested`, `MaxIterations`)
- presets for `interactive`, `background`, and `watchdog` intervals/sample windows

This lets Rust consumers reuse the same loop semantics as the CLI instead of rebuilding timers,
stop conditions, and signal handling themselves.

### FFI and Go Watchdog Support

For bindings, the release adds a polling-style runner lifecycle rather than cross-language
callbacks:

```c
sysprims_proc_guard_runner_create(...)
sysprims_proc_guard_runner_tick(...)
sysprims_proc_guard_runner_stop(...)
sysprims_proc_guard_runner_free(...)
```

Go now layers typed support on top with `GuardPreset`, `GuardRunnerConfig`, `NewGuardRunner`,
`Tick()`, `Stop()`, and `Close()`. TypeScript gets the one-shot `guardStep()` surface in this
release; the managed runner remains Rust/FFI/Go for now. The recommended path for most non-Rust
consumers remains a native runtime loop around `GuardStep`, but the polling runner is available for
teams that want the managed Rust-side contract.

Devrev hardening for this surface included:

- synchronized runner state in Rust so concurrent polling and stop requests do not race
- serialized handle lifecycle in Go to avoid `Tick`/`Stop`/`Close` misuse
- create-time validation of static guard config so bad inputs fail before a long-running handle is
  returned

### Provenance: `ancestors`

This release also adds a new provenance surface across Rust, CLI, FFI, Go, and TypeScript:

```bash
sysprims ancestors <pid> --max-depth 10 --json
```

That fills the "what spawned this?" gap that shows up immediately once sysprims can identify a hot
plugin helper or runaway descendant. Operators can now go from hot process discovery to a parent
chain without leaving sysprims.

### Daemon Mode + Pidfile Management

For long-running hosts and edge agents, `sysprims guard` now supports background execution and
pidfile management on Unix:

```bash
sysprims guard 27776 --daemon --preset watchdog --yes
sysprims guard 27776 --status
sysprims guard 27776 --stop
```

Key behaviors:

- `--daemon` detaches with `setsid()` and redirects stdio to null
- `--pidfile <PATH>` overrides the default `/tmp/sysprims-guard-<root-pid>.pid`
- pidfiles are removed on clean shutdown via drop-based lifecycle cleanup
- stale or invalid pidfiles are cleaned up instead of being trusted blindly
- `--status` and `--stop` verify that the pidfile target is actually a live sysprims guard process
- daemon startup now waits for initialization to complete before reporting success

Windows intentionally remains out of scope for daemon mode in v0.1.15; the CLI returns a clear
not-supported error and directs operators to a service manager.

### Self-Discovery: Find Running Guards with sysprims

This release also closes the observability gap around long-running guards. A running guard now sets
a best-effort platform title where supported, and sysprims process inspection rewrites matching
guard cmdlines to `sysprims-guard:<root_pid>` for ergonomic lookup. On Linux, the kernel-visible
thread name remains truncated by `PR_SET_NAME`, so the full identity primarily comes from
cmdline-backed discovery inside sysprims itself.

That enables workflows like:

```bash
sysprims descendants 1 --name sysprims-guard --max-levels all
sysprims pstat --name sysprims-guard:27776 --table
```

The implementation was hardened to account for real CLI invocation shape, including global flags
appearing before the `guard` subcommand.

### Shared Runtime Primitives

Long-running loops also gained a shared runtime foundation in `sysprims-core`:

- `now_rfc3339()` for consistent timestamp rendering
- `Tick` for drift-resistant periodic scheduling
- `GuardSignals` for signal-aware shutdown with consistent stop semantics

These changes matter beyond `guard` itself because they give future long-running sysprims workflows
a shared timing and shutdown contract instead of ad hoc per-command logic.

### Release Hardening

Release preparation for v0.1.15 also tightened the delivery path:

- stronger release preflight guidance
- explicit TypeScript binding validation in the release path
- clean prepush validation restored before release cut
- clearer repository policy that prebuilt native binding artifacts come from CI, not local builds

### Upgrade Notes

- `sysprims guard` gains additive new flags only; existing foreground guard invocations continue to
  work.
- `kill-descendants --cascade` is additive; existing non-cascade behavior stays the default.
- Unix daemon mode is new; Windows remains intentionally unsupported in this release.
- FFI consumers must rebuild shared/static libraries to pick up the new guard runner exports.
- `ancestors` is additive across all supported surfaces.
- Go verification for the new runner surface currently uses a freshly built local `sysprims-ffi`
  artifact during development until release workflows refresh checked-in prebuilt libraries.

---

## v0.1.14 - 2026-02-24

**Status:** Process Intelligence & Go Team Depth Release

This release closes the gap between what sysprims knows about a process and what it exposes to
callers. The headline capability is `proc_ext` — environment variables and thread count surfaced
through the Rust library, FFI, and Go/TypeScript bindings — enabling Go teams to replace `ps` and
`lsof` shell-outs with typed, license-clean library calls. A secondary focus is CPU measurement
parity on process-tree commands, fixing a dogfooding gap where lifetime averaging missed 2 of 4
actively spinning zombie processes.

### Highlights

- **`proc_ext`**: `env` and `thread_count` on `ProcessInfo` — opt-in via `ProcessOptions`, zero
  cost when not requested. Available in Rust, FFI, Go, and TypeScript.
- **CPU mode on `descendants`/`kill-descendants`**: `--cpu-mode monitor --sample 3s` now applies
  to all process-tree commands, not just `pstat`. Catches bursty/spinning processes that lifetime
  averaging misses.
- **Schema compliance fix**: `pstat --pid --json` now emits the `schema_id` envelope required by
  ADR-0005. Previously returned a flat object — a contract violation, not just a style issue.
- **Contextual hints**: `--cpu-above` without `--cpu-mode monitor` emits a one-line stderr hint
  suggesting the more accurate measurement mode. Suppress with `SYSPRIMS_NO_HINTS=1` or `--json`.
- **CLI help system**: `sysprims help <topic>` subcommand (`cpu-mode`, `signals`, `safety`) plus
  `after_help` examples on high-complexity subcommands.
- **Release hardening**: Makefile quality gates now run goneat across non-Rust files, repository
  formatting was normalized for non-markdown assets, `rsfulmen` was updated to `0.1.4`, and stale
  `cargo-deny` source allowlists were removed to eliminate false-medium security findings.

### `proc_ext`: Environment Variables and Thread Count

Activates the `proc_ext` extension defined in ADR-0002 (`# Extended info (env, threads, IO)`),
designed into the architecture from the start and implemented in this release.

New optional fields on `ProcessInfo` (default `null`/`None`):

```rust
pub env: Option<BTreeMap<String, String>>,  // opt-in: ProcessOptions::with_env()
pub thread_count: Option<u32>,              // opt-in: ProcessOptions::with_threads()
```

**Go binding** — the primary consumer target:

```go
// Replace: ps eww -p <pid> + text parsing
// Replace: ps -M -p <pid> + line count
info, err := sysprims.ProcessGetWithOptions(pid, &sysprims.ProcessOptions{
    IncludeEnv:     true,
    IncludeThreads: true,
})
fmt.Println(info.Env["NODE_ENV"])
fmt.Println(info.ThreadCount)
```

**Platform coverage:**

| Platform |               `env`                |         `thread_count`         |
| -------- | :--------------------------------: | :----------------------------: |
| Linux    |       `/proc/[pid]/environ`        | `/proc/[pid]/status` (Threads) |
| macOS    | `sysctl(KERN_PROCARGS2)` env block | `proc_taskinfo.pti_threadnum`  |
| Windows  |   Not supported v0.1.14 (`null`)   |           Toolhelp32           |

macOS uses the same `KERN_PROCARGS2` kernel buffer introduced for cmdline in v0.1.13 — env is
the next block after argv. Same syscall, second pass over the same data.

**Security**: reads same-uid processes only. EPERM → `env: null`, no error propagation.

### CPU Mode Parity on Tree Commands

**The dogfooding gap** (2026-02-18): Four zombie VSCodium plugin processes were spinning at ~100%
CPU. Lifetime mode found 2 of 4. Monitor mode with 3-second sampling found all 4. The
`kill-descendants --cpu-above` workflow — the use case shown in the README — could not reliably
target the offending processes with lifetime CPU averaging.

```bash
# v0.1.13 — missed 2 of 4 spinning zombie processes
descendants 14796 --cpu-above 80
→ 2 matched

# v0.1.14 — finds all 4
descendants 14796 --cpu-mode monitor --sample 3s --cpu-above 80
→ 4 matched (all showing 100–101% over the sample window)
```

Full surgical cleanup workflow:

```bash
sysprims descendants 14796 --cpu-mode monitor --sample 3s --cpu-above 80 --tree
sysprims kill-descendants 14796 --cpu-mode monitor --sample 3s --cpu-above 80 --signal KILL --yes
```

### Schema Compliance Fix: `pstat --pid --json`

**Before (v0.1.13):**

```json
{"pid": 1234, "name": "nginx", "cpu_percent": 0.5, ...}
```

**After (v0.1.14):**

```json
{
  "schema_id": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/process-info.schema.json",
  "timestamp": "...",
  "processes": [{"pid": 1234, "name": "nginx", "cpu_percent": 0.5, ...}]
}
```

Root cause: the `--pid` code path short-circuited to direct `ProcessInfo` serialization instead of
routing through the `SnapshotResult` envelope used by the list path. CLI-only fix, no library
changes.

### Release Hardening (Post-Feature Complete)

- **Quality-gate parity**: `make fmt`, `make fmt-check`, and `make lint` now execute goneat for
  non-Rust file types while retaining strict Rust checks (`cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`).
- **Formatting normalization**: Non-markdown files (workflows, schemas, role config, TS config,
  and goneat config) were normalized in a single sweep to stabilize formatter/linter output.
- **Dependency refresh**: `rsfulmen` pin advanced from `0.1.2` to `0.1.4` and lockfile refreshed.
- **Security policy cleanup**: Removed stale `deny.toml` `allow-git` / `[sources.allow-org]`
  entries that were generating `unmatched-source` and `unmatched-organization` medium findings in
  `goneat assess --categories security`.

### Upgrade Notes

- **Mostly additive** — `proc_ext` fields (`env`, `thread_count`) are `null` by default; existing
  callers are unaffected unless they opt in via `ProcessOptions`.
- **Breaking for `pstat --pid --json` consumers**: output gains `schema_id` wrapper and moves to
  `processes: [...]` array. This fixes an ADR-0005 contract violation — the old flat output was
  non-conformant.
- `descendants` and `kill-descendants` CLI flags are strictly additive — no existing invocations
  break.
- Go binding option types are additive — no existing call sites break.

---

## v0.1.13 - 2026-02-13

**Status:** macOS Command-Line Fidelity Fix & Binding Coverage

This release fixes a high-severity bug where `processList()` returned truncated `cmdline` on macOS (just the process name instead of the full argument vector), breaking downstream consumers that filter by command-line arguments. It also exports v0.1.12 process tree capabilities to the FFI layer and Go/TypeScript bindings.

### Highlights

- **macOS cmdline fix**: `cmdline` now returns the full argument vector (e.g. `["bun", "run", "scripts/dev.ts", "--root", "/path"]`) instead of `["bun"]`
- **FFI coverage**: `descendants` and `kill-descendants` now available through C-ABI FFI
- **Go binding**: `Descendants()` and `KillDescendants()` with option pattern
- **TypeScript binding**: `descendants()` and `killDescendants()` via N-API

### Bug Fix: macOS `cmdline` Truncation

**Before (v0.1.12):**

```json
{ "pid": 12345, "name": "bun", "cmdline": ["bun"] }
```

**After (v0.1.13):**

```json
{
  "pid": 12345,
  "name": "bun",
  "cmdline": ["bun", "run", "scripts/dev.ts", "--root", "/some/path"]
}
```

**Root cause:** The macOS implementation used `proc_name()` as a placeholder for `cmdline`, which only returns the process name (16 chars max). The fix uses `sysctl(CTL_KERN, KERN_PROCARGS2)` — the same kernel API that `ps` uses — to read the actual argv.

**Impact:** Any consumer filtering by `cmdline` arguments on macOS was affected. Known affected: kitfly `discoverOrphans()` which filters by `p.cmdline.some(arg => arg.includes("scripts/dev.ts"))`.

**Safety hardening (devrev):**

- PID 0 and overflow-range PIDs rejected before sysctl call
- `argc` capped at 4096 to prevent pathological allocation from malformed kernel data
- Empty argv entries filtered (consistent with Linux `/proc/[pid]/cmdline` behavior)

### FFI & Binding Coverage (Wave 1)

v0.1.12 added `descendants` and `kill-descendants` to the CLI and Rust crates. This release makes them available to language binding consumers:

| Function            | FFI | Go  | TypeScript |
| ------------------- | :-: | :-: | :--------: |
| `descendants()`     | New | New |    New     |
| `killDescendants()` | New | New |    New     |

**FFI functions:**

```c
int32_t sysprims_proc_descendants(const char *config_json, char **result_json_out);
int32_t sysprims_proc_kill_descendants(const char *config_json, char **result_json_out);
```

Safety enforcement happens in the FFI layer — bindings get PID 1 protection, self-exclusion, and parent protection for free.

### Upgrade Notes

- **No breaking changes** — all changes are additive
- macOS consumers will immediately see full `cmdline` data where previously truncated
- Consumers filtering by `cmdline` may see more matches than before (this is correct behavior)
- FFI shared library must be rebuilt for all platform targets to include new exports
