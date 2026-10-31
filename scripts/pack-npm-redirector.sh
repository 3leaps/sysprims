#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Build the bare-name npm redirector tarball (`sysprims-<version>.tgz`) that
# re-exports `@3leaps/sysprims` at the exact same version so `npm install sysprims`
# resolves to the scoped package and reserves the unscoped name against squatting.
#
# Usage: pack-npm-redirector.sh [out-dir] [version]
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
out_dir="${1:-$root/dist/npm-redirector}"
version="${2:-$(tr -d ' \t\r\n' <"$root/VERSION")}"

die() {
	echo "error: $*" >&2
	exit 1
}

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version ($version) must be X.Y.Z"
command -v npm >/dev/null 2>&1 || die "npm is required to pack the redirector package"

work="$(mktemp -d "${TMPDIR:-/tmp}/sysprims-npm-redirector.XXXXXX")"
trap 'rm -rf "$work"' EXIT
chmod 700 "$work"

cat >"$work/package.json" <<EOF
{
  "name": "sysprims",
  "version": "${version}",
  "description": "GPL-free cross-platform process utilities library — forwards to @3leaps/sysprims",
  "license": "MIT OR Apache-2.0",
  "repository": {
    "type": "git",
    "url": "git+https://github.com/3leaps/sysprims.git"
  },
  "homepage": "https://github.com/3leaps/sysprims",
  "bugs": {
    "url": "https://github.com/3leaps/sysprims/issues"
  },
  "type": "module",
  "main": "./index.cjs",
  "module": "./index.js",
  "types": "./index.d.ts",
  "exports": {
    ".": {
      "types": "./index.d.ts",
      "import": "./index.js",
      "require": "./index.cjs"
    }
  },
  "files": [
    "index.js",
    "index.cjs",
    "index.d.ts",
    "README.md"
  ],
  "dependencies": {
    "@3leaps/sysprims": "${version}"
  },
  "engines": {
    "node": ">=18"
  }
}
EOF

cat >"$work/index.js" <<'EOF'
export * from "@3leaps/sysprims";
EOF

cat >"$work/index.cjs" <<'EOF'
"use strict";
module.exports = require("@3leaps/sysprims");
EOF

cat >"$work/index.d.ts" <<'EOF'
export * from "@3leaps/sysprims";
EOF

cat >"$work/README.md" <<'EOF'
# sysprims

This package forwards to [`@3leaps/sysprims`](https://www.npmjs.com/package/@3leaps/sysprims), the official scoped package for 3 Leaps `sysprims` (GPL-free cross-platform process utilities, signal dispatch, process groups, and timeout supervision).

For new projects, depend on `@3leaps/sysprims` directly:
EOF
printf '\n```bash\n%s %s @3leaps/sysprims\n```\n' npm install >>"$work/README.md"

cp "$root/LICENSE-MIT" "$work/LICENSE-MIT"
cp "$root/LICENSE-APACHE" "$work/LICENSE-APACHE"

mkdir -p "$out_dir"
out_abs="$(cd "$out_dir" && pwd -P)"
(
	cd "$work"
	npm pack --ignore-scripts --pack-destination "$out_abs" >/dev/null
)

tgz="$out_abs/sysprims-${version}.tgz"
[[ -s "$tgz" ]] || die "expected $tgz was not produced"
echo "[ok] packed bare-name redirector $tgz (-> @3leaps/sysprims@${version})"
