# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

---

## v0.1.0 - 2026-01-14

**Status:** Internal (pipeline validation)

Initial release validating CI/CD pipeline and release signing workflow. Not intended for public distribution.

### Highlights

- **Group-by-default tree-kill**: When you timeout a process, the entire process tree terminates together. No orphans, no leaked resources.
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

### Known Limitations

- FFI surface is minimal (scaffolding only)
- No language bindings yet
- CLI `kill -l` not implemented

### What's Next (v0.1.1)

- Complete FFI surface (13 functions)
- `sysprims-proc` listening ports: best-effort port-to-PID mapping with schema-backed JSON output
- Go bindings: `ListeningPorts()` wrapper for port-to-process mapping
- CLI polish (`kill -l`, `kill --group`)
- Container test fixture for privileged tests
- Language binding scaffolds

---

*For older releases, see [CHANGELOG.md](CHANGELOG.md) or individual release notes in `docs/releases/`.*
