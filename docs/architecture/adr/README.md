# Architecture Decision Records

This directory contains Architecture Decision Records (ADRs) for the sysprims project.

## Index

| ID | Title | Status | Date |
|----|-------|--------|------|
| [0000](./0000-adr-process.md) | ADR Process | Accepted | 2025-12-31 |
| [0001](./0001-license-policy.md) | License Policy | Accepted | 2025-12-31 |
| [0002](./0002-crate-structure.md) | Crate Structure | Accepted | 2025-12-31 |
| [0003](./0003-group-by-default.md) | Group-by-Default Process Control | Accepted | 2025-12-31 |
| [0004](./0004-ffi-design.md) | FFI Design | Accepted | 2025-12-31 |
| [0005](./0005-schema-contracts.md) | Schema Contracts | Accepted | 2025-12-31 |
| [0006](./0006-dependency-governance.md) | Dependency Governance | Accepted | 2025-12-31 |
| [0007](./0007-platform-abstraction.md) | Platform Abstraction Strategy | Accepted | 2025-12-31 |
| [0008](./0008-error-handling.md) | Error Handling Strategy | Accepted | 2025-12-31 |
| [0009](./0009-logging-strategy.md) | Logging Strategy | Accepted | 2025-12-31 |
| [0010](./0010-schema-management-and-versioning.md) | Schema Management and Versioning | Accepted | 2025-12-31 |
| [0011](./0011-pid-validation-safety.md) | PID Validation Safety | Accepted | 2025-12-31 |
| [0012](./0012-language-bindings-distribution.md) | Language Bindings Distribution | **Proposed** | 2026-01-15 |

## Status Definitions

| Status | Meaning |
|--------|---------|
| **Proposed** | Under discussion, not yet decided |
| **Accepted** | Decision made, implementation may be pending |
| **Deprecated** | No longer relevant but kept for history |
| **Superseded** | Replaced by another ADR (link provided) |

## Creating a New ADR

1. Copy `TEMPLATE.md` to `NNNN-title-in-kebab-case.md`
2. Fill in all sections
3. Submit PR with `adr` label
4. Update this index after merge

## ADR Format

All ADRs follow the format in [TEMPLATE.md](./TEMPLATE.md):

- **Title**: Short descriptive name
- **Status**: Current state
- **Context**: Why this decision is needed
- **Decision**: What we decided
- **Consequences**: Impact of the decision
- **Alternatives Considered**: Other options evaluated
