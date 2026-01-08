# Quality Gates

A PR is mergeable only if all gates pass.

## Required Gates (CI)

| Gate | Command | Failure Action |
|------|---------|----------------|
| Format | `cargo fmt --all -- --check` | Block merge |
| Lint | `cargo clippy --workspace --all-targets --all-features` | Block merge (warnings as errors) |
| Test | `cargo test --workspace --all-features` | Block merge |
| License | `cargo deny check licenses` | Block merge |
| Advisories | `cargo deny check advisories` | Block merge |
| Schema validation | Golden tests + meta-validation | Block merge |
| FFI smoke | C + Go + Python + TypeScript | Block merge |

## Review Gates (Humans)

### FFI Surface Changes

Any change to FFI surface requires explicit reviewer sign-off from:
- Lead maintainer
- Bindings maintainer (if it impacts a wrapper)

### Schema Changes

Any schema change requires:
- Schema version update
- Changelog note
- Golden test update
- Meta-validation pass (JSON Schema 2020-12)

## "Stop the Line" Conditions

These conditions MUST block all merges until resolved:

- Undefined memory ownership in FFI
- Non-deterministic process cleanup in timeout without being observable
- Schema drift (output no longer validates)
- Runtime validation bypass without ADR exemption

## References

- [SAFETY.md](../../SAFETY.md)
- [Schema Validation Policy](schema-validation-policy.md)
- [MAINTAINERS.md](../../MAINTAINERS.md)
