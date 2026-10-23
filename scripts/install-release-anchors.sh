#!/usr/bin/env bash
# Install a reviewed public anchor pair. Existing anchors are never
# overwritten; a rotation is a reviewed edit, not a re-run of this script.
# On failure only files this invocation created are removed.
# Usage: install-release-anchors.sh <staging-dir> <dest-dir> [verify command...]
set -euo pipefail
staging="${1:?staging directory required}"
dest="${2:?destination required}"
shift 2
names=(expected-fingerprints.txt expected-fingerprints.ndjson)
for name in "${names[@]}"; do
	[[ -s "$staging/$name" && ! -L "$staging/$name" ]] || {
		echo 'error: incomplete anchor pair' >&2
		exit 1
	}
	[[ ! -e "$dest/$name" && ! -L "$dest/$name" ]] || {
		echo "error: $dest/$name exists; anchors are never overwritten" >&2
		exit 1
	}
done
[[ ! -L "$dest" && (! -e "$dest" || -d "$dest") ]] || {
	echo 'error: unsafe anchor directory' >&2
	exit 1
}
mkdir -p "$dest"
owned=()
rollback() {
	trap - EXIT INT TERM HUP
	local path
	for path in ${owned[@]+"${owned[@]}"}; do rm -f "$path"; done
}
trap rollback EXIT
trap 'rollback; exit 130' INT
trap 'rollback; exit 143' TERM
trap 'rollback; exit 129' HUP
# Each anchor is created with O_EXCL semantics (noclobber); a file that appears
# meanwhile is left alone and fails the installation.
for name in "${names[@]}"; do
	if ! (set -C && cat "$staging/$name" >"$dest/$name") 2>/dev/null; then
		echo "error: $dest/$name appeared during installation; it was left untouched" >&2
		exit 1
	fi
	owned+=("$dest/$name")
done
if [[ "$#" -gt 0 ]]; then
	"$@"
fi
trap - EXIT INT TERM HUP
