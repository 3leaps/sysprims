# TypeScript Capability Intent Matrix

This matrix is the policy source for the public `@3leaps/sysprims` surface.
The TypeScript package binds Rust directly through N-API. The former C ABI
comparison surface was retired in v0.5.0.

The audited projection is:

`Rust capability intent -> N-API runtime export -> emitted declaration graph -> public TypeScript values, types, and documentation`

## TypeScript Projection Dispositions

- `exposed`: intended to be part of the public TypeScript package surface.
- `excluded`: intentionally absent from the public TypeScript package surface.
- `internal-only`: an `excluded` N-API compatibility or implementation symbol
  that may remain callable inside the native package but is not supported at the
  TypeScript package root.

These dispositions govern only TypeScript projection.

Existing public values and types remain patch-compatible. Additive aliases,
types, and deprecation notices are allowed; removal, rename, semantic change,
or incompatible narrowing or widening is not.

## Baseline Summary

| Surface | Count | Policy |
| --- | ---: | --- |
| N-API runtime functions | 36 | Every function is classified below. |
| Public TypeScript functions | 33 | All remain public and patch-compatible. |
| Other public runtime values | 14 | `SysprimsError`, `SysprimsErrorCode`, `ContainedProcess`, `ProcessHandle`, `MAX_SAFE_PID`, `SIGHUP`, `SIGINT`, `SIGQUIT`, `SIGKILL`, `SIGPIPE`, `SIGALRM`, `SIGTERM`, `SIGUSR1`, and `SIGUSR2` remain public. |
| Explicit package-root type exports | 45 | All remain public and patch-compatible. |

## Public Capability Matrix

