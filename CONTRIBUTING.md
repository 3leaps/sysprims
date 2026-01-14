# Contributing to sysprims

Thanks for helping build **sysprims** — GPL-free process utilities + embeddable APIs.

This repo aims to be:
- **small and predictable** (library-first, schema-backed),
- **cross-platform** (Linux/macOS/Windows),
- **license-clean** (no GPL/AGPL, tight dependency policy),
- **binding-friendly** (Go/Python/TypeScript).

## Quick start (contributors)

1) Install Rust (stable) and the repo toolchain:

- `rustup toolchain install stable`
- `rustup component add rustfmt clippy`

2) Run the full local quality loop:

- `cargo fmt --all`
- `cargo clippy --workspace --all-targets --all-features`
- `cargo test --workspace --all-features`

> CI is the source of truth; see `docs/QUALITY_GATES.md`.

## How to contribute

### Issues
- **Bug reports**: include OS, architecture, Rust version, expected vs actual behavior, and minimal repro steps.
- **Feature requests**: explain the use case, expected CLI behavior, and any schema impacts.

### Pull requests
PRs should be small, focused, and include tests.

**PR checklist**
- [ ] Tests added or updated (unit/integration/golden/ffi-smoke)
- [ ] `cargo fmt` clean
- [ ] `cargo clippy` clean
- [ ] JSON outputs updated *with schema_id* if applicable
- [ ] Docs updated (if behavior changed)

## Dependency and licensing policy (high-level)

- Allowed: MIT / Apache-2.0 / BSD / ISC / 0BSD (see CI allowlist)
- Disallowed: GPL / AGPL
- Avoid: LGPL unless explicitly reviewed and documented

If you propose a new dependency:
- explain why it is needed
- prefer `default-features = false`
- consider a feature-gated "heavy" option instead of a hard dependency

## Code of Conduct and Security

This project follows organization-wide policies. See SECURITY and code-of-conduct references in the org policy repository (linked from the repo README).

If you find a security issue, **do not open a public issue**; follow the security policy for private disclosure.

## Maintainers

Maintainers may request:
- API/FFI adjustments to preserve stability
- schema versioning updates
- additional cross-platform tests
