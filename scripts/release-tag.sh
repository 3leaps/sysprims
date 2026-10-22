#!/usr/bin/env bash
# Create every tag in the release set, signed and local only, at HEAD.
# Publication is a separate command (release-push-tag.sh).
set -euo pipefail
# shellcheck source=scripts/release-tag-common.sh
source "$(dirname "$0")/release-tag-common.sh"
cd "$tag_root"
tag_version
tag_identity
tag_selector
tag_key_home
tag_checkout
tags=()
while IFS= read -r line; do tags+=("$line"); done < <(tag_set)
[[ "${#tags[@]}" -ge 1 ]] || tag_die 'empty release tag set'
for tag in "${tags[@]}"; do
	[[ -z "$(git ls-remote --tags origin "refs/tags/$tag")" ]] || tag_die "remote tag $tag already exists"
	[[ -z "$(git show-ref --tags "refs/tags/$tag" || true)" ]] || tag_die "local tag $tag already exists"
	tag_message_file "$tag" >/dev/null
done
export GIT_COMMITTER_NAME="$SYSPRIMS_TAGGER_NAME" GIT_COMMITTER_EMAIL="$SYSPRIMS_TAGGER_EMAIL"
for tag in "${tags[@]}"; do
	git tag -s -a --cleanup=verbatim -u "$SYSPRIMS_PGP_KEY_ID" -F "$(tag_message_file "$tag")" "$tag" HEAD
done
"$tag_root/scripts/release-verify-tag.sh"
echo "[ok] ${#tags[@]} signed tag(s) created locally; nothing was pushed"
