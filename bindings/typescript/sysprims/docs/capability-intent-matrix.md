# TypeScript Capability Intent Matrix

This matrix is the policy source for the public `@3leaps/sysprims` surface.
The TypeScript package binds Rust directly through N-API. The C ABI is a
sibling comparison surface, not the implementation layer for TypeScript.

The audited projection is:

`Rust capability intent -> N-API runtime export -> emitted declaration graph -> public TypeScript values, types, and documentation`

## TypeScript Projection Dispositions

- `exposed`: intended to be part of the public TypeScript package surface.
- `excluded`: intentionally absent from the public TypeScript package surface.
- `internal-only`: an `excluded` N-API compatibility or implementation symbol
  that may remain callable inside the native package but is not supported at the
  TypeScript package root.

These dispositions govern only TypeScript projection. They do not change the
visibility, stability, or compatibility policy of the sibling public C ABI;
an exported C symbol excluded here remains public C API.

Existing public values and types remain patch-compatible. Additive aliases,
types, and deprecation notices are allowed; removal, rename, semantic change,
or incompatible narrowing or widening is not.

## Baseline Summary

| Surface | Count | Policy |
| --- | ---: | --- |
| C-ABI runtime functions | 35 | Every function is classified below. |
| N-API runtime functions | 22 | Every function is classified below. |
| Public TypeScript functions | 22 | All remain public and patch-compatible. |
| Other public runtime values | 2 | `SysprimsError` and `SysprimsErrorCode` remain public. |
| Explicit package-root type exports | 38 | All remain public and patch-compatible. |

## Public Capability Matrix

