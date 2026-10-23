#!/usr/bin/env bash
# Push the verified local release tag set in one atomic push, then verify the
# remote tags.
set -euo pipefail
# shellcheck source=scripts/release-tag-common.sh
source "$(dirname "$0")/release-tag-common.sh"
cd "$tag_root"
tag_version
"$tag_root/scripts/release-verify-tag.sh"
tag_require_clean
tag_load_set HEAD
tags=("${RELEASE_TAGS[@]}")
refs=()
for tag in "${tags[@]}"; do
	[[ -z "$(git ls-remote --tags origin "refs/tags/$tag")" ]] || tag_die "remote tag $tag already exists"
	refs+=("refs/tags/$tag")
done
git push --atomic origin "${refs[@]}"
"$tag_root/scripts/release-verify-remote-tag.sh"