| Logical capability | Rust anchor | N-API symbols | Public TypeScript values and types | Disposition | Platform and lifecycle notes |
| --- | --- | --- | --- | --- | --- |
| Process lookup | `sysprims_proc::get_process_with_options` | `sysprimsProcGetEx` | `procGet`; `ProcessInfo`, `ProcessOptions`, `ProcessState` | exposed | Environment and thread details are opt-in and platform-dependent. JavaScript inputs must be validated before integer conversion; native PID validation remains defense in depth. |
| Process snapshot | `sysprims_proc::snapshot_filtered_with_options` | `sysprimsProcListEx` | `processList`; `ProcessFilter`, `ProcessOptions`, `ProcessSnapshot`, `ProcessInfo`, `ProcessState` | exposed | Filter and extended-field support remains best-effort where the platform restricts inspection. |
| Listening ports | `sysprims_proc::listening_ports` | `sysprimsProcListeningPorts` | `listeningPorts`; `PortFilter`, `PortBinding`, `PortBindingsSnapshot`, `Protocol` | exposed | Attribution can be incomplete under platform permissions; warnings are part of the result contract. |
| Descendant traversal | `sysprims_proc::descendants_with_config_and_options` with default process-detail options | `sysprimsProcDescendants` | `descendants`; `CpuMode`, `DescendantsLevel`, `DescendantsOptions`, `DescendantsResult`, `ProcessFilter`, `ProcessInfo` | exposed | Monitor CPU mode blocks for its sampling interval; warnings report partial visibility. |
| Descendant process detail enrichment | `sysprims_proc::descendants_with_config_and_options` with non-default `ProcessOptions` | additive extension of `sysprimsProcDescendants` required | `descendants`; `DescendantsOptions` additively extends existing `ProcessOptions`; `ProcessInfo` | exposed in Phase B | Detail collection is explicit opt-in, matching `procGet` and `processList`. Environment values may contain sensitive data, thread/environment visibility is permission-dependent, and tree-wide detail can increase result size and latency. |
| Descendant remediation | composition of `sysprims_proc::{descendants_with_config_and_options, select_descendant_targets}` and `sysprims_signal::kill_many` | `sysprimsProcKillDescendants` | `killDescendants`; `KillDescendantsFailure`, `KillDescendantsOptions`, `KillDescendantsResult`, `ProcessFilter` | exposed | PID 0, PID 1, values above `i32::MAX`, self, protected ancestors, and unowned groups are not valid smoke-test targets. |
| Stateless guard evaluation | `sysprims_proc::guard_step` | `sysprimsProcGuardStep` | `guardStep`; `GuardAction`, `GuardConfig`, `GuardEvent`, `GuardRule` | exposed | Remediation remains disabled unless `action_enabled` is explicit. |
| Ancestor traversal | `sysprims_proc::ancestors` | `sysprimsProcAncestors` | `ancestors`; `AncestorsOptions`, `AncestorsResult`, `ProcessInfo` | exposed | Read-only traversal; warnings represent partial visibility. |
| PID wait | `sysprims_proc::wait_pid` | `sysprimsProcWaitPid` | `waitPID`; `WaitPidResult` | exposed | Blocking polling on Unix and native process waiting on Windows. |
| File descriptor listing | `sysprims_proc::list_fds` | `sysprimsProcListFds` | `listFds`; `FdFilter`, `FdSnapshot`; additive root exports `FdInfo`, `FdKind` required | exposed | Linux and macOS are supported; Windows returns `NotSupported`. `FdInfo` and `FdKind` are already transitive public declaration dependencies. |
| Current process group | `sysprims_session::getpgid` | `sysprimsSelfGetpgid` | `selfPGID` | exposed | Unix only; Windows returns `NotSupported`. |
| Current session | `sysprims_session::getsid` | `sysprimsSelfGetsid` | `selfSID` | exposed | Unix only; Windows returns `NotSupported`. |
| New session spawn | `sysprims_session::run_setsid` | `sysprimsRunSetsid` | `runSetsid`; `RunSetsidConfig`, `SessionIdentifierProvenance`, `SessionKind`, `SessionSpawnResult`, `SessionSpawnStatus`, `SessionSpawnVerb` | exposed | Unix only. Waiting is blocking; detached children can outlive the JavaScript process. |
| Nohup spawn | `sysprims_session::run_nohup` | `sysprimsRunNohup` | `runNohup`; `RunNohupConfig`, `SessionIdentifierProvenance`, `SessionKind`, `SessionSpawnResult`, `SessionSpawnStatus`, `SessionSpawnVerb` | exposed | Unix only. The child inherits the caller group, which must not be treated as owned group-kill authority. |
| Signal one process | `sysprims_signal::kill`, `sysprims_signal::validate_pid` | `sysprimsSignalSend` | `signalSend`, `validatePid`; `MAX_SAFE_PID`, `SIGHUP`, `SIGINT`, `SIGQUIT`, `SIGKILL`, `SIGPIPE`, `SIGALRM`, `SIGTERM`, `SIGUSR1`, `SIGUSR2` | exposed | Rust rejects unsafe PID values before platform conversion. Windows supports a limited signal set. |
| Signal one process group | `sysprims_signal::killpg`, `sysprims_signal::validate_pgid` | `sysprimsSignalSendGroup` | `signalSendGroup`, `terminateGroup`, `forceKillGroup`, `validatePgid` | exposed | Unix only. Public APIs never encode groups as negative PIDs. |
| Signal catalog and name-based dispatch | `sysprims_signal::{resolve_signal_name, match_signal_names, kill_by_name, kill_group_by_name}` | `sysprimsResolveSignalName`, `sysprimsMatchSignalNames`, `sysprimsKillByName`, `sysprimsKillGroupByName` | `resolveSignalName`, `matchSignalNames`, `killByName`, `killGroupByName` | exposed | Resolves and globs canonical signal names and short IDs across the 4-language signal conformance catalog. |
| Process-bound signal and exit handle | `sysprims_signal::{open_process_handle, open_process_handle_verified, ProcessHandle}` | `sysprimsProcessHandleOpen`, `sysprimsProcessHandleExited`, `sysprimsProcessHandleSignal`, `sysprimsProcessHandleClose` | `openProcessHandle`, `openProcessHandleVerified`, `ProcessHandle`; `OpenProcessHandleOptions`, `SignalBinding` | exposed | Brackets handle registration with native process start-identity checks and binds signal delivery to an OS descriptor/handle (`pidfd` on Linux `>= 5.3`, `HANDLE` on Windows) or re-checks the `kqueue` exit watch and `kinfo_proc` start-identity before numeric `kill` on macOS. |
| Graceful direct termination | `sysprims_signal::terminate` | `sysprimsTerminate` | `terminate` | exposed | SIGTERM on Unix; immediate `TerminateProcess` semantics on Windows. |
| Forced direct termination | `sysprims_signal::force_kill` | `sysprimsForceKill` | `forceKill` | exposed | SIGKILL on Unix and immediate termination on Windows. |
| JavaScript batch signal conveniences | TypeScript composition over native signal operations | none | `killMany`, `terminateMany`, `forceKillMany`; `BatchKillFailure`, `BatchKillResult` | exposed | Operations validate and report per-PID failures. They do not add native batch authority. |
| PID-based tree termination | `sysprims_timeout::terminate_tree` | `sysprimsTerminateTree` | `terminateTree`; `TerminateTreeConfig`, `TerminateTreeResult` | exposed | Legacy PID-based operation. Windows has no owned Job handle and is best-effort. |
| PID-returning grouped spawn | `sysprims_timeout::spawn_in_group` | `sysprimsSpawnInGroup` | `spawnInGroup`; `SpawnInGroupConfig`, `SpawnInGroupResult` | exposed | Unix creates a process group. Windows fails before spawn because a PID cannot retain Job ownership. |
| Managed contained spawn | `sysprims_timeout::spawn_contained` via the native lifecycle registry | `sysprimsContainmentSpawn` | `spawnContained`; `ContainedProcess`, `SpawnContainedOptions` | exposed | Sole constructor. Always contained. Unix success is `guaranteed` plus `cooperative_group`. Windows fails before spawn. Native code owns the child; callers receive a generation-checked token, never a PID. |
| Managed containment lifecycle | `sysprims_timeout::managed` registry over `ContainmentGuard` | `sysprimsContainmentIdentity`, `sysprimsContainmentPoll`, `sysprimsContainmentWait`, `sysprimsContainmentTerminate`, `sysprimsContainmentClose` | `ContainedProcess`; `ContainmentSnapshot`, `ContainmentIdentity`, `ContainmentCompletion`, `ContainedProcessWaitOptions` | exposed | Promise/async-worker off the event loop. Wait cancellation abandons the waiter only. Close is deterministic; a finalizer is a leak backstop. Invalid, stale, foreign, and reused tokens fail closed. |
| Timeout execution | `sysprims_timeout::run_with_timeout` | none | none | excluded | A public asynchronous contract needs separately approved native ownership, cancellation, concurrency limits, Node teardown, and guaranteed child cleanup. A blocking N-API projection would stall the JavaScript event loop. |
| Error values | `sysprims_core::SysprimsError` | N-API call result envelopes | `SysprimsError`, `SysprimsErrorCode` | exposed | Public wrappers convert native result envelopes into exceptions. |

