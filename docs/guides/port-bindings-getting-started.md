---
title: "Port Bindings: Getting Started"
status: "Draft"
last_updated: "2026-01-17"
---

# Port Bindings: Getting Started

This guide shows how to use sysprims to map a listening TCP/UDP port to an owning PID
(and optionally basic process info), with a focus on the Go bindings.

## Concept

- API: `listening_ports(filter: Option<&PortFilter>) -> PortBindingsSnapshot`
- FFI: `sysprims_proc_listening_ports(filter_json, result_json_out)` (JSON-only)
- Go: `sysprims.ListeningPorts(filter *PortFilter) (*PortBindingsSnapshot, error)`

## Output contract (best-effort)

- Linux + Windows: intended to reliably attribute **self-listener** (PID/port you just bound).
- macOS: best-effort. SIP/TCC can prevent socket enumeration/attribution even for same-user processes.
- When attribution is partial, the snapshot includes `warnings: []string`.

## Preview assets (local build, release-like layout)

Developers wishing to access early or pre-release assets on a preview basis can coordinate with our team
to run local build targets.

Assets will appear in `dist/local` in a layout essentially identical to how they appear in the release
assets on GitHub (and are also often available in `dist/release` locally).

The versions in `dist/local` are volatile and subject to rapid change; use only during agreed beta cycles.

Example (FFI bundle):

- `dist/local/release/sysprims-ffi/`
  - `libsysprims_ffi.a`
  - `include/sysprims.h`
  - `include/sysprims-go.h`
  - `LOCAL.txt`

## Go usage

### Install (preview)

In your consumer repository `go.mod`, add a temporary replace to use the local Go bindings:

```go
require github.com/3leaps/sysprims/bindings/go/sysprims v0.0.0

replace github.com/3leaps/sysprims/bindings/go/sysprims => /Users/davethompson/dev/3leaps/sysprims/bindings/go/sysprims
```

Then ensure the local static library exists for your platform:

```bash
cd /Users/davethompson/dev/3leaps/sysprims
make build-local-go
```

### Example: find PID on a port

```go
package main

import (
    "fmt"
    "net"
    "os"

    sysprims "github.com/3leaps/sysprims/bindings/go/sysprims"
)

func main() {
    ln, err := net.Listen("tcp", "127.0.0.1:0")
    if err != nil {
        panic(err)
    }
    defer ln.Close()

    port := uint16(ln.Addr().(*net.TCPAddr).Port)
    pid := uint32(os.Getpid())

    proto := sysprims.ProtocolTCP
    snap, err := sysprims.ListeningPorts(&sysprims.PortFilter{Protocol: &proto, LocalPort: &port})
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
