#!/usr/bin/env bash
# The fingerprint anchors are release assets: checksums refuse to run without
# them, and fetching them from a published release verifies the signed
# manifests and the tagged commit's blobs, writes only into a new directory,
# and never into keys/. Each refusal below is built so that exactly one check
# can catch it.
set -euo pipefail
# shellcheck source=scripts/release-test-fixture.sh
source "$(dirname "$0")/release-test-fixture.sh"
version=1.2.3
tag="v$version"
repo="$fixture_dir/repo"
read -r _ subkey < <(fixture_keys "$fixture_dir/gpg")
read -r _ other_subkey < <(fixture_keys "$fixture_dir/gpg-other")
minisign -G -W -p "$fixture_dir/pinned.pub" -s "$fixture_dir/pinned.key" >/dev/null
minisign -G -W -p "$fixture_dir/other.pub" -s "$fixture_dir/other.key" >/dev/null

fixture_repo "$repo" "$version" publish 0
cp "$fixture_root/scripts/"{verify-signatures.sh,release-fetch-anchors.sh,generate-checksums.sh} "$repo/scripts/"
gpg --homedir "$fixture_dir/gpg" --batch --armor --export-options export-minimal --export "$subkey!" \
	>"$repo/docs/security/release-signing-keys.asc"
cp "$fixture_dir/pinned.pub" "$repo/docs/security/sysprims-minisign.pub"
printf '%s\n' "$subkey" >"$repo/config/release/signing-subkey.txt"
printf 'gpg %s\nminisign %s\n' "$subkey" "$(printf 'a%.0s' $(seq 64))" >"$repo/keys/expected-fingerprints.txt"
printf '{"kind":"gpg"}\n{"kind":"minisign"}\n' >"$repo/keys/expected-fingerprints.ndjson"
fixture_commit "$repo" pins-and-anchors
git -C "$repo" tag -a -m "Release $version" "$tag"
tag_blob() { git -C "$repo" cat-file blob "$tag:$1"; }
keys_before="$(cd "$repo/keys" && shasum ./*)"

# Checksums refuse to run unless the notes and both anchors are staged.
payload="$fixture_dir/payload"
mkdir "$payload"
for name in LICENSE-APACHE LICENSE-MIT "release-notes-$tag.md" "sbom-$version.cdx.json" \
	"sysprims-$version-"{darwin-amd64,darwin-arm64,linux-amd64-musl,linux-amd64,linux-arm64-musl,linux-arm64}.tar.gz \
	"sysprims-$version-"{windows-amd64,windows-arm64}.zip "sysprims-ffi-$version-libs.tar.gz" sysprims.h; do
	printf '%s\n' "$name" >"$payload/$name"
done
for missing in expected-fingerprints.txt expected-fingerprints.ndjson; do
	for name in expected-fingerprints.txt expected-fingerprints.ndjson; do tag_blob "keys/$name" >"$payload/$name"; done
	rm "$payload/$missing"
	expect_fail_with "$missing not in " "$repo/scripts/generate-checksums.sh" "$payload" "$tag"
	grep -q 'make release-stage-anchors' "$fixture_dir/last.out"
	[[ ! -e "$payload/SHA256SUMS" ]]
done
# A symlinked anchor is not staged either.
tag_blob keys/expected-fingerprints.txt >"$fixture_dir/anchor-target"
tag_blob keys/expected-fingerprints.ndjson >"$payload/expected-fingerprints.ndjson"
rm -f "$payload/expected-fingerprints.txt"
ln -s "$fixture_dir/anchor-target" "$payload/expected-fingerprints.txt"
expect_fail_with 'expected-fingerprints.txt not in' "$repo/scripts/generate-checksums.sh" "$payload" "$tag"
rm "$payload/expected-fingerprints.txt"
tag_blob keys/expected-fingerprints.txt >"$payload/expected-fingerprints.txt"
"$repo/scripts/generate-checksums.sh" "$payload" "$tag" >/dev/null
for name in expected-fingerprints.txt expected-fingerprints.ndjson; do
	grep -q "  $name\$" "$payload/SHA256SUMS" && grep -q "  $name\$" "$payload/SHA512SUMS"
done