The public type block also includes `JsonObject` only as an internal helper. It
is not referenced by the emitted package-root declarations and remains excluded.

## N-API Runtime Export Classification

NAPI-RS converts the Rust snake-case function names below to the camel-case
runtime names shown here.

| N-API runtime export | Rust anchor in `native/src/lib.rs` | Public mapping | Disposition | Rationale when excluded |
| --- | --- | --- | --- | --- |
| `sysprimsAbiVersion` | `sysprims_abi_version` | none | excluded, internal-only | Native diagnostic/compatibility metadata. |
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
| `sysprimsSignalSendGroup` | `sysprims_signal_send_group` | `signalSendGroup`, `terminateGroup`, `forceKillGroup` | exposed |  |
| `sysprimsResolveSignalName` | `sysprims_resolve_signal_name` | `resolveSignalName`, `ProcessHandle` | exposed |  |
| `sysprimsMatchSignalNames` | `sysprims_match_signal_names` | `matchSignalNames` | exposed |  |
| `sysprimsKillByName` | `sysprims_kill_by_name` | `killByName` | exposed |  |
| `sysprimsKillGroupByName` | `sysprims_kill_group_by_name` | `killGroupByName` | exposed |  |
| `sysprimsProcessHandleOpen` | `sysprims_process_handle_open` | `openProcessHandle`, `openProcessHandleVerified` | exposed |  |
| `sysprimsProcessHandleExited` | `sysprims_process_handle_exited` | `ProcessHandle` | exposed |  |
| `sysprimsProcessHandleSignal` | `sysprims_process_handle_signal` | `ProcessHandle` | exposed |  |
| `sysprimsProcessHandleClose` | `sysprims_process_handle_close` | `ProcessHandle` | exposed |  |
| `sysprimsTerminate` | `sysprims_terminate` | `terminate` | exposed |  |
| `sysprimsForceKill` | `sysprims_force_kill` | `forceKill` | exposed |  |
| `sysprimsTerminateTree` | `sysprims_terminate_tree` | `terminateTree` | exposed |  |
| `sysprimsRunSetsid` | `sysprims_run_setsid` | `runSetsid` | exposed |  |
| `sysprimsRunNohup` | `sysprims_run_nohup` | `runNohup` | exposed |  |
| `sysprimsSpawnInGroup` | `sysprims_spawn_in_group` | `spawnInGroup` | exposed |  |
| `sysprimsContainmentSpawn` | `sysprims_timeout::containment_spawn` | `spawnContained` | exposed | Native token is a bigint, never a JavaScript number. |
| `sysprimsContainmentIdentity` | `sysprims_timeout::containment_identity` | `ContainedProcess` | exposed |  |
| `sysprimsContainmentPoll` | `sysprims_timeout::containment_poll` | `ContainedProcess` | exposed |  |
| `sysprimsContainmentWait` | `sysprims_timeout::containment_wait` | `ContainedProcess` | exposed | Async worker; abort abandons the waiter only. |
| `sysprimsContainmentTerminate` | `sysprims_timeout::containment_terminate` | `ContainedProcess` | exposed |  |
| `sysprimsContainmentClose` | `sysprims_timeout::containment_close` | `ContainedProcess` | exposed |  |

