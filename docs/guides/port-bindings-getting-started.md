---
title: "Port Bindings: Getting Started"
status: "Published"
last_updated: "2026-01-17"
---

# Port Bindings: Getting Started

This guide shows how to use sysprims to map a listening TCP/UDP port to an owning PID
(and optionally basic process info), with a focus on the pure-Go `proc` package.

## Concept

- API: `listening_ports(filter: Option<&PortFilter>) -> PortBindingsSnapshot`
- Go: `proc.ListeningPorts(filter *proc.PortFilter) (*proc.PortBindingsSnapshot, error)`
  (`github.com/3leaps/sysprims/proc`); `proc.ProcessByPort(port, protocol)` returns
  the owning process directly

## Output contract (best-effort)

- Linux + Windows: intended to reliably attribute **self-listener** (PID/port you just bound).
- macOS: best-effort. SIP/TCC can prevent socket enumeration/attribution even for same-user processes.
- When attribution is partial, the snapshot includes `warnings: []string`.

## Go usage

### Install

The `proc` package is pure Go (`CGO_ENABLED=0`) in the root module, which
requires Go 1.26.0 or later (replace with the latest version from
[releases](https://github.com/3leaps/sysprims/releases)):

```bash
go get github.com/3leaps/sysprims@vX.Y.Z
```

For local development against a checkout, add a replace directive in your
consumer's `go.mod`:

```go
replace github.com/3leaps/sysprims => /path/to/sysprims
```

### Example: find PID on a port

```go
package main

import (
    "fmt"
    "net"
    "os"

    "github.com/3leaps/sysprims/proc"
)

func main() {
    ln, err := net.Listen("tcp", "127.0.0.1:0")
    if err != nil {
        panic(err)
    }
    defer ln.Close()

    port := uint16(ln.Addr().(*net.TCPAddr).Port)
    pid := uint32(os.Getpid())

    proto := proc.ProtocolTCP
    snap, err := proc.ListeningPorts(&proc.PortFilter{Protocol: &proto, LocalPort: &port})
    if err != nil {
        panic(err)
    }

    found := false
    for _, b := range snap.Bindings {
        if b.LocalPort == port && b.PID != nil && *b.PID == pid {
            found = true
            break
        }
    }

    fmt.Printf("found=%v warnings=%v\n", found, snap.Warnings)
}
```

## Troubleshooting

- If you see `PermissionDenied` on macOS, keep your existing fallback (e.g. `lsof`-based) during beta.
- If `net.Listen("127.0.0.1:0")` fails in a sandboxed runner, treat port-to-PID mapping as unavailable.
