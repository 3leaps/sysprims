#!/usr/bin/env bash
# Verify the minisign and PGP signatures on both checksum manifests.
# Usage: verify-signatures.sh [dir]   (SYSPRIMS_RELEASE_TAG=vX.Y.Z required)
#
# The keys used are the public pins committed in the tagged commit, read as git
# blobs. The staged copies in the directory must match them byte-for-byte; they
# are never used to verify anything.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
dir="${1:-dist/release}"
die() {
	echo "error: $*" >&2
	exit 1
}
[[ -d "$dir" ]] || die "directory $dir does not exist"
: "${SYSPRIMS_RELEASE_TAG:?SYSPRIMS_RELEASE_TAG=vX.Y.Z is required}"
object="$(git -C "$root" rev-parse --verify --quiet "refs/tags/$SYSPRIMS_RELEASE_TAG")" || die "$SYSPRIMS_RELEASE_TAG is absent"
commit="$(git -C "$root" rev-parse "${object}^{commit}")"

scratch="$(mktemp -d)"
trap 'gpgconf --homedir "$scratch/gpg" --kill all >/dev/null 2>&1 || true; rm -rf "$scratch"' EXIT
pin() {
	local path="$1" out="$2"
	[[ "$(git -C "$root" ls-tree "$commit" -- "$path" | awk '{print $1}')" == 100644 ]] ||
		die "committed $path is required in the tagged commit"
	git -C "$root" cat-file blob "$commit:$path" >"$out"
	[[ -s "$out" ]] || die "committed $path is empty"
}
pin docs/security/sysprims-minisign.pub "$scratch/minisign.pub"
pin docs/security/release-signing-keys.asc "$scratch/keys.asc"
pin config/release/signing-subkey.txt "$scratch/subkey.txt"
cmp -s "$scratch/minisign.pub" "$dir/sysprims-minisign.pub" ||
	die 'staged sysprims-minisign.pub differs from the pin in the tagged commit'
cmp -s "$scratch/keys.asc" "$dir/sysprims-release-signing-key.asc" ||
	die 'staged sysprims-release-signing-key.asc differs from the pin in the tagged commit'

errors=0
for manifest in SHA256SUMS SHA512SUMS; do
	[[ -f "$dir/$manifest" ]] || die "$manifest is missing"
	[[ -f "$dir/$manifest.minisig" ]] || die "$manifest.minisig is missing"
	if minisign -Vm "$dir/$manifest" -x "$dir/$manifest.minisig" -p "$scratch/minisign.pub" >/dev/null; then
		echo "[ok] $manifest minisign signature valid under the committed pin"
	else
		echo "[!!] $manifest minisign signature invalid under the committed pin" >&2
		errors=$((errors + 1))
	fi
done

mkdir -m 700 "$scratch/gpg"
gpg --homedir "$scratch/gpg" --batch --quiet --import "$scratch/keys.asc" || die 'committed OpenPGP pin import failed'
subkey="$(tr -d '\r\n' <"$scratch/subkey.txt")"
for manifest in SHA256SUMS SHA512SUMS; do
	[[ -f "$dir/$manifest.asc" ]] || die "$manifest.asc is missing"
	if gpg --homedir "$scratch/gpg" --batch --status-fd 1 --verify "$dir/$manifest.asc" "$dir/$manifest" 2>/dev/null |
		awk -v subkey="$subkey" '$2=="VALIDSIG" {valid++; if ($3 != subkey) bad++} $2 ~ /^(BADSIG|ERRSIG|EXPKEYSIG|REVKEYSIG)$/ {bad++}
			END {exit (valid == 1 && !bad) ? 0 : 1}'; then
		echo "[ok] $manifest PGP signature valid under the committed signing subkey"
	else
		echo "[!!] $manifest PGP signature invalid under the committed signing subkey" >&2
		errors=$((errors + 1))
	fi
done

[[ "$errors" == 0 ]] || die "$errors signature verification error(s)"
echo '[ok] all signatures verified against the pins committed in the tagged commit'
