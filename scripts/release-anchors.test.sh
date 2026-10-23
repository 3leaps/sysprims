#!/usr/bin/env bash
# Anchor derivation reads the committed exports, preserves decernor's records
# byte-for-byte, never overwrites, and binds the checker by absolute path only.
set -euo pipefail
# shellcheck source=scripts/release-test-fixture.sh
source "$(dirname "$0")/release-test-fixture.sh"
decernor="$(fixture_decernor)"
repo="$fixture_dir/repo"
read -r _ subkey < <(fixture_keys "$fixture_dir/gpg")
fixture_repo "$repo" 1.2.3 publish 0
gpg --homedir "$fixture_dir/gpg" --batch --armor --export-options export-minimal --export "$subkey!" \
	>"$repo/docs/security/release-signing-keys.asc"
fixture_minisign_pub "$repo/docs/security/sysprims-minisign.pub" 1
printf '%s\n' "$subkey" >"$repo/config/release/signing-subkey.txt"
fixture_commit "$repo" pins
insert() { (cd "$repo" && SYSPRIMS_DECERNOR_BIN="$decernor" ./scripts/release-insert-anchors.sh); }

# Checker binding: required, absolute, a regular executable, not a symlink, and
# really decernor. Neither PATH nor an unprefixed variable stands in for it.
(cd "$repo" && expect_fail_with 'SYSPRIMS_DECERNOR_BIN is required' env -u SYSPRIMS_DECERNOR_BIN DECERNOR_BIN="$decernor" ./scripts/release-insert-anchors.sh)
(cd "$repo" && expect_fail_with 'absolute executable' env SYSPRIMS_DECERNOR_BIN=decernor ./scripts/release-insert-anchors.sh)
ln -s "$decernor" "$fixture_dir/decernor-link"
(cd "$repo" && expect_fail_with 'not a symlink' env SYSPRIMS_DECERNOR_BIN="$fixture_dir/decernor-link" ./scripts/release-insert-anchors.sh)
printf '#!/bin/sh\necho "notdecernor 9.9.9"\n' >"$fixture_dir/impostor"
chmod +x "$fixture_dir/impostor"
(cd "$repo" && expect_fail_with 'does not identify as decernor' env SYSPRIMS_DECERNOR_BIN="$fixture_dir/impostor" ./scripts/release-insert-anchors.sh)
# shellcheck disable=SC2016 # The single-quoted text is the impostor script body.
printf '#!/bin/sh\nif [ "$2" = "-e" ]; then printf "Version: 0.1.7\\nCommit: x\\nBuild Date: x\\nGo Version: x\\nGofulmen: x\\nCrucible: x\\n"; else echo "decernor 0.1.7"; fi\n' >"$fixture_dir/old"
chmod +x "$fixture_dir/old"
(cd "$repo" && expect_fail_with '>= 0.1.8' env SYSPRIMS_DECERNOR_BIN="$fixture_dir/old" ./scripts/release-insert-anchors.sh)
[[ ! -e "$repo/keys/expected-fingerprints.txt" ]]

# The exports come from the committed tree: an uncommitted change is ignored.
fixture_minisign_pub "$repo/docs/security/sysprims-minisign.pub" 2
insert >/dev/null
git -C "$repo" checkout -q -- docs/security/sysprims-minisign.pub
committed_pub="$fixture_dir/committed.pub"
git -C "$repo" cat-file blob HEAD:docs/security/sysprims-minisign.pub >"$committed_pub"
"$decernor" fingerprint verify --anchors "$repo/keys/expected-fingerprints.txt" \
	--anchors-ndjson "$repo/keys/expected-fingerprints.ndjson" \
	--gpg "$repo/docs/security/release-signing-keys.asc" --minisign "$committed_pub" >/dev/null

# The NDJSON is decernor's own record lines, byte-for-byte.
"$decernor" fingerprint "$repo/docs/security/release-signing-keys.asc" --class public --kind gpg \
	--format ndjson --path-mode none --gpg-role primary >"$fixture_dir/expected.ndjson"
"$decernor" fingerprint "$committed_pub" --class public --kind minisign --format ndjson --path-mode none |
	grep -F '"fingerprint_scheme":"minisign-public-blob-sha256-v1"' >>"$fixture_dir/expected.ndjson"
cmp "$fixture_dir/expected.ndjson" "$repo/keys/expected-fingerprints.ndjson"

# Existing anchors are never overwritten.
cp "$repo/keys/expected-fingerprints.txt" "$fixture_dir/before.txt"
expect_fail_with 'never overwritten' insert
cmp "$fixture_dir/before.txt" "$repo/keys/expected-fingerprints.txt"

# A missing committed export fails before any write.
rm -rf "$repo/keys"
git -C "$repo" rm -q docs/security/sysprims-minisign.pub
fixture_commit "$repo" drop-pub
expect_fail_with 'committed regular file' insert
[[ ! -e "$repo/keys/expected-fingerprints.txt" ]]

