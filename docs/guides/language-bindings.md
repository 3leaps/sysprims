# Language Bindings Guide

This guide covers building and using sysprims language bindings and packages (Go, Python, TypeScript).

## Overview

sysprims exposes its primitives to three languages beyond Rust:

- **Go**: pure-Go (`CGO_ENABLED=0`) packages in the root module
  `github.com/3leaps/sysprims` (`syserror`, `signal`, `session`, `timeout`,
  `proc`, `evidence`, `procobserve`), plus the separate
  `github.com/3leaps/sysprims/go/recurrence` module. No C toolchain or native
  libraries are required.
- **Python**: `sysprims` on PyPI, a PyO3 `cp310-abi3` extension module built
  from the Rust crates (`bindings/python`).
- **TypeScript**: `@3leaps/sysprims`, a Node-API (N-API) native addon built
  with napi-rs (`bindings/typescript/sysprims`).

Python and TypeScript link the Rust crates directly. Rust, Go, Python, and
TypeScript share conformance vectors under `testdata/conformance/` (`signal`,
`session`, `timeout`, `proc`).

## Platform Matrix

See [Platform Support Matrix](../standards/platform-support.md) for the canonical reference.

| Platform            | Rust Target (TypeScript)     | Go `GOOS/GOARCH` |
| ------------------- | ---------------------------- | ---------------- |
| Linux x64 (glibc)   | `x86_64-unknown-linux-gnu`   | `linux/amd64`    |
| Linux x64 (musl)    | `x86_64-unknown-linux-musl`  | `linux/amd64`    |
| Linux arm64 (glibc) | `aarch64-unknown-linux-gnu`  | `linux/arm64`    |
| Linux arm64 (musl)  | `aarch64-unknown-linux-musl` | `linux/arm64`    |
| macOS arm64         | `aarch64-apple-darwin`       | `darwin/arm64`   |
| Windows x64         | `x86_64-pc-windows-msvc`     | `windows/amd64`  |
| Windows arm64       | `aarch64-pc-windows-msvc`    | `windows/arm64`  |

**Not supported**: macOS x64 (Intel Macs) - end-of-life hardware as of v0.1.7.

Go packages are pure Go, so a Go build needs no C toolchain on any platform,
including Windows. TypeScript native addons are built with MSVC on Windows x64
and arm64. Python wheel coverage is listed in the
[Platform Support Matrix](../standards/platform-support.md).

## Managed contained spawn

Go `SpawnContained` and TypeScript `spawnContained` are the only binding
constructors that return owned containment. They always spawn contained;
there is no foreground mode. The handle is a generation-checked native
token, not a PID. Close/dispose is deterministic. Existing `RunWithTimeout`,
`TerminateTree` / `terminateTree`, and `SpawnInGroup` / `spawnInGroup` stay
as they are.

Unix success reports `guaranteed` race-free acquisition and retained
group-signaling eligibility with a `cooperative_group` boundary. Descendants
that leave the group are outside that boundary, so `guaranteed` does not mean
OS-enforced non-escape. Windows rejects this constructor before executing argv.
The same handle provides identity, poll, wait, terminate, and close methods;
PID fields are diagnostic evidence, never a transferable ownership capability.

Execution deadlines belong to the native owner and run without language-side
polling. A wait timeout returns an active/running snapshot and does not terminate
the child or cancel another operation's cleanup. Zero means an unbounded wait.
A waiter that itself acquires cleanup ownership may also spend the configured
cleanup window completing it. An already-exited leader remains `completed` when
first observed after its execution deadline. `leader_status == timed_out` means
execution-deadline enforcement; the separate `timed_out` flag describes cleanup
reap timeout.

Always close explicitly (`defer` in Go, `try/finally` or async disposal in
TypeScript). Successful close is idempotent; failed close retains the token for
retry. Finalizers are leak backstops, not deterministic disposal. TypeScript
wait cancellation ends the caller's wait, not native ownership or cleanup.
See the `timeout` package documentation
(`github.com/3leaps/sysprims/timeout`) and the
[TypeScript README](../../bindings/typescript/sysprims/README.md) for examples.

## Go Packages

For port-to-process mapping (listening ports), see `docs/guides/port-bindings-getting-started.md`.

### Installation

```bash
go get github.com/3leaps/sysprims@vX.Y.Z
# Recurrence rules live in their own module:
go get github.com/3leaps/sysprims/go/recurrence@vX.Y.Z
```

The root module requires Go 1.26.0 or later; `go/recurrence` targets Go 1.21.
Import the packages you need, for example `github.com/3leaps/sysprims/proc`
or `github.com/3leaps/sysprims/signal`.

### Local Development

