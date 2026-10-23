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
helper="$(cd "$(dirname "$0")" && pwd -P)/write-exclusive.py"
for name in "${names[@]}"; do
	# Created with O_EXCL|O_NOFOLLOW and written through that descriptor, so a
	# file or symlink that appears at the path is never written or followed.
	status=0
	python3 "$helper" "$staging/$name" "$dest/$name" || status=$?
	case "$status" in
	0) owned+=("$dest/$name") ;;
	3)
		echo "error: $dest/$name appeared during installation; it was left untouched" >&2
		exit 1
		;;
	*)
		# Any status but 3 means the helper created the path; it is ours to remove.
		owned+=("$dest/$name")
		echo "error: writing $dest/$name failed" >&2
		exit 1
		;;
	esac
done
if [[ "$#" -gt 0 ]]; then
	"$@"
fi
trap - EXIT INT TERM HUP
