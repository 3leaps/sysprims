#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="${SYSPRIMS_REPO_ROOT:-$(git -C "$SCRIPT_DIR/.." rev-parse --show-toplevel)}"
MODE="${SYSPRIMS_TAG_GUARD_MODE:-pre-tag}"
REMOTE="${SYSPRIMS_TAG_REMOTE:-origin}"

cd "$PROJECT_ROOT"

fail() {
	echo "error: $*" >&2
	exit 1
}

node "$SCRIPT_DIR/version-pack.mjs" plan-check --root "$PROJECT_ROOT"
node "$SCRIPT_DIR/version-pack.mjs" check --root "$PROJECT_ROOT"
VERSION="$(tr -d '\r\n' <VERSION)"
EXPECTED_TAG="v${VERSION}"
# The release tag is an explicit input; it is never derived from VERSION.
[ -n "${SYSPRIMS_RELEASE_TAG:-}" ] || fail "SYSPRIMS_RELEASE_TAG is required (expected ${EXPECTED_TAG})"
INTENDED_TAG="$SYSPRIMS_RELEASE_TAG"
[ "$INTENDED_TAG" = "$EXPECTED_TAG" ] || fail "intended release tag ${INTENDED_TAG} does not equal ${EXPECTED_TAG}"

require_remote_annotated_tag() {
	local tag="$1"
	local ref="refs/tags/${tag}"
	local lines object peeled sha name
	lines="$(git ls-remote --tags "$REMOTE" "$ref" "${ref}^{}")"
	object=""
	peeled=""
	while IFS=$'\t' read -r sha name; do
		case "$name" in
		"$ref") object="$sha" ;;
		"${ref}^{}") peeled="$sha" ;;
		esac
	done <<<"$lines"
	[ -n "$object" ] || fail "remote exact tag ${tag} does not exist"
	[ -n "$peeled" ] || fail "remote tag ${tag} must be annotated"
	[ "$peeled" = "$HEAD_COMMIT" ] || fail "remote tag ${tag} peels to ${peeled}, not HEAD ${HEAD_COMMIT}"
	git show-ref --verify --quiet "$ref" || fail "local annotated tag object ${tag} is unavailable for message verification"
	[ "$(git cat-file -t "$ref")" = "tag" ] || fail "local tag ${tag} must be annotated"
	[ "$(git rev-parse "$ref")" = "$object" ] || fail "local and remote tag objects differ for ${tag}"
}

# Both modes read the release decisions from git blobs at HEAD, so the tree
# must be clean: an uncommitted plan, prefix list or go.mod cannot change them.
if [ -n "$(git status --porcelain --untracked-files=normal)" ]; then
	git status --short >&2
	fail "${MODE} guard requires a clean working tree"
fi
HEAD_COMMIT="$(git rev-parse 'HEAD^{commit}')"
PLAN_BLOB="$(git cat-file blob "${HEAD_COMMIT}:docs/releases/${EXPECTED_TAG}.json")" || fail "release plan is missing at ${HEAD_COMMIT}"
GO_LOCK_PHASE="$(node -e 'process.stdout.write(String(JSON.parse(process.argv[1]).surfaces.go.lock_phase))' "$PLAN_BLOB")"
GO_RETAINED="$(node -e 'process.stdout.write(String(JSON.parse(process.argv[1]).surfaces.go.retained_version || ""))' "$PLAN_BLOB")"
# The one shared resolver decides the required tag set from the commit.
SET_OUTPUT="$(node "$SCRIPT_DIR/release-tag-set.mjs" tags --commit "$HEAD_COMMIT" --tag "$INTENDED_TAG")" ||
	fail "cannot resolve the release tag set at ${HEAD_COMMIT}"
REQUIRED_TAGS=()
while IFS= read -r line; do
	[ -n "$line" ] && REQUIRED_TAGS+=("$line")
done <<<"$SET_OUTPUT"
[ "${#REQUIRED_TAGS[@]}" -ge 1 ] && [ "${REQUIRED_TAGS[0]}" = "$EXPECTED_TAG" ] ||
	fail "the release tag set at ${HEAD_COMMIT} is empty or does not start with ${EXPECTED_TAG}"
in_set() {
	local wanted="$1" tag
	for tag in "${REQUIRED_TAGS[@]}"; do [ "$tag" = "$wanted" ] && return 0; done
	return 1
}

case "$MODE" in
pre-tag)
	[ "$GO_LOCK_PHASE" = "resolved" ] || fail "Go lock_phase must be resolved before pre-tag guard (found ${GO_LOCK_PHASE})"
	node "$SCRIPT_DIR/release-integrity.mjs" provenance --root "$PROJECT_ROOT"
	echo "[ok] pre-tag guard: clean coherent plan intends ${REQUIRED_TAGS[*]}"
	;;
post-tag)
	[ "$GO_LOCK_PHASE" = "resolved" ] || fail "Go lock_phase must be resolved before tag guard (found ${GO_LOCK_PHASE})"
	for tag in "${REQUIRED_TAGS[@]}"; do
		require_remote_annotated_tag "$tag"
	done
	# A listed prefix outside the set must not have a tag at this version.
	PREFIXES="$(git cat-file blob "${HEAD_COMMIT}:config/release/release-tags.txt")" || fail "release-tags.txt is missing at ${HEAD_COMMIT}"
	while IFS= read -r prefix; do
		[ -n "$prefix" ] || continue
		tag="${prefix}${VERSION}"
		in_set "$tag" && continue
		if git ls-remote --exit-code --tags "$REMOTE" "refs/tags/${tag}" >/dev/null 2>&1; then
			fail "${tag} exists but the release at ${HEAD_COMMIT} does not include it"
		fi
	done <<<"$PREFIXES"
	if ! in_set "bindings/go/sysprims/${EXPECTED_TAG}"; then
		[ -n "$GO_RETAINED" ] || fail "Go skip plan has no retained_version"
		retained="bindings/go/sysprims/v${GO_RETAINED}"
		lines="$(git ls-remote --tags "$REMOTE" "refs/tags/${retained}" "refs/tags/${retained}^{}")"
		printf '%s\n' "$lines" | grep -Fq "refs/tags/${retained}^{}" || fail "retained annotated Go tag ${retained} is missing"
	fi
	node "$SCRIPT_DIR/release-integrity.mjs" provenance --root "$PROJECT_ROOT"
	echo "[ok] post-tag guard: exact remote annotated tags, provenance, and version plan agree at ${HEAD_COMMIT}"
	;;
*) fail "unknown SYSPRIMS_TAG_GUARD_MODE=${MODE}; expected pre-tag or post-tag" ;;
esac
