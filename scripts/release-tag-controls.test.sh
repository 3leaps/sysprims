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

# Every checklist `make` target exists. The target list is captured first so a
# missing or unreadable checklist fails instead of yielding an empty loop.
checklist_targets="$(grep -oE 'make [a-z][a-z0-9-]+' "$fixture_root/RELEASE_CHECKLIST.md" | awk '{print $2}' | sort -u)"
[[ -n "$checklist_targets" ]] || {
	echo "error: RELEASE_CHECKLIST.md names no make targets" >&2
	exit 1
}
while IFS= read -r target; do
	grep -Eq "^${target}:" "$fixture_root/Makefile" || {
		echo "error: RELEASE_CHECKLIST.md names missing make target $target" >&2
		exit 1
	}
done <<<"$checklist_targets"

# The checklist sets the tag environment (clearing a bare RELEASE_TAG) before
# the first target that reads the tag.
guarded_targets="$(python3 "$fixture_root/scripts/release-make-guard.py" "$fixture_root")"
env_line="$(grep -n '^unset RELEASE_TAG$' "$fixture_root/RELEASE_CHECKLIST.md" | head -1 | cut -d: -f1)"
[[ -n "$env_line" ]] || {
	echo 'error: RELEASE_CHECKLIST.md never clears a bare RELEASE_TAG' >&2
	exit 1
}
first_guarded="$(grep -noE 'make [a-z][a-z0-9-]+' "$fixture_root/RELEASE_CHECKLIST.md" |
	while IFS=: read -r line text; do grep -qx "${text#make }" <<<"$guarded_targets" && {
		echo "$line"
		break
	}; done)"
[[ -n "$first_guarded" && "$env_line" -lt "$first_guarded" ]] || {
	echo "error: RELEASE_CHECKLIST.md uses a tag-reading target (line $first_guarded) before clearing RELEASE_TAG (line $env_line)" >&2
	exit 1
}
# The analysis on synthetic Makefiles: a tag-free target that demands the tag
# is reported as over-guarded (not reclassified as tag-reading because of the
# guard itself), and a tag-reading target without the guard is reported.
synthetic="$fixture_dir/synthetic"
mkdir -p "$synthetic/scripts"
cp "$fixture_root/scripts/release-integrity.mjs" "$synthetic/scripts/"
write_make() {
	cat >"$synthetic/Makefile" <<MK
RELEASE_TAG_FREE_TARGETS := release-require-tag release-free
.NOTPARALLEL: release-reads
release-require-tag:
	@test -n "\$(SYSPRIMS_RELEASE_TAG)"
release-reads: release-require-tag ## reads
	@test -n "\$(SYSPRIMS_RELEASE_TAG)"
release-free: $1 ## no tag
	@true
MK
}
write_make ""
python3 "$fixture_root/scripts/release-make-guard.py" "$synthetic" >/dev/null
write_make release-require-tag
expect_fail_with 'release-free reads no release tag but requires one' python3 "$fixture_root/scripts/release-make-guard.py" "$synthetic"
sed -i.bak 's/^release-reads: release-require-tag /release-reads: /' "$synthetic/Makefile"
expect_fail_with 'release-reads reads the release tag but does not start with release-require-tag' python3 "$fixture_root/scripts/release-make-guard.py" "$synthetic"

# Provenance reads no tag, so the footer check runs without one.
if grep -qx release-guard-provenance <<<"$guarded_targets"; then
	echo 'error: release-guard-provenance is classified as reading the release tag' >&2
	exit 1
fi

