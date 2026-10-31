#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# release-stage-published.sh stages only an undrafted, immutable GitHub release
# whose tag set, commit, asset inventory, digests, checksums, public keys,
# anchors and signatures all verify against the tagged commit.
set -euo pipefail
# shellcheck source=scripts/release-test-fixture.sh
source "$(dirname "$0")/release-test-fixture.sh"
decernor="$(fixture_decernor)"
version=1.2.3
repo="$fixture_dir/repo"
messages="$fixture_dir/messages/v$version"
read -r _ subkey < <(fixture_keys "$fixture_dir/gpg")

fixture_repo "$repo" "$version" publish 1
cp "$fixture_root/Makefile" "$repo/Makefile"
cp "$fixture_root/scripts/"{release-stage-published.sh,release-integrity.mjs,version-pack.mjs,verify-signatures.sh,sign-release-assets.sh,generate-checksums.sh} "$repo/scripts/"
printf '#!/usr/bin/env bash\nexit 0\n' >"$repo/scripts/release-guard-tag-version.sh"
chmod +x "$repo/scripts/release-guard-tag-version.sh"
printf 'notes\n' >"$repo/docs/releases/v$version.md"
gpg --homedir "$fixture_dir/gpg" --batch --armor --export-options export-minimal --export "$subkey!" \
	>"$repo/docs/security/release-signing-keys.asc"
minisign -G -W -p "$repo/docs/security/sysprims-minisign.pub" -s "$fixture_dir/minisign.key" >/dev/null 2>&1
printf '%s\n' "$subkey" >"$repo/config/release/signing-subkey.txt"
fixture_commit "$repo" pins
SYSPRIMS_DECERNOR_BIN="$decernor" "$repo/scripts/release-insert-anchors.sh" >/dev/null
fixture_commit "$repo" anchors

tags=("v$version" "bindings/go/sysprims/v$version" "go/recurrence/v$version")
fixture_messages "$messages" "$version" "${tags[@]}"
for tag in "${tags[@]}"; do
	GNUPGHOME="$fixture_dir/gpg" GIT_COMMITTER_NAME='3 Leaps Infosec Team' GIT_COMMITTER_EMAIL=infosec@3leaps.net \
		git -C "$repo" tag -s -a --cleanup=verbatim -u "$subkey!" -F "$messages/${tag//\//__}.txt" "$tag" HEAD
done
commit="$(git -C "$repo" rev-parse "v$version^{commit}")"
tag_object="$(git -C "$repo" rev-parse "refs/tags/v$version")"

remote="$fixture_dir/remote"
mkdir -p "$remote"
node --input-type=module - "$remote" "$version" <<'EOF'
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { expectedInventory } from "./scripts/release-integrity.mjs";
const [, , dir, version] = process.argv;
for (const name of expectedInventory(version).base) {
  if (!name.startsWith("release-notes-") && !name.startsWith("expected-fingerprints.")) {
    writeFileSync(join(dir, name), `${name}\n`);
  }
}
EOF
cp "$repo/docs/releases/v$version.md" "$remote/release-notes-v$version.md"
(cd "$repo" && SYSPRIMS_RELEASE_TAG="v$version" ./scripts/release-stage-public.sh --anchors "$remote" >/dev/null)
(cd "$repo" && ./scripts/generate-checksums.sh "$remote" "v$version" >/dev/null)
(cd "$repo" && SYSPRIMS_MINISIGN_KEY="$fixture_dir/minisign.key" SYSPRIMS_PGP_KEY_ID="$subkey!" \
	SYSPRIMS_GPG_HOMEDIR="$fixture_dir/gpg" ./scripts/sign-release-assets.sh "v$version" "$remote" >/dev/null)
(cd "$repo" && SYSPRIMS_RELEASE_TAG="v$version" SYSPRIMS_DECERNOR_BIN="$decernor" ./scripts/release-stage-public.sh "$remote" >/dev/null)

meta="$fixture_dir/release.json"
write_meta() {
	node --input-type=module - "$remote" "$meta" "$version" "$commit" "${1:-false}" "${2:-true}" <<'EOF'
import { createHash } from "node:crypto";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
const [, , dir, out, version, commit, draft, immutable] = process.argv;
const assets = readdirSync(dir).sort().map((name) => ({
  name,
  digest: `sha256:${createHash("sha256").update(readFileSync(join(dir, name))).digest("hex")}`,
}));
writeFileSync(out, JSON.stringify({
  tag_name: `v${version}`,
  target_commitish: commit,
  draft: draft === "true",
  immutable: immutable === "true",
  assets,
}));
EOF
}

stage() {
	rm -rf "$repo/dist/published" "$repo/dist/published.anchor"
	(cd "$repo" && SYSPRIMS_RELEASE_TAG="v$version" SYSPRIMS_DECERNOR_BIN="$decernor" \
		SYSPRIMS_PUBLISHED_RELEASE_METADATA_FILE="$meta" SYSPRIMS_REMOTE_ASSET_DIR="$remote" \
		./scripts/release-stage-published.sh "$repo/dist/published")
}

# Draft or mutable release is refused.
write_meta true true
expect_fail_with 'still a draft' stage
[[ ! -e "$repo/dist/published" && ! -e "$repo/dist/published.anchor" ]]
write_meta false false
expect_fail_with 'is not immutable' stage
[[ ! -e "$repo/dist/published" && ! -e "$repo/dist/published.anchor" ]]

# Wrong target_commitish is refused.
write_meta false true
node -e 'const fs=require("fs"); const d=JSON.parse(fs.readFileSync(process.argv[1])); d.target_commitish="0".repeat(40); fs.writeFileSync(process.argv[1], JSON.stringify(d));' "$meta"
expect_fail_with 'target_commitish .* does not match' stage

# Tampered asset digest is refused.
write_meta false true
node -e 'const fs=require("fs"); const d=JSON.parse(fs.readFileSync(process.argv[1])); d.assets[0].digest="sha256:"+"0".repeat(64); fs.writeFileSync(process.argv[1], JSON.stringify(d));' "$meta"
expect_fail_with 'digest mismatch' stage

# Valid immutable published release stages cleanly and writes the anchor.
write_meta false true
stage >/dev/null
[[ -d "$repo/dist/published" && -f "$repo/dist/published.anchor" ]]
grep -qx "tag=v$version" "$repo/dist/published.anchor"
grep -qx "object=$tag_object" "$repo/dist/published.anchor"
grep -qx "commit=$commit" "$repo/dist/published.anchor"

# Refuses to overwrite an existing directory or anchor when invoked directly.
expect_fail_with 'already exists' bash -c "cd '$repo' && SYSPRIMS_RELEASE_TAG='v$version' SYSPRIMS_DECERNOR_BIN='$decernor' ./scripts/release-stage-published.sh '$repo/dist/published'"

# Driving the Makefile target (make release-stage-published) cleans prior dist/published and stages with exact Makefile arguments.
(cd "$repo" && env -u RELEASE_TAG SYSPRIMS_RELEASE_TAG="v$version" SYSPRIMS_DECERNOR_BIN="$decernor" \
	SYSPRIMS_PUBLISHED_RELEASE_METADATA_FILE="$meta" SYSPRIMS_REMOTE_ASSET_DIR="$remote" \
	make -s release-stage-published >/dev/null)
[[ -d "$repo/dist/published" && -f "$repo/dist/published.anchor" ]]
grep -qx "tag=v$version" "$repo/dist/published.anchor"
grep -qx "object=$tag_object" "$repo/dist/published.anchor"
grep -qx "commit=$commit" "$repo/dist/published.anchor"
echo '[ok] release-stage-published controls passed'
