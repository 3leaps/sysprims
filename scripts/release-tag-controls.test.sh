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

# Which targets read the release tag is derived from the recipes and the
# scripts they run (scripts/release-make-guard.py); each must start with
# release-require-tag and none may be listed as tag-free.
guarded="$(python3 "$fixture_root/scripts/release-make-guard.py" "$fixture_root")"
for expected in release release-download release-export-keys release-upload release-publish release-preflight \
	release-tag release-push-tag release-verify-tag release-verify-remote-tag release-crates-publish release-guard-signed; do
	grep -qx "$expected" <<<"$guarded" || {
		echo "error: the guard analysis does not see $expected as tag-reading" >&2
		exit 1
	}
done

# Live: every guarded target refuses a bare RELEASE_TAG (environment, command
# line, set but empty) and a missing SYSPRIMS_RELEASE_TAG before any other
# prerequisite or recipe runs. The shim replaces every script with one that
# records it ran, so any side effect is visible.
sentinel="$fixture_dir/ran"
shim="$fixture_dir/shim-root"
mkdir -p "$shim/scripts"
cp "$fixture_root/Makefile" "$fixture_root/VERSION" "$shim/"
for script in "$fixture_root"/scripts/*; do
	name="$(basename "$script")"
	printf '#!/usr/bin/env bash\necho %q >>%q\n' "$name" "$sentinel" >"$shim/scripts/$name"
	chmod +x "$shim/scripts/$name"
done
printf '#!/usr/bin/env bash\necho node >>%q\n' "$sentinel" >"$fixture_dir/node"
chmod +x "$fixture_dir/node"
# shim_make [-u VAR]... [VAR=value]... make ARGS: env flags first, then PATH.
shim_make() {
	local flags=()
	while [[ "${1:-}" == -u ]]; do
		flags+=("$1" "$2")
		shift 2
	done
	env ${flags[@]+"${flags[@]}"} PATH="$fixture_dir:$PATH" "$@"
}
while IFS= read -r target; do
	for origin in env cli empty missing; do
		rm -f "$sentinel"
		case "$origin" in
		env) expect_fail_with 'RELEASE_TAG is not accepted' shim_make RELEASE_TAG=v9.9.9 SYSPRIMS_RELEASE_TAG=v1.2.3 make -s -C "$shim" "$target" ;;
		cli) expect_fail_with 'RELEASE_TAG is not accepted' shim_make -u RELEASE_TAG SYSPRIMS_RELEASE_TAG=v1.2.3 make -s -C "$shim" "$target" RELEASE_TAG=v9.9.9 ;;
		empty) expect_fail_with 'RELEASE_TAG is not accepted' shim_make RELEASE_TAG= SYSPRIMS_RELEASE_TAG=v1.2.3 make -s -C "$shim" "$target" ;;
		missing) expect_fail_with 'SYSPRIMS_RELEASE_TAG=vX.Y.Z is required' shim_make -u RELEASE_TAG -u SYSPRIMS_RELEASE_TAG make -s -C "$shim" "$target" ;;
		esac
		[[ ! -s "$sentinel" ]] || {
			echo "error: make $target ran $(paste -sd, "$sentinel") despite an invalid tag input ($origin)" >&2
			exit 1
		}
	done
done <<<"$guarded"
# Control: with a valid tag the guard passes and the target proceeds.
rm -f "$sentinel"
shim_make -u RELEASE_TAG SYSPRIMS_RELEASE_TAG=v1.2.3 make -s -C "$shim" release-tag >/dev/null
grep -qx release-tag-operator.sh "$sentinel"
# Targets that read no tag ignore an unrelated RELEASE_TAG.
env RELEASE_TAG=v9.9.9 make -s -C "$fixture_root" -n help >/dev/null
if grep -En 'SYSPRIMS_RELEASE_TAG[[:space:]]*\?=|v\$\(VERSION\)' "$fixture_root/Makefile"; then
	echo 'error: the Makefile derives a release tag' >&2
	exit 1
fi

# One resolver decides the tag set; no other release script carries its own
# copy of the prefixes or reads go.mod or the plan from the working tree.
# Membership decisions: a filesystem test of go/recurrence/go.mod, or building
# a tag list from a hardcoded prefix.
if grep -nE "(\[ -f |existsSync\(|is_file\(\)).*go/recurrence|tags\.push\(|tags\.append\(|tags\+=\(\"(bindings|go/)" \
	"$fixture_root"/scripts/release-guard-tag-version.sh "$fixture_root"/scripts/release-integrity.mjs \
	"$fixture_root"/scripts/release-materialize-tag-messages.mjs "$fixture_root"/scripts/release-prepare-tag-message.py \
	"$fixture_root"/scripts/release-tag-common.sh; then
	echo 'error: a release script decides tag membership outside scripts/release-tag-set.mjs' >&2
	exit 1
fi
for consumer in release-guard-tag-version.sh release-integrity.mjs release-materialize-tag-messages.mjs \
	release-prepare-tag-message.py release-tag-common.sh; do
	grep -q 'release-tag-set' "$fixture_root/scripts/$consumer" || {
		echo "error: $consumer does not use the shared tag-set resolver" >&2
		exit 1
	}
done

# The release tag is required, canonical, and equal to VERSION.
expect_fail_with 'SYSPRIMS_RELEASE_TAG is required' call tag_version -u SYSPRIMS_RELEASE_TAG
expect_fail_with 'canonical vX.Y.Z' call tag_version SYSPRIMS_RELEASE_TAG=1.2.3
expect_fail_with 'does not match VERSION' call tag_version SYSPRIMS_RELEASE_TAG=v1.2.4
call tag_version SYSPRIMS_RELEASE_TAG="v$version"

# The tag set is loaded from git blobs at a commit; uncommitted edits to the
# plan, go.mod or prefix list never change it.
load() { call "tag_load_set ${1:-HEAD} && printf '%s,' \"\${RELEASE_TAGS[@]}\"" SYSPRIMS_RELEASE_TAG="v$version"; }
full="v$version,bindings/go/sysprims/v$version,go/recurrence/v$version,"
[[ "$(load)" == "$full" ]]
printf '{"surfaces":{"go":{"disposition":"skip","retained_version":"0.0.1"}}}\n' >"$repo/docs/releases/v$version.json"
rm -rf "$repo/go"
[[ "$(load)" == "$full" ]]
fixture_commit "$repo" "skip, no module"
[[ "$(load)" == "v$version," ]]
[[ "$(load HEAD~1)" == "$full" ]]
git -C "$repo" checkout -q HEAD~1 -- .
fixture_commit "$repo" restore

# A failing producer never yields a partial set. With an unknown prefix after
# "v" the resolver fails after reading the file; the loader refuses even though
# "v" alone would have looked like a valid set.
printf 'v\nother/v\n' >"$repo/config/release/release-tags.txt"
fixture_commit "$repo" bad-prefix
expect_fail_with 'cannot resolve the release tag set' load
printf 'v\nbindings/go/sysprims/v\nbindings/go/sysprims/v\n' >"$repo/config/release/release-tags.txt"
fixture_commit "$repo" repeated-prefix
expect_fail_with 'cannot resolve the release tag set' load
printf 'bindings/go/sysprims/v\nv\n' >"$repo/config/release/release-tags.txt"
fixture_commit "$repo" canonical-not-first
expect_fail_with 'cannot resolve the release tag set' load
git -C "$repo" checkout -q HEAD~3 -- config/release/release-tags.txt
fixture_commit "$repo" restore-prefixes
[[ "$(load)" == "$full" ]]
# Every consumer that iterates tags goes through tag_load_set, never a
# process-substitution loop over a producer.
if grep -nE 'done < <\(' "$fixture_root"/scripts/release-*.sh "$fixture_root"/scripts/verify-pinned-tag.sh |
	grep -v '\.test\.sh'; then
	echo 'error: a release script iterates a producer without checking its exit status' >&2
	exit 1
fi

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
