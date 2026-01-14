#!/usr/bin/env bash
# Sign release checksum manifests with minisign (and optionally PGP)
# Usage: sign-release-assets.sh <tag> [dir]
#
# Environment variables:
#   SYSPRIMS_MINISIGN_KEY  - Path to minisign secret key (required)
#   SYSPRIMS_PGP_KEY_ID    - PGP key ID for optional GPG signing (optional)
#   SYSPRIMS_GPG_HOMEDIR   - Custom GPG home directory (optional)
#
# Requires: minisign, optionally gpg
set -euo pipefail

TAG=${1:?"usage: sign-release-assets.sh <tag> [dir]"}
DIR=${2:-dist/release}

if [ ! -d "$DIR" ]; then
	echo "Error: Directory $DIR does not exist"
	exit 1
fi

if [ -z "${SYSPRIMS_MINISIGN_KEY:-}" ]; then
	echo "Error: SYSPRIMS_MINISIGN_KEY environment variable not set"
	echo ""
	echo "Set to path of your minisign secret key:"
	echo "  export SYSPRIMS_MINISIGN_KEY=/path/to/sysprims.key"
	exit 1
fi

if [ ! -f "$SYSPRIMS_MINISIGN_KEY" ]; then
	echo "Error: Minisign key not found: $SYSPRIMS_MINISIGN_KEY"
	exit 1
fi

cd "$DIR"

echo "Signing release $TAG..."

# Sign with minisign
echo ""
echo "=== Minisign Signatures ==="

for manifest in SHA256SUMS SHA512SUMS; do
	if [ -f "$manifest" ]; then
		echo "Signing $manifest with minisign..."
		minisign -S -s "$SYSPRIMS_MINISIGN_KEY" \
			-m "$manifest" \
			-t "sysprims $TAG - $(date -u +%Y-%m-%dT%H:%M:%SZ)" \
			-x "${manifest}.minisig"
		echo "[ok] Created ${manifest}.minisig"
	fi
done

# Optional PGP signing
if [ -n "${SYSPRIMS_PGP_KEY_ID:-}" ]; then
	echo ""
	echo "=== PGP Signatures ==="

	GPG_OPTS=()
	if [ -n "${SYSPRIMS_GPG_HOMEDIR:-}" ]; then
		GPG_OPTS+=("--homedir" "$SYSPRIMS_GPG_HOMEDIR")
	fi

	for manifest in SHA256SUMS SHA512SUMS; do
		if [ -f "$manifest" ]; then
			echo "Signing $manifest with PGP..."
			gpg "${GPG_OPTS[@]}" \
				--armor \
				--detach-sign \
				--local-user "$SYSPRIMS_PGP_KEY_ID" \
				--output "${manifest}.asc" \
				"$manifest"
			echo "[ok] Created ${manifest}.asc"
		fi
	done
else
	echo ""
	echo "[--] PGP signing skipped (SYSPRIMS_PGP_KEY_ID not set)"
fi

echo ""
echo "[ok] Signing complete"
ls -la "$DIR"/*.minisig "$DIR"/*.asc 2>/dev/null || true
