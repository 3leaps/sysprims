---
title: "Replace Your Shell-outs with sysprims (Go)"
status: "Draft"
last_updated: "2026-02-24"
---

# Replace Your Shell-outs with sysprims (Go)

This guide is for Go platform engineers who currently shell out to `ps`, `lsof`, and `kill`.
The goal is to replace brittle text parsing and process-control edge cases with typed APIs that
work consistently across Linux, macOS, and Windows.

## Why replace shell-outs?

Shell-outs are quick to start and expensive to maintain:

- output format differences across platforms break parsers
- process names/arguments with whitespace force fragile parsing logic
- error handling gets flattened into exit codes and stderr strings
- process-tree operations are easy to get wrong (`kill` one PID, leak descendants)

`sysprims` gives you typed structs and explicit error codes, while keeping your supply chain
license-clean (MIT/Apache-2.0, no copyleft utilities embedded into your application logic).

## Before and after

| Before (shell-out) | After (sysprims Go) |
| --- | --- |
| `exec.Command("ps", "eww", "-p", pid)` + parse env | `ProcessGetWithOptions(pid, &ProcessOptions{IncludeEnv: true})` |
| `exec.Command("ps", "-M", "-p", pid)` + count lines | `ProcessGetWithOptions(pid, &ProcessOptions{IncludeThreads: true})` |
| `exec.Command("lsof", "-p", pid)` + text parse | `ListFds(pid, nil)` |
| `exec.Command("kill", "-9", pid)` | `Kill(pid, SIGKILL)` |
| manual loops over descendants + `kill -TERM` | `KillDescendantsWithOptions(...)` with filter + `CpuModeMonitor` |

## 1) Process metadata without `ps` parsing

When you need detailed process metadata, avoid parsing `ps` output. Ask for exactly what you need.

```go
package main

import (
    "fmt"

    "github.com/3leaps/sysprims/bindings/go/sysprims"
)

func inspect(pid uint32) error {
    info, err := sysprims.ProcessGetWithOptions(pid, &sysprims.ProcessOptions{
        IncludeEnv:     true,
        IncludeThreads: true,
    })
    if err != nil {
        return err
    }

    fmt.Printf("pid=%d name=%s threads=%v\n", info.PID, info.Name, info.ThreadCount)
    if info.Env != nil {
        fmt.Printf("env vars=%d\n", len(info.Env))
    }
    return nil
}
```

Why this is better:

- No dependence on platform-specific `ps` flags.
- No regex/split pipelines for key-value extraction.
- Clear optionality (`Env` and `ThreadCount` are optional fields, not parse failures).

## 2) Open file descriptors without `lsof` text parsing

`lsof` is useful interactively, but fragile in automation. `ListFds` returns structured data:

```go
package main

import (
    "fmt"

    "github.com/3leaps/sysprims/bindings/go/sysprims"
)

func listFDs(pid uint32) error {
    snap, err := sysprims.ListFds(pid, nil)
    if err != nil {
        return err
    }

    for _, fd := range snap.Fds {
        fmt.Printf("fd=%d kind=%s path=%v\n", fd.Fd, fd.Kind, fd.Path)
    }

    if len(snap.Warnings) > 0 {
        fmt.Printf("warnings: %v\n", snap.Warnings)
    }
    return nil
}
```

You keep control of behavior in code, not in command formatting and parser maintenance.

## 3) Descendant-aware CPU targeting (no ad hoc `ps` loops)

A common operations pattern is “find hot descendants and kill them.” Doing this with shell-outs
usually means repeated `ps` snapshots, custom parent/child joins, and timing races.

`sysprims` can sample CPU in monitor mode and apply the filter directly to descendants:

```go
package main

import (
    "fmt"
    "time"

    "github.com/3leaps/sysprims/bindings/go/sysprims"
)

func killHotDescendants(rootPID uint32) error {
    cpuThreshold := 90.0

    // Preview matches first (recommended)
    preview, err := sysprims.DescendantsWithOptions(rootPID, &sysprims.DescendantsOptions{
        CpuMode:        sysprims.CpuModeMonitor,
        SampleDuration: 3 * time.Second,
        Filter: &sysprims.ProcessFilter{
            CPUAbove: &cpuThreshold,
        },
    })
    if err != nil {
        return err
    }
    fmt.Printf("matched descendants: %d\n", preview.MatchedByFilter)

    // Then terminate matching descendants only.
    result, err := sysprims.KillDescendantsWithOptions(rootPID, &sysprims.KillDescendantsOptions{
        Signal:         sysprims.SIGKILL,
        CpuMode:        sysprims.CpuModeMonitor,
        SampleDuration: 3 * time.Second,
        Filter: &sysprims.ProcessFilter{
            CPUAbove: &cpuThreshold,
        },
    })
    if err != nil {
        return err
    }

    fmt.Printf("killed=%d failed=%d skipped_safety=%d\n",
        len(result.Succeeded), len(result.Failed), result.SkippedSafety)
    return nil
}
```

The key point: monitor sampling is about correctness, not convenience. For bursty or short
runaway loops, lifetime averages can under-report active CPU consumers.

## Error handling model

With shell-outs, you usually branch on `exit != 0` and parse stderr. With `sysprims`, errors are
typed (`ErrNotFound`, `ErrPermissionDenied`, `ErrInvalidArgument`, etc.), so call sites can make
reliable decisions without brittle string matching.

## License cleanliness note

Many teams start with shell-outs because they avoid linking decisions. But once process control
becomes product logic, you need a stable dependency contract. `sysprims` is MIT/Apache-2.0 dual
licensed and designed for embedding in libraries and internal platforms without introducing GPL
obligations in your runtime integration path.

## Migration checklist

1. Identify shell-outs that are in hot paths or reliability-critical workflows.
2. Replace one pattern at a time using typed APIs (`ProcessGetWithOptions`, `ListFds`, `Kill*`).
3. Keep fallback shell-out behavior only where platform support is explicitly best-effort.
4. Add tests around structured fields rather than string output parsing.
5. Use monitor mode when your intent is active CPU targeting.

For release context and API additions in this cycle, see `docs/releases/v0.1.14.md`.
