#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Source this file, then call resolve_release_yamlprims. The workflow checkers
# read YAML only through the yamlprims CLI whose SHA-256 is committed in
# config/yamlprims-cli.txt; SYSPRIMS_YAMLPRIMS_BIN (an absolute path, no
# unprefixed variable, no working-tree build, no PATH lookup) only says where
# to read it from. The same rules apply wherever the checkers run (make check,
# CI, the release ceremony); resolving here first makes a ceremony target fail
# before it starts.
resolve_release_yamlprims() {
	local here
	[[ -n "${SYSPRIMS_YAMLPRIMS_BIN:-}" ]] || {
		echo 'error: SYSPRIMS_YAMLPRIMS_BIN is required (absolute path to the yamlprims CLI pinned in config/yamlprims-cli.txt)' >&2
		return 1
	}
	[[ "$SYSPRIMS_YAMLPRIMS_BIN" == /* ]] || {
		echo 'error: SYSPRIMS_YAMLPRIMS_BIN must be an absolute path' >&2
		return 1
	}
	here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
	# Digest, identity, minimum version and the plain JSON contract
	# (scripts/yaml_json.py, the reader the checkers use).
	python3 -I -B "$here/yaml_json.py" identify >/dev/null || return 1
	# shellcheck disable=SC2034 # Consumed by scripts sourcing this helper.
	RELEASE_YAMLPRIMS_BIN="$SYSPRIMS_YAMLPRIMS_BIN"
}