```bash
# vet and test the root module and go/recurrence with GOWORK=off CGO_ENABLED=0
make go-test
```

## CI/CD Integration

### CI: Testing Go Packages

The repository root `go.mod` declares `module github.com/3leaps/sysprims`. A
`go.work` lists `.` and `./go/recurrence` so repo-root tooling (e.g. goneat /
golangci-lint) can lint and typecheck both modules. A tiny placeholder package
under `internal/gowork/` keeps repo-root `./...` patterns resolvable.

For TypeScript bindings, we keep a minimal repo-root `package.json` as a tooling shim.
This is not a published npm package; it exists so repo-root tools that invoke npm (e.g.
goneat in `--package-mode`) do not error when run from the repository root.

The CI `test-go` job runs `go vet ./...` and `go test ./...` with `GOWORK=off`
and `CGO_ENABLED=0` on Linux, macOS, Windows x64, and Windows arm64.

### Go Release Tags

The root module resolves from the canonical `vX.Y.Z` release tag. The
`go/recurrence` module lives in a subdirectory, so it resolves from the
path-prefixed tag `go/recurrence/vX.Y.Z` on the same commit (ADR-0017):

```bash
go get github.com/3leaps/sysprims@vX.Y.Z
go get github.com/3leaps/sysprims/go/recurrence@vX.Y.Z
```

Releases through v0.4.0 also published a cgo module at `bindings/go/sysprims`
with `bindings/go/sysprims/vX.Y.Z` tags. That module was retired in v0.5.0
(ADR-0019); its existing tags remain resolvable for existing pins.

## TypeScript Bindings

TypeScript bindings use a Node-API (N-API) native addon (napi-rs).

### Platform Support

See [Platform Support Matrix](../standards/platform-support.md) for the canonical reference.

| Platform            | Status    |
| ------------------- | --------- |
| Linux x64 (glibc)   | Supported |
| Linux x64 (musl)    | Supported |
| Linux arm64 (glibc) | Supported |
| Linux arm64 (musl)  | Supported |
| macOS arm64         | Supported |
| Windows x64         | Supported |
| Windows arm64       | Supported |

**Not supported**: macOS x64 (Intel Macs) - end-of-life hardware as of v0.1.7.

Runtime support covers Node.js >= 18 and Bun >= 1.3. Bun uses the same Node-API
binding surface as Node.js; prebuilt package availability still follows the
platform matrix above.

### Installation

**From git checkout / local path (current):**

When installing from a git checkout or local path, the addon is built from source:

```bash
cd bindings/typescript/sysprims
npm install
npm run build:native  # Builds the N-API addon
```

**Requirements for building from source:**

- Rust toolchain (1.88+)
- C/C++ compiler
- Node.js 18+

**From npm:**

Prebuilt platform packages are installed automatically:

```bash
npm install @3leaps/sysprims
# Prebuilt .node binary selected based on platform
```

No build tools required when using npm prebuilds.

### API Surface

The intended TypeScript surface is recorded in the
[capability intent matrix](../../bindings/typescript/sysprims/docs/capability-intent-matrix.md)
and the generated [public API reference](../../bindings/typescript/sysprims/docs/public-api.md).
CI checks emitted declarations, N-API exports, and generated documentation for
drift.

| Function                                  | Description                                                        |
| ----------------------------------------- | ------------------------------------------------------------------ |
| `procGet(pid, options?)`                  | Get process info by PID with opt-in details                        |
| `processList(filter?, options?)`          | List processes with filtering and opt-in details                   |
| `ancestors(pid, options?)`                | Walk process ancestry                                              |
| `descendants(pid, options?)`              | Walk descendants with default-off environment/thread enrichment    |
| `listFds(pid, filter?)`                   | List process file descriptors                                      |
| `listeningPorts(filter?)`                 | Map listening ports to processes                                   |
| `waitPID(pid, timeoutMs)`                 | Wait for process exit with timeout                                 |
| `guardStep(config)`                       | Run one guard observation/remediation step                         |
| `killDescendants(pid, signal?, options?)` | Signal matching descendants with per-target results                |
| `terminateTree(pid, config?)`             | Graceful-then-kill tree termination                                |
| `signalSend(pid, signal)`                 | Send a signal to a process                                         |
| `signalSendGroup(pgid, signal)`           | Send a signal to a process group on Unix                           |
| `terminate(pid)`                          | Graceful termination                                               |
| `forceKill(pid)`                          | Immediate kill                                                     |
| `killMany(pids, signal)`                  | Signal multiple processes with per-PID results                     |
| `terminateMany(pids)`                     | Gracefully terminate multiple processes                            |
| `forceKillMany(pids)`                     | Force-kill multiple processes                                      |
| `spawnContained(argv, options?)`          | Spawn an owned contained process with native deadline and disposal |
| `spawnInGroup(config)`                    | Spawn in a new Unix process group; unsupported on Windows          |
| `runSetsid(config)`                       | Spawn a command in a new POSIX session                             |
| `runNohup(config)`                        | Spawn a SIGHUP-ignoring command on Unix                            |
| `selfPGID()`                              | Get the current process group ID on Unix                           |
| `selfSID()`                               | Get the current session ID on Unix                                 |

