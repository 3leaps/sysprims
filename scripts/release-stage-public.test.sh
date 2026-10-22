#!/usr/bin/env bash
# Staging copies the public keys from the commit the release tag points at; the
# working tree and any environment path are ignored.
set -euo pipefail
# shellcheck source=scripts/release-test-fixture.sh
source "$(dirname "$0")/release-test-fixture.sh"
decernor="$(fixture_decernor)"
version=1.2.3
repo="$fixture_dir/repo"
read -r _ subkey < <(fixture_keys "$fixture_dir/gpg")
fixture_repo "$repo" "$version" publish 0
gpg --homedir "$fixture_dir/gpg" --batch --armor --export-options export-minimal --export "$subkey!" \
	>"$repo/docs/security/release-signing-keys.asc"
fixture_minisign_pub "$repo/docs/security/sysprims-minisign.pub" 1
printf '%s\n' "$subkey" >"$repo/config/release/signing-subkey.txt"
fixture_commit "$repo" pins
SYSPRIMS_DECERNOR_BIN="$decernor" "$repo/scripts/release-insert-anchors.sh" >/dev/null
fixture_commit "$repo" anchors
git -C "$repo" tag -a -m "Release $version" "v$version"
stage() { (cd "$repo" && SYSPRIMS_RELEASE_TAG="v$version" SYSPRIMS_DECERNOR_BIN="$decernor" ./scripts/release-stage-public.sh "$@"); }
mkdir "$fixture_dir/dist"
committed_pub="$fixture_dir/committed.pub"
cp "$repo/docs/security/sysprims-minisign.pub" "$committed_pub"

# After the tag: a later commit and an uncommitted edit both change the pin.
fixture_minisign_pub "$repo/docs/security/sysprims-minisign.pub" 2
fixture_commit "$repo" later-pin
fixture_minisign_pub "$repo/docs/security/sysprims-minisign.pub" 3
SYSPRIMS_MINISIGN_PUB="$repo/docs/security/sysprims-minisign.pub" stage "$fixture_dir/dist" >/dev/null
cmp "$committed_pub" "$fixture_dir/dist/sysprims-minisign.pub"
git -C "$repo" cat-file blob "v$version:docs/security/release-signing-keys.asc" |
	cmp - "$fixture_dir/dist/sysprims-release-signing-key.asc"

expect_fail_with 'SYSPRIMS_RELEASE_TAG is required' bash -c "cd '$repo' && env -u SYSPRIMS_RELEASE_TAG SYSPRIMS_DECERNOR_BIN='$decernor' ./scripts/release-stage-public.sh '$fixture_dir/dist'"
expect_fail_with 'must exist' stage "$fixture_dir/absent"
ln -s "$fixture_dir/dist" "$fixture_dir/dist-link"
expect_fail_with 'not be a symlink' stage "$fixture_dir/dist-link"

# Anchors in the tagged commit that disagree with its pins fail staging.
git -C "$repo" checkout -q "v$version" -- docs/security/sysprims-minisign.pub
git -C "$repo" tag -d "v$version" >/dev/null
fixture_minisign_pub "$repo/docs/security/sysprims-minisign.pub" 4
fixture_commit "$repo" pin-without-anchors
git -C "$repo" tag -a -m "Release $version" "v$version"
expect_fail_with 'does not verify' stage "$fixture_dir/dist"
echo '[ok] public staging controls passed'
