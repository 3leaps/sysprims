# sysprims Go bindings

Go bindings for the `sysprims` Rust process primitives.

- Module: `github.com/3leaps/sysprims/bindings/go/sysprims`
- License: MIT OR Apache-2.0
- Focus: typed, cross-platform process control without shell-outs

## Install

```bash
go get github.com/3leaps/sysprims/bindings/go/sysprims@v0.1.14
```

## Replacing shell-outs

v0.1.14 expands the process-intelligence API so common shell-outs can be replaced directly.

Full guide: [Replace Your Shell-outs with sysprims (Go)](https://github.com/3leaps/sysprims/blob/main/docs/guides/replace-shell-outs-go.md)

| Before (shell-out) | After (sysprims Go) |
| --- | --- |
| `ps eww -p <pid>` + parsing | `ProcessGetWithOptions(pid, &ProcessOptions{IncludeEnv: true})` |
| `ps -M -p <pid>` + line counting | `ProcessGetWithOptions(pid, &ProcessOptions{IncludeThreads: true})` |
| `lsof -p <pid>` + parsing | `ListFds(pid, nil)` |
| `kill -9 <pid>` | `Kill(pid, SIGKILL)` |
| `kill` loops for descendants | `KillDescendantsWithOptions(...)` with `CpuModeMonitor` + filters |

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
