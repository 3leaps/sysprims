#!/usr/bin/env bash
# Verify that exported keys contain only public material (no secrets)
# Usage: verify-public-keys.sh [dir]
#
# Critical safety check before uploading to GitHub
set -euo pipefail

DIR=${1:-dist/release}

if [ ! -d "$DIR" ]; then
	echo "Error: Directory $DIR does not exist"
	exit 1
fi

cd "$DIR"

echo "Verifying public keys contain no secret material..."

ERRORS=0

# Check minisign public key
echo ""
echo "=== Minisign Key Check ==="

if [ -f "sysprims-minisign.pub" ]; then
	# Minisign public keys start with "untrusted comment:" and contain base64 data
	# They should NOT contain "SECRET KEY" or similar
	if grep -qi "secret" "sysprims-minisign.pub"; then
		echo "[!!] DANGER: sysprims-minisign.pub may contain secret key material!"
		ERRORS=$((ERRORS + 1))
	elif grep -q "^untrusted comment:" "sysprims-minisign.pub"; then
		echo "[ok] sysprims-minisign.pub appears to be a valid public key"
	else
		echo "[!!] sysprims-minisign.pub has unexpected format"
		ERRORS=$((ERRORS + 1))
	fi
else
	echo "[--] sysprims-minisign.pub not found"
fi

# Check PGP public key
echo ""
echo "=== PGP Key Check ==="

if [ -f "sysprims-release-signing-key.asc" ]; then
	# PGP public keys should have "PUBLIC KEY BLOCK" not "PRIVATE KEY BLOCK"
	if grep -q "PRIVATE KEY BLOCK" "sysprims-release-signing-key.asc"; then
		echo "[!!] DANGER: sysprims-release-signing-key.asc contains PRIVATE KEY!"
		ERRORS=$((ERRORS + 1))
	elif grep -q "PUBLIC KEY BLOCK" "sysprims-release-signing-key.asc"; then
		echo "[ok] sysprims-release-signing-key.asc is a public key"

		# Verify it can be imported and show key info
		GNUPGHOME=$(mktemp -d)
		export GNUPGHOME
		trap 'rm -rf "$GNUPGHOME"' EXIT

		if gpg --import sysprims-release-signing-key.asc 2>/dev/null; then
			echo "Key info:"
			gpg --list-keys 2>/dev/null | grep -A1 "^pub" || true
		fi
	else
		echo "[!!] sysprims-release-signing-key.asc has unexpected format"
		ERRORS=$((ERRORS + 1))
	fi
else
	echo "[--] sysprims-release-signing-key.asc not found"
fi

echo ""
if [ $ERRORS -eq 0 ]; then
	echo "[ok] Public key verification passed"
	exit 0
else
	echo "[!!] CRITICAL: Found $ERRORS potential secret key exposures!"
	echo "DO NOT upload these files to GitHub!"
	exit 1
fi
