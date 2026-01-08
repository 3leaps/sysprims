# Schema Contracts

Schema contracts for sysprims JSON outputs.

## SSOT Location

Schemas are authored and versioned in Crucible (fulmenhq/crucible) under the sysprims module. This repository consumes them and validates runtime outputs against them.

## Schema Specification

All schemas MUST use **JSON Schema Draft 2020-12**. See [Schema Validation Policy](schema-validation-policy.md) for full requirements.

## Rules

1. Every machine-readable output includes `schema_id`
2. Schema versioning follows semver:
   - **major**: breaking changes (field removal, type change)
   - **minor**: additive optional fields or new enum values
   - **patch**: documentation-only fixes
3. Schemas are meta-validated against JSON Schema 2020-12

## Validated Outputs

| Schema | Direction | Validation |
|--------|-----------|------------|
| `timeout-result` | Output | Required |
| `process-snapshot` | Output | Required |
| `proc-filter` | Input | Strict (no unknown keys) |

## CI Expectations

- Golden outputs validate against schemas
- Schema IDs are pinned and reviewed in PRs
- Meta-validation runs on all schema files

## References

- [ADR-0005: Schema Contracts](../architecture/adr/0005-schema-contracts.md)
- [Schema Validation Policy](schema-validation-policy.md)
