#!/usr/bin/env bash
# Download the unsigned release payload that the tag-push Release workflow built,
# from the named run and attempt on the verified tag's commit. The workflow holds
# no write authority; the draft release is created locally afterwards.
#
#   release-fetch-ci-artifacts.sh [dest_dir]
#       Requires RELEASE_RUN_ID and RELEASE_RUN_ATTEMPT. Stages the files and
#       writes dest_dir.anchor (tag, object, commit, run, attempt, artifact,
#       digest, and each file's SHA-256).
#   release-fetch-ci-artifacts.sh --verify ANCHOR
#       Re-reads the run, attempt and artifact named in ANCHOR from GitHub,
#       downloads that artifact again, and prints each file's SHA-256 from the
#       downloaded copy (`file=<sha256> <name>`). Writes nothing outside a
#       private scratch directory. The anchor is a hint naming what to fetch;
#       its file lines are never trusted.
# Requires SYSPRIMS_RELEASE_TAG. The repository is always 3leaps/sysprims.
set -euo pipefail
die() {
	echo "error: $*" >&2
	exit 1
}
repo=3leaps/sysprims
mode=fetch
if [[ "${1:-}" == --verify ]]; then
	mode=verify
	anchor_in="${2:?usage: release-fetch-ci-artifacts.sh --verify ANCHOR}"
else
	dest="${1:-dist/release}"
