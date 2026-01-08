# sysprims Architecture Documentation

> **Document Status**: Bootstrap  
> **Last Updated**: 2025-12-31  
> **Maintainer**: Platform Architecture Team

## Overview

This directory contains architectural documentation for **sysprims**, a GPL-free cross-platform process utilities library. The documentation follows the [Architecture Decision Record (ADR)](https://adr.github.io/) pattern for tracking significant technical decisions.

## Quick Links

| Document | Purpose |
|----------|---------|
| [Architecture Overview](./OVERVIEW.md) | High-level system architecture |
| [Stack Management](./stack/README.md) | Dependency governance, SBOM, licensing |
| [ADR Index](./adr/README.md) | Architecture Decision Records |
| [Integration Guide](./integration/README.md) | Ecosystem integration patterns |
| [Security Architecture](./security/README.md) | Security model and threat considerations |

## Architecture Principles

sysprims is built on the following architectural principles, derived from the ratified project proposal:

### 1. Library-First Design

```
┌─────────────────────────────────────────────────────────┐
│                     Consumer Layer                       │
│  rsfulmen │ gofulmen │ pyfulmen │ tsfulmen │ CLI users │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                    Binding Layer                         │
│      sysprims-go (CGo) │ sysprims-py (PyO3) │ sysprims-ts (NAPI-RS)   │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                      FFI Layer                           │
│              sysprims-ffi (C-ABI via cbindgen)               │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                     Core Layer                           │
│   sysprims-timeout │ sysprims-signal │ sysprims-proc │ sysprims-core       │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                   Platform Layer                         │
│         libc (POSIX) │ windows-sys (Win32)              │
└─────────────────────────────────────────────────────────┘
```

### 2. Zero License Toxicity

All dependencies must pass cargo-deny license checks. No GPL/LGPL/AGPL code paths. See [ADR-0001](./adr/0001-license-policy.md).

### 3. Group-by-Default Reliability

Process tree cleanup is the core differentiator. All timeout operations default to killing the entire process tree. See [ADR-0003](./adr/0003-group-by-default.md).

### 4. Schema-Driven Contracts

All JSON outputs conform to versioned schemas hosted in Crucible. Schema IDs are embedded in outputs for runtime version detection. See [ADR-0005](./adr/0005-schema-contracts.md).

### 5. Observable Fallbacks

When platform limitations prevent guaranteed behavior (e.g., Windows Job Object failures), the degradation is observable in output, not silent. See [ADR-0003](./adr/0003-group-by-default.md).

## Crate Dependency Graph

```
sysprims-cli
├── sysprims-timeout
│   ├── sysprims-signal
│   │   └── sysprims-core
│   └── sysprims-core
├── sysprims-signal
│   └── sysprims-core
├── sysprims-proc
│   └── sysprims-core
└── sysprims-core

sysprims-ffi
├── sysprims-timeout
├── sysprims-signal
├── sysprims-proc
└── sysprims-core
```

## Build Verification

Every build undergoes:

1. **License Analysis** — `cargo deny check licenses`
2. **Security Audit** — `cargo audit`
3. **SBOM Generation** — `cargo sbom` (SPDX format)
4. **Provenance** — goneat analysis for full dependency tree

See [Stack Management](./stack/README.md) for detailed governance.

## Related Documentation

- [Project Proposal (Ratified)](../../sysprims-project-proposal-v0-3.md)
- [Contributing Guide](../../CONTRIBUTING.md)
- [Security Policy](../../SECURITY.md)

## ADR Process

New architectural decisions follow this process:

1. Create ADR from template: `./adr/TEMPLATE.md`
2. Assign next sequential number
3. Submit PR with `adr` label
4. Require approval from Platform Architecture team
5. Merge and update this index

## Questions?

- Architecture questions: Open issue with `architecture` label
- Security concerns: See [SECURITY.md](../../SECURITY.md)
- Licensing questions: See [ADR-0001](./adr/0001-license-policy.md)