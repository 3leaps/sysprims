# ADR-0002: Crate Structure

> **Status**: Accepted  
> **Date**: 2025-12-31  
> **Authors**: Architecture Council

## Context

sysprims needs to support multiple consumption patterns:

1. **Rust library users** wanting to embed specific functionality
2. **FFI consumers** (Go, Python, TypeScript) needing stable C-ABI
3. **CLI users** wanting standalone binaries
4. **Fulmen ecosystem** re-exporting through rsfulmen/gofulmen/pyfulmen/tsfulmen

A monolithic crate would force users to compile everything. A highly fragmented structure would create dependency management overhead.

## Decision

We adopt a **granular workspace** with the following crate structure:

### Core Crates

```
crates/
├── sysprims-core/           # Shared types, errors, telemetry trait
├── sysprims-timeout/        # Process timeout execution
├── sysprims-signal/         # Signal dispatch and process groups
├── sysprims-proc/           # Process inspection
└── sysprims-cli/            # CLI binaries (depends on all above)
```

### FFI Crate

```
ffi/
└── sysprims-ffi/            # C-ABI exports (depends on core crates)
```

### Binding Packages (Separate Repos/Subdirs)

```
bindings/
├── python/             # PyO3 wrapper
├── go/                 # CGo wrapper
└── typescript/         # NAPI-RS wrapper
```

### Dependency Rules

```
sysprims-cli ──┬──► sysprims-timeout ──► sysprims-signal ──► sysprims-core
          ├──► sysprims-signal ──► sysprims-core
          ├──► sysprims-proc ──► sysprims-core
          └──► sysprims-core

sysprims-ffi ──┬──► sysprims-timeout
          ├──► sysprims-signal
          ├──► sysprims-proc
          └──► sysprims-core
```

### Visibility Rules

| Crate              | Public API                         | Internal Use          |
| ------------------ | ---------------------------------- | --------------------- |
| `sysprims-core`    | Types, errors, traits              | Platform abstractions |
| `sysprims-timeout` | `run_with_timeout()`, config types | Spawn mechanics       |
| `sysprims-signal`  | `send_signal()`, `terminate()`     | Platform signal impl  |
| `sysprims-proc`    | `snapshot()`, filter types         | Platform enumeration  |
| `sysprims-cli`     | Binary entry points only           | Arg parsing           |
| `sysprims-ffi`     | C-ABI functions only               | JSON serialization    |

### Feature Flags

Feature flags are defined per-crate and propagate through workspace:

```toml
# sysprims-proc/Cargo.toml
[features]
default = []
proc_ext = []           # Extended info (env, threads, IO)
sysinfo_backend = ["dep:sysinfo"]
tracing = ["dep:tracing"]
```

## Consequences

### Positive

- Users can depend on only what they need (`sysprims-timeout` without `sysprims-proc`)
- Clear separation of concerns
- Feature flags allow size/capability tradeoffs
- CLI and library can evolve somewhat independently

### Negative

- More crates to maintain
- Cross-crate changes require coordinated releases
- Workspace complexity

### Neutral

- All crates versioned together (workspace.package.version)
- Single Cargo.lock for consistency
- CI builds entire workspace

## Alternatives Considered

### Alternative 1: Monolithic Crate

Single `sysprims` crate with feature flags for each component.

**Rejected**: Feature flag combinations become complex; users compiling `sysprims-timeout` would still need `sysprims-proc` in their dependency tree.

### Alternative 2: Fully Separate Repositories

Each crate in its own repository.

**Rejected**: Coordination overhead too high; cross-crate changes become painful.

### Alternative 3: Two Crates (Core + CLI)

`sysprims-core` (library) and `sysprims-cli` (binaries).

**Rejected**: Doesn't allow selective feature usage; FFI boundary unclear.

## References

- [Rust Workspace Documentation](https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html)
- [seekable-zstd structure](https://github.com/3leaps/seekable-zstd) (similar pattern)