| Logical capability | Rust anchor | C-ABI symbols | N-API symbols | Public TypeScript values and types | Disposition | Platform and lifecycle notes |
| --- | --- | --- | --- | --- | --- | --- |
| Process lookup | `sysprims_proc::get_process_with_options` | `sysprims_proc_get_ex` | `sysprimsProcGetEx` | `procGet`; `ProcessInfo`, `ProcessOptions`, `ProcessState` | exposed | Environment and thread details are opt-in and platform-dependent. JavaScript inputs must be validated before integer conversion; native PID validation remains defense in depth. |
| Process snapshot | `sysprims_proc::snapshot_filtered_with_options` | `sysprims_proc_list_ex` | `sysprimsProcListEx` | `processList`; `ProcessFilter`, `ProcessOptions`, `ProcessSnapshot`, `ProcessInfo`, `ProcessState` | exposed | Filter and extended-field support remains best-effort where the platform restricts inspection. |
| Listening ports | `sysprims_proc::listening_ports` | `sysprims_proc_listening_ports` | `sysprimsProcListeningPorts` | `listeningPorts`; `PortFilter`, `PortBinding`, `PortBindingsSnapshot`, `Protocol` | exposed | Attribution can be incomplete under platform permissions; warnings are part of the result contract. |
| Descendant traversal | `sysprims_proc::descendants_with_config_and_options` with default process-detail options | `sysprims_proc_descendants` | `sysprimsProcDescendants` | `descendants`; `CpuMode`, `DescendantsLevel`, `DescendantsOptions`, `DescendantsResult`, `ProcessFilter`, `ProcessInfo` | exposed | Monitor CPU mode blocks for its sampling interval; warnings report partial visibility. |
| Descendant process detail enrichment | `sysprims_proc::descendants_with_config_and_options` with non-default `ProcessOptions` | `sysprims_proc_descendants_ex` | additive extension of `sysprimsProcDescendants` required | `descendants`; `DescendantsOptions` additively extends existing `ProcessOptions`; `ProcessInfo` | exposed in Phase B | Detail collection is explicit opt-in, matching `procGet` and `processList`. Environment values may contain sensitive data, thread/environment visibility is permission-dependent, and tree-wide detail can increase result size and latency. |
| Descendant remediation | composition of `sysprims_proc::{descendants_with_config_and_options, select_descendant_targets}` and `sysprims_signal::kill_many` | `sysprims_proc_kill_descendants_ex` | `sysprimsProcKillDescendants` | `killDescendants`; `KillDescendantsFailure`, `KillDescendantsOptions`, `KillDescendantsResult`, `ProcessFilter` | exposed | PID 0, PID 1, values above `i32::MAX`, self, protected ancestors, and unowned groups are not valid smoke-test targets. |
| Stateless guard evaluation | `sysprims_proc::guard_step` | `sysprims_proc_guard_step` | `sysprimsProcGuardStep` | `guardStep`; `GuardAction`, `GuardConfig`, `GuardEvent`, `GuardRule` | exposed | Remediation remains disabled unless `action_enabled` is explicit. |
| Ancestor traversal | `sysprims_proc::ancestors` | `sysprims_proc_ancestors` | `sysprimsProcAncestors` | `ancestors`; `AncestorsOptions`, `AncestorsResult`, `ProcessInfo` | exposed | Read-only traversal; warnings represent partial visibility. |
| PID wait | `sysprims_proc::wait_pid` | `sysprims_proc_wait_pid` | `sysprimsProcWaitPid` | `waitPID`; `WaitPidResult` | exposed | Blocking polling on Unix and native process waiting on Windows. |
| File descriptor listing | `sysprims_proc::list_fds` | `sysprims_proc_list_fds` | `sysprimsProcListFds` | `listFds`; `FdFilter`, `FdSnapshot`; additive root exports `FdInfo`, `FdKind` required | exposed | Linux and macOS are supported; Windows returns `NotSupported`. `FdInfo` and `FdKind` are already transitive public declaration dependencies. |
| Current process group | `sysprims_session::getpgid` | `sysprims_self_getpgid` | `sysprimsSelfGetpgid` | `selfPGID` | exposed | Unix only; Windows returns `NotSupported`. |
| Current session | `sysprims_session::getsid` | `sysprims_self_getsid` | `sysprimsSelfGetsid` | `selfSID` | exposed | Unix only; Windows returns `NotSupported`. |
| New session spawn | `sysprims_session::run_setsid` | `sysprims_run_setsid` | `sysprimsRunSetsid` | `runSetsid`; `RunSetsidConfig`, `SessionIdentifierProvenance`, `SessionKind`, `SessionSpawnResult`, `SessionSpawnStatus`, `SessionSpawnVerb` | exposed | Unix only. Waiting is blocking; detached children can outlive the JavaScript process. |
| Nohup spawn | `sysprims_session::run_nohup` | `sysprims_run_nohup` | `sysprimsRunNohup` | `runNohup`; `RunNohupConfig`, `SessionIdentifierProvenance`, `SessionKind`, `SessionSpawnResult`, `SessionSpawnStatus`, `SessionSpawnVerb` | exposed | Unix only. The child inherits the caller group, which must not be treated as owned group-kill authority. |
| Signal one process | `sysprims_signal::kill` | `sysprims_signal_send` | `sysprimsSignalSend` | `signalSend` | exposed | Rust rejects unsafe PID values before platform conversion. Windows supports a limited signal set. |
| Signal one process group | `sysprims_signal::killpg` | `sysprims_signal_send_group` | `sysprimsSignalSendGroup` | `signalSendGroup` | exposed | Unix only. Public APIs never encode groups as negative PIDs. |
| Graceful direct termination | `sysprims_signal::terminate` | `sysprims_terminate` | `sysprimsTerminate` | `terminate` | exposed | SIGTERM on Unix; immediate `TerminateProcess` semantics on Windows. |
| Forced direct termination | `sysprims_signal::force_kill` | `sysprims_force_kill` | `sysprimsForceKill` | `forceKill` | exposed | SIGKILL on Unix and immediate termination on Windows. |
| JavaScript batch signal conveniences | TypeScript composition over native signal operations | none | none | `killMany`, `terminateMany`, `forceKillMany`; `BatchKillFailure`, `BatchKillResult` | exposed | Operations validate and report per-PID failures. They do not add native batch authority. |
| PID-based tree termination | `sysprims_timeout::terminate_tree` | `sysprims_terminate_tree` | `sysprimsTerminateTree` | `terminateTree`; `TerminateTreeConfig`, `TerminateTreeResult` | exposed | Legacy PID-based operation. Windows has no owned Job handle and is best-effort. |
| PID-returning grouped spawn | `sysprims_timeout::spawn_in_group` | `sysprims_spawn_in_group` | `sysprimsSpawnInGroup` | `spawnInGroup`; `SpawnInGroupConfig`, `SpawnInGroupResult` | exposed | Unix creates a process group. Windows fails before spawn because a PID cannot retain Job ownership. |
| Timeout execution | `sysprims_timeout::run_with_timeout` | `sysprims_timeout_run` | none | none | excluded | A public asynchronous contract needs separately approved native ownership, cancellation, concurrency limits, Node teardown, and guaranteed child cleanup. A blocking N-API projection would stall the JavaScript event loop. |
| Error values | `sysprims_core::SysprimsError` | C error codes | N-API call result envelopes | `SysprimsError`, `SysprimsErrorCode` | exposed | Public wrappers convert native result envelopes into exceptions. C thread-local error plumbing is not exposed. |

