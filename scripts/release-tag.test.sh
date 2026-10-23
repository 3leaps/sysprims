#!/usr/bin/env bash
# make release-tag checks every input before it signs anything, signs the whole
# set at HEAD, and a failure part way through leaves no tag behind.
set -euo pipefail
# shellcheck disable=SC2119 # tag_run is called with and without overrides.
# shellcheck source=scripts/release-test-fixture.sh
source "$(dirname "$0")/release-test-fixture.sh"
decernor="$(fixture_decernor)"
version=1.2.3
repo="$fixture_dir/repo"
origin="$fixture_dir/origin.git"
messages="$fixture_dir/messages/v$version"
read -r _ subkey < <(fixture_keys "$fixture_dir/gpg")
fixture_repo "$repo" "$version" publish 1
cp "$fixture_root/scripts/"{release-tag.sh,release-verify-tag.sh} "$repo/scripts/"
gpg --homedir "$fixture_dir/gpg" --batch --armor --export-options export-minimal --export "$subkey!" \
	>"$repo/docs/security/release-signing-keys.asc"
fixture_minisign_pub "$repo/docs/security/sysprims-minisign.pub" 1
printf '%s\n' "$subkey" >"$repo/config/release/signing-subkey.txt"
fixture_commit "$repo" pins
SYSPRIMS_DECERNOR_BIN="$decernor" "$repo/scripts/release-insert-anchors.sh" >/dev/null
fixture_commit "$repo" anchors
git init -q --bare "$origin"
git -C "$repo" push -q "$origin" main
git -C "$repo" fetch -q "$origin" '+refs/heads/main:refs/remotes/origin/main'
# The scripts require the GitHub origin URL; route it to the local bare repo.
git -C "$repo" config "url.$origin.insteadOf" https://github.com/3leaps/sysprims.git
tags=("v$version" "bindings/go/sysprims/v$version" "go/recurrence/v$version")
fixture_messages "$messages" "$version" "${tags[@]}"
# shellcheck disable=SC2120 # Overrides are passed by some calls only.
tag_run() {
	local unset=()
	while [[ "${1:-}" == -u ]]; do
		unset+=("$1" "$2")
		shift 2
	done
	(cd "$repo" && env SYSPRIMS_RELEASE_TAG="v$version" SYSPRIMS_TAG_MESSAGE_DIR="$messages" \
		SYSPRIMS_TAGGER_NAME='3 Leaps Infosec Team' SYSPRIMS_TAGGER_EMAIL=infosec@3leaps.net \
		SYSPRIMS_PGP_KEY_ID="$subkey!" SYSPRIMS_GPG_HOMEDIR="$fixture_dir/gpg" \
		SYSPRIMS_DECERNOR_BIN="$decernor" "$@" env ${unset[@]+"${unset[@]}"} ./scripts/release-tag.sh)
}
no_tags() { [[ -z "$(git -C "$repo" tag -l)" ]]; }
# A git shim that records every signing attempt, so "nothing was signed" is
# checked directly rather than through the rollback.
real_git="$(command -v git)"
mkdir -p "$fixture_dir/spy"
cat >"$fixture_dir/spy/git" <<SH
#!/usr/bin/env bash
[[ "\$1" == tag && " \$* " == *" -s "* ]] && echo "\$*" >>"$fixture_dir/signed.log"
exec "$real_git" "\$@"
SH
chmod +x "$fixture_dir/spy/git"
nothing_signed() { [[ ! -s "$fixture_dir/signed.log" ]] && no_tags; }

# Inputs are checked before anything is signed.
rm -f "$fixture_dir/signed.log"
expect_fail_with 'SYSPRIMS_DECERNOR_BIN is required' tag_run -u SYSPRIMS_DECERNOR_BIN PATH="$fixture_dir/spy:$PATH"
nothing_signed
expect_fail_with 'absolute executable' tag_run SYSPRIMS_DECERNOR_BIN=decernor PATH="$fixture_dir/spy:$PATH"
nothing_signed
expect_fail_with 'differs from config/release/signing-subkey.txt' tag_run SYSPRIMS_PGP_KEY_ID="$(printf 'F%.0s' $(seq 40))!"
no_tags
expect_fail_with 'committed tagger identity' tag_run SYSPRIMS_TAGGER_NAME=Someone
no_tags
mv "$messages/go__recurrence__v$version.txt" "$fixture_dir/held.txt"
expect_fail_with "tag message for go/recurrence/v$version is missing" tag_run
no_tags
mv "$fixture_dir/held.txt" "$messages/go__recurrence__v$version.txt"

# A signing failure on a later tag rolls back the tags this run created.
mkdir -p "$fixture_dir/bin"
cat >"$fixture_dir/bin/git" <<SH
#!/usr/bin/env bash
if [[ "\$1" == tag && " \$* " == *" go/recurrence/v$version "* && " \$* " == *" -s "* ]]; then
	echo 'simulated signing failure' >&2
	exit 1
fi
exec "$real_git" "\$@"
SH
chmod +x "$fixture_dir/bin/git"
expect_fail tag_run PATH="$fixture_dir/bin:$PATH"
no_tags

# With valid inputs every tag in the set is signed at HEAD and verified.
out="$(tag_run)"
[[ "$out" == *"3 signed tag(s) created locally"* ]]
head="$(git -C "$repo" rev-parse HEAD)"
for tag in "${tags[@]}"; do
	[[ "$(git -C "$repo" rev-parse "$tag^{commit}")" == "$head" ]]
	git -C "$repo" cat-file tag "$tag" | grep -q 'BEGIN PGP SIGNATURE'
done
[[ -z "$(git -C "$origin" tag -l)" ]]
expect_fail_with 'local tag v1.2.3 already exists' tag_run
echo '[ok] release-tag controls passed'
