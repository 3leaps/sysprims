#!/usr/bin/env bash
# Stage the public signing material and the fingerprint anchors from the commit
# the release tag points at. Files are read as git blobs; the working tree and
# the environment are never consulted. Usage: release-stage-public.sh [dir]
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
# shellcheck source=scripts/release-tag-common.sh
source "$root/scripts/release-tag-common.sh"
# shellcheck source=scripts/release-decernor.sh
source "$root/scripts/release-decernor.sh"
cd "$root"
directory="${1:-dist/release}"
tag_version
resolve_release_decernor
export RELEASE_DECERNOR_BIN
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
	[[ "$mode" == 100644 ]] || {
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
copy_blob docs/security/release-signing-keys.asc "$directory/sysprims-release-signing-key.asc"
copy_blob docs/security/sysprims-minisign.pub "$directory/sysprims-minisign.pub"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
copy_blob keys/expected-fingerprints.txt "$scratch/anchors.txt"
copy_blob keys/expected-fingerprints.ndjson "$scratch/anchors.ndjson"
"$root/scripts/validate-release-anchors.sh" "$scratch/anchors.txt" "$scratch/anchors.ndjson" \
	"$directory/sysprims-release-signing-key.asc" "$directory/sysprims-minisign.pub"
"$root/scripts/verify-public-keys.sh" "$directory"
echo "[ok] staged public keys from $commit ($SYSPRIMS_RELEASE_TAG)"