The public type block also includes `JsonObject` only as an internal helper. It
is not referenced by the emitted package-root declarations and remains excluded.

## N-API Runtime Export Classification

NAPI-RS converts the Rust snake-case function names below to the camel-case
runtime names shown here.

| N-API runtime export | Rust anchor in `native/src/lib.rs` | Public mapping | Disposition | Rationale when excluded |
| --- | --- | --- | --- | --- |
| `sysprimsAbiVersion` | `sysprims_abi_version` | none | excluded, internal-only | Native diagnostic/compatibility metadata; the direct N-API package does not consume the C ABI. |
| `sysprimsProcGet` | `sysprims_proc_get` | none; `procGet` uses `sysprimsProcGetEx` | excluded, internal-only | Compatibility alias would duplicate the options-capable public operation. |
| `sysprimsProcGetEx` | `sysprims_proc_get_ex` | `procGet` | exposed |  |
| `sysprimsProcList` | `sysprims_proc_list` | none; `processList` uses `sysprimsProcListEx` | excluded, internal-only | Compatibility alias would duplicate the options-capable public operation. |
| `sysprimsProcListEx` | `sysprims_proc_list_ex` | `processList` | exposed |  |
| `sysprimsProcListeningPorts` | `sysprims_proc_listening_ports` | `listeningPorts` | exposed |  |
| `sysprimsProcListFds` | `sysprims_proc_list_fds` | `listFds` | exposed |  |
| `sysprimsProcWaitPid` | `sysprims_proc_wait_pid` | `waitPID` | exposed |  |
| `sysprimsProcDescendants` | `sysprims_proc_descendants` | `descendants` | exposed; Phase B extension required | Baseline accepts traversal/filter/CPU configuration with default process-detail options. Phase B parses additive `includeEnv`/`includeThreads` fields and passes the resulting `ProcessOptions` to Rust. |
| `sysprimsProcKillDescendants` | `sysprims_proc_kill_descendants` | `killDescendants` | exposed | The N-API function already accepts the public extended configuration. |
| `sysprimsProcGuardStep` | `sysprims_proc_guard_step` | `guardStep` | exposed |  |
| `sysprimsProcAncestors` | `sysprims_proc_ancestors` | `ancestors` | exposed |  |
| `sysprimsSelfGetpgid` | `sysprims_self_getpgid` | `selfPGID` | exposed |  |
| `sysprimsSelfGetsid` | `sysprims_self_getsid` | `selfSID` | exposed |  |
| `sysprimsSignalSend` | `sysprims_signal_send` | `signalSend` | exposed |  |
| `sysprimsSignalSendGroup` | `sysprims_signal_send_group` | `signalSendGroup` | exposed |  |
| `sysprimsTerminate` | `sysprims_terminate` | `terminate` | exposed |  |
| `sysprimsForceKill` | `sysprims_force_kill` | `forceKill` | exposed |  |
| `sysprimsTerminateTree` | `sysprims_terminate_tree` | `terminateTree` | exposed |  |
| `sysprimsRunSetsid` | `sysprims_run_setsid` | `runSetsid` | exposed |  |
| `sysprimsRunNohup` | `sysprims_run_nohup` | `runNohup` | exposed |  |
| `sysprimsSpawnInGroup` | `sysprims_spawn_in_group` | `spawnInGroup` | exposed |  |

The N-API result objects `SysprimsCallJsonResult`, `SysprimsCallU32Result`, and
`SysprimsCallVoidResult` are internal transport shapes, not supported public
constructors. The package currently has no `exports` map and ships `dist/`, so
consumers can technically deep-import `dist/ffi` and reach native aliases marked
internal-only above. Those paths are unsupported implementation details. Adding
an `exports` map would break existing deep imports and is outside this
patch-compatible phase; drift checks therefore enforce the package-root surface
rather than claiming physical encapsulation.

