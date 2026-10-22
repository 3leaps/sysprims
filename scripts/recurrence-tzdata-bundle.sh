#!/usr/bin/env bash
# Build the pinned tzdb bundle for the recurrence crate and its Go twin.
#
# Maintainer step, never part of a build: it downloads the pinned IANA
# tzdata/tzcode releases, verifies both against SHA-256 pins, compiles zic
# from tzcode (the host zic is never used), compiles every roster zone with
# `zic -b fat`, and writes one container file plus its SHA-256 pin to both
# the Rust crate and the Go module. Builds read only the committed bundle.
#
# Usage: scripts/recurrence-tzdata-bundle.sh [--check]
#   --check  rebuild in a scratch dir and fail unless the committed bundle
#            and pin are byte-identical to the rebuild.
set -euo pipefail

TZ_VERSION="2026e"
TZDATA_SHA256="b26882805f26aac59d5b222978e6580484b834ccdc98be89df2f05a6dc53a652"
TZCODE_SHA256="cc3d27ca2a0d8399504551b920970d80af83bfb9c216e8082a15491921935d54"
BASE_URL="https://data.iana.org/time-zones/releases"
ZONE_FILES=(africa antarctica asia australasia europe northamerica southamerica etcetera)

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUNDLE_NAME="tzif-bundle-${TZ_VERSION}.bin"
DESTS=("${ROOT}/crates/sysprims-recurrence/tzdata" "${ROOT}/go/recurrence/tzdata")

check=0
if [[ "${1:-}" == "--check" ]]; then check=1; elif [[ $# -gt 0 ]]; then
  echo "usage: $0 [--check]" >&2; exit 2
fi

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

work="$(mktemp -d "${TMPDIR:-/tmp}/sysprims-tzbundle.XXXXXX")"
trap 'rm -rf "$work"' EXIT

for kind in tzdata tzcode; do
  file="${kind}${TZ_VERSION}.tar.gz"
  curl -fsSL --proto '=https' --tlsv1.2 -o "${work}/${file}" "${BASE_URL}/${file}"
  want="TZDATA_SHA256"; [[ "$kind" == tzcode ]] && want="TZCODE_SHA256"
  got="$(sha256 "${work}/${file}")"
  if [[ "$got" != "${!want}" ]]; then
    echo "error: ${file} sha256 ${got} does not match pin ${!want}" >&2; exit 1
  fi
  mkdir -p "${work}/src"
  tar -xzf "${work}/${file}" -C "${work}/src"
done

(cd "${work}/src" && make -s zic >/dev/null)
"${work}/src/zic" --version | grep -q "${TZ_VERSION}" || {
  echo "error: built zic does not report ${TZ_VERSION}" >&2; exit 1; }

# Accepted roster (contract A5): Zone names in the eight source files plus
# zone.tab locations. Names only in backward/backzone/factory are excluded.
(cd "${work}/src" && {
  awk '$1 == "Zone" { print $2 }' "${ZONE_FILES[@]}"
  awk '!/^#/ && NF >= 3 { print $3 }' zone.tab
} | LC_ALL=C sort -u > "${work}/roster.txt")

# backward is compiled so zone.tab locations that are Links (Europe/Oslo)
# have TZif data under their own name; the roster still decides acceptance.
mkdir -p "${work}/out"
(cd "${work}/src" && ./zic -b fat -d "${work}/out" "${ZONE_FILES[@]}" backward)

# Container: a text header, then the concatenated TZif blobs.
#   SYSPRIMS-TZIF-BUNDLE 1
#   tzdb <version>
#   zones <n>
#   <name> <offset> <length>      (n lines, roster order; offset from data start)
#   <empty line>
#   <data>
python3 - "${work}" "${TZ_VERSION}" <<'PY'
import pathlib, sys
work, version = pathlib.Path(sys.argv[1]), sys.argv[2]
names = (work / "roster.txt").read_text().split()
blobs, lines, offset = [], [], 0
for name in names:
    data = (work / "out" / name).read_bytes()
    if not data.startswith(b"TZif"):
        sys.exit(f"error: {name} is not TZif")
    lines.append(f"{name} {offset} {len(data)}")
    blobs.append(data)
    offset += len(data)
header = "SYSPRIMS-TZIF-BUNDLE 1\ntzdb %s\nzones %d\n%s\n\n" % (version, len(names), "\n".join(lines))
(work / "bundle.bin").write_bytes(header.encode("ascii") + b"".join(blobs))
PY

digest="$(sha256 "${work}/bundle.bin")"
printf '%s  %s\n' "$digest" "$BUNDLE_NAME" > "${work}/bundle.sha256"

status=0
for dest in "${DESTS[@]}"; do
  if [[ $check -eq 1 ]]; then
    for f in "$BUNDLE_NAME:bundle.bin" "${BUNDLE_NAME%.bin}.sha256:bundle.sha256"; do
      if ! cmp -s "${dest}/${f%%:*}" "${work}/${f#*:}"; then
        echo "error: ${dest}/${f%%:*} differs from the pinned rebuild" >&2; status=1
      fi
    done
  else
    mkdir -p "$dest"
    cp "${work}/bundle.bin" "${dest}/${BUNDLE_NAME}"
    cp "${work}/bundle.sha256" "${dest}/${BUNDLE_NAME%.bin}.sha256"
  fi
done
[[ $status -eq 0 ]] && echo "[ok] ${BUNDLE_NAME} sha256:${digest} ($(wc -l < "${work}/roster.txt" | tr -d ' ') zones)"
exit $status
