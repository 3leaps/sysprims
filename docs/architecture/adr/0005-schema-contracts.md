# ADR-0005: Schema Contracts

> **Status**: Accepted  
> **Date**: 2025-12-31  
> **Authors**: Architecture Council

## Context

sysprims outputs JSON for automation (`--json` flags, FFI returns). Consumers need:

1. **Stability**: Field names and types shouldn't change unexpectedly
2. **Versioning**: Clear indication of schema version
3. **Validation**: Ability to verify outputs conform to contract
4. **Migration**: Path forward when schemas evolve

Without formal contracts, each binding might interpret outputs differently, leading to fragmentation.

## Decision

### Schema Location (SSOT)

sysprims schemas are owned by sysprims and published under the 3leaps schema host:

```
https://schemas.3leaps.dev/sysprims/
```

Within this repository, schemas live under `schemas/` using a filename-based path scheme:

```
schemas/
├── timeout/
│   └── v1.0.0/
│       └── timeout-result.schema.json
└── process/
    └── v1.0.0/
        ├── process-info.schema.json
        └── process-filter.schema.json
```

CI validates outputs against these local schema files (via goneat).

### Schema ID Embedding

Every JSON output includes a `schema_id` field:

```json
{
  "schema_id": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/process-info.schema.json",
  "timestamp": "2025-12-31T12:00:00Z",
  "processes": [...]
}
```

This enables:
- Runtime version detection
- Binding routing to correct parser
- Audit trail for debugging

### Version Format

Schema versions follow `vMAJOR.MINOR.PATCH`:

| Change Type | Version Bump |
|-------------|--------------|
| Add optional field | Minor |
| Add enum value | Minor |
| Remove required field | **Major** |
| Change field meaning | **Major** |
| Remove enum value | **Major** |
| Change field type | **Major** |
| Documentation only | Patch |

### Compatibility Guarantees

**Within major version**: All outputs remain valid against earlier minor schemas.

**Example**:
- v1.0.0 output validates against v1.0.0 schema ✓
- v1.1.0 output validates against v1.0.0 schema ✓ (new fields ignored)
- v2.0.0 output may NOT validate against v1.x schema

### CI Validation

Every sysprims build:
1. Runs golden tests with expected JSON outputs
2. Validates outputs against schemas
3. Fails if schema_id is missing or invalid

```yaml
# CI step
- name: Validate schemas
  run: |
    cargo test --test golden_tests
    ./scripts/validate-schemas.sh
```

### Deprecation Process

1. Field deprecated in minor version (docs + warning)
2. Field removed in next major version
3. Migration guide published with major release

### Filter Validation

The `proc-filter` schema defines allowed filter keys. Unknown keys result in `SYSPRIMS_ERR_INVALID_ARGUMENT`:

```json
{
  "name_contains": "nginx",    // ✓ Valid
  "cpu_above": 50,             // ✓ Valid
  "custom_field": "foo"        // ✗ Error: unknown key
}
```

This prevents bindings from inventing incompatible filter dialects.

## Consequences

### Positive

- Clear contracts between sysprims and consumers
- Schema version visible at runtime
- Validation catches breaking changes early
- Migration path is explicit

### Negative

- Schema maintenance overhead
- Breaking changes require major version
- schema_id adds bytes to every output

### Neutral

- Schema hosting becomes part of release process
- Bindings can implement validation helpers
- Schema evolution rules must be documented

## Alternatives Considered

### Alternative 1: No Formal Schema

Document JSON structure in README only.

**Rejected**: Too easy for outputs to drift from documentation. No programmatic validation.

### Alternative 2: Schemas in fulmenhq/crucible

Host sysprims schemas inside fulmenhq/crucible.

**Rejected**: sysprims is a 3leaps-owned project and uses `schemas.3leaps.dev` as the SSOT host. fulmenhq/crucible schemas remain separate ecosystem contracts.

### Alternative 3: No Embedded schema_id

Keep schema version external (in docs/version matrix).

**Rejected**: Runtime version detection is valuable for bindings handling multiple sysprims versions.

### Alternative 4: GraphQL Schema

Use GraphQL SDL for contract definition.

**Rejected**: Overkill for simple JSON outputs. JSON Schema is sufficient and widely supported.

## Schema Specification

All sysprims schemas MUST use **JSON Schema Draft 2020-12**:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://schemas.3leaps.dev/sysprims/timeout/v1.0.0/timeout-result.schema.json"
}
```

See [Schema Validation Policy](../../standards/schema-validation-policy.md) for:
- Meta-validation requirements
- Runtime validation policy
- Exemption process

## References

- [JSON Schema 2020-12](https://json-schema.org/draft/2020-12/release-notes.html)
- [Semantic Versioning](https://semver.org/)
- [Schema host](https://schemas.3leaps.dev/sysprims/)
- [Schema Validation Policy](../../standards/schema-validation-policy.md)
