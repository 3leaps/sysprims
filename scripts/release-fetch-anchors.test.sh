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
# fetch <assets> <out> [-u VAR]... [VAR=value...]: the checks after the tag
# is trusted (as make passes a command-line opt-in, unless -u
# SYSPRIMS_FETCH_TRUST_LOCAL; tag trust is tested below).
fetch() {
	local assets="$1" dest="$2" unset=(-u SYSPRIMS_DECERNOR_BIN -u ANCHORS_TRUST_LOCAL_TAG) trust=(SYSPRIMS_FETCH_TRUST_LOCAL=1 MAKELEVEL=1)
	shift 2
	while [[ "${1:-}" == -u ]]; do
		unset+=("$1" "$2")
		case "$2" in SYSPRIMS_FETCH_TRUST_LOCAL) trust=() ;; MAKELEVEL) trust=(SYSPRIMS_FETCH_TRUST_LOCAL=1) ;; esac
		shift 2
	done
	(cd "$repo" && env "${unset[@]}" SYSPRIMS_RELEASE_TAG="$tag" SYSPRIMS_FETCH_ASSET_DIR="$assets" \
		${trust[@]+"${trust[@]}"} "$@" ./scripts/release-fetch-anchors.sh "$dest")
}
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

# refused <reason> <assets>: refused, no output directory created, keys/
# untouched.
n=0
refused() {
	n=$((n + 1))
	expect_fail_with "$1" fetch "$2" "$fixture_dir/out-$n"
	[[ ! -e "$fixture_dir/out-$n" ]] || {
		echo "error: a refused fetch left its output directory behind ($1)" >&2
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
# No `.` or `..` component, even where a missing directory would hide it from
# the textual keys/ test and mkdir would follow it.
for path in "$repo/nonexist/../keys/pwn" "$repo/./keys-not/x" "$fixture_dir/a/../b"; do
	expect_fail_with 'must not contain \. or \.\. components' fetch "$published" "$path"
done
[[ ! -e "$repo/keys/pwn" && ! -e "$repo/nonexist" ]]
keys_untouched
# The keys/ test applies to the fully resolved path (symlinks anywhere in it,
# components that do not exist yet), and nothing is created when it refuses.
ln -s "$repo" "$fixture_dir/repo-link"
expect_fail_with 'inside keys/' fetch "$published" "$fixture_dir/repo-link/keys/new"
[[ ! -e "$repo/keys/new" ]]
# keys/ under another spelling is still keys/: the test is by identity. On a
# case-insensitive volume, KEYS/pwn (absolute or relative) names keys/pwn.
mkdir "$fixture_dir/case-probe"
if [[ -d "$fixture_dir/CASE-PROBE" ]]; then
	for path in "$repo/KEYS/pwn" "KEYS/pwn" "$repo/Keys/pwn/deeper"; do
		expect_fail_with 'inside keys/' fetch "$published" "$path"
	done
	[[ ! -e "$repo/keys/pwn" ]] || {
		echo 'error: a case variant of keys/ let the fetch write into keys/' >&2
		exit 1
	}
	# The nearest existing directory may sit deeper inside keys/: every
	# ancestor is checked, not only the nearest.
	mkdir -p "$repo/keys/deep/er"
	expect_fail_with 'inside keys/' fetch "$published" "$repo/KEYS/deep/er/new"
	[[ ! -e "$repo/keys/deep/er/new" ]]
	rm -r "$repo/keys/deep"
	# The spelling alone does not decide it: the text prefix test passes it.
	case "$repo/KEYS/pwn/" in "$repo/keys/"*) echo 'error: the case fixture is caught by text' >&2 && exit 1 ;; esac
else
	echo '[skip] case-variant keys/ fixture: this volume is case-sensitive'
fi
rmdir "$fixture_dir/case-probe"
keys_untouched
# A failing run leaves no output directory; a failure at the second write
# removes the first anchor and the directory this run made, and a rerun then
# succeeds.
refused 'the release has no expected-fingerprints.txt' "$fixture_dir/no-such-assets"
mkdir -p "$repo/.variant"
cp "$repo/scripts/write-exclusive.py" "$repo/.variant/real-helper.py"
cp "$repo/scripts/write-exclusive.py" "$fixture_dir/helper.saved"
cat >"$repo/scripts/write-exclusive.py" <<PY
#!/usr/bin/env python3
import subprocess, sys
if sys.argv[1] != "--remove" and sys.argv[2].endswith(".ndjson"):
    sys.exit("error: injected failure on the second write")
sys.exit(subprocess.call([sys.executable, "$repo/.variant/real-helper.py", *sys.argv[1:]]))
PY
expect_fail_with 'cannot write .*expected-fingerprints.ndjson' fetch "$published" "$fixture_dir/partial"
[[ ! -e "$fixture_dir/partial" ]] || {
	echo "error: a failed write left partial state: $(find "$fixture_dir/partial")" >&2
	exit 1
}
# A directory the caller made beforehand is kept (empty), not removed.
mkdir "$fixture_dir/premade"
expect_fail_with 'cannot write' fetch "$published" "$fixture_dir/premade"
[[ -d "$fixture_dir/premade" && -z "$(find "$fixture_dir/premade" -mindepth 1 -print -quit)" ]]
cp "$fixture_dir/helper.saved" "$repo/scripts/write-exclusive.py"
fetch "$published" "$fixture_dir/partial" >/dev/null
fetch "$published" "$fixture_dir/premade" >/dev/null
for name in expected-fingerprints.txt expected-fingerprints.ndjson; do
	tag_blob "keys/$name" | cmp - "$fixture_dir/partial/$name"
done
# The path is re-checked after the directory is created: here a parent is
# swapped for a link into keys/ as the directory is made (a mkdir shim on
# PATH), so the pre-checks pass and only the re-check can refuse; nothing is
# written into keys/.
mkdir -p "$fixture_dir/swap/parent" "$fixture_dir/swap-bin"
cat >"$fixture_dir/swap-bin/mkdir" <<SH
#!/usr/bin/env bash
mv "$fixture_dir/swap/parent" "$fixture_dir/swap/parent.moved"
ln -s "$repo/keys" "$fixture_dir/swap/parent"
/bin/mkdir "\$@"
SH
chmod +x "$fixture_dir/swap-bin/mkdir"
expect_fail_with 'changed while it was being created|inside keys/' fetch "$published" "$fixture_dir/swap/parent/out" \
	PATH="$fixture_dir/swap-bin:$PATH"
[[ ! -e "$repo/keys/out/expected-fingerprints.txt" ]] || {
	echo 'error: a swapped parent let the fetch write into keys/' >&2
	exit 1
}
rmdir "$repo/keys/out" 2>/dev/null || true
rm "$fixture_dir/swap/parent"
keys_untouched
# The local tag supplies the pins, so by default the command refuses until the
# tag is proven to be the published one.
expect_fail_with 'set SYSPRIMS_DECERNOR_BIN to verify' fetch "$published" "$fixture_dir/untrusted" -u SYSPRIMS_FETCH_TRUST_LOCAL
expect_fail_with 'set SYSPRIMS_DECERNOR_BIN to verify' fetch "$published" "$fixture_dir/untrusted" SYSPRIMS_FETCH_TRUST_LOCAL=yes
# The user-facing opt-in in the environment is refused, even with the
# internal value present; the internal value outside make is not honoured.
expect_fail_with 'accepted only on the make command line' fetch "$published" "$fixture_dir/untrusted" ANCHORS_TRUST_LOCAL_TAG=1
expect_fail_with 'set SYSPRIMS_DECERNOR_BIN to verify' fetch "$published" "$fixture_dir/untrusted" -u MAKELEVEL
[[ ! -e "$fixture_dir/untrusted" ]]
# Through make: the opt-in counts only on the make command line, for that run.
shim="$fixture_dir/shim"
mkdir -p "$shim/scripts"
cp "$fixture_root/Makefile" "$fixture_root/VERSION" "$shim/"
# shellcheck disable=SC2016 # The stub's own expansions, written literally.
printf '#!/usr/bin/env bash\nprintf "%%s\\n" "trust=${SYSPRIMS_FETCH_TRUST_LOCAL:-} user=${ANCHORS_TRUST_LOCAL_TAG:-}" >>%q\n' "$fixture_dir/shim.log" \
	>"$shim/scripts/release-fetch-anchors.sh"
chmod +x "$shim/scripts/release-fetch-anchors.sh"
shim_make() { env -u RELEASE_TAG -u ANCHORS_TRUST_LOCAL_TAG SYSPRIMS_RELEASE_TAG="$tag" "$@"; }
rm -f "$fixture_dir/shim.log"
shim_make make -s -C "$shim" release-fetch-anchors ANCHORS_DIR="$fixture_dir/x" ANCHORS_TRUST_LOCAL_TAG=1
[[ "$(cat "$fixture_dir/shim.log")" == "trust=1 user=" ]]
rm -f "$fixture_dir/shim.log"
shim_make make -s -C "$shim" release-fetch-anchors ANCHORS_DIR="$fixture_dir/x"
[[ "$(cat "$fixture_dir/shim.log")" == "trust= user=" ]]
rm -f "$fixture_dir/shim.log"
expect_fail_with 'accepted only on the make command line' shim_make ANCHORS_TRUST_LOCAL_TAG=1 \
	make -s -C "$shim" release-fetch-anchors ANCHORS_DIR="$fixture_dir/x"
expect_fail_with 'accepted only on the make command line' shim_make ANCHORS_TRUST_LOCAL_TAG=1 \
	make -e -s -C "$shim" release-fetch-anchors ANCHORS_DIR="$fixture_dir/x"
expect_fail_with 'accepted only on the make command line' shim_make SYSPRIMS_FETCH_TRUST_LOCAL=1 ANCHORS_TRUST_LOCAL_TAG=1 \
	make -s -C "$shim" release-fetch-anchors ANCHORS_DIR="$fixture_dir/x"
[[ ! -e "$fixture_dir/shim.log" ]]
# An inherited internal value does not survive make: it sets its own.
shim_make SYSPRIMS_FETCH_TRUST_LOCAL=1 make -s -C "$shim" release-fetch-anchors ANCHORS_DIR="$fixture_dir/x"
[[ "$(cat "$fixture_dir/shim.log")" == "trust= user=" ]]
# With the checker available, the remote tag verification runs first: when it
# fails, nothing is fetched or written; when it passes, the fetch proceeds.
# (A stand-in records the call; the verification itself has its own suite.)
cat >"$repo/scripts/release-verify-remote-tag.sh" <<SH
#!/usr/bin/env bash
echo "\$SYSPRIMS_RELEASE_TAG" >>"$fixture_dir/remote-verified"
exit "\${REMOTE_TAG_STATUS:-0}"
SH
chmod +x "$repo/scripts/release-verify-remote-tag.sh"
expect_fail_with 'is not the verified published tag' fetch "$fixture_dir/no-such-assets" "$fixture_dir/hostile" \
	-u SYSPRIMS_FETCH_TRUST_LOCAL SYSPRIMS_DECERNOR_BIN=/checker REMOTE_TAG_STATUS=1
[[ ! -e "$fixture_dir/hostile" && "$(cat "$fixture_dir/remote-verified")" == "$tag" ]]
rm "$fixture_dir/remote-verified"
fetch "$published" "$fixture_dir/verified" -u SYSPRIMS_FETCH_TRUST_LOCAL SYSPRIMS_DECERNOR_BIN=/checker >/dev/null
[[ "$(cat "$fixture_dir/remote-verified")" == "$tag" && -f "$fixture_dir/verified/expected-fingerprints.txt" ]]
# The opt-in never skips the verification when the checker is set.
rm "$fixture_dir/remote-verified"
expect_fail_with 'is not the verified published tag' fetch "$published" "$fixture_dir/hostile" \
	SYSPRIMS_DECERNOR_BIN=/checker REMOTE_TAG_STATUS=1
keys_untouched
expect_fail_with 'SYSPRIMS_RELEASE_TAG is required' bash -c "cd '$repo' && env -u SYSPRIMS_RELEASE_TAG ./scripts/release-fetch-anchors.sh '$fixture_dir/no-tag'"
echo '[ok] anchor release-asset controls passed'
