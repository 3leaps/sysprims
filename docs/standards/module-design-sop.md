# Module Design Standard Operating Procedure

**Status:** Active
**Version:** 1.0
**Applies to:** All sysprims modules and CLI subcommands

---

## Purpose

This SOP defines the required process for designing, implementing, and documenting sysprims modules. It ensures:

- Consistent documentation across all modules
- Traceability from requirements to implementation to tests
- License hygiene with auditable provenance
- Cross-platform correctness

## Scope

This SOP applies to:

- All library crates (`sysprims-*`)
- CLI subcommands (`sysprims <subcommand>`)
- FFI exports and language bindings

## Required Artifacts

Each module MUST have the following artifacts before release:

| Artifact | Location | Purpose |
|----------|----------|---------|
| Module Spec | `docs/design/<module>/<module>-spec.md` | API contract and design rationale |
| Equivalence Tests | `docs/design/<module>/<module>-equivalence-tests.md` | Test protocol and acceptance criteria |
| Compliance Report | `docs/design/<module>/<module>-compliance.md` | Evidence that requirements are met |
| Provenance | `docs/design/<module>/<module>-provenance.md` | Sources consulted (and avoided) |

Templates for each artifact are in `docs/templates/module-design/`.

## Lifecycle Gates

### Gate A: Initiate (Before Code)

- [ ] Create module spec from template
- [ ] Declare normative references (POSIX, OS docs, etc.)
- [ ] Define required interfaces (Rust API, CLI, FFI)
- [ ] Create equivalence test protocol
- [ ] Create provenance document listing sources

### Gate B: Implement

- [ ] Implement library API first (CLI is thin wrapper)
- [ ] Follow ADR-0008 error handling
- [ ] Follow ADR-0011 PID validation (for signal-sending code)
- [ ] JSON outputs include `schema_id` per ADR-0005
- [ ] Platform-specific code uses `#[cfg()]` appropriately

### Gate C: Verify

- [ ] Unit tests pass
- [ ] Integration tests pass
- [ ] Equivalence tests pass (where applicable)
- [ ] `cargo deny check licenses` passes
- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy` passes

### Gate D: Review

- [ ] Spec accurately reflects implementation
- [ ] Compliance report has evidence links
- [ ] Provenance document is complete
- [ ] No undocumented deviations from spec

### Gate E: Release

- [ ] All gates above complete
- [ ] Compliance report updated with CI artifact links
- [ ] Version numbers aligned

## PR Checklist

If a PR changes any public surface area (Rust API, CLI flags, JSON output, FFI), it MUST:

- [ ] Update the module spec (bump version if breaking)
- [ ] Update equivalence test protocol (add new cases)
- [ ] Add/update tests
- [ ] Update compliance report

## Error Codes

Per ADR-0008, all modules use consistent error semantics:

| Code | Name | Meaning |
|------|------|---------|
| 0 | Success | Operation completed |
| 1 | InvalidArgument | Bad input, validation failed |
| 2 | SpawnFailed | Failed to spawn a child process |
| 3 | Timeout | Operation timed out |
| 4 | PermissionDenied | Access not allowed |
| 5 | NotFound | Resource doesn't exist |
| 6 | NotSupported | Platform doesn't support feature |
| 7 | GroupCreationFailed | Process group/job creation failed |
| 8 | System | Unexpected system error (errno/GetLastError) |
| 99 | Internal | Internal error (bug/unexpected state) |

## CLI Exit Codes

Per GNU conventions (where applicable):

| Condition | Exit Code |
|-----------|-----------|
| Success | 0 |
| General error | 1 |
| Timeout occurred | 124 |
| Tool itself failed | 125 |
| Command not executable | 126 |
| Command not found | 127 |
| Killed by signal N | 128+N |

## Schema Contracts

All JSON outputs MUST include a `schema_id` field per ADR-0005.

Schema ID format:
```
https://schemas.3leaps.dev/sysprims/<module>/v<major>.<minor>.<patch>/<type>.schema.json
```

## References

- [ADR-0001: License Policy](../architecture/adr/0001-license-policy.md)
- [ADR-0004: FFI Design](../architecture/adr/0004-ffi-design.md)
- [ADR-0005: Schema Contracts](../architecture/adr/0005-schema-contracts.md)
- [ADR-0007: Platform Abstraction](../architecture/adr/0007-platform-abstraction.md)
- [ADR-0008: Error Handling](../architecture/adr/0008-error-handling.md)
- [ADR-0011: PID Validation Safety](../architecture/adr/0011-pid-validation-safety.md)

---

*Last updated: 2026-01-09*
