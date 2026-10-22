#!/usr/bin/env bash
# Signed-tag ceremony invariants, shared by the tag scripts. sysprims tags one
# commit with every name in the release tag set; no signing material lives in
# this repository.
set -euo pipefail

tag_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tag_repo_slug=3leaps/sysprims

tag_die() {
	echo "error: $*" >&2
	return 1
}

# The release tag is the explicit input; it must name the canonical vX.Y.Z tag
# for the version under release.
tag_version() {
	[[ -n "${SYSPRIMS_RELEASE_TAG:-}" ]] || {
		tag_die 'SYSPRIMS_RELEASE_TAG is required'
		return 1
	}
	[[ "$SYSPRIMS_RELEASE_TAG" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || {
		tag_die 'SYSPRIMS_RELEASE_TAG must be a canonical vX.Y.Z tag'
		return 1
	}
	[[ "$SYSPRIMS_RELEASE_TAG" == "v$(tr -d '\r\n' <"$tag_root/VERSION")" ]] || {
		tag_die 'SYSPRIMS_RELEASE_TAG does not match VERSION'
		return 1
	}
}

# Every tag name for this release, one per line, in config/release/release-tags.txt
# order. The Go binding tag follows the release plan's Go disposition; the
# recurrence module tag is present whenever the module is in the tree.
tag_set() {
	local tree="${1:-$tag_root}" prefix disposition
	# shellcheck disable=SC2016 # The single-quoted script is JavaScript.
	disposition="$(cd "$tree" && node -e '
const fs = require("fs");
const v = fs.readFileSync("VERSION", "utf8").trim();
process.stdout.write(JSON.parse(fs.readFileSync(`docs/releases/v${v}.json`, "utf8")).surfaces.go.disposition);
')" || return 1
	while IFS= read -r prefix; do
		[[ -n "$prefix" ]] || continue
		case "$prefix" in
		v) ;;
		bindings/go/sysprims/v) [[ "$disposition" == publish ]] || continue ;;
		go/recurrence/v) [[ -f "$tree/go/recurrence/go.mod" ]] || continue ;;
		*)
			tag_die "unknown tag prefix in config/release/release-tags.txt: $prefix"
			return 1
			;;
		esac
		printf '%s%s\n' "$prefix" "${SYSPRIMS_RELEASE_TAG#v}"
	done <"$tag_root/config/release/release-tags.txt"
}

tag_identity() {
	[[ "${SYSPRIMS_TAGGER_NAME:-} <${SYSPRIMS_TAGGER_EMAIL:-}>" == "$(cat "$tag_root/config/release/tagger-identity.txt")" ]] || {
		tag_die 'the committed tagger identity is required'
		return 1
	}
	local value
	for value in "$SYSPRIMS_TAGGER_NAME" "$SYSPRIMS_TAGGER_EMAIL"; do
		[[ "$value" != *$'\n'* && "$value" != *$'\r'* && "$value" != *'<'* && "$value" != *'>'* ]] || {
			tag_die 'invalid tagger identity'
			return 1
		}
	done
}

