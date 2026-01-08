# Schema Validation Policy

This document defines the schema validation requirements for sysprims, ensuring structured, validated data exchange across all interfaces.

## Schema Standard

**Mandate**: All JSON schemas in sysprims MUST use **JSON Schema Draft 2020-12**.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://schemas.3leaps.dev/sysprims/timeout/v1.0.0/timeout-result.schema.json"
}
```

### Rationale

- JSON Schema 2020-12 is the current stable specification
- Improved vocabulary system for extension
- Better `$dynamicRef` for recursive schemas
- Clearer `unevaluatedProperties` semantics
- Supported by goneat `validate` and `validate-data` commands

### References

- [JSON Schema 2020-12 Specification](https://json-schema.org/specification-links.html#2020-12)
- [JSON Schema Core](https://json-schema.org/draft/2020-12/json-schema-core.html)
- [JSON Schema Validation](https://json-schema.org/draft/2020-12/json-schema-validation.html)

## Structured Data Exchange Mandate

**Policy**: All generally-occurring data exchange in sysprims MUST use structured, schema-validated formats.

### Covered Interfaces

| Interface | Input Validation | Output Validation |
|-----------|------------------|-------------------|
| CLI JSON output (`--json`) | N/A | Required |
| FFI return values | N/A | Required |
| Filter/query parameters | Required | N/A |
| Configuration files | Required | N/A |
| API request/response | Required | Required |

### Schema Identifier Embedding

Every JSON output MUST include a `schema_id` field:

```json
{
  "schema_id": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/process-info.schema.json",
  "timestamp": "2025-12-31T12:00:00Z",
  "processes": [...]
}
```

## Meta-Validation Requirements

**Policy**: All schemas authored for sysprims MUST be meta-validated against the JSON Schema 2020-12 meta-schema.

### CI Enforcement

```yaml
# In CI pipeline
- name: Meta-validate schemas
  run: |
    goneat validate --schema-dir schemas/ --meta-schema draft-2020-12
```

### Local Validation

```bash
# Validate schema files against JSON Schema 2020-12 meta-schema
goneat validate schemas/

# Validate data against a schema
goneat validate-data --schema schemas/timeout/v1.0.0/timeout-result.schema.json output.json
```

### Schema File Naming

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

## Runtime Validation Requirements

**Policy**: All data input MUST be validated at runtime. Output validation is required, but sysprims may exempt runtime jsonschema validation when ADR-approved (see ADR-0005) and replaced with CI/golden validation.

### Default Behavior

| Operation | Validation | Enforcement |
|-----------|------------|-------------|
| JSON output generation | No runtime jsonschema (ADR-0005) | CI/golden tests must catch drift |
| Filter/query parsing | Validate on parse | Hard error on invalid |
| Configuration loading | Validate on load | Hard error on invalid |
| FFI input strings | UTF-8 validation | Hard error on invalid |

### Exemption Process

Runtime validation may be exempted ONLY when:

1. **Performance impact is documented** - Measurable overhead demonstrated
2. **Risk profile is acceptable** - Static analysis or type system provides equivalent safety
3. **ADR is approved** - Maintainer approval documented in architecture decision record
4. **Monitoring is in place** - Telemetry captures validation bypass events

### Exemption ADR Template

```markdown
# ADR-NNNN: Runtime Validation Exemption for <Component>

## Context
<Why validation is being considered for exemption>

## Performance Analysis
<Benchmark data showing overhead>

## Risk Assessment
<How safety is maintained without runtime validation>

## Decision
<Specific exemption granted>

## Monitoring
<How exempted paths are observed>
```

## Golden Test Requirements

**Policy**: All schema-validated outputs MUST have corresponding golden tests.

### Golden Test Structure

```
tests/
├── golden/
│   ├── timeout-result/
│   │   ├── basic-timeout.json
│   │   ├── completed-normally.json
│   │   └── tree-kill-best-effort.json
│   └── process-info/
│       ├── single-process.json
│       └── process-tree.json
└── schema_validation_test.rs
```

### Golden Test Requirements

1. Each golden file MUST include `schema_id`
2. Golden files MUST validate against their declared schema
3. CI MUST fail if golden files become invalid

## Schema Versioning

Follow semantic versioning for schemas:

| Change | Version Bump | Example |
|--------|--------------|---------|
| Add optional field | Minor | v1.0.0 → v1.1.0 |
| Add enum value | Minor | v1.0.0 → v1.1.0 |
| Remove field | **Major** | v1.0.0 → v2.0.0 |
| Change field type | **Major** | v1.0.0 → v2.0.0 |
| Documentation only | Patch | v1.0.0 → v1.0.1 |

## SSOT Location

Schemas are Single Source of Truth (SSOT) in Crucible:

```
fulmenhq/crucible/
└── modules/
    └── sysprims/
        └── schemas/
            ├── timeout-result/v1.0.0.schema.json
            ├── process-info/v1.0.0.schema.json
            └── proc-filter/v1.0.0.schema.json
```

sysprims maintains local copies for CI validation. Crucible is authoritative.

## Tool Support

### goneat Integration

```bash
# Meta-validate all schemas
goneat validate schemas/

# Validate data against schema
goneat validate-data --schema schemas/timeout-result/v1.0.0.schema.json data.json

# Validate with specific draft
goneat validate --draft 2020-12 schemas/
```

### Rust Validation

Use `jsonschema` crate with 2020-12 support:

```rust
use jsonschema::{Draft, JSONSchema};

let schema = serde_json::from_str(SCHEMA_JSON)?;
let compiled = JSONSchema::options()
    .with_draft(Draft::Draft202012)
    .compile(&schema)?;

let result = compiled.validate(&instance);
```

## References

- [ADR-0005: Schema Contracts](../architecture/adr/0005-schema-contracts.md)
- [JSON Schema 2020-12](https://json-schema.org/draft/2020-12/release-notes.html)
- [goneat Documentation](https://github.com/fulmenhq/goneat)
- [Crucible Schema Standards](https://crucible.3leaps.dev/)
