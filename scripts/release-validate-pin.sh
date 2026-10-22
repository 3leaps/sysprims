#!/usr/bin/env bash
# Maintainer-only: inspect the public pins without changing them. By default
# the files are read as git blobs at SYSPRIMS_ANCHOR_COMMIT (default HEAD).
# `--working-tree` reads the uncommitted files instead; it exists only for the
# one-time export step before the pins are first committed. No operator
# keyring is opened.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
# shellcheck source=scripts/release-decernor.sh
source "$root/scripts/release-decernor.sh"
cd "$root"
resolve_release_decernor
export RELEASE_DECERNOR_BIN
stop() {
	echo "STOP: $1" >&2
	exit 1
}
source_mode=commit
if [[ "${1:-}" == --working-tree ]]; then
	source_mode=working-tree
elif [[ $# -gt 0 ]]; then
	stop 'usage: release-validate-pin.sh [--working-tree]'
fi
commit="$(git rev-parse --verify "${SYSPRIMS_ANCHOR_COMMIT:-HEAD}^{commit}")"
scratch="$(mktemp -d)"
trap 'gpgconf --homedir "$scratch/gpg" --kill all >/dev/null 2>&1 || true; rm -rf "$scratch"' EXIT
mkdir -m 700 "$scratch/gpg"
blob() {
	local path="$1" out="$2"
	if [[ "$source_mode" == working-tree ]]; then
		[[ -f "$path" && ! -L "$path" ]] || stop "$path must be a regular file"
		cat "$path" >"$out"
	else
		[[ "$(git ls-tree "$commit" -- "$path" | awk '{print $1}')" == 100644 ]] ||
			stop "$path must be a committed regular file at $commit"
		git cat-file blob "$commit:$path" >"$out"
	fi
	[[ -s "$out" ]] || stop "$path is empty"
}
present() {
	if [[ "$source_mode" == working-tree ]]; then [[ -e "$1" ]]; else git cat-file -e "$commit:$1" 2>/dev/null; fi
}
blob docs/security/release-signing-keys.asc "$scratch/keys.asc"
blob docs/security/sysprims-minisign.pub "$scratch/minisign.pub"
blob config/release/signing-subkey.txt "$scratch/subkey.txt"

if grep -q -e 'PRIVATE KEY BLOCK' -e 'secret key' "$scratch/keys.asc" "$scratch/minisign.pub"; then
	stop 'private material in a public pin'
fi
grep -q '^untrusted comment:' "$scratch/minisign.pub" || stop 'minisign pin lacks its comment line'
"$RELEASE_DECERNOR_BIN" fingerprint "$scratch/minisign.pub" --class public --kind minisign \
	--format ndjson --path-mode none >/dev/null 2>&1 || stop 'minisign pin is not exactly one public key'

subkey="$(tr -d '\r\n' <"$scratch/subkey.txt")"
[[ "$subkey" =~ ^[0-9A-F]{40}$ ]] || stop 'signing-subkey.txt must be one uppercase 40-hex fingerprint'
shown="$(gpg --homedir "$scratch/gpg" --batch --with-colons --show-keys "$scratch/keys.asc" 2>/dev/null)" ||
	stop 'OpenPGP pin is not a readable public export'
[[ "$(awk -F: '$1=="sec" || $1=="ssb" {n++} END {print n+0}' <<<"$shown")" == 0 ]] ||
	stop 'private material in the OpenPGP pin'
# Exactly one primary and one signing subkey, and the subkey is the committed one.
layout="$(awk -F: '$1=="pub" || $1=="sub" {kind=$1; cap=$12; want=1; next}
	want && $1=="fpr" {print kind, $10, cap; want=0}' <<<"$shown")"
[[ "$(awk '{print $1}' <<<"$layout" | paste -sd, -)" == pub,sub ]] ||
	stop 'OpenPGP pin must hold exactly one primary and one subkey'
read -r _ pin_subkey pin_cap < <(sed -n 2p <<<"$layout")
[[ "$pin_subkey" == "$subkey" && "$pin_cap" == *[sS]* ]] ||
	stop 'OpenPGP pin subkey differs from config/release/signing-subkey.txt or cannot sign'

if present keys/expected-fingerprints.txt; then
	blob keys/expected-fingerprints.txt "$scratch/anchors.txt"
	blob keys/expected-fingerprints.ndjson "$scratch/anchors.ndjson"
	"$root/scripts/validate-release-anchors.sh" "$scratch/anchors.txt" "$scratch/anchors.ndjson" \
		"$scratch/keys.asc" "$scratch/minisign.pub"
fi
gpg --homedir "$scratch/gpg" --batch --show-keys --fingerprint --with-subkey-fingerprint "$scratch/keys.asc" ||
	stop 'isolated OpenPGP display failed'
echo "[ok] public pins (${source_mode}) validated; inspect the displayed expiry before tagging"