fi
tag="${SYSPRIMS_RELEASE_TAG:-}"
[[ "$tag" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || die 'SYSPRIMS_RELEASE_TAG=vX.Y.Z is required'
for tool in gh jq python3 shasum git; do
	command -v "$tool" >/dev/null 2>&1 || die "$tool not found in PATH"
done
root="$(cd "$(dirname "$0")/.." && pwd -P)"

if [[ "$mode" == fetch ]]; then
	# Neither the destination nor the anchor may be a symlink (a dangling one
	# included): the moves and the anchor write below would follow it.
	[[ ! -L "$dest" && ! -L "$dest.anchor" ]] || die "$dest and $dest.anchor must not be symlinks"
	[[ ! -e "$dest.anchor" ]] || die "$dest.anchor exists; run make release-clean first"
	mkdir -p "$dest"
	[[ -d "$dest" && ! -L "$dest" ]] || die "$dest is not a directory"
	[[ -z "$(find "$dest" -mindepth 1 -maxdepth 1 2>/dev/null)" ]] || die "$dest is not empty; run make release-clean first"
	run_id="${RELEASE_RUN_ID:-}"
	run_attempt="${RELEASE_RUN_ATTEMPT:-}"
else
	[[ -f "$anchor_in" && ! -L "$anchor_in" ]] || die "anchor $anchor_in is missing or not a regular file"
	field() { awk -F= -v k="$1" '$1==k {print substr($0, length(k) + 2); n++} END {exit n != 1}' "$anchor_in"; }
	[[ "$(field tag)" == "$tag" ]] || die "anchor $anchor_in is not for $tag"
	run_id="$(field run)" || die 'anchor has no single run'
	run_attempt="$(field attempt)" || die 'anchor has no single attempt'
	want_artifact="$(field artifact)" || die 'anchor has no single artifact'
	want_digest="$(field digest)" || die 'anchor has no single digest'
	want_object="$(field object)" || die 'anchor has no single object'
	want_commit="$(field commit)" || die 'anchor has no single commit'
fi

# The pushed tags must be the verified local objects; record the object and
# commit every later step binds to.
"$root/scripts/release-verify-remote-tag.sh" >/dev/null || die "published tag $tag failed verification"
object="$(git -C "$root" rev-parse --verify --quiet "refs/tags/$tag")" || die "local tag $tag is absent"
commit="$(git -C "$root" rev-parse "${object}^{commit}")"
[[ "$commit" =~ ^[0-9a-f]{40}$ && "$object" =~ ^[0-9a-f]{40}$ ]] || die 'could not resolve the verified tag'
if [[ "$mode" == verify ]]; then
	[[ "$want_object" == "$object" && "$want_commit" == "$commit" ]] ||
		die 'the verified tag is no longer the object the payload was staged against'
fi

# The named attempt must be a successful push-triggered release.yml run for
# this tag on the tagged commit (re-runs share a run id).
[[ "$run_id" =~ ^[1-9][0-9]*$ ]] || die 'RELEASE_RUN_ID required (the Release workflow run for this tag)'
[[ "$run_attempt" =~ ^[1-9][0-9]*$ ]] || die 'RELEASE_RUN_ATTEMPT required (the successful attempt of that run)'
run="$(gh api "repos/$repo/actions/runs/$run_id/attempts/$run_attempt" \
	--jq '{path, event, head_branch, head_sha, status, conclusion, run_attempt}')" ||
	die "could not read run $run_id attempt $run_attempt"
jq -e --arg tag "$tag" --arg commit "$commit" --argjson attempt "$run_attempt" '
	.path == ".github/workflows/release.yml" and .event == "push" and .head_branch == $tag and
	.head_sha == $commit and .status == "completed" and .conclusion == "success" and
	.run_attempt == $attempt' <<<"$run" >/dev/null ||
	die "run $run_id attempt $run_attempt is not a successful release run for $tag at $commit"

# `gh run download` cannot select an attempt, so bind to one artifact: record its
# id and digest while the named attempt is the latest, download that exact id,
# check the bytes against the digest, then confirm nothing moved.
artifact_name="release-packages-$tag"
snapshot() {
	local latest listing
	latest="$(gh api "repos/$repo/actions/runs/$run_id" --jq '.run_attempt')" ||
		die "could not read run $run_id"
	[[ "$latest" == "$run_attempt" ]] ||
		die "run $run_id has a later attempt ($latest); artifacts belong to the latest attempt, name it"
	listing="$(gh api "repos/$repo/actions/runs/$run_id/artifacts?per_page=100")" ||
		die "could not list artifacts for run $run_id"
	# One page: refuse a run whose artifacts do not all fit on it, so "exactly
	# one" below is exact for the whole run.
	jq -e '.total_count == (.artifacts | length)' <<<"$listing" >/dev/null ||
		die "artifact list for run $run_id is incomplete"
	listing="$(jq --arg n "$artifact_name" '[.artifacts[] | select(.name == $n and .expired == false)]' <<<"$listing")" ||
		die "could not read the artifact list for run $run_id"
	jq -e --arg c "$commit" --argjson r "$run_id" \
		'length == 1 and (.[0].workflow_run.id == $r) and (.[0].workflow_run.head_sha == $c)' <<<"$listing" >/dev/null ||
		die "run $run_id does not hold exactly one unexpired $artifact_name for $commit"
	jq -r '.[0] | "\(.id) \(.digest // "")"' <<<"$listing"
}
# A failed snapshot exits its subshell only; stop here rather than read nothing.
before="$(snapshot)" || exit 1
read -r artifact_id artifact_digest <<<"$before"
[[ "$artifact_id" =~ ^[1-9][0-9]*$ ]] || die 'artifact id is malformed'
[[ "$artifact_digest" =~ ^sha256:[0-9a-f]{64}$ ]] || die "artifact $artifact_id has no sha256 digest"
if [[ "$mode" == verify ]]; then
	[[ "$artifact_id" == "$want_artifact" && "$artifact_digest" == "$want_digest" ]] ||
		die "run $run_id now names artifact $artifact_id ($artifact_digest), not the staged $want_artifact"
fi
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
gh api "repos/$repo/actions/artifacts/$artifact_id/zip" >"$work/artifact.zip" ||
	die "could not download artifact $artifact_id"
actual="sha256:$(shasum -a 256 "$work/artifact.zip" | awk '{print $1}')"
[[ "$actual" == "$artifact_digest" ]] || die "artifact $artifact_id bytes do not match its recorded digest"
after="$(snapshot)" || exit 1
[[ "$after" == "$before" ]] || die "run $run_id changed during download (artifact $before -> $after); restage"

# Extract into private scratch first: flat, unique, plain names only. Nothing
# reaches the destination unless every entry passes.
mkdir "$work/x"
python3 -I - "$work/artifact.zip" "$work/x" <<'PY' || die "artifact $artifact_id has an unexpected layout"
import pathlib, re, sys, zipfile
dest = pathlib.Path(sys.argv[2])
seen = set()
with zipfile.ZipFile(sys.argv[1]) as z:
    for info in z.infolist():
        name = info.filename
        if info.is_dir() or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._-]*', name) or name in seen:
            raise SystemExit(f'error: unexpected entry in artifact: {name!r}')
        # Unix mode bits in the high word: refuse anything but a regular file.
        mode = info.external_attr >> 16
        if mode and (mode & 0o170000) not in (0, 0o100000):
            raise SystemExit(f'error: non-regular entry in artifact: {name!r}')
        seen.add(name)
        # 'xb' refuses an existing file, so a name that only aliases an earlier
        # one (case-only on a case-insensitive filesystem) cannot overwrite it.
        try:
            with open(dest / name, 'xb') as out:
                out.write(z.read(info))
        except FileExistsError:
            raise SystemExit(f'error: entry collides with an earlier one: {name!r}')
if not seen:
    raise SystemExit('error: artifact is empty')
PY
# Each file's digest, from the digest-verified archive.
(cd "$work/x" && find . -mindepth 1 -maxdepth 1 -type f -exec basename {} \; | LC_ALL=C sort |
	while IFS= read -r name; do printf 'file=%s %s\n' "$(shasum -a 256 "$name" | awk '{print $1}')" "$name"; done) >"$work/files" ||
	die 'could not record the artifact file digests'
if [[ "$mode" == verify ]]; then
	cat "$work/files"
	exit 0
fi
mv "$work/x"/* "$dest/"
# The anchor lives outside the staged directory so it is never uploaded, and is
# written only after the packages are in place.
{
	printf 'tag=%s\nobject=%s\ncommit=%s\nrun=%s\nattempt=%s\nartifact=%s\ndigest=%s\n' \
		"$tag" "$object" "$commit" "$run_id" "$run_attempt" "$artifact_id" "$artifact_digest"
	cat "$work/files"
} >"$dest.anchor"
echo "[ok] downloaded release packages for $tag (object $object, commit $commit) from $repo run $run_id attempt $run_attempt, artifact $artifact_id into $dest"
