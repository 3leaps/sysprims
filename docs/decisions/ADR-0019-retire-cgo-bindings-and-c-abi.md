# ADR-0019: Retire the cgo Go Bindings and the C-ABI

> **Status**: Accepted
> **Date**: 2026-11-16
> **Authors**: sysprims maintainers
> **Supersedes**: ADR-0004, ADR-0014; in part ADR-0002, ADR-0008, ADR-0012, ADR-0013, ADR-0015, ADR-0016, ADR-0017, ADR-0018
> **Superseded by**: none

## Context

Through v0.4.0, sysprims shipped two surfaces built on a C-ABI:

- `sysprims-ffi` (`ffi/sysprims-ffi`), a C-ABI crate with a cbindgen-generated
  `sysprims.h` header, published as prebuilt static and shared library bundles
  (ADR-0004, ADR-0014).
- The cgo Go bindings module `github.com/3leaps/sysprims/bindings/go/sysprims`,
  which linked committed prebuilt static libraries and was tagged
  `bindings/go/sysprims/vX.Y.Z` (ADR-0012, ADR-0017).

ADR-0018 placed pure-Go (`CGO_ENABLED=0`) packages in the root module
`github.com/3leaps/sysprims`. In v0.5.0 the root module gains pure-Go
`syserror`, `signal`, `session`, `timeout`, and `proc` packages, which cover
the cgo bindings' process primitives.

The other language bindings do not use the C-ABI. The Python binding
(`sysprims-py`, PyO3, first released in v0.4.0) and the TypeScript binding
(`sysprims-ts-napi`, napi-rs, since v0.1.7) depend on the Rust crates
directly. TypeScript loaded the C-ABI through koffi only in v0.1.4 through
v0.1.6.

## Decision

1. Retire `bindings/go/sysprims` in v0.5.0. Go consumers use the pure-Go
   packages in the root module `github.com/3leaps/sysprims` (ADR-0018), which
   requires Go 1.26.0 or later. `go/recurrence` remains its own module with
   the `go/recurrence/vX.Y.Z` tag (ADR-0017).
2. Retire `sysprims-ffi`, its generated `sysprims.h` header, and the prebuilt
   FFI library bundles in v0.5.0. They are no longer built or shipped.
3. Python and TypeScript continue to bind the Rust crates directly through
   PyO3 and napi-rs.
4. Cross-language parity is carried by shared conformance vectors under
   `testdata/conformance/` (`signal`, `session`, `timeout`, `proc`), verified
   by Rust, Go, Python, and TypeScript.

## Consequences

### Positive

- Go consumers need no C toolchain and no native library archives on any
  platform.
- Releases no longer build, commit, or publish native libraries or headers.

### Negative

- Consumers of `github.com/3leaps/sysprims/bindings/go/sysprims` must move to
  the root-module packages to receive v0.5.0 and later changes.
- Consumers of the C-ABI have no supported replacement beyond the Rust crates
  and the Python and TypeScript packages.

### Neutral

- No `bindings/go/sysprims/vX.Y.Z` tag is created from v0.5.0. The v0.4.0 and
  earlier bindings tags remain resolvable for existing pins.
- The release tag set is `vX.Y.Z` plus `go/recurrence/vX.Y.Z`.

## Alternatives Considered

### Alternative 1: Keep the cgo bindings alongside the pure-Go packages

Rejected. Two Go implementations of the same primitives would need separate
maintenance, native library builds for every platform, and parity checks
against each other.

## References

- [ADR-0004: FFI Design](ADR-0004-ffi-design.md)
- [ADR-0012: Language Bindings Distribution](ADR-0012-language-bindings-distribution.md)
- [ADR-0014: FFI Artifact Groups and Binding Consumers](ADR-0014-ffi-artifact-groups-and-binding-consumers.md)
- [ADR-0017: Recurrence Go Module and Release Tag Set](ADR-0017-recurrence-go-module-and-release-tags.md)
- [ADR-0018: Native Process Evidence Surfaces and Root Pure-Go Module](ADR-0018-native-process-evidence-and-root-go-module.md)
- [v0.5.0 release notes](../releases/v0.5.0.md)
