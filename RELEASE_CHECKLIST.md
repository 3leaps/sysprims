# Release Checklist

This document walks maintainers through the build/sign/upload flow for each sysprims release.

## Prerequisites

- GPG and minisign installed
- Signing keys configured (see `docs/security/signing-runbook.md`)
- Environment variables set (see step 2 below)
- `gh` CLI authenticated with push access
- All feature PRs for this release are merged and branch is `main`

## 1. Pre-Release Preparation

### PR Merge Gate

- [ ] Confirm all in-scope PRs are merged: `gh pr list --state open`
- [ ] Confirm you are on `main`: `git branch --show-current`
- [ ] Pull latest: `git pull origin main`

### Code Quality Gates

- [ ] Ensure `main` is clean: `git status` shows no uncommitted changes
- [ ] Run PR final checks: `make pr-final` passes
- [ ] Run full test suite: `cargo test --workspace`
- [ ] Verify cargo-deny passes: `cargo deny check`
- [ ] (Recommended) Run container tests locally:
  ```bash
  docker build -t sysprims-test-fixture -f Dockerfile.container .
  docker run --rm -v $(pwd):/workspace:ro -v $(pwd)/target:/workspace/target sysprims-test-fixture
  ```
  This catches Linux-specific issues (musl builds, `/proc` behavior), privileged test edge cases,
  and cross-user permission scenarios that can't be tested on macOS. Particularly valuable when
  changes touch platform-specific code paths or signal/process handling.

### Version & Documentation

- [ ] Update `VERSION` file with new semver (e.g., `0.1.1`)
- [ ] Sync version to Cargo.toml: `make version-sync`
- [ ] Update `CHANGELOG.md` (move Unreleased to new version section)
- [ ] **Advance CHANGELOG footer compare-links** (reference-style definitions at the bottom of
  the file): set `[Unreleased]: .../compare/vX.Y.Z...HEAD` and add
  `[X.Y.Z]: .../compare/vPREV...vX.Y.Z`. Then verify every `## [x.y.z]` heading has a matching
  `[x.y.z]:` footer definition. A missing definition renders as an undefined link, and `goneat`
  / `make` do not flag it. Backfill prior-release gaps while here.
- [ ] Create release notes: `docs/releases/vX.Y.Z.md`

### Scope Control (Recommended)

- [ ] Confirm release scope is intentional and minimal.
  - For v0.1.7: keep scope to TypeScript Node-API bindings rollout only (no extra refactors).

### Pre-Tag Verification

- [ ] **Run preflight checks**: `make release-preflight`
  - Validates: working tree clean, prepush checks pass, version synced, release notes exist, local/remote sync
  - **Must pass before tagging**

### Commit & Tag

- [ ] Commit changes:
  ```bash
  git add -A
  git commit -m "release: prepare vX.Y.Z"
  ```
- [ ] Push to main:

  ```bash
  git push origin main
  ```

