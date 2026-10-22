# ADR-0017: Recurrence Go Module and Release Tag Set

> **Status**: Accepted
> **Date**: 2026-10-22
> **Authors**: sysprims maintainers
> **Supersedes**: none
> **Superseded by**: none

## Context

sysprims publishes a Go binding as a module in a subdirectory,
`github.com/3leaps/sysprims/bindings/go/sysprims`, tagged
`bindings/go/sysprims/vX.Y.Z` beside the canonical `vX.Y.Z` tag at the same
commit (ADR-0012 §6.1).

The recurrence rules (`contract: recurrence/v0`) are implemented twice: in the
Rust crate `sysprims-recurrence`, and in pure Go. The Go implementation must
build with `CGO_ENABLED=0` and must not depend on the cgo binding or its
prebuilt native libraries, so that pure-Go consumers can import it without a C
toolchain and without platform archives.

A general convention for Go modules in multi-module repositories, under
discussion separately, prefers a single module at the repository root with a
single version tag. sysprims cannot follow it for recurrence without either
coupling the pure-Go code to the cgo binding or moving the existing binding.

## Decision

1. The pure-Go recurrence implementation is its own module,
   `github.com/3leaps/sysprims/go/recurrence`, with its own `go.mod`, in the
   `go/recurrence/` directory. It does not import the cgo binding, sets no
   `replace` directives, and is not part of a `go.work` workspace; CI tests it
   with `GOWORK=off` and `CGO_ENABLED=0`, including from a copy outside the
   repository.
2. The module is tagged `go/recurrence/vX.Y.Z`, the directory-prefixed form
   Go uses to resolve versions of a module in a subdirectory.
3. A release tags one commit with every name in its tag set, all at the same
   version: `vX.Y.Z`; `bindings/go/sysprims/vX.Y.Z` when the release plan
   publishes the Go binding; and `go/recurrence/vX.Y.Z` whenever
   `go/recurrence/go.mod` is present in the tagged tree. The prefixes are
   listed in `config/release/release-tags.txt`; the tag guard, the provenance
   check, the tag-message workflow step and the signed-tag ceremony all derive
   the set from it the same way.
4. Every tag in the set is annotated, signed by the committed signing subkey,
   carries its own reviewed message, and peels to the one release commit. A
   tag that points elsewhere fails the guard.
5. This is a scoped departure from the single-module, single-tag convention,
   for this module only. The existing binding keeps its path and tag. A later
   change to either layout is a new decision, not an extension of this one.

## Consequences

### Positive

- Pure-Go consumers import recurrence with no cgo, no native libraries and no
  dependency on the binding's prebuilt archives.
- `go get github.com/3leaps/sysprims/go/recurrence@vX.Y.Z` resolves a tagged
  version at the same commit as every other surface of the release.
- The tag set is computed from one file and the tagged tree, so the guards
  cannot disagree about which tags a release needs.

### Negative

- Each release signs, pushes and verifies three tags instead of two.
- Repository tooling must treat the recurrence module explicitly: the root
  `go test ./...` does not reach it.

### Neutral

- The recurrence conformance corpus lives outside the module. A module
  downloaded on its own runs its corpus tests only when
  `RECURRENCE_CORPUS_DIR` points at a copy of the corpus.

## Alternatives Considered

### Alternative 1: Put recurrence inside the cgo binding module

Rejected. Importing recurrence would pull in cgo and the platform archives,
which defeats a pure-Go implementation.

### Alternative 2: One module at the repository root with a single tag

Rejected for now. It would require moving the existing binding, a breaking
import-path change for current consumers, and it does not by itself keep the
pure-Go code free of the cgo package.

### Alternative 3: Separate tag versions per module

Rejected. One version and one commit for the whole release keeps provenance
simple: a single signed commit identifies every published surface.

## References

- [ADR-0012: Language Bindings Distribution](ADR-0012-language-bindings-distribution.md)
- [Go modules: version control and module paths](https://go.dev/ref/mod#vcs-version)
- `config/release/release-tags.txt`, `scripts/release-guard-tag-version.sh`
