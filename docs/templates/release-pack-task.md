# Release Pack Task

Use this template for the pull request that prepares one sysprims release.
Replace every placeholder before review.

## Release

- Version: `X.Y.Z`
- Final target commit: recorded after Go Bindings Prep merges
- Surface plan: `docs/releases/vX.Y.Z.json`
- Release notes: `docs/releases/vX.Y.Z.md`

## Surface plan

- [ ] `provenance_policy` is `commit_footer`.
- [ ] Rust, CLI, and FFI are `publish`.
- [ ] Go is `publish` or `skip`; a skip names the retained version.
- [ ] Go lock phase is `pre_build` for a pending publish build or `resolved`
      for committed, hashed, and smoked prebuilts. A skip is `resolved`.
- [ ] TypeScript is `publish` or `skip`; a skip names the retained version.
- [ ] TypeScript lock phase is `pre_registry` for a new post-tag publication
      or `resolved` for registry-backed lock evidence.
- [ ] `make release-plan-check version-check` passes.

## Merge attribution

- [ ] The exact squash subject and body are prepared in a file.
- [ ] The squash body ends with the complete standard attribution footer.
- [ ] After merge, `git log -1 --format=%B` contains that footer.
- [ ] `make release-guard-provenance` passes on synchronized `main`.
- [ ] A missing footer stops the cut. Public `main` is not rewritten and no
      empty attribution commit is created. Only an approved, signed,
      version/commit/tag-message-bound receipt may authorize the exception.

## Binding preparation

- [ ] The version, changelog, notes, and surface plan are merged before Go Prep.
- [ ] Go Bindings Prep rebuilds every required platform, smokes the reported
      native version, updates the header/libraries/manifest atomically, changes
      `go.lock_phase` to `resolved`, and passes the full version check.
- [ ] The Go Prep PR uses an exact squash body with the complete attribution
      footer; its merged commit is the intended tag target.
- [ ] TypeScript validation runs on that same `main` before tags.

## Final pretag gate

- [ ] Final `main` is synchronized and green.
- [ ] `make release-preflight` passes with Go `resolved`.
- [ ] Canonical and Go annotation message files are reviewed byte for byte.
- [ ] Both tags will be annotated, use those files, and peel to the same commit.

## Post-tag order

- [ ] Exact remote tags and tagged Go prebuilts pass the shared guard.
- [ ] Planned Rust crates are published and verified in dependency order.
- [ ] GitHub assets are downloaded, notes copied, checksummed, signed, verified,
      and uploaded while the release remains draft.
- [ ] Planned TypeScript packages are staged outside the protected environment,
      then resumably published from the attested artifact.
- [ ] `make release-guard-remote` passes.
- [ ] A maintainer cues `make release-publish`, the only draft promotion target.
