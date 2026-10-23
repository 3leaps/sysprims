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
[[ "$SYSPRIMS_RELEASE_TAG" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || die 'SYSPRIMS_RELEASE_TAG must be a canonical vX.Y.Z tag'
! git symbolic-ref --quiet HEAD >/dev/null || die 'HEAD must be detached at the release tag'
object="$(git rev-parse --verify --quiet "refs/tags/$SYSPRIMS_RELEASE_TAG")" || die "$SYSPRIMS_RELEASE_TAG is absent"
[[ "$(git cat-file -t "$object")" == tag ]] || die "$SYSPRIMS_RELEASE_TAG must be an annotated tag"
[[ "$(git rev-parse "${object}^{commit}")" == "$(git rev-parse HEAD)" ]] || die 'HEAD is not the release tag commit'
[[ -z "$(git status --porcelain --untracked-files=normal)" ]] || die 'clean checkout required'
# git status does not show skip-worktree or assume-unchanged files. Refuse any
# index entry that is not an ordinary cached file (defence in depth: packaging
# below never reads this checkout's files).
flagged="$(git ls-files -v | awk '$1 != "H" {print}' | head -5)"
[[ -z "$flagged" ]] || die "index entries hide working-tree changes (skip-worktree or assume-unchanged): $flagged"
commit="$(git rev-parse "${object}^{commit}")"
# VERSION at the tagged commit decides the version published.
version="$(git cat-file blob "$commit:VERSION" | tr -d '\r\n')"
[[ "$SYSPRIMS_RELEASE_TAG" == "v$version" ]] || die 'SYSPRIMS_RELEASE_TAG does not match VERSION at the tagged commit'

# Everything that reads crate sources (plan check, dry runs, packaging,
# publication) runs in an export of the tagged commit's blobs: no checkout
# semantics (autocrlf, eol, filters, sparse-checkout, export attributes) apply,
# and every exported file is checked against its blob before cargo runs.
# cargo's target directory is a fresh directory, so the .crate hashed is the
# one just written.
scratch="$(cd "$(mktemp -d)" && pwd -P)"
tree="$scratch/tree"
trap 'rm -rf "$scratch"' EXIT
python3 "$root/scripts/release-export-tree.py" "$root" "$commit" "$tree" ||
	die 'cannot export the tagged commit with its exact bytes'
unset CARGO_BUILD_TARGET_DIR
export CARGO_TARGET_DIR="$scratch/target"
cd "$tree"
python3 "$tree/scripts/release-crates.py" check

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
	200) python3 "$tree/scripts/release-crates.py" registry-checksum "$crate" "$version" <<<"$body" ;;
	*) die "crates.io returned HTTP $status for $crate@$version" ;;
	esac
}

# sha256 of the crate packaged from the tagged commit. The .crate is removed
# first so the hash is always of the file this call produced.
local_checksum() {
	local crate="$1" file="$CARGO_TARGET_DIR/package/${1}-${version}.crate" path
	rm -f "$file"
	# No --allow-dirty: cargo refuses if any file differs from the tagged commit.
	cargo package --locked --no-verify -p "$crate" >/dev/null 2>&1 || die "cannot package $crate"
	[[ -f "$file" ]] || die "packaged crate $file not found"
	path="$(python3 "$tree/scripts/release-crates.py" paths | awk -v c="$crate" '$1==c {print $2}')"
	python3 "$tree/scripts/release-crates.py" vcs-info "$file" "$crate" "$version" "$commit" "$path" ||
		die "$crate does not record the tagged commit and its path"
	python3 -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$file"
}

# Collect the ordered plan whole and check the producer before iterating.
plan_output="$(python3 "$tree/scripts/release-crates.py" paths)" || die 'cannot read the crate publication plan'
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
