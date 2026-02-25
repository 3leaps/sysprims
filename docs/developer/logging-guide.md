# Developer Guide: Logging with `tracing`

This guide explains how to add logging to the `sysprims` codebase. It complements [ADR-0009: Logging Strategy](../architecture/adr/0009-logging-strategy.md), which covers the high-level decisions.

## Guiding Principles

1.  **Instrument Libraries, Initialize in Binaries**: All `sysprims-*` library crates should only _emit_ `tracing` events. The `sysprims-cli` binary is responsible for _initializing_ the subscriber that processes them.
2.  **Structure is Key**: We use structured logging for machine-readability. Always prefer adding key-value fields over embedding complex data in the log message.
3.  **Consistency Matters**: Use the conventional field names defined in this guide to ensure all log events are consistent and easily queryable.

## How to Add Logging

### 1. Adding an Event

To log a specific event, use one of the `tracing` macros (`trace!`, `debug!`, `info!`, `warn!`, `error!`). Include relevant data as key-value pairs.

**Example**:

```rust
use tracing::info;

// Inside a function...
info!(
    pid = 1234,
    signal_name = "TERM",
    "Successfully sent signal"
);
```

This creates a log event with a clear message and two structured fields (`pid` and `signal_name`).

### 2. Instrumenting a Function

To trace the execution of an entire function (creating a "span" that times it and associates all events within it), use the `#[instrument]` attribute.

- Add the attribute directly above the function.
- `level` controls the verbosity of the entry/exit logs. `info` or `debug` is common.
- Use `skip(...)` to prevent large or non-debuggable arguments from being logged.
- Add `fields(...)` to add parameters as structured data to the span.

**Example**:

```rust
use tracing::instrument;

#[instrument(
    level = "debug",
    skip(config),
    fields(
        duration_ms = config.duration.as_millis(),
        signal = ?config.signal
    )
)]
fn run_with_timeout(config: &TimeoutConfig) {
    // ... function logic ...
}
```

## Conventional Field Names

To ensure consistency across the codebase, please use the following standard key names for common data points. If a suitable key doesn't exist, feel free to add a new one, but consider updating this guide.

| Key Name      | Type           | Description                           | Example                    |
| ------------- | -------------- | ------------------------------------- | -------------------------- |
| `pid`         | `u32`          | A process identifier.                 | `pid = 1234`               |
| `pgid`        | `u32`          | A process group identifier (on Unix). | `pgid = 1234`              |
| `signal_name` | `&str`         | The name of a signal being sent.      | `signal_name = "TERM"`     |
| `signal_code` | `i32`          | The numeric code of a signal.         | `signal_code = 15`         |
| `duration_ms` | `u64` / `u128` | A duration in milliseconds.           | `duration_ms = 5000`       |
| `path`        | `&str`         | A file or directory path.             | `path = "/usr/bin/sleep"`  |
| `command`     | `&str`         | The command being executed.           | `command = "npm test"`     |
| `exit_code`   | `i32`          | The exit code of a completed process. | `exit_code = 0`            |
| `error_code`  | `&str` / `i32` | An internal error code or identifier. | `error_code = "E_NO_PERM"` |
| `host`        | `&str`         | A hostname or IP address.             | `host = "127.0.0.1"`       |
| `port`        | `u16`          | A network port.                       | `port = 8080`              |

## Payload Guidance

- **DO** log identifiers, state changes, and configuration values.
- **DO NOT** log large data blobs (e.g., file contents, large buffers). This can severely impact performance. Log the _metadata_ about the blob instead (e.g., `byte_count = 1024`).
- **DO NOT** log sensitive information (passwords, tokens, personal data).
- For complex types to be logged in JSON format, they must implement `serde::Serialize`.
