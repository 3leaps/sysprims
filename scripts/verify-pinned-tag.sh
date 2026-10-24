#!/usr/bin/env bash
# Verify every tag in the release set against the public pin committed in the
# tagged commit, in an isolated keyring. All tags must be annotated, name
# themselves, and peel to one commit; each must be signed by the committed
# signing subkey of the pinned primary.
# Usage: verify-pinned-tag.sh [--published]
#   --published  the tagged commit need not be HEAD (post-release checks)
# Requires SYSPRIMS_RELEASE_TAG; callers resolve the checker
# (scripts/release-decernor.sh) and export RELEASE_DECERNOR_BIN.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
# shellcheck source=scripts/release-tag-common.sh
source "$root/scripts/release-tag-common.sh"
die() {
	echo "error: $*" >&2
	exit 1
}
published=0
if [[ "${1:-}" == --published ]]; then
	published=1
elif [[ $# -gt 0 ]]; then
	die 'usage: verify-pinned-tag.sh [--published]'
fi
: "${RELEASE_DECERNOR_BIN:?resolve the checker before verifying tags}"
tag_version
canonical="$(git rev-parse --verify --quiet "refs/tags/$SYSPRIMS_RELEASE_TAG")" || die "$SYSPRIMS_RELEASE_TAG is absent"
target="$(git rev-parse "${canonical}^{commit}")"
[[ "$published" == 1 || "$target" == "$(git rev-parse HEAD)" ]] || die 'the release tag must point at HEAD'

keyring="$(mktemp -d)"
trap 'gpgconf --homedir "$keyring/home" --kill all >/dev/null 2>&1 || true; rm -rf "$keyring"' EXIT
chmod 700 "$keyring"
# The trust root is read only as inert blobs from the tagged commit; the working
# tree and the environment are never consulted for it.
committed() {
	local path="$1" out="$2" mode
	mode="$(git ls-tree "$target" -- "$path" | awk '{print $1}')"
	[[ "$mode" == 100644 ]] || die "committed $path is required in the tagged commit"
	git cat-file blob "$target:$path" >"$out"
	[[ -s "$out" ]] || die "committed $path is empty"
}
committed docs/security/release-signing-keys.asc "$keyring/keys.asc"
committed docs/security/sysprims-minisign.pub "$keyring/minisign.pub"
committed keys/expected-fingerprints.txt "$keyring/anchors.txt"
committed keys/expected-fingerprints.ndjson "$keyring/anchors.ndjson"
committed config/release/tagger-identity.txt "$keyring/tagger-identity.txt"
committed config/release/signing-subkey.txt "$keyring/signing-subkey.txt"
"$root/scripts/validate-release-anchors.sh" "$keyring/anchors.txt" "$keyring/anchors.ndjson" \
	"$keyring/keys.asc" "$keyring/minisign.pub" >/dev/null || die 'committed pin and anchors disagree'
primary="$(awk '$1=="gpg" {print $2}' "$keyring/anchors.txt")"
permitted="$(tr -d '\r\n' <"$keyring/signing-subkey.txt")"
[[ "$primary" =~ ^[0-9A-F]{40}$ && "$permitted" =~ ^[0-9A-F]{40}$ && "$permitted" != "$primary" ]] ||
	die 'committed primary or signing subkey is malformed'
mkdir -m 700 "$keyring/home"
GNUPGHOME="$keyring/home" gpg --batch --quiet --import "$keyring/keys.asc" || die 'pin import failed'
[[ "$(GNUPGHOME="$keyring/home" gpg --batch --with-colons --list-keys | awk -F: '$1=="pub" {n++} END {print n+0}')" == 1 ]] ||
	die 'the pin must hold exactly one primary key'

# The required set is decided by the tagged commit, never the checkout.
tag_load_set "$target"
count=0
for tag in "${RELEASE_TAGS[@]}"; do
	object="$(git rev-parse --verify --quiet "refs/tags/$tag")" || die "$tag is absent"
	[[ "$(git cat-file -t "$object")" == tag ]] || die "$tag must be an annotated tag"
	[[ "$(git cat-file tag "$object" | sed -n 's/^tag //p' | head -1)" == "$tag" ]] || die "$tag object names a different tag"
	[[ "$(git rev-parse "${object}^{commit}")" == "$target" ]] || die "$tag does not point at the release commit $target"
	tagger="$(git cat-file tag "$object" | sed -n 's/^tagger \(.*\) [0-9][0-9]* [+-][0-9][0-9][0-9][0-9]$/\1/p' | head -1)"
	[[ "$tagger" == "$(cat "$keyring/tagger-identity.txt")" ]] || die "$tag tagger differs from the committed identity"
	GNUPGHOME="$keyring/home" git -c gpg.program=gpg verify-tag --raw "$object" >"$keyring/status" 2>&1 || {
		awk '$1=="[GNUPG:]" {print "signature status:", $2}' "$keyring/status" >&2
		die "$tag signature is invalid under the pin"
	}
	awk -v primary="$primary" -v permitted="$permitted" '
		$1=="[GNUPG:]" && $2=="VALIDSIG" {valid++; subkey=$3; signer_primary=$NF}
		$1=="[GNUPG:]" && $2=="GOODSIG" {good++}
		$1=="[GNUPG:]" && $2 ~ /^(EXPKEYSIG|EXPSIG|REVKEYSIG|KEYREVOKED|BADSIG|ERRSIG)$/ {bad++}
		END {if (bad || good != 1 || valid != 1 || signer_primary != primary || subkey != permitted) exit 1}
	' "$keyring/status" || die "$tag is not signed by the committed signing subkey, or the key is expired or revoked"
	count=$((count + 1))
done
[[ "$count" == "${#RELEASE_TAGS[@]}" ]] || die 'not every release tag was verified'
echo "[ok] $count signed tag(s) verified at $target against the pin committed in that commit"
