#!/usr/bin/env bash
# Create every tag in the release set, signed and local only, at HEAD.
# Publication is a separate command (release-push-tag.sh).
set -euo pipefail
# shellcheck source=scripts/release-tag-common.sh
source "$(dirname "$0")/release-tag-common.sh"
# shellcheck source=scripts/release-decernor.sh
source "$(dirname "$0")/release-decernor.sh"
cd "$tag_root"
# Every input is checked before anything is fetched or signed.
tag_version
resolve_release_decernor
tag_identity
tag_selector
tag_key_home
tag_checkout
tag_load_set HEAD
tags=("${RELEASE_TAGS[@]}")
for tag in "${tags[@]}"; do
	[[ -z "$(git ls-remote --tags origin "refs/tags/$tag")" ]] || tag_die "remote tag $tag already exists"
	[[ -z "$(git show-ref --tags "refs/tags/$tag" || true)" ]] || tag_die "local tag $tag already exists"
	tag_message_file "$tag" >/dev/null
done
export GIT_COMMITTER_NAME="$SYSPRIMS_TAGGER_NAME" GIT_COMMITTER_EMAIL="$SYSPRIMS_TAGGER_EMAIL"
# A failure part way through removes only the tags this run created, so a
# rerun starts from the same clean state.
created=()
rollback() {
	local tag
	for tag in ${created[@]+"${created[@]}"}; do git tag -d "$tag" >/dev/null 2>&1 || true; done
}
trap rollback EXIT
for tag in "${tags[@]}"; do
	# OpenPGP regardless of the host's gpg.format; the operator's gpg.program
	# (and so their agent and card) is kept.
	git -c gpg.format=openpgp tag -s -a --cleanup=verbatim -u "$SYSPRIMS_PGP_KEY_ID" -F "$(tag_message_file "$tag")" "$tag" HEAD
	created+=("$tag")
done
"$tag_root/scripts/release-verify-tag.sh"
trap - EXIT
echo "[ok] ${#tags[@]} signed tag(s) created locally; nothing was pushed"