# Which targets read the release tag is derived from the recipes and the
# scripts they run (scripts/release-make-guard.py); each must start with
# release-require-tag and none may be listed as tag-free.
guarded="$(python3 "$fixture_root/scripts/release-make-guard.py" "$fixture_root")"
for expected in release release-download release-export-keys release-create-draft release-publish release-preflight \
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
# The registry check runs only on a checkout that equals the tag's commit:
# the post-tag guard runs before it.
rm -f "$sentinel"
shim_make -u RELEASE_TAG SYSPRIMS_RELEASE_TAG=v1.2.3 make -s -C "$shim" release-guard-remote >/dev/null
[[ "$(paste -sd, "$sentinel")" == release-guard-tag-version.sh,node ]] || {
	echo "error: release-guard-remote ran $(paste -sd, "$sentinel"), not the post-tag guard then its check" >&2
	exit 1
}
# Independent of any classification list: these leaf targets act on a tagged
# release (they read release notes at the tag, download or edit the GitHub
# release, or check the registries for its version). Each refuses a bare
# RELEASE_TAG, and a missing SYSPRIMS_RELEASE_TAG, with no side effect. This
# list is written by hand on purpose.
for target in release-guard-downloaded release-guard-checksummed release-require-checksummed \
	release-guard-signed release-guard-uploaded release-guard-remote release-publish \
	release-download release-checksums release-sign release-export-keys release-notes \
	release-create-draft release-verify release-verify-signatures release-crates-dry-run release-crates-publish \
	release-tag release-push-tag release-verify-tag release-verify-remote-tag release-prepare-tag-message \
	release-guard-tag-version release-guard-tag-version-post release-preflight release; do
	for origin in bare-only missing; do
		rm -f "$sentinel"
		case "$origin" in
		bare-only) expect_fail_with 'RELEASE_TAG is not accepted' shim_make -u SYSPRIMS_RELEASE_TAG RELEASE_TAG=v1.2.3 make -s -C "$shim" "$target" ;;
		missing) expect_fail_with 'SYSPRIMS_RELEASE_TAG=vX.Y.Z is required' shim_make -u RELEASE_TAG -u SYSPRIMS_RELEASE_TAG make -s -C "$shim" "$target" ;;
		esac
		[[ ! -s "$sentinel" ]] || {
			echo "error: make $target ran $(paste -sd, "$sentinel") without an explicit release tag ($origin)" >&2
			exit 1
		}
	done
done
# The integrity commands themselves refuse too, without make in front.
for command in downloaded checksummed require-checksummed signed uploaded remote publish; do
	expect_fail_with 'SYSPRIMS_RELEASE_TAG=vX.Y.Z is required' env -u RELEASE_TAG -u SYSPRIMS_RELEASE_TAG \
		node "$fixture_root/scripts/release-integrity.mjs" "$command" --root "$repo" --dir "$fixture_dir"
	expect_fail_with 'RELEASE_TAG is not accepted' env -u SYSPRIMS_RELEASE_TAG RELEASE_TAG=v1.2.3 \
		node "$fixture_root/scripts/release-integrity.mjs" "$command" --root "$repo" --dir "$fixture_dir"
done
# The version comes from the tag's commit: a tag on a commit whose VERSION
# differs is refused, and a checkout at a later VERSION does not matter.
git -C "$repo" tag -a -m t v9.9.9
expect_fail_with 'does not match VERSION 1.2.3 at its commit' env -u RELEASE_TAG SYSPRIMS_RELEASE_TAG=v9.9.9 \
	node "$fixture_root/scripts/release-integrity.mjs" downloaded --root "$repo" --dir "$fixture_dir"
git -C "$repo" tag -d v9.9.9 >/dev/null

# A target that reads no tag runs without SYSPRIMS_RELEASE_TAG and despite a
# stray bare RELEASE_TAG.
for origin in none bare; do
	rm -f "$sentinel"
	case "$origin" in
	none) shim_make -u RELEASE_TAG -u SYSPRIMS_RELEASE_TAG make -s -C "$shim" release-guard-provenance ;;
	bare) shim_make -u SYSPRIMS_RELEASE_TAG RELEASE_TAG=v9.9.9 make -s -C "$shim" release-guard-provenance ;;
	esac
	grep -qx node "$sentinel" || {
		echo "error: release-guard-provenance did not run its check ($origin)" >&2
		exit 1
	}
done
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
# Syntax only: VERSION is compared at the commit, by tag_load_set.
call tag_version SYSPRIMS_RELEASE_TAG=v1.2.4
call tag_version SYSPRIMS_RELEASE_TAG="v$version"
expect_fail_with 'cannot resolve the release tag set' call 'tag_load_set HEAD' SYSPRIMS_RELEASE_TAG=v1.2.4

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

# The prefix file must be exactly the canonical list. Any other content (an
# omitted companion, swapped companions, an extra or a repeated prefix, a
# missing final newline) is refused by every consumer, not shortened.
prefix_ok="$(cat "$repo/config/release/release-tags.txt")"
[[ "$prefix_ok" == $'v\nbindings/go/sysprims/v\ngo/recurrence/v' ]]
for bad in 'v\n' 'v\nbindings/go/sysprims/v\n' 'v\ngo/recurrence/v\n' \
	'v\ngo/recurrence/v\nbindings/go/sysprims/v\n' 'bindings/go/sysprims/v\nv\ngo/recurrence/v\n' \
	'v\nbindings/go/sysprims/v\ngo/recurrence/v\nother/v\n' 'v\nv\nbindings/go/sysprims/v\ngo/recurrence/v\n' \
	'v\nbindings/go/sysprims/v\ngo/recurrence/v'; do
	printf '%b' "$bad" >"$repo/config/release/release-tags.txt"
	fixture_commit "$repo" "prefixes $bad"
	expect_fail_with 'cannot resolve the release tag set' load
	expect_fail_with 'release-tags.txt must be exactly' bash -c "cd '$repo' && node scripts/release-tag-set.mjs tags --tag v$version"
	expect_fail_with 'release-tags.txt must be exactly' node --input-type=module -e "
		import { requiredTags } from '$fixture_root/scripts/release-integrity.mjs';
		requiredTags('$repo', 'HEAD', '$version');"
	expect_fail_with 'release-tags.txt must be exactly' env SYSPRIMS_RELEASE_TAG="v$version" \
		SYSPRIMS_TAG_MESSAGE_DIR="$fixture_dir/prefix-messages/v$version" "$repo/scripts/release-prepare-tag-message.py"
