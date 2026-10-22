#!/usr/bin/env bash
# Pin validation reads the committed pins (or, only with --working-tree, the
# uncommitted first write) and requires exactly the committed signing subkey.
set -euo pipefail
# shellcheck source=scripts/release-test-fixture.sh
source "$(dirname "$0")/release-test-fixture.sh"
decernor="$(fixture_decernor)"
repo="$fixture_dir/repo"
read -r _ subkey < <(fixture_keys "$fixture_dir/gpg")
read -r _ other_subkey < <(fixture_keys "$fixture_dir/gpg-other")
fixture_repo "$repo" 1.2.3 publish 0
fixture_commit "$repo" base
gpg --homedir "$fixture_dir/gpg" --batch --armor --export-options export-minimal --export "$subkey!" \
	>"$repo/docs/security/release-signing-keys.asc"
fixture_minisign_pub "$repo/docs/security/sysprims-minisign.pub" 1
printf '%s\n' "$subkey" >"$repo/config/release/signing-subkey.txt"
validate() { (cd "$repo" && SYSPRIMS_DECERNOR_BIN="$decernor" ./scripts/release-validate-pin.sh "$@"); }

# Before the first commit only --working-tree can see the new pins.
expect_fail_with 'committed regular file' validate
validate --working-tree >/dev/null
expect_fail_with 'usage' validate --other
fixture_commit "$repo" pins
validate >/dev/null

# Anchors, once committed, must verify against the pins.
SYSPRIMS_DECERNOR_BIN="$decernor" "$repo/scripts/release-insert-anchors.sh" >/dev/null
fixture_commit "$repo" anchors
validate >/dev/null
fixture_minisign_pub "$repo/docs/security/sysprims-minisign.pub" 2
fixture_commit "$repo" rotated-pub-without-anchors
expect_fail_with 'does not verify' validate
git -C "$repo" checkout -q HEAD~1 -- docs/security/sysprims-minisign.pub
fixture_commit "$repo" restore

# The committed subkey file must name the pin's signing subkey.
printf '%s\n' "$other_subkey" >"$repo/config/release/signing-subkey.txt"
fixture_commit "$repo" wrong-subkey
expect_fail_with 'differs from config/release/signing-subkey.txt' validate
printf 'not-a-fingerprint\n' >"$repo/config/release/signing-subkey.txt"
fixture_commit "$repo" bad-subkey
expect_fail_with 'one uppercase 40-hex fingerprint' validate
printf '%s\n' "$subkey" >"$repo/config/release/signing-subkey.txt"
fixture_commit "$repo" good-subkey
validate >/dev/null

# An export with two subkeys, or private material, is refused.
gpg --homedir "$fixture_dir/gpg" --batch --armor --export >"$repo/docs/security/release-signing-keys.asc"
gpg --homedir "$fixture_dir/gpg-other" --batch --armor --export >>"$repo/docs/security/release-signing-keys.asc"
fixture_commit "$repo" two-keys
expect_fail validate
printf -- '-----BEGIN PGP PRIVATE KEY BLOCK-----\n' >"$repo/docs/security/release-signing-keys.asc"
fixture_commit "$repo" private
expect_fail_with 'private material' validate
echo '[ok] pin validation controls passed'