## C-ABI Runtime Export Classification

The disposition column below classifies projection into TypeScript, not whether
the C symbol is public. Every symbol in this table remains part of the public C
ABI and retains its existing compatibility contract.

| C-ABI symbol | Logical mapping | TypeScript disposition | Rationale when excluded |
| --- | --- | --- | --- |
| `sysprims_version` | Package/native version metadata | excluded from TypeScript; C-public metadata | Package metadata is the public JavaScript version source. |
| `sysprims_abi_version` | C ABI metadata; N-API has an internal diagnostic equivalent | excluded from TypeScript; C-public metadata | The TypeScript package binds Rust directly and does not consume the C ABI. |
| `sysprims_get_platform` | Platform metadata | excluded | `process.platform` is the public JavaScript platform source. |
| `sysprims_free_string` | C allocator plumbing | excluded, C-only | N-API owns JavaScript string conversion and garbage collection. |
| `sysprims_last_error_code` | C thread-local error plumbing | excluded, C-only | N-API uses structured call result envelopes. |
| `sysprims_last_error` | C thread-local error plumbing | excluded, C-only | Thread-local state and manual string ownership are unsuitable public JavaScript contracts. |
| `sysprims_clear_error` | C thread-local error plumbing | excluded, C-only | N-API uses structured call result envelopes. |
| `sysprims_proc_list` | Default process snapshot alias | excluded from TypeScript; C-public compatibility alias | Public `processList` maps to the options-capable `_ex` operation. |
| `sysprims_proc_list_ex` | Process snapshot | exposed as `processList` |  |
| `sysprims_proc_get` | Default process lookup alias | excluded from TypeScript; C-public compatibility alias | Public `procGet` maps to the options-capable `_ex` operation. |
| `sysprims_proc_get_ex` | Process lookup | exposed as `procGet` |  |
| `sysprims_proc_ancestors` | Ancestor traversal | exposed as `ancestors` |  |
| `sysprims_proc_descendants` | Descendant traversal with default process-detail options | exposed as `descendants` | This is the direct C comparison for the N-API projection. |
| `sysprims_proc_descendants_ex` | Descendant environment/thread detail enrichment | exposed in Phase B as additive `descendants` options | Existing Rust and C contracts support the same opt-in `ProcessOptions` already public for process lookup and snapshots. |
| `sysprims_proc_wait_pid` | PID wait | exposed as `waitPID` |  |
| `sysprims_proc_list_fds` | File descriptor listing | exposed as `listFds` |  |
| `sysprims_proc_listening_ports` | Listening-port attribution | exposed as `listeningPorts` |  |
| `sysprims_proc_kill_descendants` | Default descendant remediation alias | excluded from TypeScript; C-public compatibility alias | Public `killDescendants` maps to the extended intent through N-API. |
| `sysprims_proc_kill_descendants_ex` | Descendant remediation | exposed as `killDescendants` |  |
| `sysprims_proc_guard_step` | Stateless guard evaluation | exposed as `guardStep` |  |
| `sysprims_proc_guard_runner_create` | Stateful guard-runner raw handle creation | excluded | Raw pointer ownership, event-loop scheduling, disposal, and post-close behavior need a dedicated native-object design. |
| `sysprims_proc_guard_runner_tick` | Stateful guard-runner raw handle polling | excluded | Raw pointer handles must not cross the public JavaScript boundary. |
| `sysprims_proc_guard_runner_stop` | Stateful guard-runner raw handle stop | excluded | Raw pointer handles must not cross the public JavaScript boundary. |
| `sysprims_proc_guard_runner_free` | Stateful guard-runner raw handle disposal | excluded | Raw pointer handles must not cross the public JavaScript boundary. |
| `sysprims_signal_send` | Signal one process | exposed as `signalSend` |  |
| `sysprims_signal_send_group` | Signal one process group | exposed as `signalSendGroup` |  |
| `sysprims_terminate` | Graceful direct termination | exposed as `terminate` |  |
| `sysprims_force_kill` | Forced direct termination | exposed as `forceKill` |  |
| `sysprims_self_getpgid` | Current process group | exposed as `selfPGID` |  |
| `sysprims_self_getsid` | Current session | exposed as `selfSID` |  |
| `sysprims_run_setsid` | New session spawn | exposed as `runSetsid` |  |
| `sysprims_run_nohup` | Nohup spawn | exposed as `runNohup` |  |
| `sysprims_timeout_run` | Timeout execution | excluded from TypeScript | No N-API projection exists. Safe asynchronous ownership, cancellation, teardown, and cleanup semantics require separate design approval. |
| `sysprims_terminate_tree` | PID-based tree termination | exposed as `terminateTree` |  |
| `sysprims_spawn_in_group` | PID-returning grouped spawn | exposed as `spawnInGroup` |  |

