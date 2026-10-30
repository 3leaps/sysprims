#!/usr/bin/env bash
# Create the draft GitHub release for the verified tag from the complete locally
# signed asset set, in one call. Runs on the maintainer machine only; CI holds no
# write authority. Assets are never replaced: an existing release refuses.
# Usage: release-create-draft.sh [dir]
# Requires SYSPRIMS_RELEASE_TAG. The release stays a draft; only
# `make release-publish` promotes it.
#
# The signed set is copied into a private snapshot first, and the snapshot is
# what is verified and uploaded, so later changes to the staging directory
# cannot reach the release. The CI-built files in the snapshot must equal the
# artifact as downloaded again from GitHub (not the local anchor's records).
set -euo pipefail
die() {
	echo "error: $*" >&2
	exit 1
}
dir="${1:-dist/release}"
tag="${SYSPRIMS_RELEASE_TAG:-}"
[[ "$tag" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || die 'SYSPRIMS_RELEASE_TAG=vX.Y.Z is required'
command -v gh >/dev/null 2>&1 || die 'gh (GitHub CLI) not found in PATH'
root="$(cd "$(dirname "$0")/.." && pwd -P)"
repo=3leaps/sysprims
[[ -d "$dir" && ! -L "$dir" ]] || die "release directory not found: $dir"
anchor="$dir.anchor"
[[ -f "$anchor" && ! -L "$anchor" ]] || die "missing $anchor; stage the CI payload with make release-download"

snapshot="$(mktemp -d)"
chmod 700 "$snapshot"
trap 'rm -rf "$snapshot"' EXIT
inventory="$(node --input-type=module -e "
	import { expectedInventory } from '$root/scripts/release-integrity.mjs';
	console.log(expectedInventory('${tag#v}').signed.join('\\n'));")" || die 'could not resolve the signed inventory'
count=0
for name in $inventory; do
	[[ -f "$dir/$name" && ! -L "$dir/$name" ]] || die "missing or not a regular file: $name"
	cp -p "$dir/$name" "$snapshot/$name"
	count=$((count + 1))
done
[[ "$count" -gt 0 ]] || die 'could not resolve the signed inventory'
cp -p "$anchor" "$snapshot.anchor"
trap 'rm -rf "$snapshot" "$snapshot.anchor"' EXIT

# The snapshot is the exact signed state: inventory, signatures and keys from
# the committed pins, tag-blob assets, checksums, and the fetch record.
node "$root/scripts/release-integrity.mjs" signed --dir "$snapshot" >/dev/null || die 'signed release state does not verify'

# The CI-built files must be the artifact as GitHub serves it now. This also
# re-verifies the pushed tags and binds them to the staged tag object.
"$root/scripts/release-fetch-ci-artifacts.sh" --verify "$snapshot.anchor" >"$snapshot.remote" ||
	die 'the CI artifact named by the staged anchor no longer verifies'
trap 'rm -rf "$snapshot" "$snapshot.anchor" "$snapshot.remote"' EXIT
# The downloaded artifact holds exactly the CI-built files (by name), and each
# equals the snapshot's copy.
built="$(node --input-type=module -e "
	import { expectedInventory, TAGGED_ASSET_NAMES } from '$root/scripts/release-integrity.mjs';
	const v = '${tag#v}';
	console.log(expectedInventory(v).base.filter((n) => !TAGGED_ASSET_NAMES(v).includes(n)).join('\\n'));")" ||
	die 'could not resolve the CI-built inventory'
names=""
while IFS= read -r line; do
	[[ "$line" =~ ^file=([0-9a-f]{64})\ ([A-Za-z0-9][A-Za-z0-9._-]*)$ ]] || die "unexpected artifact record: $line"
	sum="${BASH_REMATCH[1]}"
	name="${BASH_REMATCH[2]}"
	names+="$name"$'\n'
	[[ -f "$snapshot/$name" ]] || die "artifact file $name is not in the signed set"
	[[ "$(shasum -a 256 "$snapshot/$name" | awk '{print $1}')" == "$sum" ]] ||
		die "$name differs from the CI artifact as downloaded from GitHub"
done <"$snapshot.remote"
[[ "$(printf '%s' "$names" | LC_ALL=C sort)" == "$(printf '%s\n' "$built" | LC_ALL=C sort)" ]] ||
	die 'the downloaded artifact does not hold exactly the CI-built files'

# A release for this tag must not exist. Existence is read from the list of all
# releases (drafts included); any API failure stops here.
releases="$(gh api --paginate "repos/$repo/releases?per_page=100" --jq '.[].tag_name')" ||
	die "could not list releases for $repo; existence of $tag is unknown"
if grep -qxF -- "$tag" <<<"$releases"; then
	die "a release for $tag already exists; assets are never replaced"
fi

commit="$(awk -F= '$1=="commit" {print $2}' "$snapshot.anchor")"
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || die 'staged anchor is malformed'
files=()
for name in $inventory; do
	files+=("$snapshot/$name")
done
gh release create "$tag" --repo "$repo" --verify-tag --draft --target "$commit" \
	--title "$tag" --notes-file "$snapshot/release-notes-$tag.md" "${files[@]}"
echo "[ok] draft release $tag created with ${#files[@]} assets; it remains unpublished"
