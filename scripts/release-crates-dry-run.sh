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

# Publication state on crates.io, always the real registry. Prints "absent",
# or the published checksum.
registry_checksum() {
	local crate="$1" body status
	body="$(curl --proto '=https' --tlsv1.2 -sS -w '\n%{http_code}' \
		"https://crates.io/api/v1/crates/${crate}/${version}")" || die "cannot query crates.io for $crate"
	status="${body##*$'\n'}"
	body="${body%$'\n'*}"
	case "$status" in
	404) echo absent ;;
	200) python3 "$root/scripts/release-crates.py" registry-checksum "$crate" "$version" <<<"$body" ;;
	*) die "crates.io returned HTTP $status for $crate@$version" ;;
	esac
}

# sha256 of the crate this checkout packages, as crates.io records it.
local_checksum() {
	local crate="$1" file
	cargo package --locked --no-verify -p "$crate" >/dev/null 2>&1 || die "cannot package $crate"
	file="$root/target/package/${crate}-${version}.crate"
	[[ -f "$file" ]] || die "packaged crate $file not found"
	python3 -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$file"
}

# Collect the ordered plan whole and check the producer before iterating.
plan_output="$(python3 "$root/scripts/release-crates.py" paths)" || die 'cannot read the crate publication plan'
crates=()
paths=()
while read -r crate path; do
	[[ -n "$crate" && -n "$path" ]] || die 'malformed crate publication plan'
	crates+=("$crate")
	paths+=("$path")
done <<<"$plan_output"
[[ "${#crates[@]}" -ge 1 ]] || die 'empty crate publication plan'

patches=()
for i in "${!crates[@]}"; do
	crate="${crates[$i]}"
	path="${paths[$i]}"
	if [[ "$publish" == 1 ]]; then
		published="$(registry_checksum "$crate")"
		if [[ "$published" != absent ]]; then
			[[ "$published" == "$(local_checksum "$crate")" ]] ||
				die "$crate@$version is on crates.io with different contents; stop and investigate"
			echo "[--] $crate@$version is already on crates.io with these contents; skipping"
			continue
		fi
		printf 'Publish %s@%s to crates.io? Type the crate name to continue: ' "$crate" "$version"
		read -r answer </dev/tty
		[[ "$answer" == "$crate" ]] || die "publication of $crate not confirmed"
		cargo publish --locked -p "$crate"
		echo "[..] waiting for $crate@$version on the crates.io index"
		for _ in $(seq 1 60); do
			published="$(registry_checksum "$crate")"
			[[ "$published" != absent ]] && break
			sleep 10
		done
		[[ "$published" == "$(local_checksum "$crate")" ]] ||
			die "$crate@$version is not on crates.io with the uploaded contents; stop and investigate"
	else
		printf 'Dry-run crates.io: %s@%s\n' "$crate" "$version"
		cargo publish --dry-run --locked -p "$crate" ${patches[@]+"${patches[@]}"}
		patches+=(--config "patch.crates-io.${crate}.path=\"${path}\"")
	fi
done
if [[ "$publish" == 1 ]]; then
	echo '[ok] every listed crate is on crates.io at this version'
else
	echo '[ok] per-crate registry dry runs passed; nothing was published'
fi
