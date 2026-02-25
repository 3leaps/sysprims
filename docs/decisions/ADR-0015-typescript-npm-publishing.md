# ADR-0015: TypeScript npm Publishing Standard

> **Status**: Accepted
> **Date**: 2026-01-31
> **Authors**: cicd, devlead

## Context

sysprims provides TypeScript bindings via napi-rs, which compiles to native Node.js addons. Publishing these bindings to npm requires:

1. Building platform-specific `.node` binaries (prebuilds)
2. Publishing multiple scoped packages (one per platform + root package)
3. Authenticating with npm registry

We initially attempted OIDC trusted publishing but encountered repeated failures due to:

- npm CLI version requirements (>= 11.5.1)
- Token-based auth interfering with OIDC flow
- Workflow file version resolution from tags
- Incorrect release workflow ordering

This ADR establishes the standard configuration for reliable npm publishing.

## Decision

### 1. OIDC Trusted Publishing (No Tokens)

We use npm OIDC trusted publishing exclusively. **No `NPM_TOKEN` or `NODE_AUTH_TOKEN` secrets shall be stored** in:

- Repository secrets
- Environment secrets
- Organization secrets

Rationale: Any token present will override OIDC, causing authentication failures if the token is expired or revoked.

### 2. npm CLI Version Upgrade

The publish workflow must upgrade npm to >= 11.5.1 before publishing:

```yaml
- name: Ensure npm CLI supports OIDC
  run: |
    npm install -g npm@11.5.1
    echo "npm version: $(npm --version)"
```

Rationale: Ubuntu runners with Node 20 ship npm ~10.x, which lacks OIDC support.

### 3. Force OIDC Mode in Publish Steps

Each publish step must explicitly force OIDC mode:

```yaml
- name: Publish
  run: |
    unset NODE_AUTH_TOKEN NPM_TOKEN
    export NPM_CONFIG_USERCONFIG="$RUNNER_TEMP/npmrc-oidc"
    printf '%s\n' 'registry=https://registry.npmjs.org/' 'always-auth=false' > "$NPM_CONFIG_USERCONFIG"
    npm publish --access public
```

Rationale: `actions/setup-node` with `registry-url` creates `.npmrc` referencing `$NODE_AUTH_TOKEN`. Even without a secret, environment pollution can cause issues.

### 4. GitHub Environment Protection

The publish workflow uses GitHub environment `publish-npm` with:

- Deployment restricted to `v*` tags
- Optional: approval requirements for manual gate

```yaml
jobs:
  publish:
    environment: publish-npm
```

Rationale: Prevents accidental publishes from non-release refs.

### 5. Release Workflow Order

The release process follows this strict order:

```
1. Push all changes to main
2. Verify local/remote sync
3. Run Go Bindings workflow → merge PR
4. (Optional) Run TypeScript validation on main
5. Create and push tags (v* and bindings/go/sysprims/v*)
6. Wait for release workflow (builds artifacts)
7. Manual signing + undraft release
8. Run TypeScript N-API Prebuilds from tag
9. Run TypeScript npm Publish from tag
```

**Critical constraints:**

- Tags must point to commits that include Go bindings
- Prebuilds must run from tag ref (SHA validation in publish)
- npm publish must run from tag ref (OIDC + environment protection)

### 6. Workflow Files Run from Tag Commit

When publishing from a tag, GitHub uses the workflow YAML from the tagged commit, not from main. This means:

- Workflow fixes pushed to main don't affect tag-triggered runs
- To fix a workflow for an existing tag, the tag must be moved
- Test workflow changes on main before tagging

## Consequences

### Positive

- No long-lived secrets to manage or rotate
- Provenance attestation for supply chain security
- Clear audit trail via GitHub environments
- Reproducible releases via strict ordering

### Negative

- Workflow fixes require tag recreation (moving published tags is risky)
- More complex workflow configuration vs simple token auth
- npm CLI upgrade adds ~10s to workflow runtime

### Neutral

- Each platform package needs separate trusted publisher config on npmjs.com
- Environment protection adds manual approval step (can be removed if desired)

## Alternatives Considered

### Alternative 1: Classic Token Authentication

Use `NPM_TOKEN` secret with standard publish flow.

**Rejected**: npm is deprecating classic tokens (90-day expiry, potential future removal). OIDC is the forward-looking standard.

### Alternative 2: actions/setup-node Native OIDC

Rely on `actions/setup-node` to handle OIDC automatically.

**Rejected**: The action creates `.npmrc` that references `$NODE_AUTH_TOKEN`, which interferes with OIDC flow when any token-like value is present.

### Alternative 3: Separate Publish Workflow File

Create a dedicated workflow file without `registry-url` configuration.

**Considered but deferred**: Current approach with explicit OIDC forcing works. May revisit if cleaner patterns emerge.

## References

- [npm Trusted Publishers Documentation](https://docs.npmjs.com/trusted-publishers/)
- [Crucible Knowledge: npm OIDC](https://crucible.3leaps.dev/knowledge/cicd/registry/npm-oidc)
- [Crucible Knowledge: Workflow Version Resolution](https://crucible.3leaps.dev/knowledge/cicd/github-actions/workflow-version-resolution)
- [RELEASE_CHECKLIST.md](../../RELEASE_CHECKLIST.md) - Full release procedure
