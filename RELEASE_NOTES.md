# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

---

## v0.1.1 - 2026-01-17

**Status:** First Language Bindings Release

Completes the FFI surface and ships Go bindings with full cross-platform support.

### Highlights

- **Complete FFI Surface**: 14 C-ABI functions covering error handling, signals, process inspection, and timeout execution
- **Go Bindings**: First-class Go bindings with prebuilt static libraries for 7 platforms
- **Port-to-PID Lookup**: New `ListeningPorts()` API maps listening sockets to owning processes
- **CLI Enhancements**: `kill -l` signal listing, `kill --group` for process groups

### Go Bindings

Install via Go modules (requires CGo):

```go
import "github.com/3leaps/sysprims/bindings/go/sysprims"
```

**Key APIs:**

| Function | Description |
|----------|-------------|
| `Kill(pid, signal)` | Send signal to process |
| `Terminate(pid)` | Send SIGTERM |
| `ForceKill(pid)` | Send SIGKILL |
| `ProcessList(filter)` | List processes with optional filter |
| `ListeningPorts(filter)` | Map listening ports to PIDs (best-effort) |
| `RunWithTimeout(cmd, args, timeout, config)` | Run command with timeout and tree-kill |

**Supported Platforms:**

- Linux x64 (glibc + musl)
- Linux arm64 (glibc + musl)
- macOS x64 + arm64
- Windows x64 (via MinGW)

### Listening Ports API

Map a TCP/UDP port to its owning process:

```go
proto := sysprims.ProtocolTCP
port := uint16(8080)
snap, err := sysprims.ListeningPorts(&sysprims.PortFilter{
    Protocol: &proto,
    LocalPort: &port,
})
for _, b := range snap.Bindings {
    if b.PID != nil {
        fmt.Printf("Port %d owned by PID %d\n", b.LocalPort, *b.PID)
    }
}
```

**Platform behavior:**
- Linux + Windows: Reliably attributes self-listeners; partial attribution for other processes based on privileges
- macOS: Best-effort; SIP/TCC can restrict socket enumeration

### CLI Enhancements

```bash
# List all available signals
sysprims kill -l

# Get signal number
sysprims kill -l TERM
# Output: 15

# Send signal to process group (Unix only)
sysprims kill --group 1234
```

### What's Next (v0.1.2+)

- Python bindings (cffi/PyO3 + wheel packaging)
- TypeScript bindings (napi-rs + npm packaging)
- C conformance test suite
- Additional CLI polish

---

## v0.1.0 - 2026-01-14

**Status:** Internal (pipeline validation)

Initial release validating CI/CD pipeline and release signing workflow.

### Highlights

- **Group-by-default tree-kill**: When you timeout a process, the entire process tree terminates together
- **Cross-platform support**: Linux (glibc + musl), macOS (x64 + arm64), Windows (x64)
- **License-clean**: MIT/Apache-2.0 dual licensed, no GPL dependencies

### CLI Commands

| Command | Description |
|---------|-------------|
| `sysprims timeout` | Run commands with timeout and reliable tree-kill |
| `sysprims kill` | Send signals to processes |
| `sysprims pstat` | Process inspection and listing |

### Libraries

- `sysprims-core` - Core types and platform detection
- `sysprims-signal` - Cross-platform signal dispatch
- `sysprims-timeout` - Timeout execution with process group management
- `sysprims-proc` - Process enumeration
- `sysprims-session` - Session/setsid helpers

---

*For older releases, see [CHANGELOG.md](CHANGELOG.md) or individual release notes in `docs/releases/`.*