- [ ] **Verify local/remote sync** (required before running workflows):
      Before running any release workflows, confirm local and remote are in sync:

  ```bash
  git fetch origin
  # Must show no output (no divergence):
  git log --oneline origin/main..HEAD
  git log --oneline HEAD..origin/main
  ```

  If there's divergence, resolve it before proceeding. The Go bindings workflow
  runs on the remote HEAD, so any unpushed local commits will cause the PR merge
  to diverge from your local state. See [Troubleshooting: Local/Remote Divergence](#troubleshooting-localremote-divergence).

- [ ] Go bindings prep (required):
  - [ ] **Order:** the version/changelog/release-notes commit must already be on `main` before
    running this workflow. Prebuilt libs are built from `main` and must embed the release
    version. This merge commit becomes the tag target.
  - Run the workflow `.github/workflows/go-bindings.yml` for this version (manual; do not run on every push).
    ```bash
    VERSION=$(cat VERSION)
    gh workflow run "Go Bindings (Prep)" -f version="${VERSION}"
    ```
  - Find the created PR and review/merge it:
    ```bash
    VERSION=$(cat VERSION)
    gh pr list --search "go-bindings/v${VERSION}" --state open
    gh pr view --web "go-bindings/v${VERSION}"
    ```
  - Confirm the PR actually adds the platform libs before merging:
    - `bindings/go/sysprims/lib/<platform>/libsysprims_ffi.a`
  - Note: the workflow also regenerates `bindings/go/sysprims/include/sysprims.h`; it may not
    appear in the PR diff when the regenerated header is byte-identical to `main`.
  - Merge the PR so the prebuilt libs are present on `main` before tagging.
  - After merge: ensure `main` is green again (this merge commit is what will be tagged).

- [ ] TypeScript bindings validation (recommended):
  - After the Go bindings PR is merged (and `main` is green), run the TypeScript workflow on `main`.
    This is the normal order: push/merge to `main` first, then run the manual TS workflow on that exact commit.
    ```bash
    gh workflow run "TypeScript Bindings" --ref main
    ```

- [ ] Create and push tags (must point to the SAME commit):

  ```bash
  VERSION=$(cat VERSION)

   # IMPORTANT: tags must point to the commit that includes the merged Go bindings PR.
   # Ensure you're tagging the current main HEAD.

  # Canonical repo tag (drives .github/workflows/release.yml)
  git tag -a "v${VERSION}" -m "v${VERSION}: <brief description>"

  # Go submodule tag (required so Go resolves semver for subdir module)
  git tag -a "bindings/go/sysprims/v${VERSION}" -m "bindings/go/sysprims/v${VERSION}"

  # Push both tags
  git push origin "v${VERSION}" "bindings/go/sysprims/v${VERSION}"
  ```

Notes:

- Go requires the path-prefixed tag because the module is `github.com/3leaps/sysprims/bindings/go/sysprims`.
- Python (PyPI) and TypeScript (npm) do not use git tags for version resolution in the same way.
- See `docs/decisions/ADR-0012-language-bindings-distribution.md` and `docs/guides/language-bindings.md` for details.

### CI Verification

- [ ] Wait for GitHub Actions release workflow to complete
- [ ] Verify CI status is green on the tag
- [ ] (Recommended) Run the validation workflow against the tag:
  ```bash
  VERSION=$(cat VERSION)
  gh workflow run "Validate Release" -f tag="v${VERSION}"
  ```
- [ ] Check draft release has all expected artifacts:
  - CLI binaries (darwin-arm64, linux-amd64, linux-amd64-musl, linux-arm64, linux-arm64-musl, windows-amd64, windows-arm64)
  - FFI library tarball
  - C header (sysprims.h)
  - SBOM (sysprims-X.Y.Z.cdx.json)
  - Licenses (LICENSE-MIT, LICENSE-APACHE)

  Go bindings:
  - Confirm `bindings/go/sysprims/lib/<platform>/libsysprims_ffi.a` is present in the tagged commit so `go get` works without Rust.
    Quick check:
    ```bash
    VERSION=$(cat VERSION)
    git ls-tree -r --name-only "v${VERSION}" bindings/go/sysprims/lib | sed -n '1,20p'
    ```
    If this is empty, do not tag/publish; the Go bindings prep step above was not completed.
  - Confirm `bindings/go/sysprims/include/sysprims.h` is present in the tagged commit:
    ```bash
    VERSION=$(cat VERSION)
    git cat-file -e "v${VERSION}:bindings/go/sysprims/include/sysprims.h"
    ```
  - Confirm Windows uses GNU-ABI FFI assets for cgo compatibility:
    - windows-amd64 → `x86_64-pc-windows-gnu` (msys2/MinGW-w64)
    - windows-arm64 → `aarch64-pc-windows-gnullvm` (llvm-mingw), since v0.1.16

  TypeScript bindings (run AFTER signing, from the tag ref):
  1. Run prebuilds workflow on the tag (builds N-API binaries for all platforms):

     ```bash
     VERSION=$(cat VERSION)
     gh workflow run "TypeScript N-API Prebuilds" --ref "v${VERSION}"
     ```

     Wait for completion. This builds `.node` binaries and stages npm package directories.

  2. Run npm publish workflow on the tag (requires OIDC trusted publishing):
     ```bash
     VERSION=$(cat VERSION)
     gh workflow run "TypeScript npm Publish" --ref "v${VERSION}"
     ```
     The workflow validates:
     - Running from a `v*` tag ref (required for OIDC and environment protection)
     - Node.js >= 22.14.0 and npm >= 11.5.1 for npm trusted publishing
     - VERSION file and package.json match the tag
     - Prebuilds were built from the same commit as the tag

  Note: npm publish uses OIDC trusted publishing (no NPM_TOKEN). The workflow must run
  from a tag ref to satisfy the `publish-npm` environment protection rules, and the
  publish job intentionally uses Node.js 24 even though validation/prebuild jobs remain
  on Node.js 20.

  Integrity rule: anything we intentionally publish as a release asset must be covered by the signed checksum manifests.

### crates.io (library crates only, after the tag)

Do this only after the exact release tag is on `origin` and points at the
intended release commit. The principal or Echo lead must explicitly cue the
upload. Token and owners stay out of the tree.

`make release-check` / `cargo package --workspace --no-verify` creates local
tarballs. It does **not** publish anything to crates.io. On the first
publication of these crate names, dependent build verification happens in the
cued publish sequence after predecessor crates are indexed.

What gets published:

| Crate | crates.io |
|-------|-----------|
| `sysprims-core` | yes (first) |
| `sysprims-signal` | yes (after core is indexed) |
| `sysprims-session` | yes (after core is indexed) |
| `sysprims-proc` | yes (after signal is indexed) |
| `sysprims-timeout` | yes (last) |
| `sysprims-cli` | **never** (`publish = false`) |
| `sysprims-ffi` | **no** (`publish = false`) |
| `sysprims-ts-napi` | **no** (`publish = false`) |

Workspace `publish` stays `false`. The five public Rust libraries opt in.

Use a crates.io token scoped to the five library crate names. First upload of a
crate name requires `publish-new` and `publish-update`; later releases should
use update-only scope. Never store the token in this repository.

Publish from a clean checkout of the tag:

```bash
VERSION=$(cat VERSION)
git checkout "v${VERSION}"
cargo publish --dry-run -p sysprims-core
cargo publish -p sysprims-core
cargo info --registry crates-io "sysprims-core@${VERSION}"
cargo publish --dry-run -p sysprims-signal
cargo publish -p sysprims-signal
cargo info --registry crates-io "sysprims-signal@${VERSION}"
cargo publish --dry-run -p sysprims-session
cargo publish -p sysprims-session
cargo info --registry crates-io "sysprims-session@${VERSION}"
cargo publish --dry-run -p sysprims-proc
cargo publish -p sysprims-proc
cargo info --registry crates-io "sysprims-proc@${VERSION}"
cargo publish --dry-run -p sysprims-timeout
cargo publish -p sysprims-timeout
```

- [ ] Dry-run then publish each crate in dependency order.
- [ ] Confirm each predecessor with
      `cargo info --registry crates-io <crate>@${VERSION}` before the next
      dependent publish.
- [ ] On the first upload of these crate names, expect standalone dry-runs for
      dependent crates to fail until predecessor crates are actually indexed.
- [ ] Do **not** `cargo publish -p sysprims-cli`, `sysprims-ffi`, or
      `sysprims-ts-napi`.

Negative control:

```bash
cargo publish --dry-run -p sysprims-cli
cargo publish --dry-run -p sysprims-ffi
cargo publish --dry-run -p sysprims-ts-napi
# expected: error, crate cannot be published
```

## 2. Manual Signing (Local Machine)

### Set Environment Variables

```bash
# Source the vars file or set manually:
source ~/devsecops/vars/3leaps-sysprims-cicd.sh

# Or set individually:
export SYSPRIMS_RELEASE_TAG=v$(cat VERSION)
export SYSPRIMS_MINISIGN_KEY=/path/to/signing.key
export SYSPRIMS_MINISIGN_PUB=/path/to/signing.pub
export SYSPRIMS_PGP_KEY_ID="keyid!"
export SYSPRIMS_GPG_HOMEDIR=/path/to/gpg/homedir  # optional
```

### Signing Steps

1. **Clean previous release artifacts**

   ```bash
   make release-clean
   ```

2. **Download artifacts from GitHub draft release**

   ```bash
   make release-download
   ```

3. **Generate checksum manifests**

   ```bash
   make release-checksums
   ```

   Produces: `SHA256SUMS`, `SHA512SUMS`

   Notes:
   - Release assets are expected to be flat at the top-level of `dist/release/` (matching GitHub release assets).
   - The checksum manifests intentionally include archives, standalone headers (e.g. `sysprims.h`), any standalone libs,
     SBOM/metadata JSON, licenses, and copied release notes.

4. **Sign checksum manifests** (minisign + PGP)

   ```bash
   make release-sign
   ```

   Produces: `.minisig` and `.asc` signatures for both checksum files

5. **Export public keys**

   ```bash
   make release-export-keys
   ```

   Produces: `sysprims-minisign.pub`, `sysprims-release-signing-key.asc`

6. **Verify everything before upload**

   ```bash
   make release-verify
   ```

   Validates:
   - Checksums match artifacts
   - Signatures verify correctly
   - Exported keys are public-only (no secret key material)

7. **Copy release notes**

   ```bash
   make release-notes
   ```

   Copies `docs/releases/vX.Y.Z.md` to `dist/release/release-notes-vX.Y.Z.md`

8. **Upload signed artifacts to GitHub**

   ```bash
   make release-upload
   ```

   > **Note:** Uses `--clobber` to overwrite existing assets. Safe to rerun.

9. **Publish the release**
   ```bash
   gh release edit v$(cat VERSION) --draft=false
   ```

## 3. Post-Release Verification

- [ ] Verify release is public: `gh release view v$(cat VERSION)`
- [ ] Verify checksums match: download and verify locally
- [ ] Test binary: download and run `sysprims --version`
- [ ] Verify signatures with public keys

### Binary Verification Example

```bash
# Download and verify
curl -LO https://github.com/3leaps/sysprims/releases/download/vX.Y.Z/sysprims-X.Y.Z-darwin-arm64.tar.gz
curl -LO https://github.com/3leaps/sysprims/releases/download/vX.Y.Z/SHA256SUMS
curl -LO https://github.com/3leaps/sysprims/releases/download/vX.Y.Z/SHA256SUMS.minisig
curl -LO https://github.com/3leaps/sysprims/releases/download/vX.Y.Z/sysprims-minisign.pub

# Verify checksum
shasum -a 256 -c SHA256SUMS --ignore-missing

# Verify signature (minisign)
minisign -Vm SHA256SUMS -p sysprims-minisign.pub
```

## 4. Post-Release Version Bump

After release, bump VERSION for next development cycle:

```bash
make version-patch   # 0.1.0 -> 0.1.1
# or: make version-minor  # 0.1.0 -> 0.2.0
# or: make version-major  # 0.1.0 -> 1.0.0

git add VERSION
git commit -m "chore: bump version to $(cat VERSION)-dev"
git push origin main
```

## Quick Reference: All Release Targets

| Target                     | Description                                                                    |
| -------------------------- | ------------------------------------------------------------------------------ |
| `make release-preflight`   | **REQUIRED**: Verify pre-tag requirements (tree, checks, version, notes, sync) |
| `make release-clean`       | Remove dist/release contents                                                   |
| `make release-download`    | Download CI artifacts from GitHub                                              |
| `make release-checksums`   | Generate SHA256SUMS and SHA512SUMS                                             |
| `make release-sign`        | Sign checksums with minisign + PGP                                             |
| `make release-export-keys` | Export public signing keys                                                     |
| `make release-verify`      | Verify checksums, signatures, and keys                                         |
| `make release-notes`       | Copy release notes to dist                                                     |
| `make release-upload`      | Upload signed artifacts to GitHub                                              |
| `make release`             | Full workflow (clean → upload)                                                 |

## Troubleshooting

### "SYSPRIMS_MINISIGN_KEY not set"

Source the vars file or set the environment variable:

```bash
source ~/devsecops/vars/3leaps-sysprims-cicd.sh
```

### "No release notes found"

Create the release notes file:

```bash
mkdir -p docs/releases
# Write release notes to docs/releases/vX.Y.Z.md
```

### CI workflow failed

1. Check GitHub Actions logs
2. Fix the issue on main
3. Delete the tag and release draft
4. Start over from step 1

### Signature verification failed

1. Ensure you used the correct signing key
2. Re-run `make release-sign`
3. Re-run `make release-verify` to confirm

### Troubleshooting: Local/Remote Divergence

If you ran the Go bindings workflow while local and remote were out of sync,
the PR merge will create divergent branches:

```
Local:  A -- B -- C (unpushed local commit)
Remote: A -- B -- D -- E (Go bindings PR merge)
```

**Symptoms:**

- `git pull` fails with "divergent branches" error
- `git log origin/main..HEAD` shows local commits not on remote
- `git log HEAD..origin/main` shows remote commits not on local

**Resolution:**

1. If the local commit should be part of the release:

   ```bash
   git merge origin/main -m "Merge remote Go bindings PR"
   git push origin main
   # Wait for CI to pass, then re-run Go bindings workflow
   ```

2. If the local commit can be discarded:
   ```bash
   git reset --hard origin/main
   ```

**Prevention:** Always verify sync before running release workflows:

```bash
git fetch origin
git log --oneline origin/main..HEAD  # should be empty
git log --oneline HEAD..origin/main  # should be empty
```

## Key Rotation

If rotating signing keys, update:

- [ ] `RELEASE_CHECKLIST.md` - verification example public key
- [ ] `README.md` - verification snippet
- [ ] `docs/security/signing-runbook.md`

## Versioning Policy

Before `1.0.0`, minor releases may include breaking API or behavior changes.
Those changes must be explicit in the release notes and upgrade guidance.

- **Pre-1.0 patch** (0.1.1): Backward-compatible bug and security fixes
- **Pre-1.0 minor** (0.2.0): Features and explicitly documented breaking changes
- **Post-1.0 patch** (1.0.1): Backward-compatible bug and security fixes
- **Post-1.0 minor** (1.1.0): Backward-compatible features
- **Post-1.0 major** (2.0.0): Breaking behavior or API changes

See `docs/decisions/` for versioning decisions.
