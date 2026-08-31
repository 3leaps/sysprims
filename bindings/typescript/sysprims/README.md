# sysprims (TypeScript bindings)

TypeScript bindings for sysprims using a Node-API (N-API) native addon.

## Platform Support

Supported prebuild targets:

- Linux x64 and arm64: glibc and musl
- macOS: arm64
- Windows: x64 and arm64 (MSVC)

Runtime support:

- Node.js >= 18
- Bun >= 1.3, verified against the Node-API binding surface

## Installation

```bash
npm install @3leaps/sysprims@0.2.2
```

The package and its platform-specific native packages are published from the
verified `v0.2.2` repository tag.

For local development from this repository:

```bash
npm install
npm run build
npm run build:test
```

Native prebuild artifacts are produced by the repository release workflows.

### Repository Public API Drift Checks

The checked-in [public API reference](docs/public-api.md) is generated from
emitted declarations and checked against the reviewed capability contract, the
committed C header, and the N-API inventory.

These are repository-development commands. The generator scripts are not part
of the published runtime package.

```bash
npm run api:generate      # update docs/public-api.md
npm run api:check         # generate in a temporary directory and compare
npm run api:check:native  # also require and inspect a built local addon
```

Run `npm run build:native` before `api:check:native`. The regular check inspects
the addon automatically when one is available and otherwise validates the
static, source-independent N-API contract used by pull requests without native
artifacts.

To inventory a freshly generated header instead of the committed default, set
`SYSPRIMS_C_HEADER=/path/to/sysprims.h` or pass `--c-header /path/to/sysprims.h`
directly to `node scripts/public-api.js check`.

`make typescript-api-check` remains a standalone repository target rather than
part of `make check`: the general Rust gate does not install Node dependencies.
Pull-request CI runs `npm ci` before the drift check and also generates a fresh
C header for comparison with current Rust exports.

## API

### Process Inspection

- `procGet(pid, options?)`
- `processList(filter?, options?)`
- `ancestors(pid, options?)`
- `descendants(pid, options?)`
- `listFds(pid, filter?)`
- `listeningPorts(filter?)`
- `waitPID(pid, timeoutMs)`

`descendants(pid, options?)` collects environment and thread details only when
`includeEnv` or `includeThreads` is explicitly enabled. These options are off by
default. Environment values may contain secrets, platform permissions can limit
the available detail, and enriching an entire process tree can increase result
size and latency.

### Guard And Tree Operations

- `guardStep(config)`
- `killDescendants(pid, signal?, options?)`
- `terminateTree(pid, config?)`

### Signal Operations

- `signalSend(pid, signal)`
- `signalSendGroup(pgid, signal)`
- `terminate(pid)`
- `forceKill(pid)`
- `killMany(pids, signal)`
- `terminateMany(pids)`
- `forceKillMany(pids)`

### Spawn Operations

- `spawnInGroup(config)`
- `runSetsid(config)`
- `runNohup(config)`

### Self Introspection

- `selfPGID()`
- `selfSID()`

## Session Spawn Notes

`runSetsid` and `runNohup` take `argv: string[]`; `argv[0]` is the executable.
They do not accept a shell command string.

`runSetsid({ wait: false })` returns a spawned child PID with `sid` and `pgid`
derived structurally from that PID. `runNohup` does not create a new session:
it returns the caller session/process-group context inherited by the child.
Supervise a `runNohup` child by `pid`; do not process-group-signal the returned
`pgid`.

Detached children inherit the caller environment, with `env` entries merged as
overrides. They can outlive the caller, so scrub secrets from the caller
environment before spawning when needed. `runNohup` opens an explicit
`output_file` with append/create semantics and rejects a final symlink. `wait:
true` blocks the calling thread.

## Safety

These bindings call into a process-control library. Validate PIDs from external
input and avoid process-group signalling unless the target group is explicitly
owned and understood.
