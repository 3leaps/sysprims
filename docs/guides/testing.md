# Testing Strategy

This project's value depends on correctness across OS boundaries.

## Test Layers

### Unit Tests

- Duration parsing
- Signal mapping
- Error mapping (library <-> CLI <-> FFI)
- JSON serialization

### Integration Tests

Spawn subprocess trees and validate:
- Timeout escalation behavior
- Group-by-default tree-kill behavior
- `--foreground` (opt-out) behavior

OS-specific tests:
- Windows: Job Object behavior
- Unix: Process group signaling

### Golden Tests

- Validate JSON outputs against SSOT schemas (JSON Schema 2020-12)
- Golden JSON snapshots MUST include `schema_id`
- Meta-validate schemas in CI

### FFI Smoke Tests (Required)

**C**:
- Compile/link against `libsysprims`
- Call version function
- Run minimal timeout call
- Free returned string via `sysprims_free_string()`

**Go/Python/TypeScript**:
- Can call the library
- UTF-8 strings round-trip correctly
- Memory ownership/free is correct

## Non-Negotiable: Tree Escape Test

Per OS, include a test that:

1. Spawns a child that spawns grandchildren
2. Grandchildren attempt to detach, ignore signals, or outlive parent
3. Assert `sysprims-timeout` terminates the entire tree under Group-by-Default policy
4. Record `tree_kill_reliability` ("guaranteed" or "best_effort")

This is the **core differentiator** - see [ADR-0003](../architecture/adr/0003-group-by-default.md).

## CI Matrix

| Platform | Variants |
|----------|----------|
| Linux | musl, glibc |
| macOS | arm64, x86_64 |
| Windows | x86_64 |

## References

- [SAFETY.md](../../SAFETY.md)
- [Quality Gates](../standards/quality-gates.md)
- [ADR-0003: Group-by-Default](../architecture/adr/0003-group-by-default.md)
