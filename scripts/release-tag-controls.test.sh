#!/usr/bin/env bash
# Tag inputs: the explicit release tag, the derived tag set, the per-tag message
# files, the subkey selector and the Makefile's refusal of a bare RELEASE_TAG.
# shellcheck disable=SC2016
set -euo pipefail
# shellcheck source=scripts/release-test-fixture.sh
source "$(dirname "$0")/release-test-fixture.sh"
version=1.2.3
repo="$fixture_dir/repo"
fixture_repo "$repo" "$version" publish 1
printf '%s\n' 0123456789ABCDEF0123456789ABCDEF01234567 >"$repo/config/release/signing-subkey.txt"
fixture_commit "$repo"
lib="$repo/scripts/release-tag-common.sh"
in_repo() { (cd "$repo" && env "$@"); }
call() { in_repo "${@:2}" bash -c "source '$lib'; $1"; }

# Every checklist `make` target exists. The recurrence targets arrive with the
# recurrence module itself, so they are only required once it is in the tree.
while IFS= read -r target; do
	[[ "$target" == recurrence-* && ! -f "$fixture_root/go/recurrence/go.mod" ]] && continue
	grep -Eq "^${target}:" "$fixture_root/Makefile" || {
		echo "error: RELEASE_CHECKLIST.md names missing make target $target" >&2
		exit 1
	}
done < <(grep -oE 'make [a-z][a-z0-9-]+' "$fixture_root/RELEASE_CHECKLIST.md" | awk '{print $2}' | sort -u)

# The Makefile refuses a bare RELEASE_TAG from any origin and derives no default.
expect_fail_with 'RELEASE_TAG is not accepted' env RELEASE_TAG=v9.9.9 make -s -C "$fixture_root" -n help
expect_fail_with 'RELEASE_TAG is not accepted' env -u RELEASE_TAG make -s -C "$fixture_root" -n help RELEASE_TAG=v9.9.9
expect_fail_with 'RELEASE_TAG is not accepted' env RELEASE_TAG= make -s -C "$fixture_root" -n help
expect_fail_with 'SYSPRIMS_RELEASE_TAG=vX.Y.Z is required' env -u RELEASE_TAG -u SYSPRIMS_RELEASE_TAG make -s -C "$fixture_root" release-tag
if grep -En 'SYSPRIMS_RELEASE_TAG[[:space:]]*\?=|v\$\(VERSION\)' "$fixture_root/Makefile"; then
	echo 'error: the Makefile derives a release tag' >&2
	exit 1
fi

# The release tag is required, canonical, and equal to VERSION.
expect_fail_with 'SYSPRIMS_RELEASE_TAG is required' call tag_version -u SYSPRIMS_RELEASE_TAG
expect_fail_with 'canonical vX.Y.Z' call tag_version SYSPRIMS_RELEASE_TAG=1.2.3
expect_fail_with 'does not match VERSION' call tag_version SYSPRIMS_RELEASE_TAG=v1.2.4
call tag_version SYSPRIMS_RELEASE_TAG="v$version"

# The tag set: canonical, Go binding when published, recurrence when present.
[[ "$(call tag_set SYSPRIMS_RELEASE_TAG="v$version" | paste -sd, -)" == "v$version,bindings/go/sysprims/v$version,go/recurrence/v$version" ]]
printf '{"surfaces":{"go":{"disposition":"skip","retained_version":"0.0.1"}}}\n' >"$repo/docs/releases/v$version.json"
[[ "$(call tag_set SYSPRIMS_RELEASE_TAG="v$version" | paste -sd, -)" == "v$version,go/recurrence/v$version" ]]
rm -rf "$repo/go"
[[ "$(call tag_set SYSPRIMS_RELEASE_TAG="v$version" | paste -sd, -)" == "v$version" ]]
printf 'v\nother/v\n' >"$repo/config/release/release-tags.txt"
expect_fail_with 'unknown tag prefix' call tag_set SYSPRIMS_RELEASE_TAG="v$version"
git -C "$repo" checkout -q -- .

