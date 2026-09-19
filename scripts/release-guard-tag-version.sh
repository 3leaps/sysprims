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
INTENDED_TAG="${SYSPRIMS_RELEASE_TAG:-$EXPECTED_TAG}"
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

GO_DISPOSITION="$(node -e 'const p=require("./docs/releases/v"+require("fs").readFileSync("VERSION","utf8").trim()+".json"); process.stdout.write(p.surfaces.go.disposition)')"
GO_LOCK_PHASE="$(node -e 'const p=require("./docs/releases/v"+require("fs").readFileSync("VERSION","utf8").trim()+".json"); process.stdout.write(p.surfaces.go.lock_phase)')"
GO_RETAINED="$(node -e 'const p=require("./docs/releases/v"+require("fs").readFileSync("VERSION","utf8").trim()+".json"); process.stdout.write(p.surfaces.go.retained_version||"")')"
GO_TAG="bindings/go/sysprims/${EXPECTED_TAG}"

case "$MODE" in
pre-tag)
	[ "$GO_LOCK_PHASE" = "resolved" ] || fail "Go lock_phase must be resolved before pre-tag guard (found ${GO_LOCK_PHASE})"
	if [ -n "$(git status --porcelain)" ]; then
		git status --short >&2
		fail "pre-tag guard requires a clean working tree"
	fi
	node "$SCRIPT_DIR/release-integrity.mjs" provenance --root "$PROJECT_ROOT"
	echo "[ok] pre-tag guard: clean coherent plan intends ${EXPECTED_TAG}"
	;;
post-tag)
	[ "$GO_LOCK_PHASE" = "resolved" ] || fail "Go lock_phase must be resolved before tag guard (found ${GO_LOCK_PHASE})"
	HEAD_COMMIT="$(git rev-parse 'HEAD^{commit}')"
	require_remote_annotated_tag "$EXPECTED_TAG"
	if [ "$GO_DISPOSITION" = "publish" ]; then
		require_remote_annotated_tag "$GO_TAG"
	elif [ "$GO_DISPOSITION" = "skip" ]; then
		[ -n "$GO_RETAINED" ] || fail "Go skip plan has no retained_version"
		if git ls-remote --exit-code --tags "$REMOTE" "refs/tags/${GO_TAG}" >/dev/null 2>&1; then
			fail "Go skip plan forbids current path tag ${GO_TAG}"
		fi
		retained="bindings/go/sysprims/v${GO_RETAINED}"
		lines="$(git ls-remote --tags "$REMOTE" "refs/tags/${retained}" "refs/tags/${retained}^{}")"
		printf '%s\n' "$lines" | grep -Fq "refs/tags/${retained}^{}" || fail "retained annotated Go tag ${retained} is missing"
	else
		fail "unknown Go disposition ${GO_DISPOSITION}"
	fi
	node "$SCRIPT_DIR/release-integrity.mjs" provenance --root "$PROJECT_ROOT"
	echo "[ok] post-tag guard: exact remote annotated tags, provenance, and version plan agree at ${HEAD_COMMIT}"
	;;
*) fail "unknown SYSPRIMS_TAG_GUARD_MODE=${MODE}; expected pre-tag or post-tag" ;;
esac
