# Development Guide

## Repository Goals

- **Library-first**: CLI tools are thin wrappers around Rust crates
- **Binding-first**: Go/Python/TypeScript consumption is a first-class target
- **SSOT schemas**: Machine outputs are versioned and validated (Crucible module, JSON Schema 2020-12)

## Workspace Layout

```
sysprims/
├── crates/
│   ├── sysprims-core/      # Shared errors, types, platform abstractions
│   ├── sysprims-timeout/   # Group-by-default timeout engine
│   ├── sysprims-signal/    # Signal mapping + terminate helpers
│   ├── sysprims-session/   # Session and process-group spawn
│   ├── sysprims-proc/      # Process snapshot/inspect
│   ├── sysprims-evidence/  # Portable identity-evidence evaluation
│   ├── sysprims-recurrence/ # Recurrence rules (recurrence/v0)
│   └── sysprims-cli/       # CLI binaries (thin wrappers)
├── bindings/
│   ├── python/             # PyO3 extension module
│   └── typescript/         # napi-rs addon + TypeScript wrapper
├── syserror/ signal/ session/ timeout/ proc/  # Pure-Go packages (root module)
├── evidence/ procobserve/  # Pure-Go evidence packages (root module)
├── go/recurrence/          # Pure-Go recurrence module
├── testdata/conformance/   # Cross-language conformance vectors
└── schemas/                # Local schema copies (Crucible is SSOT)
```

## Local Commands

### Common

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features
cargo test --workspace --all-features
cargo deny check
```

### Focused Testing

```bash
cargo test --workspace --tests              # Integration tests only
make signal-crosscheck                      # Cross-language parity (also session/timeout/proc)
cargo test -p sysprims-timeout tree_escape  # Tree-escape tests
```

### Schema Validation

```bash
goneat validate schemas/                              # Meta-validate schemas
goneat validate-data --schema schemas/timeout-result/v1.0.0.schema.json output.json
```

## Cross-Platform Notes

### Windows (Job Objects)

- Group-by-default uses Job Objects for reliable process tree termination
- Some environments (nested jobs) may restrict assignment
- Behavior MUST be observable via `tree_kill_reliability` field

### Unix (Process Groups)

- Owned group-by-default uses a pre-exec `setsid` hook and sealed receipt
- `killpg()` targets members that remain in the cooperative process group
- `guaranteed` covers race-free acquisition and signaling eligibility, not
  descendant non-escape

## Binding Development

**Rule**: Python and TypeScript bindings MUST NOT re-implement business logic.
They call the Rust crates directly. The Go packages are a pure-Go
reimplementation and MUST match Rust behavior on the shared conformance vectors.

| Language   | Technology                | Source                                |
| ---------- | ------------------------- | ------------------------------------- |
| Go         | Pure Go (`CGO_ENABLED=0`) | Root module packages, `go/recurrence` |
| Python     | PyO3/maturin              | `bindings/python`                     |
| TypeScript | napi-rs                   | `bindings/typescript/sysprims`        |

All languages MUST:

- Follow UTF-8 string contracts
- Pass the conformance vectors under `testdata/conformance/`

## Adding a New CLI Flag

1. Add to library config struct first (`TimeoutConfig`, etc.)
2. Wire to CLI argument parser
3. Update schema if JSON output changes
4. Add unit and integration tests
5. Update documentation

## References

- [Code Standards](../standards/code-standards.md)
- [Testing Guide](testing.md)
- [ADR-0002: Crate Structure](../architecture/adr/0002-crate-structure.md)
- [ADR-0004: FFI Design](../architecture/adr/0004-ffi-design.md)
