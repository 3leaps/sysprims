#!/usr/bin/env bash
# Pin validation reads the committed pins (or, only with --dir, a staging
# directory used by the first export) and requires exactly the committed
# signing subkey. The one-time export validates before it writes anything.
set -euo pipefail
# shellcheck disable=SC2119 # export_pin is called with and without overrides.
# shellcheck source=scripts/release-test-fixture.sh
source "$(dirname "$0")/release-test-fixture.sh"
decernor="$(fixture_decernor)"
repo="$fixture_dir/repo"
read -r _ subkey < <(fixture_keys "$fixture_dir/gpg")
read -r _ other_subkey < <(fixture_keys "$fixture_dir/gpg-other")
fixture_repo "$repo" 1.2.3 publish 0
fixture_commit "$repo" base
gpg --homedir "$fixture_dir/gpg" --batch --armor --export-options export-minimal --export "$subkey!" \
	>"$repo/docs/security/release-signing-keys.asc"
fixture_minisign_pub "$repo/docs/security/sysprims-minisign.pub" 1
printf '%s\n' "$subkey" >"$repo/config/release/signing-subkey.txt"
validate() { (cd "$repo" && SYSPRIMS_DECERNOR_BIN="$decernor" ./scripts/release-validate-pin.sh "$@"); }

# Before the first commit only --dir can see the new pins.
expect_fail_with 'committed regular file' validate
validate --dir "$repo" >/dev/null
expect_fail_with 'usage' validate --other
expect_fail_with 'usage' validate --working-tree
expect_fail_with 'staging directory must exist' validate --dir "$fixture_dir/absent"
fixture_commit "$repo" pins
validate >/dev/null

# Anchors, once committed, must verify against the pins.
SYSPRIMS_DECERNOR_BIN="$decernor" "$repo/scripts/release-insert-anchors.sh" >/dev/null
fixture_commit "$repo" anchors
validate >/dev/null
fixture_minisign_pub "$repo/docs/security/sysprims-minisign.pub" 2
fixture_commit "$repo" rotated-pub-without-anchors
expect_fail_with 'does not verify' validate
git -C "$repo" checkout -q HEAD~1 -- docs/security/sysprims-minisign.pub
fixture_commit "$repo" restore

# The committed subkey file must name the pin's signing subkey.
printf '%s\n' "$other_subkey" >"$repo/config/release/signing-subkey.txt"
fixture_commit "$repo" wrong-subkey
expect_fail_with 'differs from config/release/signing-subkey.txt' validate
printf 'not-a-fingerprint\n' >"$repo/config/release/signing-subkey.txt"
fixture_commit "$repo" bad-subkey
expect_fail_with 'one uppercase 40-hex fingerprint' validate
printf '%s\n' "$subkey" >"$repo/config/release/signing-subkey.txt"
fixture_commit "$repo" good-subkey
validate >/dev/null

