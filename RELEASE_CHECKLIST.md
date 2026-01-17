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
- [ ] Create and push tag:
  ```bash
  git tag -a v$(cat VERSION) -m "vX.Y.Z: <brief description>"
  git push origin v$(cat VERSION)
  ```

### CI Verification

- [ ] Wait for GitHub Actions release workflow to complete
- [ ] Verify CI status is green on the tag
- [ ] Check draft release has all expected artifacts:
  - CLI binaries (darwin-amd64, darwin-arm64, linux-amd64, linux-amd64-musl, linux-arm64, linux-arm64-musl, windows-amd64)
  - FFI library tarball
  - C header (sysprims.h)
  - SBOM (sysprims-X.Y.Z.cdx.json)
  - Licenses (LICENSE-MIT, LICENSE-APACHE)

  Integrity rule: anything we intentionally publish as a release asset must be covered by the signed checksum manifests.

## 2. Manual Signing (Local Machine)

### Set Environment Variables

```bash
# Source the vars file or set manually:
source ~/devsecops/vars/3leaps-sysprims-cicd.sh

# Or set individually:
export RELEASE_TAG=v$(cat VERSION)
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

See `docs/architecture/adr/` for versioning decisions.
