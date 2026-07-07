#!/usr/bin/env bash
set -euo pipefail

workflow=".github/workflows/typescript-npm-publish.yml"
min_node="22.14.0"
min_npm="11.5.1"

fail() {
	echo "error: $*" >&2
	exit 1
}

normalize_version() {
	local value="${1#v}"
	value="$(printf '%s' "$value" | tr -d "\"'")"
	value="${value%%[!0-9.]*}"
	IFS=. read -r major minor patch _ <<<"$value"
	printf '%s.%s.%s\n' "${major:-0}" "${minor:-0}" "${patch:-0}"
}

version_ge() {
	local got
	local min
	got="$(normalize_version "$1")"
	min="$(normalize_version "$2")"

	IFS=. read -r got_major got_minor got_patch <<<"$got"
	IFS=. read -r min_major min_minor min_patch <<<"$min"

	if ((got_major > min_major)); then
		return 0
	fi
	if ((got_major == min_major && got_minor > min_minor)); then
		return 0
	fi
	if ((got_major == min_major && got_minor == min_minor && got_patch >= min_patch)); then
		return 0
	fi
	return 1
}

[[ -f "$workflow" ]] || fail "missing workflow: $workflow"

node_version="$(
	sed -nE "s/^[[:space:]]*node-version:[[:space:]]*['\"]?([^'\"]+)['\"]?.*/\1/p" "$workflow" | head -n 1
)"
[[ -n "$node_version" ]] || fail "publish workflow has no setup-node node-version"
version_ge "$node_version" "$min_node" || fail "publish workflow uses Node $node_version, need >= $min_node"

declared_min_node="$(
	sed -nE "s/.*MIN_TRUSTED_PUBLISH_NODE:[[:space:]]*['\"]?([^'\"]+)['\"]?.*/\1/p" "$workflow" | head -n 1
)"
[[ -n "$declared_min_node" ]] || fail "workflow does not declare MIN_TRUSTED_PUBLISH_NODE"
version_ge "$declared_min_node" "$min_node" || fail "MIN_TRUSTED_PUBLISH_NODE is $declared_min_node, need >= $min_node"

declared_min_npm="$(
	sed -nE "s/.*MIN_TRUSTED_PUBLISH_NPM:[[:space:]]*['\"]?([^'\"]+)['\"]?.*/\1/p" "$workflow" | head -n 1
)"
[[ -n "$declared_min_npm" ]] || fail "workflow does not declare MIN_TRUSTED_PUBLISH_NPM"
version_ge "$declared_min_npm" "$min_npm" || fail "MIN_TRUSTED_PUBLISH_NPM is $declared_min_npm, need >= $min_npm"

# shellcheck disable=SC2016
grep -Fq 'npm install -g "npm@${MIN_TRUSTED_PUBLISH_NPM}"' "$workflow" ||
	fail "workflow must install the declared npm trusted publishing minimum"
grep -q 'process.versions.node' "$workflow" ||
	fail "workflow must hard-fail when Node is below the trusted publishing minimum"
# shellcheck disable=SC2016
grep -Fq 'NPM_VERSION="$(npm --version)"' "$workflow" ||
	fail "workflow must hard-fail when npm is below the trusted publishing minimum"

echo "[ok] npm trusted publishing runtime guard passed"
