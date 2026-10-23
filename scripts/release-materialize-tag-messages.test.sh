#!/usr/bin/env bash
# The workflow step that fetches the release tags and writes their messages
# covers the full tag set decided by the release commit, accepts signed tags
# (the OpenPGP armor is not part of the message) and rejects a wrong subject,
# a missing footer, or a companion tag at another commit.
set -euo pipefail
# shellcheck source=scripts/release-test-fixture.sh
source "$(dirname "$0")/release-test-fixture.sh"
version=1.2.3
origin="$fixture_dir/origin.git"
repo="$fixture_dir/repo"
messages="$fixture_dir/messages"
fixture_repo "$repo" "$version" publish 1
cp "$fixture_root/scripts/release-materialize-tag-messages.mjs" "$repo/scripts/"
read -r _ subkey < <(fixture_keys "$fixture_dir/gpg")
# Signed exactly as the ceremony signs: git tag -s with the subkey.
stag() { GNUPGHOME="$fixture_dir/gpg" git -C "$repo" tag -s -a --cleanup=verbatim -u "$subkey!" -F "$2" "$1" "${3:-HEAD}"; }
fixture_commit "$repo"
git init -q --bare "$origin"
git -C "$repo" remote set-url origin "$origin"
tags=("v$version" "bindings/go/sysprims/v$version" "go/recurrence/v$version")
fixture_messages "$fixture_dir/src" "$version" "${tags[@]}"
for tag in "${tags[@]}"; do
	stag "$tag" "$fixture_dir/src/${tag//\//__}.txt"
done
git -C "$repo" cat-file tag "v$version" | grep -q 'BEGIN PGP SIGNATURE'
git -C "$repo" push -q origin main "${tags[@]/#/refs/tags/}"
run() {
	rm -rf "$messages" "$fixture_dir/ci"
	mkdir -p "$messages"
	git clone -q "$origin" "$fixture_dir/ci"
	(cd "$fixture_dir/ci" && RELEASE_TAG="v$version" MESSAGE_DIR="$messages" node scripts/release-materialize-tag-messages.mjs)
}
run >/dev/null
for tag in "${tags[@]}"; do
	cmp "$fixture_dir/src/${tag//\//__}.txt" "$messages/${tag//\//__}.txt"
done

# Without the recurrence module, only two tags are fetched and written.
git -C "$repo" rm -rq go/recurrence
fixture_commit "$repo" no-recurrence
git -C "$repo" tag -d "${tags[@]}" >/dev/null
git -C "$repo" push -q origin --delete "${tags[@]/#/refs/tags/}"
for tag in "v$version" "bindings/go/sysprims/v$version"; do
	stag "$tag" "$fixture_dir/src/${tag//\//__}.txt"
done
git -C "$repo" push -q origin main "refs/tags/v$version" "refs/tags/bindings/go/sysprims/v$version"
run >/dev/null
[[ ! -e "$messages/go__recurrence__v$version.txt" ]]

# A wrong subject or a missing footer fails.
retag() {
	git -C "$repo" tag -d "$1" >/dev/null
	stag "$1" "$2" "${3:-HEAD}"
	git -C "$repo" push -q -f origin "refs/tags/$1"
}
sed '1s/.*/Release 0.0.0/' "$fixture_dir/src/v$version.txt" >"$fixture_dir/bad-subject.txt"
retag "v$version" "$fixture_dir/bad-subject.txt"
expect_fail_with 'unexpected annotation subject' run
sed '/^Committer-of-Record/d' "$fixture_dir/src/v$version.txt" >"$fixture_dir/no-footer.txt"
retag "v$version" "$fixture_dir/no-footer.txt"
expect_fail_with 'attribution footer' run
retag "v$version" "$fixture_dir/src/v$version.txt"
run >/dev/null

# A companion tag at another commit fails, even when validly signed.
git -C "$repo" commit -q --allow-empty -m later
git -C "$repo" push -q origin main
retag "bindings/go/sysprims/v$version" "$fixture_dir/src/bindings__go__sysprims__v$version.txt" HEAD
expect_fail_with 'does not point at the release commit' run
retag "bindings/go/sysprims/v$version" "$fixture_dir/src/bindings__go__sysprims__v$version.txt" "v$version^{commit}"
run >/dev/null

# The set comes from the release commit, not the checkout: a later main that
# drops the recurrence module does not shrink the set for an earlier release.
git -C "$repo" checkout -q "v$version^{commit}"
git -C "$repo" checkout -q -b recheck
mkdir -p "$repo/go/recurrence"
printf 'module github.com/3leaps/sysprims/go/recurrence\n' >"$repo/go/recurrence/go.mod"
fixture_commit "$repo" add-module-later
git -C "$repo" push -q origin recheck:main -f
run >/dev/null
[[ ! -e "$messages/go__recurrence__v$version.txt" ]]

# A tag absent from the remote fails the fetch.
git -C "$repo" push -q origin --delete "refs/tags/bindings/go/sysprims/v$version"
expect_fail run
echo '[ok] tag-message materialization controls passed'
