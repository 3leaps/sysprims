# Code Standards

These standards are optimized for correctness, portability, and stable interfaces.

## Rust Style

- `rustfmt` is required
- Prefer explicit types at module boundaries
- Prefer small modules: one responsibility per module
- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)

## Error Handling

Define a small, stable error enum in `sysprims-core`:

```rust
pub enum SysprimsError {
    InvalidArgument { message: String },
    NotSupported { feature: String, platform: String },
    PermissionDenied { pid: u32, operation: String },
    NotFound { pid: u32 },
    Io { source: std::io::Error },
    Internal { message: String },
}
```

Map errors to:

- CLI exit codes (GNU-compatible)
- FFI `SysprimsErrorCode`
- JSON error objects (schema-backed)

See [ADR-0008: Error Handling](../architecture/adr/0008-error-handling.md).

## Unsafe Code Policy

- Unsafe is allowed **only** in platform/FFI boundary modules
- Every `unsafe` block MUST have a comment explaining:
  - Why unsafe is required
  - What invariants are relied on
  - How it is tested

```rust
// SAFETY: ptr is guaranteed non-null by caller contract.
// Invariant: buffer has at least `len` bytes allocated.
// Tested: ffi_null_ptr_test, ffi_buffer_overflow_test
unsafe { ... }
```

## Feature Flags

- Default features MUST remain minimal
- Any "heavier" dependency MUST be feature-gated and off by default

```toml
[features]
default = []
sysinfo-backend = ["sysinfo"]
tracing = ["dep:tracing"]
```

## Cross-Platform Invariants

"Group-by-default" MUST behave consistently:

| Platform | Mechanism      | Guarantee                    |
| -------- | -------------- | ---------------------------- |
| Windows  | Job Objects    | Guaranteed (when assignable) |
| Unix     | Process groups | Guaranteed                   |

If a platform limitation prevents guarantees, the limitation MUST be:

- Observable in structured output (`tree_kill_reliability` field)
- Documented
- Tested

See [ADR-0003: Group-by-Default](../architecture/adr/0003-group-by-default.md).

## FFI Standards

- All FFI strings are UTF-8
- Ownership MUST be explicit:
  - Functions returning `char*` require `sysprims_free_string()`
- Avoid NULL-terminated arrays; prefer `len` + pointer
- ABI changes require versioning discipline and smoke tests

See [ADR-0004: FFI Design](../architecture/adr/0004-ffi-design.md).

## JSON Output Standards

- Every JSON output MUST include `schema_id`
- All schemas MUST use JSON Schema 2020-12
- Runtime validation is required (see [Schema Validation Policy](schema-validation-policy.md))

## Output Hygiene

Follow [Crucible Coding Baseline](https://crucible.3leaps.dev/coding/baseline):

| Stream | Purpose                          |
| ------ | -------------------------------- |
| STDOUT | Structured data output (JSON)    |
| STDERR | Diagnostic output (logs, errors) |

## References

- [Repository Conventions](repository-conventions.md)
- [Schema Validation Policy](schema-validation-policy.md)
- [Crucible Coding Baseline](https://crucible.3leaps.dev/coding/baseline)
