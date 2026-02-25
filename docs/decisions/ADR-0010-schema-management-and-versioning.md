# ADR-0010: Schema Management and Versioning

> **Status**: Proposed
> **Date**: 2025-12-31
> **Authors**: Dev Lead

## Context

As a project that produces and consumes structured data (JSON outputs, logs, filters), we require a formal strategy for managing and versioning schemas to provide stability and clarity for consumers. This strategy must cover hosting, organization, and a clear versioning policy that supports both initial development and long-term stability.

## Decision

We adopt the following conventions for all data schemas within the `sysprims` project.

### 1. Canonical Hosting

The single source of truth for schema definitions is `https://schemas.sysprims.dev`.

- The `$id` field within each schema must point to its canonical URL on this host.
- The path will be structured as `/<topic>/<schema-name>/<version>`.

### 2. Repository Structure

- Schemas are stored in the main repository under the `schemas/` directory.
- The directory structure mirrors the URL path: `schemas/<version>/<topic>/<schema-name>.schema.yaml`.
- Schemas will be authored in **YAML** to allow for comments and improve readability.

### 3. Versioning Strategy

To prevent confusion and tight coupling between the versions of the codebase and its schemas, we adopt a two-phase approach.

#### Phase 1: Alpha-Stage Versioning (`v0`, `v1`, etc.)

- During the initial, pre-1.0.0 development of the repository, schemas will use a simple, non-SemVer, monolithic version identifier (e.g., `v0`).
- This explicitly signals that the schema is unstable and subject to breaking changes.
- It decouples the schema's maturity from the repository's `0.y.z` version, as they may evolve at different rates.
- Example: `schemas/v0/observability/log-event.schema.yaml`

#### Phase 2: Post-1.0.0 Versioning (SemVer)

- Once the repository reaches a stable `1.0.0` release, schemas will transition to Semantic Versioning (e.g., `v1.0.0`, `v1.1.0`).
- Version bumps will follow standard SemVer rules:
  - **MAJOR** (`v1.0.0` -> `v2.0.0`): A breaking change (removing a field, changing a type, removing an enum value).
  - **MINOR** (`v1.0.0` -> `v1.1.0`): A non-breaking, additive change (adding a new optional field, adding an enum value).
  - **PATCH** (`v1.0.0` -> `v1.0.1`): A non-breaking change that does not alter the structure (e.g., fixing a typo in a `description`).
- Example: `schemas/v1.1.0/proc/process-info.schema.yaml`

## Consequences

- **Positive**:
  - Schema versions are decoupled from the repository version during early development, providing flexibility.
  - The `v0` convention clearly communicates instability to consumers.
  - A clear plan exists for migrating to a stable, predictable SemVer strategy post-1.0.0.
  - The schema host provides a stable, long-term location for schema definitions.
- **Negative**:
  - Requires a conscious transition from the `v0` convention to the SemVer convention at the `1.0.0` milestone.
- **Neutral**:
  - Build tooling (`goneat`) will be configured to validate schemas from this structure.

## Alternatives Considered

- **Use Repository SemVer for Schemas**: Tightly couple schema versions with the crate versions (e.g., a schema change in `0.2.5` would also be version `0.2.5`). This was rejected because a minor code change might not warrant a schema change, and vice-versa, making the coupling confusing.
