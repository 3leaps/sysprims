# npm Publishing with Trusted Publishing (OIDC)

This guide documents how sysprims TypeScript bindings are published to npm using
[OIDC trusted publishing](https://docs.npmjs.com/trusted-publishers/), eliminating
the need for long-lived npm tokens.

## Overview

Trusted publishing creates a trust relationship between npm and GitHub Actions using
OpenID Connect (OIDC). When configured, npm accepts publishes from authorized workflows
using short-lived, cryptographically-signed tokens that cannot be extracted or reused.

**Benefits over token-based publishing:**

- No long-lived secrets to manage or rotate
- Tokens cannot be accidentally exposed in logs
- Automatic provenance attestation for supply chain security
- Scoped to specific workflow files

## Prerequisites

- Node.js v22.14.0 or later for the publish workflow
- npm CLI v11.5.1 or later
- GitHub-hosted runners (self-hosted runners not yet supported)
- Package must already exist on npm (first publish requires manual/token approach)
- `package.json` must include `repository.url` field

## One-Time Setup

### 1. First Publish (Manual)

The first version of a package must be published manually or with a token before
trusted publishing can be configured:

```bash
cd bindings/typescript/sysprims
npm publish --access public
```

You will need to authenticate with `npm login` and provide OTP if 2FA is enabled.

### 2. Configure Trusted Publisher on npmjs.com

After the package exists:

1. Navigate to https://www.npmjs.com/package/@3leaps/sysprims/access
2. Find the **Trusted Publisher** section
3. Click **GitHub Actions**
4. Configure:
   - **Organization or user**: `3leaps`
   - **Repository**: `sysprims`
   - **Workflow filename**: `registry-publish.yml`
   - **Environment name**: `publish-npm`
5. Click **Set up connection**

Repeat for each platform package (`@3leaps/sysprims-linux-x64-gnu`, etc.).

### 3. Restrict Token Access (Recommended)

After verifying trusted publishing works:

1. Navigate to package Settings → Publishing access
2. Select **"Require two-factor authentication and disallow tokens"**
3. Save changes

This ensures only OIDC-authenticated workflows can publish.

## Workflow Configuration

Only the protected `publish-npm` (and `publish-pypi`) jobs in `registry-publish.yml` receive:

```yaml
permissions:
  contents: read
  id-token: write # Required for OIDC
```

The preceding `stage` job has no environment and no `id-token` permission. It
validates the exact tag and surface plan, downloads and verifies the signed
tarballs from the immutable GitHub Release (`scripts/release-stage-published.sh --npm-only`),
and uploads a tag-and-commit-bound manifest containing the package names,
versions, tarball filenames, SHA-256, SRI, and content lists.

Key points:

- **Do NOT set NODE_AUTH_TOKEN** - must be completely unset for OIDC fallback
- Use `registry-url: 'https://registry.npmjs.org'` in setup-node
- Use Node.js 24 for the publish workflow. The workflow hard-fails if Node.js is
  below 22.14.0 or npm is below 11.5.1.
- Run on GitHub-hosted runners only (`ubuntu-24.04`)

### Environment protection (`publish-npm`)

The protected `publish-npm` job deploys to the `publish-npm` environment. Its
deployment ref policy allows only:

| Ref | Type | Pattern |
| --- | ---- | ------- |
| Release tag | tag | `v*` |

`main` and branch refs are deliberately not allowlisted — do not add them.
Publication must always be dispatched from the release tag ref:

```bash
VERSION=$(cat VERSION)
gh workflow run registry-publish.yml --ref "v${VERSION}" \
  -f tag="v${VERSION}" -f registry=npm
```

## Publishing Process

### Automated (Preferred)

After the signed immutable GitHub Release is published:

```bash
VERSION=$(cat VERSION)
gh workflow run registry-publish.yml --ref "v${VERSION}" \
  -f tag="v${VERSION}" -f registry=all
```

The workflow:

1. Validates the exact remote annotated tags, surface plan, and provenance
2. Stages and verifies the signed `.tgz` tarballs directly from the immutable GitHub Release (zero rebuild)
3. Compares every exact package/version with the registry; dry-run stops here
4. Enters `publish-npm` only for a real publication
5. Skips an already-present package only when its registry tarball is identical
6. Publishes/verifies all seven native packages before the root package (with a 180s CDN readiness poll window)

The sequence is resumable after any partial native publication. A same-version
registry package with different bytes or integrity stops the run.

Rerun the same workflow after an interruption. Do not replace the attested
staging artifact with locally packed bytes.

## Troubleshooting

### "Unable to authenticate" error

- Verify workflow filename matches exactly (case-sensitive, include `.yml`)
- Ensure using GitHub-hosted runners, not self-hosted
- Check `id-token: write` permission is set
- Confirm `NODE_AUTH_TOKEN` is NOT set (not even empty string)

### 404 on publish

npm could not match workflow to trusted publisher configuration:

- Check organization name matches GitHub URL exactly (case-sensitive)
- Verify `package.json` has correct `repository.url`
- Confirm workflow file exists at `.github/workflows/registry-publish.yml`

### Publish job denied by environment protection

The run's ref is not on the `publish-npm` allowlist (for example, a dispatch
from `main`). Always dispatch from the `v*` release tag ref (`--ref "v${VERSION}"`).

### Provenance not generated

Automatic provenance requires:

- Publishing via OIDC (not token)
- Public repository
- Public package

Private repositories cannot generate provenance even for public packages.

## Security Considerations

- Each package can only have one trusted publisher at a time
- Workflow filename is part of the trust anchor - changing it requires reconfiguration
- Consider using GitHub environments with approval requirements for additional control
- Regularly audit trusted publisher configurations

## References

- [npm Trusted Publishers Documentation](https://docs.npmjs.com/trusted-publishers/)
- [GitHub Actions OIDC Documentation](https://docs.github.com/en/actions/deployment/security-hardening-your-deployments/about-security-hardening-with-openid-connect)
- [OpenSSF Trusted Publishers Specification](https://repos.openssf.org/trusted-publishers-for-all-package-repositories)
