#!/usr/bin/env bash
# Fetch the fingerprint anchors published with a release, verified.
# Usage: release-fetch-anchors.sh <dir>   (SYSPRIMS_RELEASE_TAG=vX.Y.Z required)
#
# Downloads the two anchors, both checksum manifests, their minisign and PGP
# signatures and the staged public keys into a scratch directory, then:
#   - verifies the manifest signatures with the pins committed at the tag
#     (scripts/verify-signatures.sh; the staged keys must equal those pins),
#   - requires each anchor to match its line in both signed manifests,
#   - requires each anchor to equal keys/<anchor> at the tag's commit.
# Only then are the anchors written, exclusively, into <dir>, which must be new
# or empty and outside keys/. Any mismatch refuses; nothing in keys/ is written.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
# shellcheck source=scripts/release-tag-common.sh
source "$root/scripts/release-tag-common.sh"
cd "$root"
die() {
	echo "error: $*" >&2
	exit 1
}
[[ $# -eq 1 && -n "$1" ]] || die 'usage: release-fetch-anchors.sh <dir>'
tag_version
out="$1"
[[ ! -L "$out" ]] || die "$out must not be a symlink"
# Where the output resolves, checked before anything is created: its nearest
# existing ancestor, resolved, plus the rest of the path.
existing="$out" rest=""
while [[ ! -e "$existing" ]]; do
	rest="/$(basename "$existing")$rest"
	existing="$(dirname "$existing")"
done
out_resolved="$(cd "$existing" && pwd -P)$rest"
case "$out_resolved/" in
"$root/keys/"*) die "$out is inside keys/; the committed anchors are never written by this command" ;;
esac
if [[ -e "$out" ]]; then
	[[ -d "$out" && -z "$(find "$out" -mindepth 1 -maxdepth 1 -print -quit)" ]] || die "$out must be a new or empty directory"
else
	mkdir -p "$out"
fi
out_real="$(cd "$out" && pwd -P)"
[[ "$out_real" == "$out_resolved" ]] || die "$out changed while it was being created"
object="$(git rev-parse --verify --quiet "refs/tags/$SYSPRIMS_RELEASE_TAG")" || die "$SYSPRIMS_RELEASE_TAG is absent"
commit="$(git rev-parse "${object}^{commit}")"

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
anchors=(expected-fingerprints.txt expected-fingerprints.ndjson)
assets=("${anchors[@]}" SHA256SUMS SHA512SUMS SHA256SUMS.minisig SHA512SUMS.minisig SHA256SUMS.asc SHA512SUMS.asc
	sysprims-minisign.pub sysprims-release-signing-key.asc)
if [[ -n "${SYSPRIMS_FETCH_ASSET_DIR:-}" ]]; then
	# Tests supply the published assets from a local directory.
	for name in "${assets[@]}"; do
		[[ -f "$SYSPRIMS_FETCH_ASSET_DIR/$name" ]] || die "the release has no $name"
		cp "$SYSPRIMS_FETCH_ASSET_DIR/$name" "$scratch/$name"
	done
else
	for name in "${assets[@]}"; do
		gh release download "$SYSPRIMS_RELEASE_TAG" --repo "$tag_repo_slug" --dir "$scratch" --pattern "$name" ||
			die "the release has no $name"
	done
fi
for name in "${assets[@]}"; do
	[[ -f "$scratch/$name" && ! -L "$scratch/$name" ]] || die "the release has no $name"
done

"$root/scripts/verify-signatures.sh" "$scratch" >/dev/null || die 'the release checksum signatures do not verify under the pins committed at the tag'
for name in "${anchors[@]}"; do
	sha256="$(shasum -a 256 "$scratch/$name" | awk '{print $1}')"
	sha512="$(shasum -a 512 "$scratch/$name" | awk '{print $1}')"
	[[ "$(awk -v n="$name" '$2==n || $2=="./"n {print $1}' "$scratch/SHA256SUMS")" == "$sha256" ]] ||
		die "$name does not match the signed SHA256SUMS"
	[[ "$(awk -v n="$name" '$2==n || $2=="./"n {print $1}' "$scratch/SHA512SUMS")" == "$sha512" ]] ||
		die "$name does not match the signed SHA512SUMS"
	mode="$(git ls-tree "$commit" -- "keys/$name" | awk '{print $1}')"
	[[ "$mode" == 100644 || "$mode" == 100755 ]] || die "committed keys/$name is required in the tagged commit"
	git cat-file blob "$commit:keys/$name" | cmp -s - "$scratch/$name" ||
		die "$name differs from keys/$name in the tagged commit"
done
for name in "${anchors[@]}"; do
	identity="$(python3 "$root/scripts/write-exclusive.py" "$scratch/$name" "$out_real/$name")" ||
		die "cannot write $out/$name"
	[[ -n "$identity" ]]
done
echo "[ok] $SYSPRIMS_RELEASE_TAG anchors verified (signed manifests, tagged commit) and written to $out"
