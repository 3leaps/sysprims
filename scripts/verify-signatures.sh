#!/usr/bin/env bash
# Verify signatures on checksum manifests
# Usage: verify-signatures.sh [dir]
#
# Verifies both minisign and PGP signatures if present
set -euo pipefail

DIR=${1:-dist/release}

if [ ! -d "$DIR" ]; then
  echo "Error: Directory $DIR does not exist"
  exit 1
fi

cd "$DIR"

echo "Verifying signatures in $DIR..."

ERRORS=0

# Verify minisign signatures
echo ""
echo "=== Minisign Verification ==="

if [ -f "sysprims-minisign.pub" ]; then
  for manifest in SHA256SUMS SHA512SUMS; do
    if [ -f "$manifest" ] && [ -f "${manifest}.minisig" ]; then
      echo "Verifying $manifest..."
      if minisign -Vm "$manifest" -p sysprims-minisign.pub; then
        echo "[ok] $manifest signature valid"
      else
        echo "[!!] $manifest signature INVALID"
        ERRORS=$((ERRORS + 1))
      fi
    elif [ -f "$manifest" ]; then
      echo "[!!] Missing signature: ${manifest}.minisig"
      ERRORS=$((ERRORS + 1))
    fi
  done
else
  echo "[!!] sysprims-minisign.pub not found - cannot verify minisign signatures"
  ERRORS=$((ERRORS + 1))
fi

# Verify PGP signatures
echo ""
echo "=== PGP Verification ==="

if [ -f "sysprims-release-signing-key.asc" ]; then
  # Import the key temporarily
  GNUPGHOME=$(mktemp -d)
  export GNUPGHOME
  trap 'rm -rf "$GNUPGHOME"' EXIT

  gpg --import sysprims-release-signing-key.asc 2>/dev/null

  for manifest in SHA256SUMS SHA512SUMS; do
    if [ -f "$manifest" ] && [ -f "${manifest}.asc" ]; then
      echo "Verifying $manifest PGP signature..."
      if gpg --verify "${manifest}.asc" "$manifest" 2>/dev/null; then
        echo "[ok] $manifest PGP signature valid"
      else
        echo "[!!] $manifest PGP signature INVALID"
        ERRORS=$((ERRORS + 1))
      fi
    elif [ -f "${manifest}.asc" ]; then
      echo "[!!] ${manifest}.asc exists but $manifest not found"
      ERRORS=$((ERRORS + 1))
    fi
  done
else
  echo "[--] No PGP key found - skipping PGP verification"
fi

echo ""
if [ $ERRORS -eq 0 ]; then
  echo "[ok] All signatures verified"
  exit 0
else
  echo "[!!] $ERRORS signature verification errors"
  exit 1
fi
