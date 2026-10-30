#!/usr/bin/env bash
# Stub-gh controls for release-fetch-ci-artifacts.sh (draft creation is covered
# in release-integrity.test.mjs).
# No live GitHub call is made; the stub records every invocation.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
repo="$scratch/repo"
git init -q -b main "$repo"
git -C "$repo" config user.name 'Fixture'
git -C "$repo" config user.email fixture@example.invalid
printf 'x\n' >"$repo/file"
git -C "$repo" add file
mkdir -p "$repo/docs/releases"
printf '# fixture v1.2.3\n\nCurated notes.\n' >"$repo/docs/releases/v1.2.3.md"
git -C "$repo" add docs/releases/v1.2.3.md
git -C "$repo" commit -qm fixture
git -C "$repo" tag -am 'Fixture release' v1.2.3
commit="$(git -C "$repo" rev-parse HEAD)"
mkdir -p "$repo/scripts"
cp "$root/scripts/release-fetch-ci-artifacts.sh" "$repo/scripts/"
object="$(git -C "$repo" rev-parse refs/tags/v1.2.3)"
# Stand-in for the trusted remote-tag gate (its own suite uses real
# signatures): it fails for any tag but the fixture's, or when told the remote
# tag was replaced.
cat >"$repo/scripts/release-verify-remote-tag.sh" <<'SH'
#!/usr/bin/env bash
printf 'reverify %s\n' "$SYSPRIMS_RELEASE_TAG" >> "$FIXTURE_LOG"
[[ "$SYSPRIMS_RELEASE_TAG" == v1.2.3 ]] || { echo 'error: tag absent' >&2; exit 1; }
[[ "${FIXTURE_TAG_REPLACED:-0}" == 0 ]] || { echo 'error: tag replaced' >&2; exit 1; }
SH
chmod +x "$repo/scripts/release-verify-remote-tag.sh"

# Stub gh. FIXTURE_ATTEMPT is the run-attempt JSON the API returns and
# FIXTURE_LATEST is the latest attempt number of that run;
# FIXTURE_RELEASE_EXISTS controls `gh release view`. Every call is logged.
mkdir -p "$scratch/bin"
cat >"$scratch/bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FIXTURE_LOG"
case "$1 $2" in
"api "*)
    path="$2"
    jq_filter=""
    while [[ $# -gt 0 ]]; do
        [[ "$1" == --jq ]] && jq_filter="$2"
        shift
    done
    case "$path" in
    */attempts/*) body="${FIXTURE_ATTEMPT:-}" ;;
    */runs/*/artifacts*)
        # Listing calls are counted so a test can change the answer between the
        # before- and after-download snapshots (a re-run landing mid-download).
        n="$(($(cat "$FIXTURE_DIR/listings" 2> /dev/null || echo 0) + 1))"
        printf '%s\n' "$n" > "$FIXTURE_DIR/listings"
        if [[ -n "${FIXTURE_LISTING:-}" ]]; then
            body="$FIXTURE_LISTING"
        else
            id="${FIXTURE_ARTIFACT_ID:-501}"
            [[ "$n" -ge 2 && -n "${FIXTURE_ARTIFACT_ID_AFTER:-}" ]] && id="$FIXTURE_ARTIFACT_ID_AFTER"
            digest="${FIXTURE_DIGEST:-sha256:$(shasum -a 256 "$FIXTURE_DIR/artifact.zip" | awk '{print $1}')}"
            [[ "$n" -ge 2 && -n "${FIXTURE_DIGEST_AFTER:-}" ]] && digest="$FIXTURE_DIGEST_AFTER"
            body="$(jq -n --argjson id "$id" --arg d "$digest" --arg c "${FIXTURE_ARTIFACT_SHA:-$FIXTURE_COMMIT}" \
                --argjson e "${FIXTURE_EXPIRED:-false}" \
                '{total_count: 1, artifacts: [{id: $id, name: "release-packages-v1.2.3", expired: $e, digest: $d,
                  workflow_run: {id: 11, head_sha: $c}}]}')"
        fi
        ;;
    */artifacts/*/zip)
        [[ "$path" == */artifacts/"${FIXTURE_ARTIFACT_ID:-501}"/zip ]] || exit 1
        cat "${FIXTURE_SERVED_ZIP:-$FIXTURE_DIR/artifact.zip}"
        exit 0
        ;;
    *)
        m="$(($(cat "$FIXTURE_DIR/runs" 2> /dev/null || echo 0) + 1))"
        printf '%s\n' "$m" > "$FIXTURE_DIR/runs"
        latest="${FIXTURE_LATEST:-1}"
        [[ "$m" -ge 2 && -n "${FIXTURE_LATEST_AFTER:-}" ]] && latest="$FIXTURE_LATEST_AFTER"
        body="{\"run_attempt\":$latest}"
        ;;
    esac
    [[ -n "$body" ]] || exit 1
    if [[ -n "$jq_filter" ]]; then jq -c "$jq_filter" <<< "$body"; else printf '%s\n' "$body"; fi
    ;;
