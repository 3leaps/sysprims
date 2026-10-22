#!/usr/bin/env bash
# Vendor the recurrence/v0 schema family and conformance corpus from a crucible ref.
#
# Usage: scripts/sync-recurrence-corpus.sh <crucible-ref> [--check]
#   CRUCIBLE_DIR  crucible checkout (default: ../crucible next to this repo)
#
# Copies schemas/recurrence/v0 at <ref> via `git archive` (never the working
# tree), verifies MANIFEST.json against the expected corpus digest and every
# corpus file against its MANIFEST hash, and records the pin. It never edits
# vectors. --check fails unless the vendored tree already equals <ref>.
set -euo pipefail

EXPECTED_DIGEST="sha256:69387023c21ca147f2dd59c4e47f48447f5eb3ad77121975aab0e4fe0680b35b"
EXPECTED_TZDB="2026e"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${ROOT}/crates/sysprims-recurrence/tests/recurrence-v0"
CRUCIBLE_DIR="${CRUCIBLE_DIR:-${ROOT}/../crucible}"

ref="${1:-}"; mode="${2:-}"
if [[ -z "$ref" || ( -n "$mode" && "$mode" != "--check" ) ]]; then
  echo "usage: $0 <crucible-ref> [--check]" >&2; exit 2
fi

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

commit="$(git -C "$CRUCIBLE_DIR" rev-parse --verify "${ref}^{commit}")"
work="$(mktemp -d "${TMPDIR:-/tmp}/sysprims-recurrence-corpus.XXXXXX")"
trap 'rm -rf "$work"' EXIT

git -C "$CRUCIBLE_DIR" archive --format=tar "$commit" schemas/recurrence/v0 | tar -xf - -C "$work"
src="${work}/schemas/recurrence/v0"

digest="sha256:$(sha256 "${src}/corpus/MANIFEST.json")"
if [[ "$digest" != "$EXPECTED_DIGEST" ]]; then
  echo "error: corpus digest ${digest} at ${ref} is not the pinned ${EXPECTED_DIGEST}" >&2
  exit 1
fi

python3 - "$src" "$EXPECTED_TZDB" <<'PY'
import hashlib, json, pathlib, sys
src, tzdb = pathlib.Path(sys.argv[1]), sys.argv[2]
manifest = json.loads((src / "corpus/MANIFEST.json").read_text())
if manifest.get("tzdb_version") != tzdb:
    sys.exit(f"error: manifest tzdb {manifest.get('tzdb_version')} is not {tzdb}")
listed = set(manifest["files"])
present = {str(p.relative_to(src / "corpus")) for p in (src / "corpus").rglob("*")
           if p.is_file() and p.name != "MANIFEST.json"}
if listed != present:
    sys.exit(f"error: manifest/file set mismatch: {sorted(listed ^ present)}")
for rel, want in manifest["files"].items():
    got = "sha256:" + hashlib.sha256((src / "corpus" / rel).read_bytes()).hexdigest()
    if got != want:
        sys.exit(f"error: {rel} hash {got} != manifest {want}")
PY

cat > "${src}/PIN" <<PIN
crucible_ref ${ref}
crucible_commit ${commit}
corpus_digest ${EXPECTED_DIGEST}
tzdb_version ${EXPECTED_TZDB}
PIN

if [[ "$mode" == "--check" ]]; then
  # The ref spelling is recorded for humans; equality is judged on content.
  if ! diff -r -x PIN "$src" "$DEST" >/dev/null || \
     ! grep -qx "corpus_digest ${EXPECTED_DIGEST}" "${DEST}/PIN"; then
    echo "error: vendored corpus differs from ${ref} (${commit})" >&2
    diff -r -x PIN "$src" "$DEST" >&2 || true
    exit 1
  fi
  echo "[ok] vendored recurrence/v0 equals ${ref} (${commit})"
  exit 0
fi

rm -rf "$DEST"
mkdir -p "$(dirname "$DEST")"
cp -R "$src" "$DEST"
echo "[ok] vendored recurrence/v0 from ${ref} (${commit}) ${EXPECTED_DIGEST}"
