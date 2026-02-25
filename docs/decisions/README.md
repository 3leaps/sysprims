# Decision Records

This directory contains architectural, design, and security decision records for sysprims.

## Record Types

| Prefix | Type                         | Purpose                        |
| ------ | ---------------------------- | ------------------------------ |
| ADR    | Architecture Decision Record | Technical architecture choices |
| DDR    | Design Decision Record       | API design, data structures    |
| SDR    | Security Decision Record     | Security-related decisions     |

## Index

### Architecture Decision Records (ADR)

| ID                                                                | Title                                     | Status   |
| ----------------------------------------------------------------- | ----------------------------------------- | -------- |
| [ADR-0000](ADR-0000-adr-process.md)                               | ADR Process                               | Accepted |
| [ADR-0001](ADR-0001-license-policy.md)                            | License Policy                            | Accepted |
| [ADR-0002](ADR-0002-crate-structure.md)                           | Crate Structure                           | Accepted |
| [ADR-0003](ADR-0003-group-by-default.md)                          | Group-by-Default                          | Accepted |
| [ADR-0004](ADR-0004-ffi-design.md)                                | FFI Design                                | Accepted |
| [ADR-0005](ADR-0005-schema-contracts.md)                          | Schema Contracts                          | Accepted |
| [ADR-0006](ADR-0006-dependency-governance.md)                     | Dependency Governance                     | Accepted |
| [ADR-0007](ADR-0007-platform-abstraction.md)                      | Platform Abstraction                      | Accepted |
| [ADR-0008](ADR-0008-error-handling.md)                            | Error Handling                            | Accepted |
| [ADR-0009](ADR-0009-logging-strategy.md)                          | Logging Strategy                          | Accepted |
| [ADR-0010](ADR-0010-schema-management-and-versioning.md)          | Schema Management and Versioning          | Accepted |
| [ADR-0011](ADR-0011-pid-validation-safety.md)                     | **PID Validation Safety**                 | Accepted |
| [ADR-0012](ADR-0012-language-bindings-distribution.md)            | Language Bindings Distribution            | Accepted |
| [ADR-0013](ADR-0013-release-asset-publishing-and-verification.md) | Release Asset Publishing and Verification | Accepted |
| [ADR-0014](ADR-0014-ffi-artifact-groups-and-binding-consumers.md) | FFI Artifact Groups and Binding Consumers | Accepted |

### Design Decision Records (DDR)

| ID         | Title | Status |
| ---------- | ----- | ------ |
| _None yet_ |       |        |

### Security Decision Records (SDR)

| ID         | Title | Status |
| ---------- | ----- | ------ |
| _None yet_ |       |        |

## Creating a New Record

1. Copy the appropriate template:
   - `adr-template.md` for architecture decisions
   - `ddr-template.md` for design decisions
   - `sdr-template.md` for security decisions

2. Name the file: `<TYPE>-<NNNN>-<slug-form-title>.md`
   - Example: `ADR-0015-new-feature.md`

3. Fill in the template sections

4. Update this README index

5. Submit for review

## Decision Lifecycle

```
Proposed -> Accepted -> (Deprecated | Superseded)
```

- **Proposed**: Under discussion, not yet approved
- **Accepted**: Approved and in effect
- **Deprecated**: No longer recommended but not replaced
- **Superseded**: Replaced by another decision (link to successor)
