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
│   ├── sysprims-proc/      # Process snapshot/inspect
│   └── sysprims-cli/       # CLI binaries (thin wrappers)
├── ffi/
│   └── sysprims-ffi/       # Stable C ABI surface
├── bindings/
│   ├── go/
│   ├── python/
│   └── typescript/
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
cargo test -p sysprims-ffi                  # FFI boundary focus
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

- Group-by-default uses process groups (`setpgid`) and signals group on timeout
- `killpg()` terminates entire tree

## Binding Development

**Rule**: Bindings MUST NOT re-implement business logic. They wrap `sysprims-ffi`.

| Binding | Technology | Key Functions |
|---------|------------|---------------|
| Go | CGo | `sysprims_timeout_run`, `sysprims_proc_list` |
| Python | PyO3/maturin | Same FFI surface |
| TypeScript | NAPI-RS | Same FFI surface |

All bindings MUST:
- Follow UTF-8 string contracts
- Use `sysprims_free_string()` for memory cleanup
- Pass FFI smoke tests

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
