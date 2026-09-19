#!/usr/bin/env bash
# Download the exact unsigned base payload for one draft release.
set -euo pipefail
TAG=${1:?"usage: download-release-assets.sh <tag> [dest_dir]"}
DEST=${2:-dist/release}
VERSION=${TAG#v}
mkdir -p "$DEST"
if find "$DEST" -mindepth 1 -maxdepth 1 -print -quit | grep -q .; then
	echo "Error: $DEST is not empty; run make release-clean before download" >&2
	exit 1
fi
EXPECTED=(
	LICENSE-APACHE LICENSE-MIT
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
for name in "${EXPECTED[@]}"; do
	gh release download "$TAG" --dir "$DEST" --pattern "$name"
done
actual=$(find "$DEST" -maxdepth 1 -type f -exec basename {} \; | LC_ALL=C sort)
expected=$(printf '%s\n' "${EXPECTED[@]}" | LC_ALL=C sort)
[ "$actual" = "$expected" ] || {
	echo "Error: downloaded inventory mismatch" >&2
	diff -u <(printf '%s\n' "$expected") <(printf '%s\n' "$actual") >&2 || true
	exit 1
}
echo "[ok] Exact base payload downloaded for $TAG"
