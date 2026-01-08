#!/usr/bin/env bash
# Download release assets from GitHub
# Usage: download-release-assets.sh <tag> [dest_dir]
#
# Requires: gh CLI authenticated
set -euo pipefail

TAG=${1:?"usage: download-release-assets.sh <tag> [dest_dir]"}
DEST=${2:-dist/release}

echo "Downloading release assets for $TAG to $DEST..."

mkdir -p "$DEST"

# Download all release artifacts except signatures (those are added after signing)
gh release download "$TAG" --dir "$DEST" --clobber \
  --pattern 'sysprims-*.tar.gz' \
  --pattern 'sysprims-*.zip' \
  --pattern 'sysprims-ffi-*.tar.gz' \
  --pattern 'sysprims.h' \
  --pattern 'sbom-*.json' \
  --pattern 'LICENSE-*'

echo "Downloaded to $DEST:"
ls -la "$DEST"
