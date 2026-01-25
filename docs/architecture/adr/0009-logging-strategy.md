# ADR-0009: Logging Strategy

> **Status**: Proposed
> **Date**: 2025-12-31
> **Authors**: Dev Lead

## Context

A clear logging strategy is required for both debugging and observability. As `sysprims` is both a library and a set of CLI tools, the strategy must differentiate between these two use cases to avoid imposing behavior on consumers.

## Decision

We will adopt a two-part strategy based on the `tracing` crate, which was included as an optional dependency in the original project proposal.

### 1. Library Logging: `tracing` Facade

All library crates (`sysprims-core`, `sysprims-timeout`, etc.) will use the `tracing` crate to emit structured events.

- **No Direct Logging:** Libraries will NOT initialize any `tracing` subscriber. They will only emit events (e.g., `tracing::info!`, `tracing::error!`).
- **Contextual Information:** Events will include contextual, key-value data (e.g., `pid`, `signal_name`) to allow for rich, filterable logs.
- **Consumer Responsibility:** It is the responsibility of the final application (the binary) to install a `tracing` subscriber to process, format, and output these events.

This approach ensures that `sysprims` does not interfere with an application's own logging or observability setup.

### 2. CLI Logging: Subscriber Implementation

The `sysprims-cli` crate, as an application, WILL initialize a `tracing` subscriber.

- **Default Format:** By default, the CLI will use a human-readable, colored format that logs to `stderr`. This is optimized for interactive use.
- **Structured Format:** The CLI will provide a command-line flag (e.g., `--log-format json`) to switch to a structured JSON output format. This is for automation, scripting, and integration with log collectors.
- **Log Level Control:** The log level will be controllable via a flag (e.g., `--log-level debug`) or an environment variable.

### 2.1 Stdout Purity Model

The CLI must keep a strict separation between machine-readable command output and logs:

- **Machine-readable outputs** (schema-versioned JSON produced by subcommands like `timeout --json`) go to `stdout`.
- **Logs** go to `stderr` (both human-readable and `--log-format json`).

Rationale: stdout is treated as a data channel for programmatic consumption, while stderr is reserved for diagnostics.

### 3. Structured Log Schema

When structured JSON logging is enabled, every log line will conform to a formal schema. The full versioning and management strategy for schemas is detailed in [ADR-0010](./0010-schema-management-and-versioning.md).

- **Schema Definition:** Schemas are authored in YAML for readability. The schema is defined at `schemas/v0/observability/log-event.schema.yaml`.
- **Schema ID:** Each JSON log object will contain a `schema_id` field (`https://schemas.sysprims.dev/observability/log-event/v0`) for versioning, reflecting the `v0` (alpha) status.
- **Core Fields:** The schema will include fields like `timestamp`, `level`, `target` (module path), `message`, and a `fields` object for structured key-value data.

## Consequences

- **Positive**:
    - Library is well-behaved and does not hijack application logging.
    - CLI provides both human-friendly and machine-readable logging formats.
    - Structured logging is consistent and contract-driven via a schema.
- **Negative**:
    - Adds a `tracing` dependency to the library's public API surface (though it's a well-established standard).
    - Requires schema maintenance.
- **Neutral**:
    - Consumers of the library must use a `tracing`-compatible subscriber to see logs.

## Alternatives Considered

- **`log` Crate**: The `log` crate is a simpler facade, but `tracing` provides better support for structured, contextual data and asynchronous operations, which will be relevant for future features like `sysprims-waitfor`.
- **Direct `stderr` Logging**: Logging directly to `stderr` from the library is considered bad practice in the Rust ecosystem and was rejected.
- **No Schema for Logs**: Omitting a schema for JSON logs would violate the project's core principle of schema-driven contracts.