# Tag messages live outside the repository in one per-release directory, one
# file per tag: the tag name with "/" replaced by "__", plus ".txt".
tag_message_dir() {
	local dir="${SYSPRIMS_TAG_MESSAGE_DIR:-}" component canonical
	[[ -n "$dir" && -d "$dir" && ! -L "$dir" ]] || {
		tag_die 'SYSPRIMS_TAG_MESSAGE_DIR must name an existing directory'
		return 1
	}
	[[ "$dir" == /* && "/$dir/" != *'/../'* ]] || {
		tag_die 'SYSPRIMS_TAG_MESSAGE_DIR must be absolute without parent traversal'
		return 1
	}
	[[ "${dir%/}" == */"$SYSPRIMS_RELEASE_TAG" ]] || {
		tag_die 'SYSPRIMS_TAG_MESSAGE_DIR must end in the release tag'
		return 1
	}
	component="${dir%/}"
	while [[ "$component" != / ]]; do
		[[ ! -L "$component" ]] || {
			tag_die 'SYSPRIMS_TAG_MESSAGE_DIR may not have symlink ancestors'
			return 1
		}
		component="${component%/*}"
		[[ -n "$component" ]] || component=/
	done
	canonical="$(cd "$dir" && pwd -P)"
	case "$canonical" in "$tag_root" | "$tag_root"/*)
		tag_die 'SYSPRIMS_TAG_MESSAGE_DIR must be outside the repository'
		return 1
		;;
	esac
	printf '%s\n' "${dir%/}"
}

tag_message_file() {
	local tag="$1" dir file
	dir="$(tag_message_dir)" || return 1
	file="$dir/${tag//\//__}.txt"
	[[ -f "$file" && ! -L "$file" && -s "$file" ]] || {
		tag_die "tag message for $tag is missing"
		return 1
	}
	local subject="$tag"
	[[ "$tag" == "$SYSPRIMS_RELEASE_TAG" ]] && subject="Release ${SYSPRIMS_RELEASE_TAG#v}"
	"$tag_root/scripts/release-prepare-tag-message.py" --validate-message "$file" "$subject" || return 1
	printf '%s\n' "$file"
}

tag_origin() {
	local url host
	url="$(git config --get remote.origin.url)"
	case "$url" in
	"https://github.com/$tag_repo_slug" | "https://github.com/$tag_repo_slug.git" | \
		"git@github.com:$tag_repo_slug" | "git@github.com:$tag_repo_slug.git") ;;
	git@*:"$tag_repo_slug" | git@*:"$tag_repo_slug.git")
		host="${url#git@}"
		host="${host%%:*}"
		[[ "$(ssh -G "$host" 2>/dev/null | awk '$1=="hostname" {print $2;exit}')" == github.com ]] || {
			tag_die 'origin SSH host must resolve to github.com'
			return 1
		}
		;;
	*)
		tag_die "origin must be $tag_repo_slug on GitHub"
		return 1
		;;
	esac
}

tag_checkout() {
	tag_origin
	git fetch --quiet origin '+refs/heads/main:refs/remotes/origin/main'
	[[ -z "$(git status --porcelain --untracked-files=normal)" ]] || tag_die 'clean checkout required'
	[[ "$(git rev-parse HEAD)" == "$(git rev-parse refs/remotes/origin/main)" ]] || tag_die 'HEAD must match fetched origin/main'
}

# The operator's subkey selector must name the committed signing subkey.
tag_selector() {
	[[ "${SYSPRIMS_PGP_KEY_ID:-}" =~ ^[0-9A-F]{40}!$ ]] || {
		tag_die 'SYSPRIMS_PGP_KEY_ID must be the exact uppercase signing subkey followed by !'
		return 1
	}
	[[ "${SYSPRIMS_PGP_KEY_ID%!}" == "$(tr -d '\r\n' <"$tag_root/config/release/signing-subkey.txt")" ]] || {
		tag_die 'SYSPRIMS_PGP_KEY_ID differs from config/release/signing-subkey.txt'
		return 1
	}
}

tag_key_home() {
	[[ -n "${SYSPRIMS_GPG_HOMEDIR:-}" && -d "$SYSPRIMS_GPG_HOMEDIR" ]] || {
		tag_die 'an external SYSPRIMS_GPG_HOMEDIR is required'
		return 1
	}
	local home
	home="$(cd "$SYSPRIMS_GPG_HOMEDIR" && pwd -P)"
	case "$home" in "$tag_root" | "$tag_root"/*)
		tag_die 'SYSPRIMS_GPG_HOMEDIR must be outside the repository'
		return 1
		;;
	esac
	export GNUPGHOME="$home"
}

# Check one annotated tag object: name, target, tagger and exact message bytes.
tag_verify_object() {
	local tag="$1" target="$2" object expected actual tagger
	object="$(git rev-parse --verify --quiet "refs/tags/$tag")" || {
		tag_die "tag $tag is absent"
		return 1
	}
	[[ "$(git cat-file -t "$object")" == tag ]] || {
		tag_die "$tag must be an annotated tag"
		return 1
	}
	[[ "$(git cat-file tag "$object" | sed -n 's/^tag //p' | head -1)" == "$tag" ]] || {
		tag_die "$tag object names a different tag"
		return 1
	}
	[[ "$(git rev-parse "${object}^{commit}")" == "$target" ]] || {
		tag_die "$tag does not point at $target"
		return 1
	}
	tagger="$(git cat-file tag "$object" | sed -n 's/^tagger \(.*\) [0-9][0-9]* [+-][0-9][0-9][0-9][0-9]$/\1/p' | head -1)"
	[[ "$tagger" == "$(cat "$tag_root/config/release/tagger-identity.txt")" ]] || {
		tag_die "$tag tagger differs from the committed identity"
		return 1
	}
	expected="$(tag_message_file "$tag")" || return 1
	actual="$(mktemp)"
	if ! "$tag_root/scripts/release-tag-body.sh" "$object" >"$actual"; then
		rm -f "$actual"
		return 1
	fi
	if ! cmp -s "$expected" "$actual"; then
		rm -f "$actual"
		tag_die "$tag message differs from the prepared message"
		return 1
	fi
	rm -f "$actual"
}