"run download") exit 1 ;;
"release view") [[ "${FIXTURE_RELEASE_EXISTS:-0}" == 1 ]] ;;
"release create") exit 0 ;;
*) exit 1 ;;
esac
SH
chmod +x "$scratch/bin/gh"
export PATH="$scratch/bin:$PATH" FIXTURE_LOG="$scratch/gh.log" FIXTURE_DIR="$scratch/fx" FIXTURE_COMMIT="$commit"
mkdir -p "$FIXTURE_DIR"
# make_zip OUT NAME=CONTENT... ; a NAME of @link:TARGET adds a symlink entry.
make_zip() {
	python3 -I - "$@" <<'PY'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], 'w') as z:
    for spec in sys.argv[2:]:
        name, _, body = spec.partition('=')
        if name.startswith('@link:'):
            info = zipfile.ZipInfo(name[len('@link:'):])
            info.external_attr = (0o120777 << 16)
            z.writestr(info, body)
        else:
            z.writestr(name, body)
PY
}
# The CI artifact: a zip holding one release package.
make_zip "$FIXTURE_DIR/artifact.zip" 'sysprims-1.2.3-linux-amd64.tar.gz=pkg'
reset_listings() { rm -f "$FIXTURE_DIR/listings" "$FIXTURE_DIR/runs"; }

fail() {
	local reason="$1"
	shift
	if (cd "$repo" && "$@") >"$scratch/out" 2>&1; then
		echo "error: expected rejection: $reason" >&2
		exit 1
	fi
	grep -q "$reason" "$scratch/out" || {
		echo "error: missing named rejection: $reason" >&2
		cat "$scratch/out" >&2
		exit 1
	}
}
attempt_json() { # conclusion event head_sha attempt [path] [branch]
	printf '{"path":"%s","event":"%s","head_branch":"%s","head_sha":"%s","status":"completed","conclusion":"%s","run_attempt":%s}' \
		"${5:-.github/workflows/release.yml}" "$2" "${6:-v1.2.3}" "$3" "$1" "$4"
}
good="$(attempt_json success push "$commit" 2)"
export RELEASE_RUN_ID=11 RELEASE_RUN_ATTEMPT=2 FIXTURE_LATEST=2

# Fetch: refuses a non-canonical tag, a missing local tag, a missing run id or
# attempt, and any named attempt that is not a successful push-triggered
# release run for this tag on the tagged commit or is not the latest attempt.
SYSPRIMS_RELEASE_TAG=1.2.3 fail 'SYSPRIMS_RELEASE_TAG=vX.Y.Z is required' ./scripts/release-fetch-ci-artifacts.sh "$scratch/d0"
SYSPRIMS_RELEASE_TAG=v9.9.9 fail 'published tag v9.9.9 failed verification' ./scripts/release-fetch-ci-artifacts.sh "$scratch/d0"
RELEASE_RUN_ID='' FIXTURE_ATTEMPT="$good" fail 'RELEASE_RUN_ID required' \
	env SYSPRIMS_RELEASE_TAG=v1.2.3 ./scripts/release-fetch-ci-artifacts.sh "$scratch/d1"
RELEASE_RUN_ATTEMPT='' FIXTURE_ATTEMPT="$good" fail 'RELEASE_RUN_ATTEMPT required' \
	env SYSPRIMS_RELEASE_TAG=v1.2.3 ./scripts/release-fetch-ci-artifacts.sh "$scratch/d1"
for bad in \
	"$(attempt_json failure push "$commit" 2)" \
	"$(attempt_json success workflow_dispatch "$commit" 2)" \
	"$(attempt_json success push "$(printf '%040d' 0)" 2)" \
	"$(attempt_json success push "$commit" 1)" \
	"$(attempt_json success push "$commit" 2 .github/workflows/ci.yml)" \
	"$(attempt_json success push "$commit" 2 .github/workflows/release.yml v1.2.4)" \
	"$(attempt_json success push "$commit" 2 | sed 's/"completed"/"in_progress"/')"; do
	FIXTURE_ATTEMPT="$bad" fail 'is not a successful release run' \
		env SYSPRIMS_RELEASE_TAG=v1.2.3 ./scripts/release-fetch-ci-artifacts.sh "$scratch/d1"
