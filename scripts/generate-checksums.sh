#!/usr/bin/env bash
# Generate exact SHA256/SHA512 manifests for one immutable release payload.
set -euo pipefail

DIR=${1:-dist/release}
TAG=${2:-${SYSPRIMS_RELEASE_TAG:-}}
[ -d "$DIR" ] || {
	echo "Error: Directory $DIR does not exist" >&2
	exit 1
}
[ -n "$TAG" ] && [ "$TAG" != v ] || {
	echo "Error: release tag is required" >&2
	exit 1
}
VERSION=${TAG#v}
cd "$DIR"

# The notes and the fingerprint anchors come from the tagged commit, staged
# before the manifests so the signed checksums cover them.
for required in "release-notes-${TAG}.md" expected-fingerprints.txt expected-fingerprints.ndjson; do
	if [ ! -f "$required" ] || [ -L "$required" ]; then
		echo "Error: $required not in $DIR" >&2
		echo "Copy notes and anchors before checksums:" >&2
		echo "  make release-notes" >&2
		echo "  make release-stage-anchors" >&2
		exit 1
	fi
done

EXPECTED=(
	LICENSE-APACHE LICENSE-MIT
	"release-notes-${TAG}.md"
	expected-fingerprints.txt expected-fingerprints.ndjson
	"sbom-${VERSION}.cdx.json"
	"sysprims-${VERSION}-darwin-amd64.tar.gz"
	"sysprims-${VERSION}-darwin-arm64.tar.gz"
	"sysprims-${VERSION}-linux-amd64-musl.tar.gz"
	"sysprims-${VERSION}-linux-amd64.tar.gz"
	"sysprims-${VERSION}-linux-arm64-musl.tar.gz"
	"sysprims-${VERSION}-linux-arm64.tar.gz"
	"sysprims-${VERSION}-windows-amd64.zip"
	"sysprims-${VERSION}-windows-arm64.zip"
	"sysprims-ffi-${VERSION}-libs.tar.gz"
	sysprims.h
)
printf '%s\n' "${EXPECTED[@]}" | LC_ALL=C sort >.expected-release-files
find . -maxdepth 1 -type f ! -name '.expected-release-files' ! -name '.actual-release-files' ! -name 'SHA256SUMS' ! -name 'SHA512SUMS' -printf '%f\n' 2>/dev/null | LC_ALL=C sort >.actual-release-files || {
	find . -maxdepth 1 -type f ! -name '.expected-release-files' ! -name '.actual-release-files' ! -name 'SHA256SUMS' ! -name 'SHA512SUMS' -exec basename {} \; | LC_ALL=C sort >.actual-release-files
}
if ! cmp -s .expected-release-files .actual-release-files; then
	echo "Error: release payload inventory is incomplete or contains leftovers" >&2
	diff -u .expected-release-files .actual-release-files >&2 || true
	rm -f .expected-release-files .actual-release-files
	exit 1
fi
rm -f .expected-release-files .actual-release-files
printf '%s\n' "${EXPECTED[@]}" | LC_ALL=C sort | xargs shasum -a 256 >SHA256SUMS
printf '%s\n' "${EXPECTED[@]}" | LC_ALL=C sort | xargs shasum -a 512 >SHA512SUMS
printf '%s\n' "[ok] Exact checksum manifests generated for ${TAG}"
