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
   - **Workflow filename**: `typescript-npm-publish.yml`
   - **Environment name**: (leave blank unless using GitHub environments)
5. Click **Set up connection**

Repeat for each platform package (`@3leaps/sysprims-linux-x64-gnu`, etc.).

### 3. Restrict Token Access (Recommended)

After verifying trusted publishing works:

1. Navigate to package Settings → Publishing access
2. Select **"Require two-factor authentication and disallow tokens"**
3. Save changes

This ensures only OIDC-authenticated workflows can publish.

## Workflow Configuration

The publish workflow (`typescript-npm-publish.yml`) requires:

```yaml
permissions:
  id-token: write # Required for OIDC
  contents: read
```

Key points:

- **Do NOT set NODE_AUTH_TOKEN** - must be completely unset for OIDC fallback
- Use `registry-url: 'https://registry.npmjs.org'` in setup-node
- Run on GitHub-hosted runners only (e.g., `ubuntu-latest`)

## Publishing Process

### Automated (Preferred)

After prebuilds complete successfully:

```bash
gh workflow run typescript-npm-publish.yml
```

The workflow:

1. Validates release tag exists
2. Downloads prebuild artifacts
3. Publishes platform packages via OIDC
4. Publishes root package via OIDC

### Manual Fallback

If automated publishing fails:

```bash
cd bindings/typescript/sysprims
npm login  # Authenticate with OTP
npm publish --access public
```

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
- Confirm workflow file exists at `.github/workflows/typescript-npm-publish.yml`

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