done
FIXTURE_ATTEMPT="$good" FIXTURE_LATEST=3 fail 'has a later attempt' \
	env SYSPRIMS_RELEASE_TAG=v1.2.3 ./scripts/release-fetch-ci-artifacts.sh "$scratch/d1"

# Artifact identity. Every refusal below must leave no anchor and no package.
fetch_refuses() {
	local reason="$1" dir="$scratch/dA"
	shift
	rm -rf "$dir" "$dir.anchor"
	reset_listings
	(cd "$repo" && env FIXTURE_ATTEMPT="$good" "$@" env SYSPRIMS_RELEASE_TAG=v1.2.3 ./scripts/release-fetch-ci-artifacts.sh "$dir") \
		>"$scratch/out" 2>&1 && {
		echo "error: expected rejection: $reason" >&2
		exit 1
	}
	grep -q "$reason" "$scratch/out" || {
		echo "error: missing named rejection: $reason" >&2
		cat "$scratch/out" >&2
		exit 1
	}
	[[ ! -e "$dir.anchor" ]] || {
		echo "error: anchor written despite: $reason" >&2
		exit 1
	}
	[[ -z "$(find "$dir" -mindepth 1 2>/dev/null)" ]] || {
		echo "error: files staged despite: $reason" >&2
		exit 1
	}
}
listing() { # one JSON artifact object per argument, wrapped as a complete listing
	local IFS=,
	printf '{"total_count":%s,"artifacts":[%s]}' "$#" "$*"
}
artifact() { # id head_sha [expired] [name] [run]
	printf '{"id":%s,"name":"%s","expired":%s,"digest":"sha256:%s","workflow_run":{"id":%s,"head_sha":"%s"}}' \
		"$1" "${4:-release-packages-v1.2.3}" "${3:-false}" \
		"$(shasum -a 256 "$FIXTURE_DIR/artifact.zip" | awk '{print $1}')" "${5:-11}" "$2"
}
# A re-run attempt appears while the artifact downloads.
fetch_refuses 'has a later attempt' FIXTURE_LATEST_AFTER=3
# The artifact is replaced by another of the same name mid-download.
fetch_refuses 'changed during download' FIXTURE_ARTIFACT_ID_AFTER=502
fetch_refuses 'changed during download' FIXTURE_DIGEST_AFTER="sha256:$(printf '%064d' 7)"
# The bytes served are not the bytes the listing's digest names.
make_zip "$scratch/other.zip" 'sysprims-1.2.3-linux-amd64.tar.gz=other'
fetch_refuses 'do not match its recorded digest' FIXTURE_SERVED_ZIP="$scratch/other.zip"
fetch_refuses 'do not match its recorded digest' FIXTURE_DIGEST="sha256:$(printf '%064d' 9)"
# No digest, or a malformed one.
fetch_refuses 'has no sha256 digest' FIXTURE_LISTING="$(listing "$(artifact 501 "$commit" | sed 's/"digest":"[^"]*"/"digest":null/')")"
fetch_refuses 'has no sha256 digest' FIXTURE_LISTING="$(listing "$(artifact 501 "$commit" | sed 's/sha256:/md5:/')")"
# Built from another commit, by another run, expired, absent, or ambiguous.
fetch_refuses 'does not hold exactly one unexpired' FIXTURE_ARTIFACT_SHA="$(printf '%040d' 3)"
fetch_refuses 'does not hold exactly one unexpired' FIXTURE_LISTING="$(listing "$(artifact 501 "$commit" false release-packages-v1.2.3 12)")"
fetch_refuses 'does not hold exactly one unexpired' FIXTURE_EXPIRED=true
fetch_refuses 'does not hold exactly one unexpired' FIXTURE_LISTING='{"total_count":0,"artifacts":[]}'
# A run whose artifacts span more than the one page read is refused.
fetch_refuses 'artifact list for run 11 is incomplete' \
	FIXTURE_LISTING="$(listing "$(artifact 501 "$commit")" | sed 's/"total_count":1/"total_count":31/')"
