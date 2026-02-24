---
title: "Process Intelligence Without Shell-outs"
status: "Draft"
last_updated: "2026-02-24"
---

# Process Intelligence Without Shell-outs

Most production incidents involving subprocesses do not fail because engineers forgot a flag.
They fail because the underlying model is wrong. We ask one snapshot for a dynamic question,
parse text that changes across platforms, and then signal a process we did not identify with
high enough confidence.

The usual stack for process operations in application code is shell-outs: `ps`, `lsof`, and
`kill`. That works until the day it does not. A parser breaks on a platform quirk, CPU spikes
vanish between samples, or we terminate one PID and leave its descendants spinning.

This post walks through a concrete scenario and the practical lesson behind it: replacing
shell-outs is not just about convenience. It is about correctness.

## The scenario: runaway plugin processes

A real debugging session: VSCodium was sluggish, fans were loud, and Activity Monitor showed four
`VSCodium Helper (Plugin)` processes pinned near 100% CPU. But a lifetime-style process snapshot
only surfaced two clear outliers. The other two looked “normal enough” in that one instant.

That mismatch matters. If your data model misses active offenders, your cleanup workflow is wrong
before you type `kill`.

## Why lifetime snapshots fail this use case

Lifetime CPU is an average over process runtime. For long-lived processes, short bursts get washed
out. That is useful for some capacity views and poor for “what is burning CPU right now?”.

Activity Monitor and `top` style tools use sampled CPU over a window. `sysprims` now supports the
same semantics directly in both CLI and library APIs.

The difference is visible immediately.

### CLI workflow: sampled CPU for active diagnosis

```bash
# Find actively hot processes over a 3 second sampling window
sysprims pstat --cpu-mode monitor --sample 3s --name "VSCodium Helper" --cpu-above 80 --table
```

In this mode, CPU values are sampled and may exceed 100 for multi-core consumption. More
importantly, this catches intermittent but sustained runaway behavior that lifetime averages can
hide.

After identifying candidate children, inspect tree structure:

```bash
sysprims descendants <parent_pid> --cpu-mode monitor --sample 3s --cpu-above 80 --tree
```

Now you can target only offending descendants instead of terminating the entire parent process.

## Correctness over convenience

The old shell-out equivalent usually looks like:

1. `ps` loop every N seconds
2. parse output into ad hoc structs
3. build parent-child relationship in app code
4. run `kill` across guessed targets

Each step is a source of race conditions or parser drift.

`sysprims` collapses that into structured primitives:

- process and descendants APIs return typed fields
- CPU sampling behavior is explicit (`lifetime` vs `monitor`)
- filters are typed (`cpu_above`, `name_contains`, `pid_in`, etc.)
- tree-aware kill APIs reduce “kill one PID, leak descendants” failure modes

This is the heart of the argument: a better model produces safer operations.

## Same workflow in Go, without shell-outs

The same diagnosis and action path is available in Go bindings. No subprocess execution, no
stdout parsing, no regex-driven recovery logic.

```go
package main

import (
    "fmt"
    "time"

    "github.com/3leaps/sysprims/bindings/go/sysprims"
)

func diagnoseAndKill(rootPID uint32) error {
    threshold := 90.0

    // 1) Sample descendants in monitor mode.
    desc, err := sysprims.DescendantsWithOptions(rootPID, &sysprims.DescendantsOptions{
        CpuMode:        sysprims.CpuModeMonitor,
        SampleDuration: 3 * time.Second,
        Filter: &sysprims.ProcessFilter{
            CPUAbove: &threshold,
        },
    })
    if err != nil {
        return err
    }

    fmt.Printf("matched=%d total=%d\n", desc.MatchedByFilter, desc.TotalFound)

    // 2) Apply targeted kill to matching descendants.
    res, err := sysprims.KillDescendantsWithOptions(rootPID, &sysprims.KillDescendantsOptions{
        Signal:         sysprims.SIGKILL,
        CpuMode:        sysprims.CpuModeMonitor,
        SampleDuration: 3 * time.Second,
        Filter: &sysprims.ProcessFilter{
            CPUAbove: &threshold,
        },
    })
    if err != nil {
        return err
    }

    fmt.Printf("killed=%d failed=%d skipped_safety=%d\n",
        len(res.Succeeded), len(res.Failed), res.SkippedSafety)
    return nil
}
```

You can also enrich specific process inspection using `ProcessGetWithOptions` to include env and
thread count when needed:

```go
info, err := sysprims.ProcessGetWithOptions(pid, &sysprims.ProcessOptions{
    IncludeEnv:     true,
    IncludeThreads: true,
})
```

That replaces `ps eww -p` and `ps -M -p` parsing with typed fields.

## But what about `lsof` and direct `kill`?

These mappings are straightforward:

- `lsof -p <pid>` -> `ListFds(pid, nil)`
- `kill -9 <pid>` -> `Kill(pid, SIGKILL)`
- repeated descendant kill loops -> `KillDescendantsWithOptions(...)`

The practical gain is not only fewer lines of code. You eliminate a class of errors caused by
parsing human-oriented output and platform-specific command flags.

## Operational safety and intent

Process control has real blast radius. Any API that can terminate processes needs guardrails and
clear intent. In this stack, safety is handled in layers:

- PID validation prevents dangerous broadcast semantics from overflow edge cases
- monitor sampling is read-only (no signal side effects during sampling)
- kill-descendants safety skip counts are surfaced in result output
- config/filter validation fails fast on invalid input

Compared to shell command assembly + string parsing, this is far easier to reason about and test.

## Supply chain and licensing angle

When process control migrates from scripts into product code, licensing becomes part of the
engineering design. Shell-outs to GPL utilities are not automatically a problem for every team,
but they often trigger legal and distribution review overhead.

`sysprims` is MIT/Apache-2.0 dual licensed and designed for embedding. For teams that need
license-clean process primitives without bespoke reimplementation, that removes friction.

## The broader point

The takeaway from the runaway-plugin scenario is simple:

- If your question is “who is hot right now?”, use sampled CPU.
- If your workflow depends on process relationships, use tree-aware APIs.
- If your reliability depends on parser stability, stop parsing human output.

Replacing shell-outs is not a stylistic preference. It is a move from incidental tooling behavior
to explicit contracts that can be tested, versioned, and reasoned about.

And that is what process intelligence should look like in 2026: typed, cross-platform, and built
for correctness first.
