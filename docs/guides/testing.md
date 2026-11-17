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

### Cross-Language Conformance (Required)

Rust, Go, Python, and TypeScript verify the shared vectors under
`testdata/conformance/` (`signal`, `session`, `timeout`, `proc`):

- `make signal-crosscheck`, `make session-crosscheck`, `make timeout-crosscheck`,
  and `make proc-crosscheck` compare Rust and Go output
- Python and TypeScript test suites verify the same vectors

## Non-Negotiable: Containment Test

Per OS, include a test that:

1. Spawns a child that spawns grandchildren
2. In-containment grandchildren ignore the graceful signal or outlive the leader
3. Assert final escalation targets all remaining in-group/Job members
4. Record `tree_kill_reliability` ("guaranteed", "unproven", or "best_effort")

Test Unix session/group escape separately as a documented cooperative-group
limitation; do not interpret `guaranteed` as non-escape. This is the **core
differentiator** - see [ADR-0003](../architecture/adr/0003-group-by-default.md).

## CI Matrix

| Platform | Variants      |
| -------- | ------------- |
| Linux    | musl, glibc   |
| macOS    | arm64, x86_64 |
| Windows  | x86_64        |

## References

- [SAFETY.md](../../SAFETY.md)
- [Quality Gates](../standards/quality-gates.md)
- [ADR-0003: Group-by-Default](../architecture/adr/0003-group-by-default.md)
