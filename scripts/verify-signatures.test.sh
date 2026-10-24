#!/usr/bin/env bash
# Manifest signatures verify only under the pins committed in the tagged
# commit. Replacing the staged public keys and the signatures together after
# staging fails, and so does a signature by any other key.
set -euo pipefail
# shellcheck source=scripts/release-test-fixture.sh
source "$(dirname "$0")/release-test-fixture.sh"
version=1.2.3
repo="$fixture_dir/repo"
dist="$fixture_dir/dist"
read -r _ subkey < <(fixture_keys "$fixture_dir/gpg")
read -r primary _ < <(
	gpg --homedir "$fixture_dir/gpg" --batch --with-colons --list-keys 2>/dev/null | awk -F: '$1=="fpr" {print $10; exit}'
	echo
)
read -r _ other_subkey < <(fixture_keys "$fixture_dir/gpg-other")
# A second signing subkey on the pinned primary: valid under the pin, but not
# the committed signing subkey.
gpg --homedir "$fixture_dir/gpg" --batch --pinentry-mode loopback --passphrase '' \
	--quick-add-key "$primary" ed25519 sign 1d >/dev/null 2>&1
sibling="$(gpg --homedir "$fixture_dir/gpg" --batch --with-colons --with-subkey-fingerprint --list-keys |
	awk -F: '$1=="sub" {s=1; next} s && $1=="fpr" {print $10; s=0}' | grep -v "^$subkey$")"
[[ "$sibling" =~ ^[0-9A-F]{40}$ ]]
minisign -G -W -p "$fixture_dir/pinned.pub" -s "$fixture_dir/pinned.key" >/dev/null
minisign -G -W -p "$fixture_dir/other.pub" -s "$fixture_dir/other.key" >/dev/null

fixture_repo "$repo" "$version" publish 0
cp "$fixture_root/scripts/verify-signatures.sh" "$repo/scripts/"
# The pin carries the primary and both subkeys; only signing-subkey.txt may sign.
gpg --homedir "$fixture_dir/gpg" --batch --armor --export-options export-minimal --export "$primary" \
	>"$repo/docs/security/release-signing-keys.asc"
cp "$fixture_dir/pinned.pub" "$repo/docs/security/sysprims-minisign.pub"
printf '%s\n' "$subkey" >"$repo/config/release/signing-subkey.txt"
fixture_commit "$repo" pins
git -C "$repo" tag -a -m "Release $version" "v$version"

# sign_dist <minisign-key> <gpg-home> <subkey>: manifests and both signatures.
sign_dist() {
	rm -rf "$dist"
	mkdir -p "$dist"
	printf 'payload\n' >"$dist/payload"
	(cd "$dist" && shasum -a 256 payload >SHA256SUMS && shasum -a 512 payload >SHA512SUMS)
	for manifest in SHA256SUMS SHA512SUMS; do
		minisign -S -W -s "$1" -m "$dist/$manifest" -x "$dist/$manifest.minisig" >/dev/null
		gpg --homedir "$2" --batch --pinentry-mode loopback --passphrase '' --armor --detach-sign \
			-u "$3!" -o "$dist/$manifest.asc" "$dist/$manifest"
	done
}
stage_pinned() {
	git -C "$repo" cat-file blob "v$version:docs/security/sysprims-minisign.pub" >"$dist/sysprims-minisign.pub"
	git -C "$repo" cat-file blob "v$version:docs/security/release-signing-keys.asc" >"$dist/sysprims-release-signing-key.asc"
}
verify() { (cd "$repo" && SYSPRIMS_RELEASE_TAG="v$version" ./scripts/verify-signatures.sh "$dist"); }

sign_dist "$fixture_dir/pinned.key" "$fixture_dir/gpg" "$subkey"
stage_pinned
verify >/dev/null

# Keys and signatures replaced together after staging: the staged keys no
# longer match the pins, and the substitute signatures do not verify under them.
sign_dist "$fixture_dir/other.key" "$fixture_dir/gpg-other" "$other_subkey"
cp "$fixture_dir/other.pub" "$dist/sysprims-minisign.pub"
gpg --homedir "$fixture_dir/gpg-other" --batch --armor --export-options export-minimal --export "$other_subkey!" \
	>"$dist/sysprims-release-signing-key.asc"
expect_fail_with 'differs from the pin in the tagged commit' verify

# Pinned keys staged, signatures by other keys: verification uses the pins.
stage_pinned
expect_fail_with 'signature verification error' verify

# Pinned minisign, other PGP signer: the PGP check requires the committed subkey.
sign_dist "$fixture_dir/pinned.key" "$fixture_dir/gpg-other" "$other_subkey"
stage_pinned
expect_fail_with 'signature verification error' verify

# Signed by the pinned primary's other subkey: valid under the pin, refused
# because it is not the committed signing subkey.
sign_dist "$fixture_dir/pinned.key" "$fixture_dir/gpg" "$sibling"
stage_pinned
expect_fail_with 'signature verification error' verify

# Only the minisign public key replaced in staging (signatures untouched):
# the byte comparison with the pin refuses it.
sign_dist "$fixture_dir/pinned.key" "$fixture_dir/gpg" "$subkey"
stage_pinned
cp "$fixture_dir/other.pub" "$dist/sysprims-minisign.pub"
expect_fail_with 'staged sysprims-minisign.pub differs' verify

# Only the OpenPGP export replaced in staging: the byte comparison refuses it.
sign_dist "$fixture_dir/pinned.key" "$fixture_dir/gpg" "$subkey"
stage_pinned
gpg --homedir "$fixture_dir/gpg-other" --batch --armor --export-options export-minimal --export "$other_subkey!" \
	>"$dist/sysprims-release-signing-key.asc"
expect_fail_with 'staged sysprims-release-signing-key.asc differs' verify

# Staged keys untouched but minisign signatures by the other key: verification
# uses the pinned key, not whatever key file sits beside the manifests.
sign_dist "$fixture_dir/other.key" "$fixture_dir/gpg" "$subkey"
stage_pinned
cp "$fixture_dir/other.pub" "$dist/staged-decoy.pub"
expect_fail_with 'signature verification error' verify

# Missing signatures and a missing tag input fail.
sign_dist "$fixture_dir/pinned.key" "$fixture_dir/gpg" "$subkey"
stage_pinned
rm "$dist/SHA512SUMS.asc"
expect_fail_with 'SHA512SUMS.asc is missing' verify
expect_fail_with 'SYSPRIMS_RELEASE_TAG' bash -c "cd '$repo' && env -u SYSPRIMS_RELEASE_TAG ./scripts/verify-signatures.sh '$dist'"

# Pins edited after the tag are ignored: the tagged commit decides.
sign_dist "$fixture_dir/pinned.key" "$fixture_dir/gpg" "$subkey"
stage_pinned
cp "$fixture_dir/other.pub" "$repo/docs/security/sysprims-minisign.pub"
fixture_commit "$repo" later-pin
verify >/dev/null
echo '[ok] manifest signature controls passed'
