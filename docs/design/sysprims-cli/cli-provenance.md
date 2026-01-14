---
title: "sysprims-cli Provenance"
module: "sysprims-cli"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
---

# Provenance: sysprims-cli

This document records the sources consulted for implementing `sysprims-cli`.

## Policy

- POSIX and platform specifications are the primary reference for behavior
- BSD/MIT/Apache licensed crates are used for argument parsing
- The CLI is a thin wrapper; logic lives in library crates

## Implementation

### Design Approach

The CLI is intentionally minimal:

1. Parse arguments using clap
2. Delegate to library crates
3. Format output (JSON or table)
4. Return appropriate exit code

No complex logic lives in the CLI. This ensures:
- Functionality is testable via library APIs
- Bindings get same behavior as CLI
- CLI is just a "test vehicle" and convenience wrapper

### Dependencies

| Crate | License | Purpose |
|-------|---------|---------|
| clap | MIT/Apache-2.0 | Argument parsing |
| tracing | MIT | Logging |
| tracing-subscriber | MIT | Log formatting |

### NOT Consulted

No external CLI implementations were consulted. The CLI design follows standard Rust patterns using clap.

## Exit Code Design

Exit codes follow GNU conventions for timeout (124/125/126/127) and POSIX conventions for kill.

| Code | Source |
|------|--------|
| 124 | GNU timeout convention |
| 125 | GNU timeout convention |
| 126 | POSIX convention (not executable) |
| 127 | POSIX convention (not found) |

These conventions are documented in public specifications and widely used. No GPL source code was consulted.

## Duration Parsing

Duration parsing (5s, 2m, 500ms) is implemented using standard Rust string parsing. The format is common across many tools and not specific to any implementation.

## Certification

This module's implementation is:

- [x] Original code using permissively-licensed crates
- [x] Thin wrapper delegating to library crates
- [x] Following public conventions for exit codes

No GPL/LGPL/AGPL source code was consulted during development.

---

*Provenance version: 1.0*
*Last updated: 2026-01-09*
*Maintainer: sysprims team*