# publish <dir> [txt-bytes-file] [minisign-key gpg-home subkey]: a published
# release: the anchors (the tag's, or the given bytes), manifests over them,
# signed (by the pinned keys unless others are given), and the staged keys.
publish() {
	local dir="$1" txt="${2:-}" mkey="${3:-$fixture_dir/pinned.key}" home="${4:-$fixture_dir/gpg}" signer="${5:-$subkey}"
	rm -rf "$dir"
	mkdir -p "$dir"
	if [[ -n "$txt" ]]; then cp "$txt" "$dir/expected-fingerprints.txt"; else tag_blob keys/expected-fingerprints.txt >"$dir/expected-fingerprints.txt"; fi
	tag_blob keys/expected-fingerprints.ndjson >"$dir/expected-fingerprints.ndjson"
	(cd "$dir" && shasum -a 256 expected-fingerprints.* >SHA256SUMS && shasum -a 512 expected-fingerprints.* >SHA512SUMS)
	sign "$dir" "$mkey" "$home" "$signer"
	tag_blob docs/security/sysprims-minisign.pub >"$dir/sysprims-minisign.pub"
	tag_blob docs/security/release-signing-keys.asc >"$dir/sysprims-release-signing-key.asc"
}
sign() {
	local manifest
	for manifest in SHA256SUMS SHA512SUMS; do
		rm -f "$1/$manifest.minisig" "$1/$manifest.asc"
		minisign -S -W -s "$2" -m "$1/$manifest" -x "$1/$manifest.minisig" >/dev/null
		gpg --homedir "$3" --batch --pinentry-mode loopback --passphrase '' --armor --detach-sign \
			-u "$4!" -o "$1/$manifest.asc" "$1/$manifest"
	done
}
fetch() { (cd "$repo" && SYSPRIMS_RELEASE_TAG="$tag" SYSPRIMS_FETCH_ASSET_DIR="$1" ./scripts/release-fetch-anchors.sh "$2"); }
keys_untouched() {
	[[ "$(cd "$repo/keys" && shasum ./*)" == "$keys_before" ]] || {
		echo 'error: fetching changed keys/' >&2
		exit 1
	}
}

published="$fixture_dir/published"
publish "$published"
fetch "$published" "$fixture_dir/fetched" >/dev/null
for name in expected-fingerprints.txt expected-fingerprints.ndjson; do
	tag_blob "keys/$name" | cmp - "$fixture_dir/fetched/$name"
done
keys_untouched

# refused <reason> <assets>: refused, nothing written, keys/ untouched.
n=0
refused() {
	n=$((n + 1))
	expect_fail_with "$1" fetch "$2" "$fixture_dir/out-$n"
	[[ -z "$(find "$fixture_dir/out-$n" -mindepth 1 -print -quit 2>/dev/null)" ]] || {
		echo "error: a refused fetch wrote into its output ($1)" >&2
		exit 1
	}
	keys_untouched
}
# A release missing either anchor.
for name in expected-fingerprints.txt expected-fingerprints.ndjson; do
	publish "$published"
	rm "$published/$name"
	refused "the release has no $name" "$published"
done
# The anchor bytes are the tag's, but the signed manifest names other bytes:
# only the manifest check can see this.
for manifest in SHA256SUMS SHA512SUMS; do
	publish "$published"
	python3 - "$published/$manifest" <<'PY'
import re, sys
path = sys.argv[1]
text = open(path).read()
changed = re.sub(r"(?m)^([0-9a-f]+)(  expected-fingerprints\.txt)$", lambda m: "0" * len(m.group(1)) + m.group(2), text)
assert changed != text
open(path, "w").write(changed)
PY
	sign "$published" "$fixture_dir/pinned.key" "$fixture_dir/gpg" "$subkey"
	refused "expected-fingerprints.txt does not match the signed $manifest" "$published"
done
# An altered anchor, consistently checksummed and validly signed: only the
# comparison with the tagged commit's blob can see this.
printf 'gpg %s\nminisign %s\n' "$subkey" "$(printf 'b%.0s' $(seq 64))" >"$fixture_dir/altered.txt"
publish "$published" "$fixture_dir/altered.txt"
refused 'expected-fingerprints.txt differs from keys/expected-fingerprints.txt in the tagged commit' "$published"
# Everything consistent, but signed by keys other than the tag's pins.
publish "$published" "" "$fixture_dir/other.key" "$fixture_dir/gpg-other" "$other_subkey"
refused 'do not verify under the pins committed at the tag' "$published"
# The output must be new or empty, never a symlink, never inside keys/.
publish "$published"
mkdir "$fixture_dir/busy"
touch "$fixture_dir/busy/other"
expect_fail_with 'new or empty directory' fetch "$published" "$fixture_dir/busy"
ln -s "$fixture_dir/fetched-target" "$fixture_dir/linked"
expect_fail_with 'must not be a symlink' fetch "$published" "$fixture_dir/linked"
expect_fail_with 'inside keys/' fetch "$published" "$repo/keys/sub"
[[ ! -e "$repo/keys/sub" ]]
expect_fail_with 'inside keys/' fetch "$published" "$repo/keys"
# keys/ reached through a symlinked parent is still keys/.
ln -s "$repo/keys" "$fixture_dir/keys-link"
expect_fail_with 'inside keys/' fetch "$published" "$fixture_dir/keys-link/sub/deeper"
[[ ! -e "$repo/keys/sub" ]]
keys_untouched
expect_fail_with 'SYSPRIMS_RELEASE_TAG is required' bash -c "cd '$repo' && env -u SYSPRIMS_RELEASE_TAG ./scripts/release-fetch-anchors.sh '$fixture_dir/no-tag'"
echo '[ok] anchor release-asset controls passed'
