#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="${SYSPRIMS_REPO_ROOT:-$(git -C "$SCRIPT_DIR/.." rev-parse --show-toplevel)}"
MODE="${SYSPRIMS_TAG_GUARD_MODE:-pre-tag}"

cd "$PROJECT_ROOT"

fail() {
	echo "error: $*" >&2
	exit 1
}

node "$SCRIPT_DIR/version-pack.mjs" check --root "$PROJECT_ROOT"
VERSION="$(tr -d '\n' <VERSION)"

EXPECTED_TAG="v${VERSION}"
INTENDED_TAG="${SYSPRIMS_RELEASE_TAG:-$EXPECTED_TAG}"
GO_TAG="bindings/go/sysprims/${EXPECTED_TAG}"

[ "$INTENDED_TAG" = "$EXPECTED_TAG" ] ||
	fail "intended release tag ${INTENDED_TAG} does not equal ${EXPECTED_TAG}"

case "$MODE" in
pre-tag)
	if [ -n "$(git status --porcelain)" ]; then
		echo "error: pre-tag guard requires a clean working tree" >&2
		git status --short >&2
		exit 1
	fi
	echo "[ok] pre-tag guard: clean coherent pack intends ${EXPECTED_TAG}"
	;;
post-tag)
	CANONICAL_REF="refs/tags/${EXPECTED_TAG}"
	git show-ref --verify --quiet "$CANONICAL_REF" ||
		fail "exact canonical tag ${EXPECTED_TAG} does not exist"
	[ "$(git cat-file -t "$CANONICAL_REF")" = "tag" ] ||
		fail "canonical tag ${EXPECTED_TAG} must be annotated"
	HEAD_COMMIT="$(git rev-parse 'HEAD^{commit}')"
	CANONICAL_COMMIT="$(git rev-parse "${CANONICAL_REF}^{commit}")"
	[ "$CANONICAL_COMMIT" = "$HEAD_COMMIT" ] ||
		fail "canonical tag ${EXPECTED_TAG} peels to ${CANONICAL_COMMIT}, not HEAD ${HEAD_COMMIT}"

	if [ "${SYSPRIMS_REQUIRE_GO_TAG:-1}" = "1" ]; then
		GO_REF="refs/tags/${GO_TAG}"
		git show-ref --verify --quiet "$GO_REF" ||
			fail "exact Go module tag ${GO_TAG} does not exist"
		[ "$(git cat-file -t "$GO_REF")" = "tag" ] ||
			fail "Go module tag ${GO_TAG} must be annotated"
		GO_COMMIT="$(git rev-parse "${GO_REF}^{commit}")"
		[ "$GO_COMMIT" = "$CANONICAL_COMMIT" ] ||
			fail "canonical and Go tags peel to different commits (${CANONICAL_COMMIT} != ${GO_COMMIT})"
	fi

	echo "[ok] post-tag guard: exact annotated tags and version pack agree at ${HEAD_COMMIT}"
	;;
*)
	fail "unknown SYSPRIMS_TAG_GUARD_MODE=${MODE}; expected pre-tag or post-tag"
	;;
esac