The N-API result objects `SysprimsCallJsonResult`, `SysprimsCallU32Result`, and
`SysprimsCallVoidResult` are internal transport shapes, not supported public
constructors. The package currently has no `exports` map and ships `dist/`, so
consumers can technically deep-import `dist/ffi` and reach native aliases marked
internal-only above. Those paths are unsupported implementation details. Adding
an `exports` map would break existing deep imports and is outside this
patch-compatible phase; drift checks therefore enforce the package-root surface
rather than claiming physical encapsulation.

## Pure-Rust Intent and Explicit Exclusions

| Rust capability | N-API | Public TypeScript | Disposition | Rationale |
| --- | --- | --- | --- | --- |
| `sysprims_timeout::spawn_contained` | `sysprimsContainmentSpawn` | `spawnContained` | exposed through the managed handle | Bindings never receive the Rust `Command`/`Child`. The registry performs the owned spawn and returns a capability token. |
| `sysprims_timeout::adopt_contained` | none | none | excluded | Post-spawn adoption is `unproven`; JavaScript cannot transfer a generic owned child and receive it back on acquisition failure. `adoptContained(pid)` is forbidden. |
| `sysprims_timeout::ContainmentGuard` | none as a class field | none as a reconstructible object | excluded as a raw type | The guard stays native-owned. A PID must never reconstruct or borrow a guard. |
| `sysprims_timeout::ContainmentOutcome` | projected by the matching N-API tasks | `ContainmentSnapshot` | exposed as snapshot evidence | Outcome data is readable only through the managed handle. |
| `sysprims_timeout::ContainmentIdentity` | projected through the managed snapshot | `ContainmentIdentity` | exposed as read-only evidence | PID, start time, and executable are diagnostic evidence, not caller-constructible termination authority. |
| `sysprims_timeout::ContainmentChild` | none | none | excluded | Rust adapter trait includes platform-specific owned child and raw-handle requirements. |
| `sysprims_timeout::{run_with_timeout, run_with_timeout_default}` | none | none | excluded | The default function is a Rust convenience alias. Both remain outside TypeScript until a safe asynchronous lifecycle contract is approved. |
| `sysprims_proc::GuardRunner` managed loop | none | none | excluded | Native blocking scheduling and signal integration do not define a safe JavaScript event-loop and disposal contract. Public `guardStep` remains available. |
| `sysprims_signal::{kill_many, terminate_many, force_kill_many}` | none | `killMany`, `terminateMany`, `forceKillMany` | exposed through JavaScript composition | Existing public conveniences preserve per-PID error reporting without exposing a new native authority. |
| `sysprims_signal::{resolve_signal_name, match_signal_names, kill_by_name, kill_group_by_name}` | `sysprimsResolveSignalName`, `sysprimsMatchSignalNames`, `sysprimsKillByName`, `sysprimsKillGroupByName` | `resolveSignalName`, `matchSignalNames`, `killByName`, `killGroupByName` | exposed | Shared 4-language signal catalog and name-based dispatch surface. |
| `sysprims_signal::{open_process_handle, open_process_handle_verified, ProcessHandle}` | `sysprimsProcessHandleOpen`, `sysprimsProcessHandleExited`, `sysprimsProcessHandleSignal`, `sysprimsProcessHandleClose` | `openProcessHandle`, `openProcessHandleVerified`, `ProcessHandle` | exposed | Process-bound exit watch and signal handle (`ProcessHandle`) across macOS (`kqueue`), Linux (`pidfd`), and Windows (`HANDLE`). |
| `sysprims_signal::{terminate_group, force_kill_group}` | `sysprimsSignalSendGroup` | `terminateGroup`, `forceKillGroup` | exposed | Group termination conveniences over `signalSendGroup`. |
| `sysprims_proc::{snapshot, snapshot_with_options, snapshot_filtered}` | default N-API aliases or none | `processList` | exposed through the options-capable operation | These Rust convenience variants collapse into the single public filtered/options-capable operation. |
| `sysprims_proc::{get_process, descendants, descendants_with_options, descendants_with_config}` | default N-API aliases or none | `procGet`, `descendants` | exposed through options-capable operations | These Rust convenience variants do not require duplicate JavaScript values. Phase B adds opt-in descendant detail fields to the existing `descendants` operation rather than another public function. |
| `sysprims_proc::{cpu_total_time_ns, select_descendant_targets}` | none | none | excluded, implementation support | These operations support CPU enrichment and remediation selection internally; neither is a standalone cross-layer capability contract. |
| `sysprims_proc::process_by_port` | none | none | excluded | This convenience lookup has no N-API projection or distinct result schema; consumers can compose `listeningPorts` with `procGet`. |
| `sysprims_proc::{is_live, is_fully_gone}` | none | none | excluded | Internal lifecycle predicates answer PID-slot state, not process identity; exposing them invites unsafe authorization decisions. |
| `sysprims_session::{setsid, setpgid}` | none | none | excluded | These mutate the Node host process/session or arbitrary process groups and are unsuitable JavaScript authority. Spawn operations retain the safe owned-child boundary. |
| `sysprims_session::{getsid, getpgid}` for arbitrary PIDs | current-process queries only | `selfSID`, `selfPGID` | excluded beyond current process | Rust accepts PID 0 for self, but public JavaScript keeps explicit current-process queries and does not inherit low-level PID-0 semantics. |

