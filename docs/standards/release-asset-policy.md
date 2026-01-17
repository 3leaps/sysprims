---
title: "Release Asset Policy"
description: "Concrete conventions for sysprims release/preview assets"
author: "OpenCode"
author_of_record: "Dave Thompson <dave.thompson@3leaps.net>"
supervised_by: "@3leapsdave"
date: "2026-01-17"
status: "draft"
---

This policy codifies the concrete implementation details for release and preview artifacts.

Principles are defined in `docs/architecture/adr/0013-release-asset-publishing-and-verification.md`.

## Directories

- `dist/release/`: staging area for CI-built assets downloaded from GitHub releases. Used for checksum
  generation, signing, verification, and re-upload.
- `dist/local/`: staging area for locally built preview assets (volatile). Used only during coordinated
  beta cycles.

Both directories are gitignored.

## Integrity

- Any asset published to GitHub Releases MUST be included in `SHA256SUMS` and `SHA512SUMS`.
- The checksum manifests are the signed unit of trust; signatures cover all referenced assets.

## Current Asset Layout (Release)

GitHub release assets are intentionally **flat** (top-level). Any richer structure belongs inside archives.

Locally, release assets are expected to be present at the top level of `dist/release/` when generating checksums.

Examples:
- `sysprims-<version>-<platform>.tar.gz`
- `sysprims-ffi-<version>-libs.tar.gz`
- `sysprims.h`
- `LICENSE-MIT`, `LICENSE-APACHE`
- `MANIFEST.json` (when shipped)

## Local Preview Assets

Local preview assets SHOULD mirror the GitHub release asset layout by path and filename where feasible.
If a preview only includes a subset of platforms, it still uses the same top-level naming conventions and/or
bundles, but will be incomplete.

Minimum expectation for FFI preview testing (single-platform):
- `dist/local/release/sysprims-ffi/`
  - `libsysprims_ffi.a`
  - `include/sysprims.h`
  - `include/sysprims-go.h`
  - `LOCAL.txt`

`LOCAL.txt` exists to prevent consumers from confusing these with CI-built artifacts.

## Build Targets

- `make build-local-go` builds the FFI library for the current platform and:
  - copies it to `bindings/go/sysprims/lib/local/<platform>/` for cgo builds
  - stages a release-like preview bundle under `dist/local/release/sysprims-ffi/`
- `make dist-local-clean` removes `dist/local`.
- `make release-clean` removes `dist/release`.

## Notes

- If we change the structure of release assets (e.g. nested directories), we must update
  `scripts/generate-checksums.sh` accordingly so all assets remain covered by signed checksums.