fetch_refuses 'does not hold exactly one unexpired' FIXTURE_LISTING="$(listing "$(artifact 501 "$commit")" "$(artifact 502 "$commit")")"
# An expired twin does not make a live artifact ambiguous; a differently named
# one is ignored.
FIXTURE_LISTING="$(listing "$(artifact 501 "$commit")" "$(artifact 499 "$commit" true)" "$(artifact 498 "$commit" false release-packages-v1.2.2)")"
reset_listings
rm -rf "$scratch/dB" "$scratch/dB.anchor"
(cd "$repo" && FIXTURE_ATTEMPT="$good" FIXTURE_LISTING="$FIXTURE_LISTING" env SYSPRIMS_RELEASE_TAG=v1.2.3 ./scripts/release-fetch-ci-artifacts.sh "$scratch/dB") >/dev/null
[[ -f "$scratch/dB/sysprims-1.2.3-linux-amd64.tar.gz" && -f "$scratch/dB.anchor" ]]
# Zip layouts that are not flat, unique, plain files refuse before staging.
cp "$FIXTURE_DIR/artifact.zip" "$scratch/good.zip"
for layout in \
	'nested/sysprims-1.2.3-linux-amd64.tar.gz=pkg' \
	'../sysprims-1.2.3-linux-amd64.tar.gz=pkg' \
	'.hidden=pkg' \
	'@link:sysprims-1.2.3-linux-amd64.tar.gz=/etc/passwd'; do
	make_zip "$FIXTURE_DIR/artifact.zip" 'sysprims-1.2.3-darwin-arm64.tar.gz=ok' "$layout"
	fetch_refuses 'unexpected layout'
done
make_zip "$FIXTURE_DIR/artifact.zip" 'sysprims-1.2.3-linux-amd64.tar.gz=a' 'sysprims-1.2.3-linux-amd64.tar.gz=b' 2>/dev/null
fetch_refuses 'unexpected layout'
make_zip "$FIXTURE_DIR/artifact.zip"
fetch_refuses 'unexpected layout'
# Names that differ only in case alias on a case-insensitive filesystem; on a
# case-sensitive one both land, and the draft's exact-set check refuses the
# extra file. Either way nothing is silently overwritten.
make_zip "$FIXTURE_DIR/artifact.zip" 'SHA256SUMS=real' 'sha256sums=shadow'
probe="$scratch/case-probe"
mkdir -p "$probe"
printf 'a' >"$probe/CaseProbe"
if [[ -e "$probe/caseprobe" ]]; then
	fetch_refuses 'unexpected layout'
fi
cp "$scratch/good.zip" "$FIXTURE_DIR/artifact.zip"
reset_listings
# Refuses a non-empty destination.
mkdir -p "$scratch/full"
printf 'stale\n' >"$scratch/full/stale"
FIXTURE_ATTEMPT="$good" fail 'is not empty' env SYSPRIMS_RELEASE_TAG=v1.2.3 ./scripts/release-fetch-ci-artifacts.sh "$scratch/full"
# Downloads exactly the bound artifact id from the named, verified attempt, and
# re-reads the run and the listing after the download.
: >"$FIXTURE_LOG"
(cd "$repo" && FIXTURE_ATTEMPT="$good" env SYSPRIMS_RELEASE_TAG=v1.2.3 ./scripts/release-fetch-ci-artifacts.sh "$scratch/d2") >/dev/null
grep -q '^api repos/3leaps/sysprims/actions/artifacts/501/zip$' "$FIXTURE_LOG"
if grep -q '^run download' "$FIXTURE_LOG"; then exit 1; fi
[[ "$(grep -c '^api repos/3leaps/sysprims/actions/runs/11/artifacts?per_page=100$' "$FIXTURE_LOG")" == 2 ]]
[[ "$(grep -n '' "$FIXTURE_LOG" | grep -E 'artifacts(/501/zip|\?per_page=100)$' | tail -1)" == *'/runs/11/artifacts?per_page=100' ]]
digest="sha256:$(shasum -a 256 "$FIXTURE_DIR/artifact.zip" | awk '{print $1}')"
[[ "$(cat "$scratch/d2.anchor")" == "$(printf 'tag=v1.2.3\nobject=%s\ncommit=%s\nrun=11\nattempt=2\nartifact=501\ndigest=%s\nfile=%s sysprims-1.2.3-linux-amd64.tar.gz' "$object" "$commit" "$digest" "$(printf pkg | shasum -a 256 | awk '{print $1}')")" ]]
[[ -f "$scratch/d2/sysprims-1.2.3-linux-amd64.tar.gz" ]]

