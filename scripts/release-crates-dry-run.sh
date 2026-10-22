#!/usr/bin/env bash
# Package or publish the library crates in registry dependency order.
#   (no flag)  per-crate `cargo publish --dry-run`; nothing is uploaded
#   --publish  one maintainer-cued upload per crate, in order
# Both run only from a clean detached checkout of the annotated release tag.
# --publish also requires every earlier crate to be on the crates.io index at
# this version before it uploads the next. CI never holds a registry token.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
die() {
	echo "error: $*" >&2
	exit 1
}
publish=0
if [[ "${1:-}" == --publish ]]; then
	publish=1
	shift
fi
[[ $# -eq 0 ]] || die 'usage: release-crates-dry-run.sh [--publish]'
[[ "$publish" == 0 || -z "${CI:-}" ]] || die 'crates.io publication is a local maintainer step, never CI'

: "${SYSPRIMS_RELEASE_TAG:?SYSPRIMS_RELEASE_TAG=vX.Y.Z is required}"
version="$(tr -d '\r\n' <VERSION)"
[[ "$SYSPRIMS_RELEASE_TAG" == "v$version" ]] || die 'SYSPRIMS_RELEASE_TAG does not match VERSION'
! git symbolic-ref --quiet HEAD >/dev/null || die 'HEAD must be detached at the release tag'
object="$(git rev-parse --verify --quiet "refs/tags/$SYSPRIMS_RELEASE_TAG")" || die "$SYSPRIMS_RELEASE_TAG is absent"
[[ "$(git cat-file -t "$object")" == tag ]] || die "$SYSPRIMS_RELEASE_TAG must be an annotated tag"
[[ "$(git rev-parse "${object}^{commit}")" == "$(git rev-parse HEAD)" ]] || die 'HEAD is not the release tag commit'
[[ -z "$(git status --porcelain --untracked-files=normal)" ]] || die 'clean checkout required'
python3 "$root/scripts/release-crates.py" check

indexed() {
	"${SYSPRIMS_CARGO_INFO:-cargo}" info --registry crates-io "$1@$version" >/dev/null 2>&1
}

patches=()
while read -r crate path; do
	if [[ "$publish" == 1 ]]; then
		if indexed "$crate"; then
			echo "[--] $crate@$version is already on crates.io; skipping"
		else
			printf 'Publish %s@%s to crates.io? Type the crate name to continue: ' "$crate" "$version"
			read -r answer </dev/tty
			[[ "$answer" == "$crate" ]] || die "publication of $crate not confirmed"
			cargo publish --locked -p "$crate"
			echo "[..] waiting for $crate@$version on the crates.io index"
			for _ in $(seq 1 60); do
				indexed "$crate" && break
				sleep 10
			done
			indexed "$crate" || die "$crate@$version did not appear on the index; stop and investigate"
		fi
	else
		printf 'Dry-run crates.io: %s@%s\n' "$crate" "$version"
		cargo publish --dry-run --locked -p "$crate" ${patches[@]+"${patches[@]}"}
		patches+=(--config "patch.crates-io.${crate}.path=\"${path}\"")
	fi
done < <(python3 "$root/scripts/release-crates.py" paths)
if [[ "$publish" == 1 ]]; then
	echo '[ok] every listed crate is on crates.io at this version'
else
	echo '[ok] per-crate registry dry runs passed; nothing was published'
fi
