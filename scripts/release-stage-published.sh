#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Download the published GitHub Release for SYSPRIMS_RELEASE_TAG into
# dist/published and verify it against the signed tag and the pins committed in
# that tag before exposing the directory.
#
# Usage: release-stage-published.sh [dist-dir]
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
# shellcheck source=scripts/release-tag-common.sh
source "$root/scripts/release-tag-common.sh"
# shellcheck source=scripts/release-decernor.sh
source "$root/scripts/release-decernor.sh"
die() {
	echo "error: $*" >&2
	exit 1
}
[[ $# -le 1 ]] || die 'usage: release-stage-published.sh [dist-dir]'
tag_version
tag="$SYSPRIMS_RELEASE_TAG"
out="${1:-$root/dist/published}"
anchor="${out%/}.anchor"
[[ ! -e "$out" && ! -L "$out" ]] || die "$out already exists; run make release-clean first"
[[ ! -e "$anchor" && ! -L "$anchor" ]] || die "$anchor already exists; run make release-clean first"

resolve_release_decernor
RELEASE_DECERNOR_BIN="$RELEASE_DECERNOR_BIN" "$root/scripts/verify-pinned-tag.sh" --published >/dev/null
tag_object="$(git rev-parse --verify "refs/tags/$tag")"
commit="$(git rev-parse "${tag_object}^{commit}")"
version="$(git cat-file blob "$commit:VERSION" | tr -d ' \t\r\n')"
[[ "v$version" == "$tag" ]] || die "tagged VERSION ($version) does not match $tag"

work="$(mktemp -d "${TMPDIR:-/tmp}/sysprims-published.XXXXXX")"
trap 'rm -rf "$work"' EXIT
chmod 700 "$work"
mkdir -m 700 "$work/assets"

if [[ -n "${SYSPRIMS_PUBLISHED_RELEASE_METADATA_FILE:-}" ]]; then
	cp "$SYSPRIMS_PUBLISHED_RELEASE_METADATA_FILE" "$work/release.json"
else
	gh api "repos/3leaps/sysprims/releases/tags/$tag" >"$work/release.json"
fi

if [[ -n "${SYSPRIMS_REMOTE_ASSET_DIR:-}" ]]; then
	cp "$SYSPRIMS_REMOTE_ASSET_DIR"/* "$work/assets/"
else
	gh release download "$tag" --repo 3leaps/sysprims --dir "$work/assets"
fi

node --input-type=module - "$root" "$work/assets" "$work/release.json" "$version" "$commit" <<'EOF'
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { lstatSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { assertInventory, expectedInventory, verifyChecksums, verifyTaggedAssets } from "./scripts/release-integrity.mjs";

const [, , root, assetsDir, releaseJsonPath, version, commit] = process.argv;
const tag = `v${version}`;
const fail = (msg) => { console.error(`error: ${msg}`); process.exit(1); };
const sha256 = (p) => createHash("sha256").update(readFileSync(p)).digest("hex");

const release = JSON.parse(readFileSync(releaseJsonPath, "utf8"));
if (release.tag_name !== tag) fail(`published release tag_name (${release.tag_name}) does not match ${tag}`);
if (release.target_commitish !== commit) fail(`published release target_commitish (${release.target_commitish}) does not match tagged commit ${commit}`);
if (release.draft !== false) fail(`release ${tag} is still a draft; run make release-publish first`);
if (release.immutable !== true) fail(`published release ${tag} is not immutable (immutable=${release.immutable})`);

for (const entry of readdirSync(assetsDir)) {
  const st = lstatSync(join(assetsDir, entry));
  if (st.isSymbolicLink() || !st.isFile()) fail(`downloaded asset is not a regular file: ${entry}`);
}

const { base, signed } = expectedInventory(version);
assertInventory(assetsDir, signed, "published release");

const byName = new Map((release.assets ?? []).map((a) => [a.name, a]));
if (byName.size !== signed.length) fail("published release asset count mismatch");
for (const name of signed) {
  const asset = byName.get(name);
  if (!asset) fail(`published release metadata missing asset: ${name}`);
  const digest = asset.digest?.replace(/^sha256:/, "");
  if (!/^[0-9a-f]{64}$/.test(digest ?? "")) fail(`published release asset has no SHA-256 digest: ${name}`);
  if (digest !== sha256(join(assetsDir, name))) fail(`published release digest mismatch for ${name}`);
}

verifyTaggedAssets(root, assetsDir, version);
for (const [blobPath, assetName] of [
  ["docs/security/release-signing-keys.asc", "sysprims-release-signing-key.asc"],
  ["docs/security/sysprims-minisign.pub", "sysprims-minisign.pub"],
]) {
  const blob = spawnSync("git", ["cat-file", "blob", `${commit}:${blobPath}`], { cwd: root }).stdout;
  if (!blob?.length) fail(`committed ${blobPath} is empty in ${commit}`);
  if (!readFileSync(join(assetsDir, assetName)).equals(blob)) {
    fail(`${assetName} in the published release differs from ${commit}:${blobPath}`);
  }
}
verifyChecksums(assetsDir, base);
EOF

"$root/scripts/verify-public-keys.sh" "$work/assets" >/dev/null
"$root/scripts/verify-signatures.sh" "$work/assets" >/dev/null

{
	printf 'tag=%s\nobject=%s\ncommit=%s\n' "$tag" "$tag_object" "$commit"
	node --input-type=module - "$work/assets" "$version" <<'EOF'
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { expectedInventory } from "./scripts/release-integrity.mjs";
const [, , assetsDir, version] = process.argv;
for (const name of expectedInventory(version).base) {
  const digest = createHash("sha256").update(readFileSync(join(assetsDir, name))).digest("hex");
  console.log(`file=${digest} ${name}`);
}
EOF
} >"$work/anchor"

mkdir -p "$(dirname "$out")"
mv "$work/assets" "$out"
"$root/scripts/write-exclusive.py" "$work/anchor" "$anchor"
echo "[ok] verified published release $tag staged in $out ($anchor)"
