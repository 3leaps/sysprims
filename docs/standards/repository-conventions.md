# Repository Conventions

This document defines naming conventions, file organization, and documentation standards for sysprims.

## Naming Conventions

sysprims follows Rust ecosystem conventions as defined in [RFC 430](https://github.com/rust-lang/rfcs/blob/master/text/0430-finalizing-naming-conventions.md) and the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/naming.html).

### Summary Table

| Asset Type          | Convention             | Examples                                       |
| ------------------- | ---------------------- | ---------------------------------------------- |
| Crate names         | `kebab-case`           | `sysprims-core`, `sysprims-timeout`            |
| Module files        | `snake_case`           | `process_info.rs`, `timeout_config.rs`         |
| Root repo files     | `SCREAMING_CASE`       | `README.md`, `LICENSE-MIT`, `CONTRIBUTING.md`  |
| Documentation files | `kebab-case`           | `getting-started.md`, `0001-license-policy.md` |
| Directories         | `kebab-case`           | `docs/architecture/`, `crates/sysprims-core/`  |
| Types/Traits        | `UpperCamelCase`       | `TimeoutConfig`, `ProcessInfo`                 |
| Functions/Methods   | `snake_case`           | `run_with_timeout()`, `get_process_info()`     |
| Constants/Statics   | `SCREAMING_SNAKE_CASE` | `DEFAULT_TIMEOUT`, `MAX_PID`                   |
| Features            | `kebab-case`           | `sysinfo-backend`, `proc-ext`                  |

### Root Repository Files

Standard files at repository root use `SCREAMING_CASE` (all caps, no separators except for license variants):

```
README.md
LICENSE-MIT
LICENSE-APACHE
CONTRIBUTING.md
CHANGELOG.md
MAINTAINERS.md
AGENTS.md
SECURITY.md
```

**Exception**: `REPOSITORY_SAFETY_PROTOCOLS.md` should be renamed to follow this pattern. Use `SAFETY.md` or keep content in `SECURITY.md`.

### Documentation Files

Documentation within `docs/` uses `kebab-case`:

```
docs/
├── architecture/
│   ├── adr/
│   │   ├── 0001-license-policy.md
│   │   └── 0002-crate-structure.md
│   └── overview.md
├── standards/
│   └── repository-conventions.md
└── getting-started.md
```

### Rust Source Files

Follow Rust conventions - `snake_case` for all `.rs` files:

```
src/
├── lib.rs
├── error.rs
├── process_info.rs
├── timeout_config.rs
└── platform/
    ├── mod.rs
    ├── unix.rs
    └── windows.rs
```

### Crate and Package Names

Use `kebab-case` for crate names in `Cargo.toml`:

```toml
[package]
name = "sysprims-core"
```

Cargo automatically converts to `snake_case` for Rust imports:

```rust
use sysprims_core::TimeoutConfig;
```

## Document Frontmatter

Documentation files SHOULD include YAML frontmatter for organization and AI accountability.

### Standard Document

```yaml
---
title: "Document Title"
description: "Brief description of purpose"
author: "@githubhandle"
date: "2025-12-31"
status: "draft"
---
```

### AI-Assisted Document (Supervised Mode)

```yaml
---
title: "Document Title"
description: "Brief description"
author: "Claude Sonnet"
author_of_record: "Dave Thompson <dave.thompson@3leaps.net>"
supervised_by: "@3leapsdave"
date: "2025-12-31"
status: "draft"
---
```

### Status Values

| Status       | Meaning               |
| ------------ | --------------------- |
| `draft`      | Work in progress      |
| `review`     | Ready for review      |
| `approved`   | Reviewed and approved |
| `deprecated` | No longer current     |

## Directory Structure

```
sysprims/
├── crates/                    # Rust workspace members
│   ├── sysprims-core/
│   ├── sysprims-timeout/
│   ├── sysprims-signal/
│   ├── sysprims-proc/
│   └── sysprims-cli/
├── ffi/                       # FFI exports
│   └── sysprims-ffi/
├── bindings/                  # Language bindings
│   ├── go/
│   ├── python/
│   └── typescript/
├── docs/                      # Documentation
│   ├── architecture/
│   │   └── adr/              # Architecture Decision Records
│   └── standards/            # Repository standards
├── tests/                     # Integration tests
├── examples/                  # Usage examples
└── schemas/                   # JSON schemas (local copies)
```

## ADR Naming

Architecture Decision Records use numbered `kebab-case`:

```
NNNN-short-title.md
```

Examples:

- `0001-license-policy.md`
- `0002-crate-structure.md`
- `0003-group-by-default.md`

## References

- [Rust API Guidelines - Naming](https://rust-lang.github.io/api-guidelines/naming.html)
- [RFC 430 - Naming Conventions](https://github.com/rust-lang/rfcs/blob/master/text/0430-finalizing-naming-conventions.md)
- [Crucible Frontmatter Standard](https://crucible.3leaps.dev/repository/frontmatter)
