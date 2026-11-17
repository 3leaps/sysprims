# Release Pack Task

Use this template for the pull request that prepares one sysprims release.
Replace every placeholder before review.

## Release

- Version: `X.Y.Z`
- Final target commit: recorded after the release pack merges
- Surface plan: `docs/releases/vX.Y.Z.json`
- Release notes: `docs/releases/vX.Y.Z.md`

## Surface plan

- [ ] `provenance_policy` is `commit_footer`.
- [ ] Rust and CLI are `publish`; the plan declares no `ffi` surface.
- [ ] Go is `publish` or `skip`; a skip names the retained version.
- [ ] Go lock phase is `resolved` (pure Go, no prebuilt native libraries).
- [ ] Python is `publish` or `skip`; a skip names the retained version.
- [ ] TypeScript is `publish` or `skip`; a skip names the retained version.
- [ ] TypeScript has no lock phase: platform packages are pinned at publish
      time, and the committed lock must not resolve them.
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

- [ ] The version, changelog, notes, and surface plan are merged; the merged
      release-pack commit is the intended tag target.
- [ ] TypeScript validation runs on that same `main` before tags.

## Final pretag gate

- [ ] Final `main` is synchronized and green.
- [ ] `make release-preflight` passes with Go `resolved`.
- [ ] Canonical and recurrence annotation message files are reviewed byte for
      byte.
- [ ] `SYSPRIMS_TAG_MESSAGE_DIR` points to those reviewed files, named
      `vX.Y.Z.txt` and `go__recurrence__vX.Y.Z.txt`, for the post-tag
      stored-message comparison.
- [ ] Every tag will be annotated, use those files, and peel to the same commit.

## Post-tag order

- [ ] Exact remote tags pass the shared guard.
- [ ] Planned Rust crates are published and verified in dependency order.
- [ ] GitHub assets are downloaded, notes copied, checksummed, signed, verified,
      and uploaded while the release remains draft.
- [ ] Planned TypeScript packages are staged outside the protected environment,
      then resumably published from the attested artifact.
- [ ] `make release-guard-remote` passes.
- [ ] A maintainer cues `make release-publish`, the only draft promotion target.
