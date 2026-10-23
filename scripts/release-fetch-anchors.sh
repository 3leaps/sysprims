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
# or empty and outside keys/. Any mismatch refuses; nothing in keys/ is written,
# and a failed run leaves nothing it created behind.
#
# The pins come from the local release tag, so that tag must be the signed,
# published one. With SYSPRIMS_DECERNOR_BIN set, the remote tag verification
# (scripts/release-verify-remote-tag.sh) runs first. Without it the command
# refuses, unless ANCHORS_TRUST_LOCAL_TAG=1 states that
# `make release-verify-remote-tag` has already passed on this clone.
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
# No `.` or `..` component: the path names its directory literally.
IFS=/ read -r -a parts <<<"$out"
for part in "${parts[@]}"; do
	[[ "$part" != . && "$part" != .. ]] || die "$out must not contain . or .. components"
done
# realpath resolves every existing symlink in the full path.
real_path() { python3 -c 'import os, sys; print(os.path.realpath(sys.argv[1], strict=False))' "$1"; }
outside_keys() {
	case "$1/" in
	"$root/keys/"*) die "$out is inside keys/; the committed anchors are never written by this command" ;;
	esac
}
out_target="$(real_path "$out")"
outside_keys "$out_target"
if [[ -e "$out" ]]; then
	[[ -d "$out" && -z "$(find "$out" -mindepth 1 -maxdepth 1 -print -quit)" ]] || die "$out must be a new or empty directory"
fi

# The local tag supplies the pins, so it must be the verified published tag.
if [[ -n "${SYSPRIMS_DECERNOR_BIN:-}" ]]; then
	"$root/scripts/release-verify-remote-tag.sh" >/dev/null ||
		die "$SYSPRIMS_RELEASE_TAG is not the verified published tag; nothing was fetched"
elif [[ "${ANCHORS_TRUST_LOCAL_TAG:-}" != 1 ]]; then
	die "set SYSPRIMS_DECERNOR_BIN to verify $SYSPRIMS_RELEASE_TAG against the published tag first, or, if make release-verify-remote-tag has already passed on this clone, ANCHORS_TRUST_LOCAL_TAG=1"
fi
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
# Only now is anything created. On a failure, what this run wrote is removed
# (by ownership token) and the directory too, if this run made it.
created_dir=""
written=()
undo() {
	local entry
	for entry in ${written[@]+"${written[@]}"}; do
		python3 "$root/scripts/write-exclusive.py" --remove "${entry#* }" "${entry%% *}" || true
	done
	[[ -z "$created_dir" ]] || rmdir "$created_dir" 2>/dev/null || true
	rm -rf "$scratch"
}
trap undo EXIT
if [[ ! -e "$out" ]]; then
	mkdir "$out" || die "cannot create $out (its parent must exist)"
	created_dir="$out"
fi
out_real="$(cd "$out" && pwd -P)"
[[ "$out_real" == "$out_target" ]] || die "$out changed while it was being created"
outside_keys "$out_real"
for name in "${anchors[@]}"; do
	identity="$(python3 "$root/scripts/write-exclusive.py" "$scratch/$name" "$out_real/$name")" ||
		die "cannot write $out/$name"
	written+=("$identity $out_real/$name")
done
written=()
created_dir=""
trap 'rm -rf "$scratch"' EXIT
echo "[ok] $SYSPRIMS_RELEASE_TAG anchors verified (signed manifests, tagged commit) and written to $out"
