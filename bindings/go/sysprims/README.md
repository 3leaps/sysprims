# sysprims Go bindings

Go bindings for the `sysprims` Rust process primitives.

- Module: `github.com/3leaps/sysprims/bindings/go/sysprims`
- License: MIT OR Apache-2.0
- Focus: typed, cross-platform process control without shell-outs

## Install

```bash
go get github.com/3leaps/sysprims/bindings/go/sysprims@v0.2.3
```

The Go module resolves `v0.2.3` through the repository's path-prefixed
`bindings/go/sysprims/v0.2.3` tag. That tag and the canonical `v0.2.3` tag
identify the same reviewed commit.

### Windows toolchain

Go cgo on Windows requires a GNU-ABI C compiler driver. Install the one for your architecture:

- **Windows x64**: [msys2](https://www.msys2.org/) with `pacman -S mingw-w64-x86_64-gcc`
- **Windows arm64**: [llvm-mingw](https://github.com/mstorsjo/llvm-mingw) (`*-ucrt-aarch64.zip` release), with `aarch64-w64-mingw32-gcc` on `PATH`; available since v0.1.16

Linux and macOS consumers need no extra toolchain beyond the platform default.

## Managed contained spawn (on `main`; unreleased)

`SpawnContained(SpawnContainedConfig)` spawns from an argv vector and returns
`*ContainedProcess`. Its methods are `Identity`, `Poll`, `Wait`, `Terminate`, and
`Close`. `Wait(0)` is unbounded; negative durations are rejected. Positive
sub-millisecond durations are rounded up to 1ms.

```go
executionMS := uint64(5000)
h, err := sysprims.SpawnContained(sysprims.SpawnContainedConfig{
    Argv: []string{"sleep", "30"},
    ExecutionTimeoutMS: &executionMS,
})
if err != nil {
    return err
}
defer h.Close() // Always arrange cleanup, including on earlier errors.

snapshot, err := h.Wait(6 * time.Second)
if err != nil {
    return err
}
_ = snapshot.LeaderStatus
if err := h.Close(); err != nil {
    return err // The handle remains usable for a cleanup retry.
}
```

Unix success reports `guaranteed` race-free acquisition and retained
group-signaling eligibility with `cooperative_group` boundary strength. This is
a cooperative process group; descendants that leave it are outside the
boundary, so `guaranteed` does not mean OS-enforced non-escape. Windows rejects
managed spawn before argv runs. The handle owns native lifecycle authority; its
PID fields are diagnostic only.

The execution deadline runs natively without polling. A bounded wait returns an
active/running snapshot on timeout, including while another caller cleans up;
it does not terminate the process. A wait that itself owns cleanup may take the
configured grace/kill window to finish. A fast child first observed after its
deadline remains `completed`. `leader_status == timed_out` records execution
deadline enforcement; the separate `timed_out` field records cleanup reap timeout.

Successful close is idempotent. A failed close preserves the handle for retry.
Finalizers are a leak backstop; close explicitly for deterministic disposal.

## Replacing shell-outs

v0.1.14 expands the process-intelligence API so common shell-outs can be replaced directly.

Full guide: [Replace Your Shell-outs with sysprims (Go)](https://github.com/3leaps/sysprims/blob/main/docs/guides/replace-shell-outs-go.md)

| Before (shell-out)               | After (sysprims Go)                                                 |
| -------------------------------- | ------------------------------------------------------------------- |
| `ps eww -p <pid>` + parsing      | `ProcessGetWithOptions(pid, &ProcessOptions{IncludeEnv: true})`     |
| `ps -M -p <pid>` + line counting | `ProcessGetWithOptions(pid, &ProcessOptions{IncludeThreads: true})` |
| `lsof -p <pid>` + parsing        | `ListFds(pid, nil)`                                                 |
| `kill -9 <pid>`                  | `Kill(pid, SIGKILL)`                                                |
| `kill` loops for descendants     | `KillDescendantsWithOptions(...)` with `CpuModeMonitor` + filters   |

### Minimal setup

```go
import (
    "time"

    "github.com/3leaps/sysprims/bindings/go/sysprims"
)
```

### Example: env and thread metadata (instead of `ps` parsing)

```go
info, err := sysprims.ProcessGetWithOptions(pid, &sysprims.ProcessOptions{
    IncludeEnv:     true,
    IncludeThreads: true,
})
if err != nil {
    return err
}

// info.Env and info.ThreadCount are typed fields.
```

### Example: FDs by PID (instead of `lsof -p` parsing)

```go
fds, err := sysprims.ListFds(pid, nil)
if err != nil {
    return err
}

for _, fd := range fds.Fds {
    _ = fd.Kind
    _ = fd.Path
}
```

### Example: kill hot descendants with monitor sampling

```go
cpu := 90.0
result, err := sysprims.KillDescendantsWithOptions(rootPID, &sysprims.KillDescendantsOptions{
    Signal:         sysprims.SIGKILL,
    CpuMode:        sysprims.CpuModeMonitor,
    SampleDuration: 3 * time.Second,
    Filter: &sysprims.ProcessFilter{
        CPUAbove: &cpu,
    },
})
if err != nil {
    return err
}

_ = result.Succeeded
```

Why this is better than shell-outs: typed JSON-backed structures, no brittle text parsing,
and consistent behavior across Linux/macOS/Windows from one API.