## Pure-Rust Intent and Explicit Exclusions

| Rust capability | C ABI | N-API | Public TypeScript | Disposition | Rationale |
| --- | --- | --- | --- | --- | --- |
| `sysprims_timeout::spawn_contained` | none | none | none | excluded | Windows guaranteed spawn fails closed until create-suspended Job assignment exists. Returning only a PID would discard ownership. |
| `sysprims_timeout::adopt_contained` | none | none | none | excluded | Post-spawn adoption is `unproven`; JavaScript cannot transfer a generic owned child and receive it back on acquisition failure. `adoptContained(pid)` is forbidden. |
| `sysprims_timeout::ContainmentGuard` | none | none | none | excluded | The guard owns the child and process-group or Job capability, has one-shot finalization, retains ownership on failure, and kills on active drop. A PID must never reconstruct or borrow a guard. |
| `sysprims_timeout::ContainmentOutcome` | none | none | none | excluded | Outcome data is meaningful only with a native-owned guard lifecycle that is intentionally outside this surface. |
| `sysprims_timeout::ContainmentIdentity` | none | none | none | excluded | PID, start time, and executable are immutable native evidence, not caller-constructible termination authority. |
| `sysprims_timeout::ContainmentChild` | none | none | none | excluded | Rust adapter trait includes platform-specific owned child and raw-handle requirements. |
| `sysprims_timeout::{run_with_timeout, run_with_timeout_default}` | C exposes only the options-capable operation | none | none | excluded | The default function is a Rust convenience alias. Both remain outside TypeScript until a safe asynchronous lifecycle contract is approved. |
| `sysprims_proc::GuardRunner` managed loop | C polling handle family | none | none | excluded | Native blocking scheduling and signal integration do not define a safe JavaScript event-loop and disposal contract. Public `guardStep` remains available. |
| `sysprims_signal::{kill_many, terminate_many, force_kill_many}` | none | none | `killMany`, `terminateMany`, `forceKillMany` | exposed through JavaScript composition | Existing public conveniences preserve per-PID error reporting without exposing a new native authority. |
| `sysprims_signal::kill_by_name` | none | none | none | excluded | Number-based `signalSend` is the canonical binding operation; exposing catalog name resolution would add a second signal contract. |
| `sysprims_signal::match_signal_names` | none | none | none | excluded | Signal catalog discovery is not part of the current binding contract and has no cross-layer schema. |
| `sysprims_signal` re-exported `rsfulmen::foundry::signals::*` catalog helpers and metadata | none | none | none | excluded | Catalog lookup/resolution, listing, exit-code conversion, behavior, and platform-support helpers have no C/N-API projection or TypeScript schema. Numeric signal dispatch remains the binding contract. |
| `sysprims_signal::{terminate_group, force_kill_group}` | none | none | none | excluded, convenience aliases | `signalSendGroup` already projects the underlying group operation without adding duplicate public values. |
| `sysprims_proc::{snapshot, snapshot_with_options, snapshot_filtered}` | default C aliases or none | default N-API aliases or none | `processList` | exposed through the options-capable operation | These Rust convenience variants collapse into the single public filtered/options-capable operation. |
| `sysprims_proc::{get_process, descendants, descendants_with_options, descendants_with_config}` | default C aliases or none | default N-API aliases or none | `procGet`, `descendants` | exposed through options-capable operations | These Rust convenience variants do not require duplicate JavaScript values. Phase B adds opt-in descendant detail fields to the existing `descendants` operation rather than another public function. |
| `sysprims_proc::{cpu_total_time_ns, select_descendant_targets}` | none | none | none | excluded, implementation support | These operations support CPU enrichment and remediation selection internally; neither is a standalone cross-layer capability contract. |
| `sysprims_proc::process_by_port` | none | none | none | excluded | This convenience lookup has no C/N-API projection or distinct result schema; consumers can compose `listeningPorts` with `procGet`. |
| `sysprims_proc::{is_live, is_fully_gone}` | none | none | none | excluded | Internal lifecycle predicates answer PID-slot state, not process identity; exposing them invites unsafe authorization decisions. |
| `sysprims_session::{setsid, setpgid}` | none | none | none | excluded | These mutate the Node host process/session or arbitrary process groups and are unsuitable JavaScript authority. Spawn operations retain the safe owned-child boundary. |
| `sysprims_session::{getsid, getpgid}` for arbitrary PIDs | C/N-API expose current-process queries only | current-process queries only | `selfSID`, `selfPGID` | excluded beyond current process | Rust accepts PID 0 for self, but public JavaScript keeps explicit current-process queries and does not inherit low-level PID-0 semantics. |

