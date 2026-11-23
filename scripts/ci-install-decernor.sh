#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Install decernor for CI and registry-publish jobs: download the linux-amd64
# release archive named in config/release/decernor-ci-sha256.txt, check its
# SHA-256 against that file, extract only the decernor binary into a new
# directory under RUNNER_TEMP, and export its absolute path as
# SYSPRIMS_DECERNOR_BIN through GITHUB_ENV. Nothing else is fetched, built or
# installed, and nothing is put on PATH. CI only: maintainers set
# SYSPRIMS_DECERNOR_BIN themselves.
# Usage: ci-install-decernor.sh
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
config="$root/config/release/decernor-ci-sha256.txt"
die() {
	echo "error: $*" >&2
	exit 1
}
[[ $# -eq 0 ]] || die 'usage: ci-install-decernor.sh'
[[ -n "${RUNNER_TEMP:-}" && -n "${GITHUB_ENV:-}" ]] || die 'RUNNER_TEMP and GITHUB_ENV are required (CI only)'
[[ "${RUNNER_OS:-}/${RUNNER_ARCH:-}" == Linux/X64 ]] || die "no pinned decernor archive for runner ${RUNNER_OS:-?}/${RUNNER_ARCH:-?}"
[[ -f "$config" && ! -L "$config" ]] || die "$config is required"
[[ "$(grep -c . "$config")" == 1 ]] || die "exactly one pin line is required in $config"
read -r want archive extra <"$config" || die "unreadable pin line in $config"
[[ -z "${extra:-}" ]] || die "malformed pin line in $config"
[[ "$want" =~ ^[0-9a-f]{64}$ ]] || die "malformed sha256 in $config"
[[ "$archive" =~ ^decernor_([0-9]+\.[0-9]+\.[0-9]+)_linux_amd64\.tar\.gz$ ]] || die "malformed archive name in $config: $archive"
version="${BASH_REMATCH[1]}"
dir="$(mktemp -d "$RUNNER_TEMP/decernor.XXXXXX")"
curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location --retry 3 \
	--output "$dir/$archive" "https://github.com/3leaps/decernor/releases/download/v$version/$archive"
got="$(sha256sum "$dir/$archive" | cut -c1-64)"
[[ "$got" == "$want" ]] || die "$archive checksum $got != $want"
# Exactly one member named decernor, and it is a regular file (not a link,
# hardlink or directory), before anything is extracted.
[[ "$(tar -tzf "$dir/$archive" | grep -cx 'decernor')" == 1 ]] || die "$archive must hold exactly one decernor member"
[[ "$(tar -tvzf "$dir/$archive" decernor | cut -c1)" == - ]] || die "$archive member decernor is not a regular file"
tar -xzf "$dir/$archive" -C "$dir" decernor
rm -f "$dir/$archive"
bin="$dir/decernor"
[[ -f "$bin" && ! -L "$bin" && -x "$bin" ]] || die "$archive did not yield an executable decernor binary"
[[ "$("$bin" version)" == "decernor $version" ]] || die "extracted binary does not identify as decernor $version"
echo "SYSPRIMS_DECERNOR_BIN=$bin" >>"$GITHUB_ENV"
echo "decernor $version installed at $bin"
