#!/usr/bin/env bash
# Stage public material from the commit the release tag points at. Files are
# read as git blobs; the working tree and the environment are never consulted.
#   release-stage-public.sh --anchors [dir]  the fingerprint anchors, before the
#                                            checksums (they are release assets)
#   release-stage-public.sh [dir]            the public keys, after signing; the
#                                            staged anchors must still equal the
#                                            tag's blobs and must verify the keys
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
# shellcheck source=scripts/release-tag-common.sh
source "$root/scripts/release-tag-common.sh"
# shellcheck source=scripts/release-decernor.sh
source "$root/scripts/release-decernor.sh"
cd "$root"
stage_mode=keys
if [[ "${1:-}" == --anchors ]]; then
	stage_mode=anchors
	shift
fi
directory="${1:-dist/release}"
tag_version
if [[ "$stage_mode" == keys ]]; then
	resolve_release_decernor
	export RELEASE_DECERNOR_BIN
fi
[[ -d "$directory" && ! -L "$directory" ]] || {
	echo "error: staging directory $directory must exist and not be a symlink" >&2
	exit 1
}
object="$(git rev-parse --verify --quiet "refs/tags/$SYSPRIMS_RELEASE_TAG")" || {
	echo "error: $SYSPRIMS_RELEASE_TAG is absent" >&2
	exit 1
}
commit="$(git rev-parse "${object}^{commit}")"
copy_blob() {
	local path="$1" output="$2" mode
	mode="$(git ls-tree "$commit" -- "$path" | awk '{print $1}')"
	[[ "$mode" == 100644 || "$mode" == 100755 ]] || {
		echo "error: committed $path is required in the tagged commit" >&2
		exit 1
	}
	[[ ! -L "$output" ]] || {
		echo "error: unsafe staging output $output" >&2
		exit 1
	}
	git cat-file blob "$commit:$path" >"$output"
	[[ -s "$output" ]] || {
		echo "error: committed $path is empty" >&2
		exit 1
	}
}
anchors=(expected-fingerprints.txt expected-fingerprints.ndjson)
if [[ "$stage_mode" == anchors ]]; then
	for name in "${anchors[@]}"; do
		[[ ! -e "$directory/$name" && ! -L "$directory/$name" ]] || {
			echo "error: $directory/$name already exists" >&2
			exit 1
		}
		copy_blob "keys/$name" "$directory/$name"
	done
	echo "[ok] staged the fingerprint anchors from $commit ($SYSPRIMS_RELEASE_TAG)"
	exit 0
fi
copy_blob docs/security/release-signing-keys.asc "$directory/sysprims-release-signing-key.asc"
copy_blob docs/security/sysprims-minisign.pub "$directory/sysprims-minisign.pub"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
# The anchors staged before the checksums must still be the tag's blobs, and
# must verify the keys just staged.
for name in "${anchors[@]}"; do
	copy_blob "keys/$name" "$scratch/$name"
	if [[ ! -f "$directory/$name" || -L "$directory/$name" ]] || ! cmp -s "$scratch/$name" "$directory/$name"; then
		echo "error: staged $name differs from keys/$name in the tagged commit" >&2
		exit 1
	fi
done
"$root/scripts/validate-release-anchors.sh" "$directory/expected-fingerprints.txt" "$directory/expected-fingerprints.ndjson" \
	"$directory/sysprims-release-signing-key.asc" "$directory/sysprims-minisign.pub"
"$root/scripts/verify-public-keys.sh" "$directory"
echo "[ok] staged public keys from $commit ($SYSPRIMS_RELEASE_TAG)"