Numeric PID, process-group, signal, depth, duration, port, and filter inputs
are validated before JavaScript coercion or native loading and revalidated at
the N-API boundary. Owned containment is available through `spawnContained`,
including its native execution deadline. TypeScript does not project the legacy
Rust timeout runner or adopt lifecycle ownership from a PID.

### Filter Conventions

Filter fields use **snake_case** to match schema conventions directly:

```typescript
// ProcessFilter
const filter = {
  name_contains: "nginx", // substring match
  cpu_above: 50, // percentage
  memory_above_kb: 100000, // kilobytes
};

// PortFilter
const portFilter = {
  protocol: "tcp",
  local_port: 8080,
};
```

### v0.1.6 Supervisor Primitives

**waitPID** - Wait for a process to exit with timeout:

```typescript
import { waitPID } from "@3leaps/sysprims";

const outcome = waitPID(pid, 10000); // 10 seconds
if (outcome.timed_out) {
  console.log("Process did not exit in time");
} else {
  console.log(`Exited with code ${outcome.exit_code}`);
}
```

**spawnInGroup** - Spawn process in a new Unix process group:

```typescript
import { spawnInGroup } from "@3leaps/sysprims";

const result = spawnInGroup({
  argv: ["./worker.sh", "--id", "42"],
  cwd: "/app",
  env: { LOG_LEVEL: "debug" },
});

console.log(`Spawned PID ${result.pid}`);
```

`spawnInGroup` reports `best_effort` on Unix because the binding returns only a
PID and cannot retain a receipt-bound child capability. It fails closed on
Windows because it cannot retain the Job handle required for later tree
cleanup. Callers should not treat PID-only `terminateTree` as owned
containment.

**terminateTree** - Graceful-then-kill tree termination:

```typescript
import { terminateTree } from "@3leaps/sysprims";

const outcome = terminateTree(pid, {
  grace_timeout_ms: 5000,
  kill_timeout_ms: 2000,
});

if (outcome.escalated) {
  console.log("Had to escalate to kill");
}
```

### Config Types (v0.1.6+)

**SpawnInGroupConfig:**

- `argv`: Command and arguments (required)
- `cwd`: Working directory (optional)
- `env`: Environment overrides (optional)

**TerminateTreeConfig:**

- `signal`: Initial signal (default: SIGTERM/15)
- `grace_timeout_ms`: Wait before escalation (default: 10000)
- `kill_signal`: Kill signal (default: SIGKILL/9)
- `kill_timeout_ms`: Wait after kill (default: 2000)

### How It Works

At load time, the binding:

1. Detects the current platform (`process.platform` + `process.arch`)
2. Loads the appropriate `.node` binary (either locally built or a prebuilt binary)
3. Exposes typed functions to JavaScript

### Local Development

```bash
cd bindings/typescript/sysprims
npm install
npm run build
npm run build:native
npm test
```

## Adding New Features

When adding a new primitive:

1. **Rust**: Implement it in the owning crate under `crates/`.
2. **Go**: Add the pure-Go twin to the matching root-module package
   (`signal`, `session`, `timeout`, `proc`, ...).
3. **Python / TypeScript**: Expose it in `bindings/python/src/` and
   `bindings/typescript/sysprims/native/src/` (plus the TypeScript wrapper in
   `bindings/typescript/sysprims/src/`).
4. **Conformance**: Add or extend vectors under `testdata/conformance/` so all
   four languages verify the same behavior.
5. **Documentation**: Update this guide if behavior is platform-specific.

## References

- [ADR-0012: Language Bindings Distribution](../decisions/ADR-0012-language-bindings-distribution.md)
- [ADR-0017: Recurrence Go Module and Release Tag Set](../decisions/ADR-0017-recurrence-go-module-and-release-tags.md)
- [ADR-0018: Native Process Evidence Surfaces and Root Pure-Go Module](../decisions/ADR-0018-native-process-evidence-and-root-go-module.md)
- [ADR-0019: Retire the cgo Go Bindings and the C-ABI](../decisions/ADR-0019-retire-cgo-bindings-and-c-abi.md)
