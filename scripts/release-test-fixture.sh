#!/usr/bin/env bash
# Shared synthetic fixtures for the release-tooling tests. Every key is
# generated here, in a temporary GPG home, for the test only; no operator
# keyring, signing key or estate material is read.
# Source this file; it sets fixture_* variables and helpers.

fixture_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
fixture_dir="$(cd "$(mktemp -d)" && pwd -P)"
fixture_cleanup() {
	gpgconf --homedir "$fixture_dir/gpg" --kill all >/dev/null 2>&1 || true
	gpgconf --homedir "$fixture_dir/gpg-other" --kill all >/dev/null 2>&1 || true
	rm -rf "$fixture_dir"
}
trap fixture_cleanup EXIT

fixture_decernor() {
	local bin="${SYSPRIMS_DECERNOR_BIN:-}"
	if [[ -z "$bin" ]]; then
		# Tests may locate a checker; the release scripts themselves never do.
		bin="$(command -v decernor || true)"
	fi
	[[ "$bin" == /* && -x "$bin" ]] || {
		echo 'error: release tooling tests need decernor >= 0.1.8 (set SYSPRIMS_DECERNOR_BIN)' >&2
		exit 1
	}
	# Resolve symlinks so the ceremony resolver accepts the path.
	python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$bin"
}

expect_fail() {
	if "$@" >"$fixture_dir/last.out" 2>&1; then
		echo "error: expected rejection: $*" >&2
		cat "$fixture_dir/last.out" >&2
		exit 1
	fi
}

expect_fail_with() {
	local pattern="$1"
	shift
	expect_fail "$@"
	grep -Eq -- "$pattern" "$fixture_dir/last.out" || {
		echo "error: rejection for '$*' did not mention: $pattern" >&2
		cat "$fixture_dir/last.out" >&2
		exit 1
	}
}

# fixture_keys <home>: primary (certify only) plus one signing subkey.
# Prints "<primary> <subkey>".
fixture_keys() {
	local home="$1" primary subkey
	mkdir -m 700 "$home"
	gpg --homedir "$home" --batch --pinentry-mode loopback --passphrase '' \
		--quick-gen-key 'Synthetic release <synthetic@example.invalid>' ed25519 cert 1d >/dev/null 2>&1
	primary="$(gpg --homedir "$home" --batch --with-colons --list-keys | awk -F: '$1=="fpr" {print $10; exit}')"
	gpg --homedir "$home" --batch --pinentry-mode loopback --passphrase '' \
		--quick-add-key "$primary" ed25519 sign 1d >/dev/null 2>&1
	subkey="$(gpg --homedir "$home" --batch --with-colons --with-subkey-fingerprint --list-keys |
		awk -F: '$1=="sub" {s=1; next} s && $1=="fpr" {print $10; exit}')"
	printf '%s %s\n' "$primary" "$subkey"
}

# fixture_minisign_pub <path> <byte>: a structurally valid synthetic public key.
fixture_minisign_pub() {
	python3 - "$1" "$2" <<'PY'
import base64
import pathlib
import sys
path, byte = sys.argv[1], int(sys.argv[2])
blob = b'Ed' + bytes([byte]) * 8 + bytes([byte]) * 32
pathlib.Path(path).write_text('untrusted comment: synthetic public key\n' + base64.b64encode(blob).decode() + '\n')
PY
}

# fixture_repo <dir> <version> <go-disposition> <with-recurrence 0|1>
# A git repo carrying the release scripts, plan, VERSION and tag config, with a
# fake origin so the scripts' origin checks pass.
fixture_repo() {
	local repo="$1" version="$2" go="$3" recurrence="$4"
	mkdir -p "$repo/scripts" "$repo/config/release" "$repo/docs/releases" "$repo/docs/security" "$repo/keys"
	cp "$fixture_root"/scripts/{write-exclusive.py,release-workflow-pin.py,release-tag-set.mjs,release-tag-common.sh,release-decernor.sh,release-prepare-tag-message.py,release-tag-body.sh,verify-pinned-tag.sh,validate-release-anchors.sh,release-insert-anchors.sh,install-release-anchors.sh,release-validate-pin.sh,release-stage-public.sh,verify-public-keys.sh} "$repo/scripts/"
	cp "$fixture_root"/config/release/{release-tags.txt,tagger-identity.txt} "$repo/config/release/"
	fixture_workflow_pin "$repo"
	printf '%s\n' "$version" >"$repo/VERSION"
	if [[ "$go" == publish ]]; then
		printf '{"surfaces":{"go":{"disposition":"publish"}}}\n' >"$repo/docs/releases/v$version.json"
	else
		printf '{"surfaces":{"go":{"disposition":"skip","retained_version":"0.0.1"}}}\n' >"$repo/docs/releases/v$version.json"
	fi
	if [[ "$recurrence" == 1 ]]; then
		mkdir -p "$repo/go/recurrence"
		printf 'module github.com/3leaps/sysprims/go/recurrence\n' >"$repo/go/recurrence/go.mod"
	fi
	git -C "$repo" init -q -b main
	git -C "$repo" config user.name fixture
	git -C "$repo" config user.email fixture@example.invalid
	git -C "$repo" config commit.gpgsign false
	git -C "$repo" config tag.gpgsign false
	git -C "$repo" remote add origin https://github.com/3leaps/sysprims.git
}

# fixture_workflow_pin <repo>: a minimal tag-triggered workflow and its pin.
fixture_workflow_pin() {
	mkdir -p "$1/.github/workflows" "$1/config/release"
	printf 'on:\n  push:\n    tags: [v*]\npermissions:\n  contents: read\njobs: {}\n' >"$1/.github/workflows/release.yml"
	printf '%s  .github/workflows/release.yml\n' "$(shasum -a 256 "$1/.github/workflows/release.yml" | cut -c1-64)" \
		>"$1/config/release/release-workflow.sha256"
}

fixture_commit() {
	git -C "$1" add -A
	git -C "$1" commit -qm "${2:-fixture}"
}

# fixture_messages <dir> <version> <tags...>: complete, valid tag messages.
fixture_messages() {
	local dir="$1" version="$2" tag subject
	shift 2
	mkdir -p "$dir"
	for tag in "$@"; do
		subject="$tag"
		[[ "$tag" == "v$version" ]] && subject="Release $version"
		printf '%s\n\nSynthetic release text.\n\nGenerated by Model via Tool under supervision of @3leapsdave\n\nRole: devlead\nCo-authored-by: Model <noreply@3leaps.net>\nCommitter-of-Record: @3leapsdave\n' \
			"$subject" >"$dir/${tag//\//__}.txt"
	done
}
