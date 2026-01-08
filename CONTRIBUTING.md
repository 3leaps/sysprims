# Contributing to sysprims

## FFI & Memory Safety Standards
To support our multi-language bindings (Go, Python, TS), all FFI contributions must adhere to:

1. **Opaque Pointers**: Long-lived objects must remain in Rust memory. Return `*mut SysprimsHandle` to the caller.
2. **JSON String Returns**: Complex data structures must be returned as UTF-8 JSON strings conforming to the `fulmenhq/crucible` schemas.
3. **The Destructor Rule**: Any string allocated by Rust and passed to the FFI must be freed via `sysprims_free_string(char*)`. **Do not use C's free()**.
4. **Schema Stability**: All JSON outputs must include the `schema_id` field to support migration safety.

## CI Requirements
- **Musl-First**: Linux features must remain compatible with the `musl` target for Distroless environments.
- **Zero Warnings**: CI is configured with `RUSTFLAGS="-Dwarnings"`.