# A symlinked destination or anchor (dangling included) refuses before any
# side effect.
mkdir -p "$scratch/elsewhere"
ln -s "$scratch/elsewhere" "$scratch/linked"
FIXTURE_ATTEMPT="$good" fail 'must not be symlinks' env SYSPRIMS_RELEASE_TAG=v1.2.3 ./scripts/release-fetch-ci-artifacts.sh "$scratch/linked"
[[ -z "$(find "$scratch/elsewhere" -mindepth 1)" ]]
ln -s "$scratch/nowhere" "$scratch/d3.anchor"
FIXTURE_ATTEMPT="$good" fail 'must not be symlinks' env SYSPRIMS_RELEASE_TAG=v1.2.3 ./scripts/release-fetch-ci-artifacts.sh "$scratch/d3"
[[ ! -e "$scratch/nowhere" ]]
# The repository is fixed; an override variable is not honoured.
: >"$FIXTURE_LOG"
rm -rf "$scratch/d4" "$scratch/d4.anchor"
reset_listings
(cd "$repo" && FIXTURE_ATTEMPT="$good" SYSPRIMS_GITHUB_REPO=fork/sysprims env SYSPRIMS_RELEASE_TAG=v1.2.3 ./scripts/release-fetch-ci-artifacts.sh "$scratch/d4") >/dev/null
if grep -q 'fork/' "$FIXTURE_LOG"; then
	echo 'error: fetch honoured a repository override' >&2
	exit 1
fi

# --verify reads GitHub again for the run, attempt and artifact the anchor
# names and prints file digests from the downloaded copy; the anchor's own file
# lines are never echoed back.
verify() { (cd "$repo" && env FIXTURE_ATTEMPT="$good" SYSPRIMS_RELEASE_TAG=v1.2.3 "$@" ./scripts/release-fetch-ci-artifacts.sh --verify "$scratch/v.anchor"); }
cp "$scratch/d2.anchor" "$scratch/v.anchor"
reset_listings
[[ "$(verify)" == "$(grep '^file=' "$scratch/d2.anchor")" ]]
# An anchor whose file lines were edited still yields the downloaded digests.
sed -i.bak 's/^file=[0-9a-f]*/file=0000000000000000000000000000000000000000000000000000000000000000/' "$scratch/v.anchor"
reset_listings
[[ "$(verify)" == "$(grep '^file=' "$scratch/d2.anchor")" ]]
cp "$scratch/d2.anchor" "$scratch/v.anchor"
verify_refuses() {
	local reason="$1"
	shift
	reset_listings
	if verify "$@" >"$scratch/out" 2>&1; then
		echo "error: --verify accepted: $reason" >&2
		exit 1
	fi
	grep -q "$reason" "$scratch/out" || {
		echo "error: --verify missing named rejection: $reason" >&2
		cat "$scratch/out" >&2
		exit 1
	}
}
# The anchor names another artifact, or the run now holds another one.
sed -i.bak 's/^artifact=501$/artifact=777/' "$scratch/v.anchor"
verify_refuses 'not the staged 777'
cp "$scratch/d2.anchor" "$scratch/v.anchor"
verify_refuses 'not the staged 501' FIXTURE_ARTIFACT_ID=502
sed -i.bak "s/^digest=.*/digest=sha256:$(printf '%064d' 5)/" "$scratch/v.anchor"
verify_refuses 'not the staged 501'
cp "$scratch/d2.anchor" "$scratch/v.anchor"
# The tag moved, or the anchor names another tag object.
verify_refuses 'failed verification' FIXTURE_TAG_REPLACED=1
sed -i.bak "s/^object=.*/object=$(printf '%040d' 4)/" "$scratch/v.anchor"
verify_refuses 'no longer the object'
cp "$scratch/d2.anchor" "$scratch/v.anchor"
# Missing or repeated identity fields refuse.
grep -v '^run=' "$scratch/d2.anchor" >"$scratch/v.anchor"
verify_refuses 'no single run'
cat "$scratch/d2.anchor" <(printf 'attempt=3\n') >"$scratch/v.anchor"
verify_refuses 'no single attempt'
cp "$scratch/d2.anchor" "$scratch/v.anchor"
# The named attempt is no longer the latest.
verify_refuses 'has a later attempt' FIXTURE_LATEST=3
ln -sf "$scratch/d2.anchor" "$scratch/v-link.anchor"
reset_listings
if (cd "$repo" && env FIXTURE_ATTEMPT="$good" SYSPRIMS_RELEASE_TAG=v1.2.3 ./scripts/release-fetch-ci-artifacts.sh --verify "$scratch/v-link.anchor") >/dev/null 2>&1; then
	echo 'error: --verify accepted a symlinked anchor' >&2
	exit 1
fi

echo '[ok] CI-artifact fetch controls passed'
