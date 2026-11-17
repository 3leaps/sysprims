#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Install the yamlprims CLI for the CI workflow checkers: download this
# runner's release archive named in config/yamlprims-cli.txt, check its SHA-256
# against that file, extract only the binary into a new directory under
# RUNNER_TEMP and check its own committed SHA-256, check it again with
# scripts/yaml_json.py (the checkers' reader), and export
# SYSPRIMS_YAMLPRIMS_BIN through GITHUB_ENV. Nothing else is fetched, built
# or installed. CI only: maintainers set SYSPRIMS_YAMLPRIMS_BIN themselves.
# Usage: ci-install-yamlprims.sh <python>
set -euo pipefail
python="${1:?python interpreter required}"
root="$(cd "$(dirname "$0")/.." && pwd)"
config="$root/config/yamlprims-cli.txt"
die() {
	echo "error: $*" >&2
	exit 1
}
[[ -n "${RUNNER_TEMP:-}" && -n "${GITHUB_ENV:-}" ]] || die 'RUNNER_TEMP and GITHUB_ENV are required (CI only)'
case "${RUNNER_OS:-}/${RUNNER_ARCH:-}" in
Linux/X64) platform=linux-amd64 ext=tar.gz member=./yamlprims ;;
Linux/ARM64) platform=linux-arm64 ext=tar.gz member=./yamlprims ;;
macOS/ARM64) platform=darwin-arm64 ext=tar.gz member=./yamlprims ;;
Windows/X64) platform=windows-amd64 ext=zip member=yamlprims.exe ;;
Windows/ARM64) platform=windows-arm64 ext=zip member=yamlprims.exe ;;
*) die "no yamlprims archive for runner ${RUNNER_OS:-?}/${RUNNER_ARCH:-?}" ;;
esac
version="$(sed -n 's/^release \([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)$/\1/p' "$config")"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "exactly one release line is required in $config"
archive="yamlprims-$version-$platform.$ext"
want="$(awk -v a="$archive" 'NF == 2 && $2 == a {print $1}' "$config")"
[[ "$want" =~ ^[0-9a-f]{64}$ ]] || die "exactly one sha256 for $archive is required in $config"
tmp="$(cygpath -u "$RUNNER_TEMP" 2>/dev/null || echo "$RUNNER_TEMP")"
dir="$(mktemp -d "$tmp/yamlprims.XXXXXX")"
curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location --retry 3 \
	--output "$dir/$archive" "https://github.com/3leaps/yamlprims/releases/download/v$version/$archive"
if command -v sha256sum >/dev/null; then got="$(sha256sum "$dir/$archive" | cut -c1-64)"; else got="$(shasum -a 256 "$dir/$archive" | cut -c1-64)"; fi
[[ "$got" == "$want" ]] || die "$archive checksum $got != $want"
want_bin="$(awk -v p="$platform" 'NF == 3 && $1 == "binary" && $2 == p {print $3}' "$config")"
[[ "$want_bin" =~ ^[0-9a-f]{64}$ ]] || die "exactly one binary sha256 for $platform is required in $config"
# The interpreter opens the archive itself, so its own (possibly Windows)
# path form is used. Only the binary is extracted, and only if its digest is
# the committed one.
native() { cygpath -w "$1" 2>/dev/null || echo "$1"; }
bin="$(native "$dir/${member#./}")"
"$python" -I -B "$(native "$root/scripts/ci-extract-yamlprims.py")" "$(native "$dir/$archive")" "$member" "$bin" "$want_bin"
rm -f "$dir/$archive"
SYSPRIMS_YAMLPRIMS_BIN="$bin" "$python" -I -B "$(native "$root/scripts/yaml_json.py")" identify >/dev/null
SYSPRIMS_YAMLPRIMS_BIN="$bin" "$python" -I -B "$(native "$root/scripts/yaml_json.py")" version
echo "SYSPRIMS_YAMLPRIMS_BIN=$bin" >>"$GITHUB_ENV"
