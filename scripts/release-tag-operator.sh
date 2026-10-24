#!/usr/bin/env bash
# Maintainer entry point: prepare the tag messages, create the signed tags
# locally, or push them. Each mode re-checks its inputs before any side effect.
set -euo pipefail

mode="${1:-}"
case "$mode" in
prepare-message | local-tag | remote-push) ;;
*)
	echo 'error: expected prepare-message, local-tag or remote-push' >&2
	exit 1
	;;
esac

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$root"
# Check the sole tag input before an optional loader can use it to select a cut.
: "${SYSPRIMS_RELEASE_TAG:?set SYSPRIMS_RELEASE_TAG=vX.Y.Z before loading the environment}"
[[ "$SYSPRIMS_RELEASE_TAG" == "v$(tr -d '\r\n' <VERSION)" ]] || {
	echo 'error: SYSPRIMS_RELEASE_TAG must match VERSION' >&2
	exit 1
}
intended="$SYSPRIMS_RELEASE_TAG"

if [[ "${SYSPRIMS_APPROVED_ENV_LOADER+set}" == set ]]; then
	loader="$SYSPRIMS_APPROVED_ENV_LOADER"
	[[ "$loader" == /* && -f "$loader" && -r "$loader" && ! -L "$loader" ]] || {
		echo 'error: the approved environment loader must be a readable absolute regular file' >&2
		exit 1
	}
	loader_dir="$(cd "$(dirname "$loader")" && pwd -P)"
	case "$loader_dir/" in "$root/" | "$root/"*)
		echo 'error: the approved environment loader must be outside the repository' >&2
		exit 1
		;;
	esac
	# The loader may set shell options; hide its output, check its status, and
	# restore strict mode before any later guard.
	# shellcheck disable=SC1090
	if source "$loader" >/dev/null 2>&1; then loader_ok=1; else loader_ok=0; fi
	set -euo pipefail
	[[ "$loader_ok" == 1 ]] || {
		echo 'error: the approved environment loader failed' >&2
		exit 1
	}
fi

# A loader may change directory or the tag; bind every guard to this cut.
cd "$root" || exit 1
[[ "$(pwd -P)" == "$root" && "${SYSPRIMS_RELEASE_TAG:-}" == "$intended" ]] || {
	echo 'error: the loader changed the repository root or the release tag' >&2
	exit 1
}
export SYSPRIMS_RELEASE_TAG SYSPRIMS_TAG_MESSAGE_DIR
if [[ "$mode" == prepare-message ]]; then
	exec "$root/scripts/release-prepare-tag-message.py"
fi
export SYSPRIMS_TAGGER_NAME SYSPRIMS_TAGGER_EMAIL SYSPRIMS_PGP_KEY_ID SYSPRIMS_GPG_HOMEDIR SYSPRIMS_DECERNOR_BIN

# shellcheck source=scripts/release-tag-common.sh
source "$root/scripts/release-tag-common.sh"
# shellcheck source=scripts/release-decernor.sh
source "$root/scripts/release-decernor.sh"
# Every local input is checked before the first network or git side effect
# (tag_checkout fetches): the checker, tag syntax, tagger, subkey selector,
# branch, clean tree, the tag set at HEAD and every message.
resolve_release_decernor
tag_version
tag_identity
tag_selector
[[ "$(git symbolic-ref --quiet --short HEAD)" == main ]] || {
	echo 'error: release tags are created from main' >&2
	exit 1
}
tag_require_clean
tag_load_set HEAD
for tag in "${RELEASE_TAGS[@]}"; do
	tag_message_file "$tag" >/dev/null
done
tag_checkout

case "$mode" in
local-tag) "$root/scripts/release-tag.sh" ;;
remote-push) "$root/scripts/release-push-tag.sh" ;;
esac