# An export with two subkeys, or private material, is refused.
gpg --homedir "$fixture_dir/gpg" --batch --armor --export >"$repo/docs/security/release-signing-keys.asc"
gpg --homedir "$fixture_dir/gpg-other" --batch --armor --export >>"$repo/docs/security/release-signing-keys.asc"
fixture_commit "$repo" two-keys
expect_fail_with 'exactly one primary and one subkey' validate
printf -- '-----BEGIN PGP PRIVATE KEY BLOCK-----\n' >"$repo/docs/security/release-signing-keys.asc"
fixture_commit "$repo" private
expect_fail_with 'private material' validate
# One-time export: every input is checked, then the pins are validated in
# scratch, and only then written. A bad checker, a bad selector or an export
# that fails validation leaves no file behind.
export_repo="$fixture_dir/export"
fixture_repo "$export_repo" 1.2.3 publish 0
cp "$fixture_root/scripts/release-export-pin.sh" "$export_repo/scripts/"
fixture_commit "$export_repo" base
fixture_minisign_pub "$fixture_dir/approved.pub" 1
# export_pin [-u VAR] [VAR=value]...: defaults, then the test's overrides.
# shellcheck disable=SC2120 # Overrides are passed by some calls only.
export_pin() {
	local unset=()
	while [[ "${1:-}" == -u ]]; do
		unset+=("$1" "$2")
		shift 2
	done
	(cd "$export_repo" && env SYSPRIMS_GPG_HOMEDIR="$fixture_dir/gpg" SYSPRIMS_MINISIGN_PUB_EXPORT="$fixture_dir/approved.pub" \
		SYSPRIMS_DECERNOR_BIN="$decernor" SYSPRIMS_PGP_KEY_ID="$subkey!" "$@" \
		env ${unset[@]+"${unset[@]}"} ./scripts/release-export-pin.sh)
}
no_pins() {
	[[ ! -e "$export_repo/docs/security/release-signing-keys.asc" && ! -e "$export_repo/docs/security/sysprims-minisign.pub" &&
		! -e "$export_repo/config/release/signing-subkey.txt" ]]
}
expect_fail_with 'SYSPRIMS_DECERNOR_BIN is required' export_pin -u SYSPRIMS_DECERNOR_BIN
no_pins
expect_fail_with 'absolute executable' export_pin SYSPRIMS_DECERNOR_BIN=decernor
no_pins
expect_fail_with '40 uppercase hex' export_pin SYSPRIMS_PGP_KEY_ID="$subkey"
no_pins
# A selector for a key the home does not hold exports nothing usable.
expect_fail_with 'did not validate|export failed' export_pin SYSPRIMS_PGP_KEY_ID="$other_subkey!"
no_pins
export_pin >/dev/null
[[ "$(cat "$export_repo/config/release/signing-subkey.txt")" == "$subkey" ]]
cmp "$fixture_dir/approved.pub" "$export_repo/docs/security/sysprims-minisign.pub"
expect_fail_with 'already exists' export_pin
# Export whose last installing write fails after creating its file (the real
# helper's own cleanup is tested directly in release-anchors.test.sh): the
# earlier pins this run created are removed, and the file whose helper exited
# with an unexpected status is not claimed, so it is left in place.
pins_reset() {
	rm -f "$export_repo/docs/security/release-signing-keys.asc" "$export_repo/docs/security/sysprims-minisign.pub" \
		"$export_repo/config/release/signing-subkey.txt"
}
pins_reset
cp "$export_repo/scripts/write-exclusive.py" "$fixture_dir/real-helper.py"
# stub_helper <python run for signing-subkey.txt>: the real helper otherwise.
stub_helper() {
	cat >"$export_repo/scripts/write-exclusive.py" <<PY
#!/usr/bin/env python3
import os, subprocess, sys
dest = sys.argv[-2] if sys.argv[1] == "--remove" else sys.argv[2]
if sys.argv[1] != "--remove" and dest.endswith("signing-subkey.txt"):
$1
sys.exit(subprocess.call([sys.executable, "$fixture_dir/real-helper.py", *sys.argv[1:]]))
PY
}
stub_helper '    open(dest, "x").write("partial")
    os.kill(os.getpid(), 9)'
status=0
export_pin >"$fixture_dir/export.out" 2>&1 || status=$?
[[ "$status" != 0 ]]
grep -q 'writing config/release/signing-subkey.txt failed (status 137)' "$fixture_dir/export.out" || {
	echo 'error: the export did not fail at the installing write' >&2
	cat "$fixture_dir/export.out" >&2
	exit 1
}
[[ ! -e "$export_repo/docs/security/release-signing-keys.asc" && ! -e "$export_repo/docs/security/sysprims-minisign.pub" ]]
[[ "$(cat "$export_repo/config/release/signing-subkey.txt")" == partial ]] || {
	echo 'error: a file of unknown ownership was removed' >&2
	exit 1
}
pins_reset
# Rollback is by identity: a pin this run created but that was replaced by
# another file before the rollback is left alone.
stub_helper '    os.unlink("docs/security/sysprims-minisign.pub")
    open("docs/security/sysprims-minisign.pub", "x").write("replacement")
    sys.exit(3)'
expect_fail_with 'signing-subkey.txt appeared during export' export_pin
[[ ! -e "$export_repo/docs/security/release-signing-keys.asc" ]]
[[ "$(cat "$export_repo/docs/security/sysprims-minisign.pub")" == replacement ]] || {
	echo 'error: the rollback removed a file that replaced the one it created' >&2
	exit 1
}
pins_reset
cp "$fixture_dir/real-helper.py" "$export_repo/scripts/write-exclusive.py"
echo '[ok] pin validation controls passed'
