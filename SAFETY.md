# Repository Safety Protocols

This repository implements OS-facing process control and inspection utilities. That makes it powerful—and easy to misuse. These safety protocols exist to keep development, testing, and releases safe and reproducible.

## Operational Danger Classification

### Level 1 — Catastrophic

- Anything that can delete/overwrite user data, wipe disks, alter system boot, or mass-kill critical processes
- **Never run in CI**
- Never merge code that performs these actions without strong guardrails

### Level 2 — High Risk

- Process tree termination, signal escalation, privileged inspection (e.g., reading another user's process info)
- Must be **explicit**, **tested**, and **documented** with safe defaults

### Level 3 — Medium Risk

- Changes to FFI surfaces, schemas, release pipelines
- Requires extra review per MAINTAINERS.md

## Explicit Authorization Protocol

Any code path that:

- kills more than one process,
- kills a process *tree*,
- changes system-wide settings,
- performs privileged inspection,

must require **explicit opt-in** via:

- a CLI flag (`--foreground` to disable tree-kill, `--strict-tree-kill`, etc.), or
- a library config field (default must be safe), or
- a documented "dangerous API" module boundary

**Note**: sysprims uses **group-by-default** semantics - tree-kill is the *default* because it's the safe behavior for CI/CD. The opt-out (`--foreground`) is for legacy compatibility.

## Quality Gate Requirements

All of the following must pass in CI:

| Gate | Command | Failure Action |
|------|---------|----------------|
| Format | `cargo fmt --check` | Block merge |
| Lint | `cargo clippy` | Block merge (warnings are errors) |
| Tests | `cargo test` | Block merge |
| License | `cargo deny check licenses` | Block merge |
| Advisories | `cargo deny check advisories` | Block merge |
| Schema validation | Golden tests | Block merge |

## Safety Testing Requirements

### Non-Negotiable Tests

**Tree-escape test** (per OS):

1. Spawn a child that spawns grandchildren
2. Grandchildren attempt to detach/ignore signals
3. Verify timeout kills all descendants under Group-by-Default policy
4. Assert: no orphaned processes remain

See [ADR-0003](docs/decisions/ADR-0003-group-by-default.md) for the core differentiator.

**FFI memory ownership**:

1. Allocate strings via `sysprims_*` functions
2. Free via `sysprims_free_string()` only
3. Verify no leaks, no double-frees, no use-after-free

See [ADR-0004](docs/decisions/ADR-0004-ffi-design.md) for FFI contracts.

## Release Safety

Release pipeline must:

- Generate SBOM (`cargo sbom`) and THIRD_PARTY_NOTICES.md
- Publish checksums for all artifacts
- Avoid executing untrusted code during packaging
- Validate all binaries against target platforms

## References

- [AGENTS.md](AGENTS.md) - Agent operational protocols
- [MAINTAINERS.md](MAINTAINERS.md) - Review requirements
- [ADR-0001](docs/decisions/ADR-0001-license-policy.md) - License policy
- [ADR-0003](docs/decisions/ADR-0003-group-by-default.md) - Group-by-default (core differentiator)
- [ADR-0004](docs/decisions/ADR-0004-ffi-design.md) - FFI design