done
printf '%s\n' "$prefix_ok" >"$repo/config/release/release-tags.txt"
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
# The full `make release` sequence, with every script and node replaced by a
# recorder (node also records its integrity command): the tagged notes and the
# anchors are staged before any phase guard reads the payload, and the draft is
# created last, after the signed guard.
# The integrity recorder notes whether the tagged notes were already staged.
printf '#!/usr/bin/env bash\nif [[ "$1" == */release-integrity.mjs ]]; then n=nonotes; [[ -f %q ]] && n=notes; echo "integrity:$2:$n" >>%q; else echo node >>%q; fi\n' \
	"$shim/dist/release/release-notes-v1.2.3.md" "$sentinel" "$sentinel" >"$fixture_dir/node"
chmod +x "$fixture_dir/node"
mkdir -p "$shim/docs/releases"
printf 'notes\n' >"$shim/docs/releases/v1.2.3.md"
rm -f "$sentinel"
# The fetch recorder creates the staging directory, as the real fetch does.
cp "$shim/scripts/release-fetch-ci-artifacts.sh" "$fixture_dir/fetch-recorder"
printf '#!/usr/bin/env bash\necho release-fetch-ci-artifacts.sh >>%q\nmkdir -p "$1"\n' "$sentinel" >"$shim/scripts/release-fetch-ci-artifacts.sh"
shim_make -u RELEASE_TAG SYSPRIMS_RELEASE_TAG=v1.2.3 SYSPRIMS_MINISIGN_KEY=/nonexistent make -s -C "$shim" release >/dev/null
if grep -E '^integrity:(downloaded|checksummed|require-checksummed|signed|uploaded):nonotes$' "$sentinel"; then
	echo 'error: make release ran a payload guard before the tagged notes were staged' >&2
	exit 1
fi
order="$(sed -E 's/^(integrity:[a-z-]+):(no)?notes$/\1/' "$sentinel" | grep -v -E '^(node|integrity:provenance|release-guard-tag-version\.sh)$' | paste -sd' ' -)"
position() {
	local i=0 word
	for word in $order; do
		i=$((i + 1))
		[[ "$word" == "$1" ]] && {
			echo "$i"
			return
		}
	done
	echo 0
}
for step in release-fetch-ci-artifacts.sh release-stage-public.sh integrity:downloaded generate-checksums.sh \
	sign-release-assets.sh integrity:signed release-create-draft.sh integrity:uploaded; do
	[[ "$(position "$step")" -gt 0 ]] || {
		echo "error: make release never ran $step: $order" >&2
		exit 1
	}
done
before() { [[ "$(position "$1")" -lt "$(position "$2")" ]] || {
	echo "error: make release ran $2 before $1: $order" >&2
	exit 1
}; }
before release-fetch-ci-artifacts.sh release-stage-public.sh
before release-stage-public.sh integrity:downloaded
before integrity:downloaded generate-checksums.sh
before generate-checksums.sh sign-release-assets.sh
before integrity:signed release-create-draft.sh
before release-create-draft.sh integrity:uploaded
[[ "$(grep -c '^release-create-draft\.sh$' "$sentinel")" == 1 ]]
cp "$fixture_dir/fetch-recorder" "$shim/scripts/release-fetch-ci-artifacts.sh"
# release-notes fails when the copy fails (no staging directory).
rm -rf "$shim/dist"
expect_fail_with 'release-notes' shim_make -u RELEASE_TAG SYSPRIMS_RELEASE_TAG=v1.2.3 make -C "$shim" release-notes
printf '#!/usr/bin/env bash\necho node >>%q\n' "$sentinel" >"$fixture_dir/node"
chmod +x "$fixture_dir/node"

echo '[ok] tag input controls passed'