## Public Type Export Inventory

The emitted package root must continue to export these 45 explicit type names:

`AncestorsOptions`, `AncestorsResult`, `BatchKillFailure`, `BatchKillResult`,
`ContainedProcessWaitOptions`, `ContainmentCompletion`, `ContainmentIdentity`,
`ContainmentSnapshot`, `CpuMode`, `DescendantsLevel`, `DescendantsOptions`,
`DescendantsResult`, `FdFilter`, `FdSnapshot`, `GuardAction`, `GuardConfig`,
`GuardEvent`, `GuardRule`, `KillDescendantsFailure`, `KillDescendantsOptions`,
`KillDescendantsResult`, `OpenProcessHandleOptions`, `PortBinding`,
`PortBindingsSnapshot`, `PortFilter`, `ProcessFilter`, `ProcessInfo`,
`ProcessOptions`, `ProcessSnapshot`, `ProcessState`, `Protocol`, `RunNohupConfig`,
`RunSetsidConfig`, `SessionIdentifierProvenance`, `SessionKind`,
`SessionSpawnResult`, `SessionSpawnStatus`, `SessionSpawnVerb`, `SignalBinding`,
`SpawnContainedOptions`, `SpawnInGroupConfig`, `SpawnInGroupResult`,
`TerminateTreeConfig`, `TerminateTreeResult`, and `WaitPidResult`.

`ContainedProcess`, `ProcessHandle`, `SysprimsError`, and `SysprimsErrorCode`
also remain available in both the value and type namespaces. `FdInfo` and
`FdKind` remain package-root type exports because existing public `FdSnapshot`
and `FdFilter` declarations already reference them.

## Projection and Drift Policy

The implementation phase must enforce this matrix without changing policy:

1. Extract public values and type-only exports from emitted package declarations
   and their export graph, not from regular expressions over source files.
2. Inventory runtime N-API exports mechanically. Every new
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
