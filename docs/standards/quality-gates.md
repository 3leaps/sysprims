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

Note: The repo includes a minimal root `go.mod` plus `go.work` to support repo-root Go tooling (goneat/golangci-lint).
Go sources live under `bindings/go/sysprims/`, plus a small placeholder package under `internal/gowork/`.

Similarly, we include a minimal repo-root `package.json` as a tooling shim so repo-root npm invocations (from tooling like goneat) do not fail.

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

## CI Workflow Strategy

### Core CI (`ci.yml`)

Runs on every push to `main` and on PRs. Validates Rust code quality:
- Format, lint, test, license checks
- Cross-platform matrix (Linux, macOS, Windows)

### Binding Workflows (Manual Trigger Only)

The `go-bindings.yml` and `typescript-bindings.yml` workflows are **manually triggered** (`workflow_dispatch` only). They do not run on push or PR.

**Rationale:**
- Binding validation requires building FFI shared libraries, which is expensive
- Running on every push causes CI thrashing with no benefit during active development
- Bindings are validated as a pre-release step, not on every commit
- This keeps the feedback loop fast for core Rust development

**When to run binding workflows:**
1. Before tagging a release (validates bindings work with current FFI)
2. After significant FFI changes (manual verification)
3. When debugging binding-specific issues

### Release Workflow (`release.yml`)

Triggered manually or by tag push. Builds all artifacts and publishes releases.

### Release Validation (`validate-release.yml`)

Optional post-publish smoke test to verify released artifacts are accessible and functional.

**What it validates:**
- Go bindings: Module is fetchable via `go get`
- TypeScript bindings: FFI bundle downloads and tests pass

**When to run:**
- After publishing a release (manual spot-check)
- When investigating user-reported download/installation issues

This workflow is separate from binding workflows because it tests the *published release assets*, not the source code. Binding workflows validate "does the code work?" while this validates "can users download and use the release?"

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
