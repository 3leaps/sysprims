# ADR-0013: Release Asset Publishing and Verification

> **Status**: Proposed  
> **Date**: 2026-01-17  
> **Authors**: devlead, Architecture Council

## Context

sysprims publishes multiple kinds of release artifacts:

- CLI binaries and associated archives
- Static FFI libraries (for language bindings)
- Generated C headers
- Documentation and metadata artifacts (licenses, manifests)

This repository intentionally does **not** commit build outputs or release artifacts. Instead, we use
local directories under `dist/` for:

- Downloading CI-built release assets from GitHub for signing/verification
- Building and sharing preview artifacts during beta cycles

Because consumers may rely on these artifacts for process control and automation, we need a clear,
repeatable integrity and provenance model.

## Decision

### 1. Build Outputs Are Not Source Artifacts

- Build outputs and release artifacts are **not** committed to the repository.
- All build and release artifacts are written under `dist/` (which is gitignored).

Rationale:
- Keeps the repo lean and source-focused
- Avoids accidental drift between source and generated outputs
- Supports the existing signing workflow that treats artifacts as external deliverables

### 2. Every Published Asset Must Be Covered by Signed Checksums

Any file intentionally published as a GitHub Release asset MUST be included in the checksum manifests
(SHA-256 and SHA-512) and therefore covered by signature verification.

This includes (non-exhaustive):
- Executable archives (`*.tar.gz`, `*.zip`)
- Static libraries (standalone or archived)
- Generated headers (`*.h`)
- Metadata files intentionally shipped alongside artifacts (e.g., `LICENSE-*`, selected `*.json`)
- Documentation archives (if intentionally published)

Rationale:
- Headers and libraries are inputs to downstream builds; they must be authenticated just like binaries
- Prevents a class of "unsigned sidecar" issues where only the main archive is trusted

### 3. Release Asset Directory Conventions (Local)

We use two local directories with distinct intent:

- `dist/release/`: local workspace used to download official CI-built assets from GitHub and then
  generate checksums, sign checksum manifests, and upload signatures back to the release.
- `dist/local/`: local workspace used for preview/beta artifacts built from a working tree.

Rules:
- `dist/local/` assets are volatile and MUST NOT be treated as official deliverables.
- The layout of `dist/local/` SHOULD mirror the release asset layout closely enough that downstream
  consumers can wire up to it by path during a coordinated beta cycle. A preview may include only a
  subset of platforms.

### 4. Flat Release Asset Layout (and checksum scope)

Release assets are intentionally flat at the GitHub release top-level. Any internal structure belongs
inside archives.

Checksum generation MUST cover headers (and any other loose files) that are shipped as standalone assets.

If checksum tooling only scans a single directory depth (e.g., `dist/release/*`), then any loose
assets that are intended to be published MUST be present at that top level (or the tooling must be
updated accordingly).

Rationale:
- Ensures the checksum/signing system is complete and mechanically verifiable

### 5. Implementation Details Are Policy

This ADR defines non-negotiable integrity rules and directory intent.

Concrete file naming, archive structure, and build target behavior are captured in a policy document
so they can evolve without revisiting the ADR each time.

## Consequences

### Positive

- Consumers can cryptographically validate *all* published artifacts (including headers)
- Clear separation between official release assets (`dist/release`) and preview artifacts (`dist/local`)
- Reduced risk of shipping unauthenticated build inputs to language binding users

### Negative

- Requires discipline: any new asset type added to releases must be included in checksum generation
- Some release tooling may need adjustment if assets are nested instead of flat

### Neutral

- This does not mandate a specific archive layout; it mandates integrity coverage
- Existing workflows (download, checksum, sign, verify, upload) remain valid and become explicit

## Alternatives Considered

### Alternative 1: Commit generated headers and/or artifacts

Rejected:
- Creates noisy diffs and repo bloat
- Makes it unclear which artifacts are authoritative (repo vs release)

### Alternative 2: Checksum only the primary archives

Rejected:
- Leaves sidecar assets (headers, manifests, docs) unauthenticated
- Downstream builds can be compromised by an untrusted header even if binaries are signed

### Alternative 3: Store all artifacts only inside archives

Not chosen as a requirement:
- Viable, but not necessary to achieve integrity
- Some consumers benefit from standalone headers; the key requirement is checksum coverage

## References

- `scripts/generate-checksums.sh`
- `RELEASE_CHECKLIST.md`
- `Makefile` release targets (`release-download`, `release-checksums`, `release-sign`, `release-verify`)
- ADR-0004: FFI Design (`docs/architecture/adr/0004-ffi-design.md`)
- ADR-0012: Language Bindings Distribution (`docs/architecture/adr/0012-language-bindings-distribution.md`)
