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
npm install @3leaps/sysprims
```

For local development from this repository:

```bash
npm install
npm run build
npm run build:test
```

Native prebuild artifacts are produced by the repository release workflows.

## API

### Process Inspection

- `procGet(pid, options?)`
- `processList(filter?, options?)`
- `ancestors(pid, options?)`
- `descendants(pid, options?)`
- `listFds(pid, filter?)`
- `listeningPorts(filter?)`
- `waitPID(pid, timeoutMs)`

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
