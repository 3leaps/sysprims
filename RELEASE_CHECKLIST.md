# Release Checklist

This document walks maintainers through the build/sign/upload flow for each sysprims release.

## Prerequisites

- GPG and minisign installed
- Signing keys configured (see `docs/security/signing-runbook.md`)
- Environment variables set (see step 2 below)
- `gh` CLI authenticated with push access

## 1. Pre-Release Preparation

### Code Quality Gates

- [ ] Ensure `main` is clean: `git status` shows no uncommitted changes
- [ ] Run pre-push checks: `make prepush` passes
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
- [ ] Create release notes: `docs/releases/vX.Y.Z.md`

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

- [ ] Go bindings prep (required):
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
    - `bindings/go/sysprims/include/sysprims.h`
  - Merge the PR so the prebuilt libs are present on `main` before tagging.

- [ ] Create and push tags (must point to the SAME commit):
  ```bash
  VERSION=$(cat VERSION)

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
- [ ] Check draft release has all expected artifacts:
  - CLI binaries (darwin-amd64, darwin-arm64, linux-amd64, linux-amd64-musl, linux-arm64, linux-arm64-musl, windows-amd64)
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
  - Confirm Windows uses GNU target assets (`x86_64-pc-windows-gnu`) for cgo compatibility.

  TypeScript bindings (recommended):
  - Validate that the published FFI bundle contains the shared libraries the TypeScript package expects.
    Run the workflow `.github/workflows/typescript-bindings.yml` in "from-release" mode against the draft release:
    ```bash
    VERSION=$(cat VERSION)
    gh workflow run "TypeScript Bindings" -f tag="v${VERSION}"
    ```
    This downloads `sysprims-ffi-${VERSION}-libs.tar.gz` from the draft release, extracts the platform shared lib
    into `bindings/typescript/sysprims/_lib/<platform>/`, and runs the TS test suite on each OS runner.

  Integrity rule: anything we intentionally publish as a release asset must be covered by the signed checksum manifests.

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

| Target | Description |
|--------|-------------|
| `make release-clean` | Remove dist/release contents |
| `make release-download` | Download CI artifacts from GitHub |
| `make release-checksums` | Generate SHA256SUMS and SHA512SUMS |
| `make release-sign` | Sign checksums with minisign + PGP |
| `make release-export-keys` | Export public signing keys |
| `make release-verify` | Verify checksums, signatures, and keys |
| `make release-notes` | Copy release notes to dist |
| `make release-upload` | Upload signed artifacts to GitHub |
| `make release` | Full workflow (clean → upload) |

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

## Key Rotation

If rotating signing keys, update:
- [ ] `RELEASE_CHECKLIST.md` - verification example public key
- [ ] `README.md` - verification snippet
- [ ] `docs/security/signing-runbook.md`

## Versioning Policy

- **Patch** (0.1.1): Bug fixes, security patches
- **Minor** (0.2.0): New features, backward-compatible
- **Major** (1.0.0): Breaking changes, API changes

See `docs/decisions/` for versioning decisions.