# Validation: decernor rejects anchors that do not match the exports.
git -C "$repo" checkout -q HEAD~1 -- docs/security/sysprims-minisign.pub
fixture_commit "$repo" restore-pub
insert >/dev/null
fixture_minisign_pub "$fixture_dir/other.pub" 3
(
	RELEASE_DECERNOR_BIN="$decernor"
	export RELEASE_DECERNOR_BIN
	"$repo/scripts/validate-release-anchors.sh" "$repo/keys/expected-fingerprints.txt" \
		"$repo/keys/expected-fingerprints.ndjson" "$repo/docs/security/release-signing-keys.asc" \
		"$repo/docs/security/sysprims-minisign.pub" >/dev/null
	expect_fail_with 'does not verify' "$repo/scripts/validate-release-anchors.sh" \
		"$repo/keys/expected-fingerprints.txt" "$repo/keys/expected-fingerprints.ndjson" \
		"$repo/docs/security/release-signing-keys.asc" "$fixture_dir/other.pub"
	sed 's/^minisign .*/minisign 0000000000000000000000000000000000000000000000000000000000000000/' \
		"$repo/keys/expected-fingerprints.txt" >"$fixture_dir/edited.txt"
	expect_fail_with 'text and records differ' "$repo/scripts/validate-release-anchors.sh" \
		"$fixture_dir/edited.txt" "$repo/keys/expected-fingerprints.ndjson" \
		"$repo/docs/security/release-signing-keys.asc" "$repo/docs/security/sysprims-minisign.pub"
)
# Rollback removes only what this invocation created. A file that appears at
# the destination after the absence check is left untouched, and a failing
# verify step removes the pair this run installed and nothing else.
stage="$fixture_dir/stage"
mkdir -p "$stage" "$fixture_dir/dest"
printf 'gpg A\nminisign B\n' >"$stage/expected-fingerprints.txt"
printf '{}\n{}\n' >"$stage/expected-fingerprints.ndjson"
racer="$fixture_dir/racer.sh"
cat >"$racer" <<SH
#!/usr/bin/env bash
# Runs as the verify step after the txt anchor exists: simulates a concurrent
# writer that created the ndjson anchor first, so this run never owned it.
exit 1
SH
chmod +x "$racer"
printf 'concurrent\n' >"$fixture_dir/dest/expected-fingerprints.ndjson"
# The pre-check refuses an existing destination outright.
expect_fail_with 'never overwritten' "$fixture_root/scripts/install-release-anchors.sh" "$stage" "$fixture_dir/dest"
[[ "$(cat "$fixture_dir/dest/expected-fingerprints.ndjson")" == concurrent ]]
[[ ! -e "$fixture_dir/dest/expected-fingerprints.txt" ]]
rm "$fixture_dir/dest/expected-fingerprints.ndjson"
# A file created between the check and the write (race) survives.
appear="$fixture_dir/appear"
mkdir "$appear"
# The shim runs at the script's mkdir, after the absence check and before any
# anchor is opened, and creates the ndjson anchor as a concurrent writer would.
mkdir "$fixture_dir/race-bin"
cat >"$fixture_dir/race-bin/mkdir" <<SH
#!/usr/bin/env bash
/bin/mkdir "\$@"
printf 'raced\\n' >"$appear/expected-fingerprints.ndjson"
SH
chmod +x "$fixture_dir/race-bin/mkdir"
expect_fail_with 'appeared during installation' env PATH="$fixture_dir/race-bin:$PATH" "$fixture_root/scripts/install-release-anchors.sh" "$stage" "$appear"
[[ "$(cat "$appear/expected-fingerprints.ndjson")" == raced ]]
[[ ! -e "$appear/expected-fingerprints.txt" ]]
# A failing verify step removes this run's pair only.
expect_fail "$fixture_root/scripts/install-release-anchors.sh" "$stage" "$fixture_dir/dest" "$racer"
[[ ! -e "$fixture_dir/dest/expected-fingerprints.txt" && ! -e "$fixture_dir/dest/expected-fingerprints.ndjson" ]]
printf 'unrelated\n' >"$fixture_dir/dest/other"
"$fixture_root/scripts/install-release-anchors.sh" "$stage" "$fixture_dir/dest"
[[ "$(cat "$fixture_dir/dest/other")" == unrelated ]]
# A copy that writes part of an anchor and then fails: the partial file was
# created by this run, so it is removed with the rest.
partial="$fixture_dir/partial"
mkdir -p "$partial" "$fixture_dir/partial-bin"
cat >"$fixture_dir/partial-bin/cat" <<SH
#!/usr/bin/env bash
if [[ "\$1" == *expected-fingerprints.ndjson ]]; then printf 'half'; exit 1; fi
exec /bin/cat "\$@"
SH
chmod +x "$fixture_dir/partial-bin/cat"
expect_fail env PATH="$fixture_dir/partial-bin:$PATH" "$fixture_root/scripts/install-release-anchors.sh" "$stage" "$partial"
[[ -z "$(ls -A "$partial")" ]] || {
	echo "error: a partially written anchor survived: $(ls -A "$partial")" >&2
	exit 1
}
echo '[ok] anchor and checker-binding controls passed'