# Message files: one per tag, outside the repo, under a directory named for the
# tag, complete, with the expected subject.
messages="$fixture_dir/messages/v$version"
fixture_messages "$messages" "$version" "v$version" "bindings/go/sysprims/v$version"
msg() { call "tag_message_file '$1'" SYSPRIMS_RELEASE_TAG="v$version" SYSPRIMS_TAG_MESSAGE_DIR="${2:-$messages}"; }
[[ "$(msg "v$version")" == "$messages/v$version.txt" ]]
[[ "$(msg "bindings/go/sysprims/v$version")" == "$messages/bindings__go__sysprims__v$version.txt" ]]
expect_fail_with 'is missing' msg "go/recurrence/v$version"
expect_fail_with 'must end in the release tag' msg "v$version" "$fixture_dir/messages"
mkdir -p "$fixture_dir/real/v$version"
ln -s "$fixture_dir/real" "$fixture_dir/link"
expect_fail_with 'symlink ancestors' msg "v$version" "$fixture_dir/link/v$version"
mkdir -p "$repo/inside/v$version"
expect_fail_with 'outside the repository' msg "v$version" "$repo/inside/v$version"
cp "$messages/v$version.txt" "$fixture_dir/good.txt"
sed -i.bak '1s/.*/Release 9.9.9/' "$messages/v$version.txt"
expect_fail_with 'subject must be' msg "v$version"
cp "$fixture_dir/good.txt" "$messages/v$version.txt"
sed -i.bak 's/^Role: devlead$/Role: <role>/' "$messages/v$version.txt"
expect_fail_with 'placeholder' msg "v$version"
cp "$fixture_dir/good.txt" "$messages/v$version.txt"
sed -i.bak '/^Committer-of-Record/d' "$messages/v$version.txt"
expect_fail_with 'attribution footer' msg "v$version"
cp "$fixture_dir/good.txt" "$messages/v$version.txt"
printf 'trailing space \n' >>"$messages/v$version.txt"
expect_fail_with 'trailing whitespace' msg "v$version"
cp "$fixture_dir/good.txt" "$messages/v$version.txt"
msg "v$version" >/dev/null

# The preparer writes a skeleton per tag and never clobbers a reviewed file.
prep="$fixture_dir/prep/v$version"
in_repo SYSPRIMS_RELEASE_TAG="v$version" SYSPRIMS_TAG_MESSAGE_DIR="$prep" ./scripts/release-prepare-tag-message.py >/dev/null
[[ -f "$prep/v$version.txt" && -f "$prep/bindings__go__sysprims__v$version.txt" ]]
grep -q '^Release 1.2.3$' "$prep/v$version.txt"
cp "$fixture_dir/good.txt" "$prep/v$version.txt"
in_repo SYSPRIMS_RELEASE_TAG="v$version" SYSPRIMS_TAG_MESSAGE_DIR="$prep" ./scripts/release-prepare-tag-message.py >/dev/null
cmp "$fixture_dir/good.txt" "$prep/v$version.txt"
expect_fail_with 'must match VERSION' in_repo SYSPRIMS_RELEASE_TAG=v0.0.1 SYSPRIMS_TAG_MESSAGE_DIR="$fixture_dir/prep/v0.0.1" ./scripts/release-prepare-tag-message.py

# The subkey selector must be the committed subkey, exact and with "!".
call tag_selector SYSPRIMS_PGP_KEY_ID='0123456789ABCDEF0123456789ABCDEF01234567!'
expect_fail_with 'followed by !' call tag_selector SYSPRIMS_PGP_KEY_ID=0123456789ABCDEF0123456789ABCDEF01234567
expect_fail_with 'followed by !' call tag_selector SYSPRIMS_PGP_KEY_ID='0123456789abcdef0123456789abcdef01234567!'
expect_fail_with 'differs from config/release/signing-subkey.txt' call tag_selector SYSPRIMS_PGP_KEY_ID='FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF!'

# Tagger identity must be the committed one, and free of header injection.
call tag_identity SYSPRIMS_TAGGER_NAME='3 Leaps Infosec Team' SYSPRIMS_TAGGER_EMAIL=infosec@3leaps.net
expect_fail_with 'committed tagger identity' call tag_identity SYSPRIMS_TAGGER_NAME=Someone SYSPRIMS_TAGGER_EMAIL=infosec@3leaps.net

# Origin must be the GitHub repository.
call tag_origin
git -C "$repo" remote set-url origin https://example.invalid/3leaps/sysprims.git
expect_fail_with 'origin must be' call tag_origin
echo '[ok] tag input controls passed'
