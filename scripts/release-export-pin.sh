#!/usr/bin/env bash
# Maintainer-only, one time: write the public pins that are then committed.
# The operator environment feeds this first write only; every later step reads
# the committed files. Nothing that already exists is overwritten.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
stop() {
	echo "STOP: $1" >&2
	exit 1
}
: "${SYSPRIMS_GPG_HOMEDIR:?load the approved GPG home}"
: "${SYSPRIMS_PGP_KEY_ID:?load the approved signing-subkey selector (40 hex, ending in !)}"
: "${SYSPRIMS_MINISIGN_PUB_EXPORT:?path to the approved minisign public key for this first write}"
[[ "$SYSPRIMS_GPG_HOMEDIR" == /* && -d "$SYSPRIMS_GPG_HOMEDIR" ]] || stop 'approved GPG home must be an absolute directory'
[[ "$SYSPRIMS_PGP_KEY_ID" =~ ^[0-9A-F]{40}!$ ]] || stop 'signing-subkey selector must be 40 uppercase hex ending in !'
[[ -f "$SYSPRIMS_MINISIGN_PUB_EXPORT" && -s "$SYSPRIMS_MINISIGN_PUB_EXPORT" && ! -L "$SYSPRIMS_MINISIGN_PUB_EXPORT" ]] ||
	stop 'minisign public export must be a nonempty regular file, not a symlink'
python3 - "$SYSPRIMS_GPG_HOMEDIR" "$HOME/.gnupg" "$root" <<'PY'
from pathlib import Path
import sys
home, default, repo = (Path(p).resolve() for p in sys.argv[1:4])
if home == default:
    raise SystemExit('STOP: the default GPG home is not approved for this export')
if home == repo or repo in home.parents:
    raise SystemExit('STOP: the GPG home must be outside the repository')
PY
asc=docs/security/release-signing-keys.asc
pub=docs/security/sysprims-minisign.pub
subkey=config/release/signing-subkey.txt
for path in "$asc" "$pub" "$subkey"; do
	[[ ! -e "$path" && ! -L "$path" ]] || stop "$path already exists; pins are written once and reviewed"
done
[[ ! -L docs/security && ! -L config/release ]] || stop 'pin directories must not be symlinks'
if grep -q -e 'secret key' -e 'PRIVATE KEY' "$SYSPRIMS_MINISIGN_PUB_EXPORT"; then
	stop 'the minisign export contains private material'
fi
# Build and validate all three pins in scratch first; nothing in the tree is
# written until they pass.
scratch="$(mktemp -d)"
helper="$root/scripts/write-exclusive.py"
# Created files as "<dev:ino> <path>", recorded from the helper's report; each
# is removed on failure only while it is still that inode.
created=()
cleanup() {
	local entry
	for entry in ${created[@]+"${created[@]}"}; do python3 "$helper" --remove "${entry#* }" "${entry%% *}"; done
	rm -rf "$scratch"
}
trap cleanup EXIT
mkdir -p "$scratch/docs/security" "$scratch/config/release"
gpg --homedir "$SYSPRIMS_GPG_HOMEDIR" --batch --armor --export-options export-minimal \
	--export "$SYSPRIMS_PGP_KEY_ID" >"$scratch/$asc" || stop 'OpenPGP public export failed'
cat "$SYSPRIMS_MINISIGN_PUB_EXPORT" >"$scratch/$pub"
printf '%s\n' "${SYSPRIMS_PGP_KEY_ID%!}" >"$scratch/$subkey"
"$root/scripts/release-validate-pin.sh" --dir "$scratch" || stop 'the exported pins did not validate; nothing was written'
mkdir -p docs/security config/release
for path in "$asc" "$pub" "$subkey"; do
	# Created with O_EXCL|O_NOFOLLOW and written through that descriptor.
	status=0
	identity="$(python3 "$helper" "$scratch/$path" "$path")" || status=$?
	case "$status" in
	0)
		[[ "$identity" =~ ^[0-9]+:[0-9]+$ ]] || stop "no identity reported for $path; it was left in place"
		created+=("$identity $path")
		;;
	3) stop "$path appeared during export; nothing more was written" ;;
	*)
		# Only 0 means created and ours; any other status leaves it unknown.
		stop "writing $path failed (status $status); the files this run created were removed, and $path was left in place for inspection"
		;;
	esac
done
created=()
echo "[ok] wrote $asc, $pub and $subkey; review and commit them, then run make release-insert-anchors"