## Public Type Export Inventory

The emitted package root must continue to export these 38 explicit type names:

`AncestorsOptions`, `AncestorsResult`, `BatchKillFailure`, `BatchKillResult`,
`CpuMode`, `DescendantsLevel`, `DescendantsOptions`, `DescendantsResult`,
`FdFilter`, `FdSnapshot`, `GuardAction`, `GuardConfig`, `GuardEvent`, `GuardRule`,
`KillDescendantsFailure`, `KillDescendantsOptions`, `KillDescendantsResult`,
`PortBinding`, `PortBindingsSnapshot`, `PortFilter`, `ProcessFilter`,
`ProcessInfo`, `ProcessOptions`, `ProcessSnapshot`, `ProcessState`, `Protocol`,
`RunNohupConfig`, `RunSetsidConfig`, `SessionIdentifierProvenance`,
`SessionKind`, `SessionSpawnResult`, `SessionSpawnStatus`, `SessionSpawnVerb`,
`SpawnInGroupConfig`, `SpawnInGroupResult`, `TerminateTreeConfig`,
`TerminateTreeResult`, and `WaitPidResult`.

`SysprimsError` and `SysprimsErrorCode` also remain available in both the value
and type namespaces. `FdInfo` and `FdKind` are proposed additive package-root
type exports because existing public `FdSnapshot` and `FdFilter` declarations
already reference them.

## Projection and Drift Policy

The implementation phase must enforce this matrix without changing policy:

1. Extract public values and type-only exports from emitted package declarations
   and their export graph, not from regular expressions over source files.
2. Inventory runtime N-API exports and C-ABI functions mechanically. Every new
   callable symbol requires a matrix row before projection work proceeds.
3. Verify every `exposed` row maps to emitted public values, types, and generated
   documentation. Verify every `excluded` row remains absent from the public
   package root.
4. Provide deterministic generate and check commands. Check mode generates to a
   temporary location or verifies a clean diff; CI never regenerates and accepts
   changed output in the same step.
5. Preserve Node `>=18`. Behavior smoke runs on native Linux x64, macOS arm64,
   and Windows x64 under Node 18, Node 22, and pinned Bun 1.3.x. Cross-compiled
   artifacts receive build and package checks only.
6. Process-control smoke owns every target it acts on. PID 0, PID 1,
   `u32::MAX`, unowned processes, and unowned groups are forbidden. Tests retain
   spawned child handles, validate ownership before action, and prove cleanup.
7. Every public PID and PGID input is validated as a finite JavaScript integer
   from 1 through `i32::MAX` before any `>>> 0`, native conversion, or native
   call. In particular, values such as `4294967297` must be rejected rather than
   aliased to PID 1. Other numeric inputs are validated against their documented
   ranges before `>>> 0`, `| 0`, or native conversion. The documented
   `maxLevels: Infinity` sentinel is preserved through an explicit conversion to
   `u32::MAX`; other non-finite values are rejected. Native validation remains
   mandatory defense in depth. Batch results preserve the validated input value
   and never report a lossy coercion.
8. New tooling adds no runtime dependency or dependency-major migration and must
   pass license, advisory, ABI/schema, packaging, and prebuild gates.
