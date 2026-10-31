#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# PDR-0011 Cargo credential-provider and zero-compile crate publication
# controls against a local sparse test registry.
set -euo pipefail
# shellcheck source=scripts/release-test-fixture.sh
source "$(dirname "$0")/release-test-fixture.sh"
export PYTHONDONTWRITEBYTECODE=1
unset CI CARGO_REGISTRY_TOKEN CARGO_REGISTRIES_CRATES_IO_TOKEN

msrv="$(sed -n 's/^rust-version = "\(.*\)"/\1/p' "$fixture_root/Cargo.toml" | head -n 1)"
if ! command -v cargo >/dev/null || ! cargo "+$msrv" --version >/dev/null 2>&1; then
	echo "error: this test needs cargo with the $msrv toolchain" >&2
	exit 1
fi

prov="$fixture_root/scripts/release-cargo-provider.py"
expect_fail_with 'run by Cargo as a credential provider' python3 -I "$prov"

version=1.2.3
tag="v$version"
digest="$(printf 'a%.0s' $(seq 64))"
printf 'dummy-token\n' >"$fixture_dir/token"
printf '{"index-url":"https://github.com/rust-lang/crates.io-index","name":"sysprims-core","vers":"%s","cksum":"%s"}' \
	"$version" "$digest" >"$fixture_dir/approval.json"

ask() {
	printf '%s\n' "$1" | SYSPRIMS_CARGO_APPROVAL="$fixture_dir/approval.json" \
		SYSPRIMS_CARGO_TOKEN_FILE="$fixture_dir/token" \
		SYSPRIMS_CARGO_PROVIDER_LOG="$fixture_dir/unit-provider.log" \
		python3 -I "$prov" --cargo-plugin | tail -1
}

reg='{"index-url":"https://github.com/rust-lang/crates.io-index","name":"crates-io"}'
[[ "$(ask "{\"v\":1,\"registry\":$reg,\"kind\":\"get\",\"operation\":\"read\"}")" == *'"token": "sysprims-read-placeholder-not-a-credential"'* ]]
[[ "$(ask "{\"v\":1,\"registry\":$reg,\"kind\":\"get\",\"operation\":\"publish\",\"name\":\"sysprims-core\",\"vers\":\"$version\",\"cksum\":\"$digest\"}")" == *'"token": "dummy-token"'* ]]
for request in \
	"{\"v\":1,\"registry\":$reg,\"kind\":\"get\",\"operation\":\"publish\",\"name\":\"sysprims-core\",\"vers\":\"$version\",\"cksum\":\"$(printf '%064d' 0)\"}" \
	"{\"v\":1,\"registry\":$reg,\"kind\":\"get\",\"operation\":\"publish\",\"name\":\"sysprims-signal\",\"vers\":\"$version\",\"cksum\":\"$digest\"}" \
	"{\"v\":1,\"registry\":$reg,\"kind\":\"get\",\"operation\":\"publish\",\"name\":\"sysprims-core\",\"vers\":\"9.9.9\",\"cksum\":\"$digest\"}" \
	"{\"v\":1,\"registry\":{\"index-url\":\"https://example.invalid/index\"},\"kind\":\"get\",\"operation\":\"publish\",\"name\":\"sysprims-core\",\"vers\":\"$version\",\"cksum\":\"$digest\"}" \
	"{\"v\":1,\"registry\":$reg,\"kind\":\"get\",\"operation\":\"yank\",\"name\":\"sysprims-core\",\"vers\":\"$version\"}" \
	"{\"v\":1,\"registry\":$reg,\"kind\":\"login\"}"; do
	[[ "$(ask "$request")" == *'"Err"'* ]] || {
		echo "error: provider released a token for $request" >&2
		exit 1
	}
done
if grep -q 'dummy-token' "$fixture_dir/unit-provider.log"; then
	echo 'error: provider logged the registry token' >&2
	exit 1
fi

printf '{"index-url":"x","name":"sysprims-core","vers":"1.2.3","cksum":"bad"}' >"$fixture_dir/bad-approval.json"
expect_fail_with 'malformed approval' env SYSPRIMS_CARGO_APPROVAL="$fixture_dir/bad-approval.json" \
	SYSPRIMS_CARGO_TOKEN_FILE="$fixture_dir/token" python3 -I "$prov" --cargo-plugin </dev/null
: >"$fixture_dir/empty-token"
expect_fail_with 'empty token' bash -c "printf '%s\n' '{\"v\":1,\"registry\":$reg,\"kind\":\"get\",\"operation\":\"publish\",\"name\":\"sysprims-core\",\"vers\":\"$version\",\"cksum\":\"$digest\"}' | SYSPRIMS_CARGO_APPROVAL='$fixture_dir/approval.json' SYSPRIMS_CARGO_TOKEN_FILE='$fixture_dir/empty-token' python3 -I '$prov' --cargo-plugin"

# End-to-end integration with real Cargo against the local sparse registry.
repo="$fixture_dir/repo"
fixture_repo "$repo" "$version" publish 0
cp "$fixture_root"/scripts/{release-crates-dry-run.sh,release-crates.py,release-export-tree.py,release-cargo-provider.py,release-source-check.py,check-registry-config.py,write-exclusive.py} "$repo/scripts/"
printf 'sysprims-core\n' >"$repo/config/release/publishable-crates.txt"
mkdir -p "$repo/crates/sysprims-core/src"
cat >"$repo/Cargo.toml" <<'TOML'
[workspace]
members = ["crates/sysprims-core"]
resolver = "2"

[workspace.package]
version = "1.2.3"
edition = "2021"
rust-version = "1.88.0"
license = "MIT OR Apache-2.0"
publish = false
TOML
cat >"$repo/crates/sysprims-core/Cargo.toml" <<'TOML'
[package]
name = "sysprims-core"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
publish = true
description = "Synthetic sysprims-core fixture"
readme = "README.md"
TOML
printf '# sysprims-core\n' >"$repo/crates/sysprims-core/README.md"
printf 'pub fn core_ready() -> bool { true }\n' >"$repo/crates/sysprims-core/src/lib.rs"
printf 'target/\n' >"$repo/.gitignore"
(cd "$repo" && cargo generate-lockfile --offline -q)

# Poisoned cargo in the repo's bin/ directory: relative PATH entries must be stripped.
mkdir -p "$repo/bin" "$fixture_dir/outside"
printf '#!/usr/bin/env bash\ntouch "%s/poisoned"\nexit 0\n' "$fixture_dir" >"$repo/bin/cargo"
printf '#!/usr/bin/env bash\ntouch "%s/poisoned"\nexit 0\n' "$fixture_dir" >"$fixture_dir/outside/cargo"
chmod +x "$repo/bin/cargo" "$fixture_dir/outside/cargo"

fixture_commit "$repo" release
git -C "$repo" tag -a -m "Release $version" "$tag"
git -C "$repo" checkout -q --detach "$tag"
commit="$(git -C "$repo" rev-parse HEAD)"

# Compute the exact .crate digest produced by real cargo package --no-verify --locked.
pkg_check="$fixture_dir/pkg-check"
python3 "$repo/scripts/release-export-tree.py" "$repo" "$commit" "$pkg_check/tree"
(cd "$pkg_check/tree" && env -i PATH="$PATH" HOME="$pkg_check/home" CARGO_HOME="$pkg_check/home" \
	CARGO_TARGET_DIR="$pkg_check/target" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
	RUSTUP_TOOLCHAIN="$msrv" cargo package --locked --no-verify -p sysprims-core >/dev/null 2>&1)
expected_crate="$pkg_check/target/package/sysprims-core-$version.crate"
digest="$(shasum -a 256 "$expected_crate" | cut -c1-64)"

registry="$fixture_dir/registry"
python3 "$fixture_root/scripts/test-registry.py" "$registry" 0 dummy-token >/dev/null 2>&1 &
registry_pid=$!
trap 'kill "$registry_pid" 2>/dev/null || true; fixture_cleanup' EXIT
for _ in $(seq 1 50); do
	[[ -f "$registry/ready" ]] && break
	sleep 0.1
done
port="$(cat "$registry/ready")"
index="sparse+http://127.0.0.1:$port/index/"
api="http://127.0.0.1:$port"
dl="http://127.0.0.1:$port/dl"

export SYSPRIMS_RELEASE_TAG="$tag"
export SYSPRIMS_CARGO_TOKEN_FILE="$fixture_dir/token"
export SYSPRIMS_INDEX_WAIT_TRIES=20 SYSPRIMS_INDEX_WAIT_SECONDS=0.2

publish() {
	(cd "$repo" && PATH="::bin:../outside:$PATH" "$repo/scripts/release-crates-dry-run.sh" \
		--publish --registry-index "$index" --registry-api "$api" --registry-dl "$dl" "$@")
}
puts() { grep -c '"m": "PUT"' "$registry/requests.log" 2>/dev/null || true; }

# 1. With real Cargo: a mismatched approval checksum gets no token and uploads nothing.
gate_home="$fixture_dir/gate-home"
mkdir -p "$gate_home"
printf '[registries.testreg]\nindex = "%s"\n[registry]\nglobal-credential-providers = ["%s"]\n' \
	"$index" "$prov" >"$gate_home/config.toml"
printf '{"index-url":"%s","name":"sysprims-core","vers":"%s","cksum":"%s"}' \
	"$index" "$version" "$(printf '%064d' 0)" >"$fixture_dir/wrong-approval.json"
if (cd "$pkg_check/tree" && env -i PATH="$PATH" HOME="$gate_home" CARGO_HOME="$gate_home" \
	CARGO_TARGET_DIR="$pkg_check/gate-target" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
	RUSTUP_TOOLCHAIN="$msrv" SYSPRIMS_CARGO_APPROVAL="$fixture_dir/wrong-approval.json" \
	SYSPRIMS_CARGO_TOKEN_FILE="$fixture_dir/token" \
	cargo publish --no-verify --locked --registry testreg -p sysprims-core) >"$fixture_dir/gate.out" 2>&1; then
	echo 'error: cargo published with a mismatched approval' >&2
	exit 1
fi
grep -q 'is not approved' "$fixture_dir/gate.out" || {
	cat "$fixture_dir/gate.out" >&2
	exit 1
}
[[ "$(puts)" == 0 ]] || {
	echo 'error: an upload reached the registry without approval' >&2
	exit 1
}

# 2. Direct environment tokens, missing token file, unconfirmed non-tty runs, and ancestor .cargo/config.toml are refused.
expect_fail_with 'unset CARGO_REGISTRY_TOKEN' bash -c "cd '$repo' && CARGO_REGISTRY_TOKEN=x '$repo/scripts/release-crates-dry-run.sh' --publish --registry-index '$index' --registry-api '$api' --registry-dl '$dl' </dev/null"
expect_fail_with 'SYSPRIMS_CARGO_TOKEN_FILE must name a file' bash -c "cd '$repo' && env -u SYSPRIMS_CARGO_TOKEN_FILE '$repo/scripts/release-crates-dry-run.sh' --publish --registry-index '$index' --registry-api '$api' --registry-dl '$dl' </dev/null"
expect_fail_with 'not confirmed' publish </dev/null
expect_fail_with 'not confirmed' bash -c "cd '$repo' && SYSPRIMS_CRATE_PUBLISH_CONFIRM='sysprims-core@$version:$(printf '%064d' 0)' '$repo/scripts/release-crates-dry-run.sh' --publish --registry-index '$index' --registry-api '$api' --registry-dl '$dl' </dev/null"
mkdir -p "$fixture_dir/.cargo"
printf '[registry]\nglobal-credential-providers = ["cargo:token"]\n' >"$fixture_dir/.cargo/config.toml"
expect_fail_with 'Cargo registry configuration may not name an index or command' publish </dev/null
rm -rf "$fixture_dir/.cargo"
[[ "$(puts)" == 0 ]]

# 3. Real publication through the release provider.
export SYSPRIMS_CRATE_PUBLISH_CONFIRM="sysprims-core@$version:$digest"
export SYSPRIMS_CARGO_PROVIDER_LOG="$fixture_dir/provider.log"
publish </dev/null >"$fixture_dir/out" 2>&1 || {
	cat "$fixture_dir/out" >&2
	exit 1
}
grep -q "sysprims-core $version published and verified ($digest)" "$fixture_dir/out"
[[ "$(puts)" -ge 1 ]]
python3 - "$fixture_dir/provider.log" "$digest" <<'PY'
import json, sys
records = [json.loads(line) for line in open(sys.argv[1], encoding="utf-8")]
ops = [(r["operation"], r["decision"]) for r in records]
assert ops == [("read", "placeholder"), ("publish", "released")], ops
assert records[1]["cksum"] == sys.argv[2], records[1]
PY
python3 - "$registry/requests.log" <<'PY'
import json, sys
for line in open(sys.argv[1], encoding="utf-8"):
    r = json.loads(line)
    if r["m"] == "GET":
        assert r["auth"] in (None, ""), r
    if r["m"] == "PUT":
        assert r["auth"] == "dummy-token", r
PY
cmp "$registry/crates/sysprims-core-$version.crate" "$expected_crate"
[[ ! -e "$fixture_dir/poisoned" ]] || {
	echo 'error: relative PATH entry executed a binary inside the checkout' >&2
	exit 1
}

# 4. Re-running when already published with identical contents skips without uploading again.
puts_before="$(puts)"
out="$(publish </dev/null)"
[[ "$out" == *"already on crates.io with these contents; skipping"* ]]
[[ "$(puts)" == "$puts_before" ]]

# 5. Corrupted download from the registry is refused after upload.
kill "$registry_pid" 2>/dev/null || true
wait "$registry_pid" 2>/dev/null || true
registry2="$fixture_dir/registry2"
mkdir -p "$registry2"
touch "$registry2/corrupt-downloads"
python3 "$fixture_root/scripts/test-registry.py" "$registry2" 0 dummy-token >/dev/null 2>&1 &
registry_pid=$!
for _ in $(seq 1 50); do
	[[ -f "$registry2/ready" ]] && break
	sleep 0.1
done
port="$(cat "$registry2/ready")"
index="sparse+http://127.0.0.1:$port/index/"
api="http://127.0.0.1:$port"
dl="http://127.0.0.1:$port/dl"
expect_fail_with 'published sysprims-core archive differs from the verified one' publish </dev/null

# 6. Yanked version on the registry is refused before any provider call or upload.
kill "$registry_pid" 2>/dev/null || true
wait "$registry_pid" 2>/dev/null || true
registry3="$fixture_dir/registry3"
mkdir -p "$registry3"
printf 'sysprims-core {"name":"sysprims-core","vers":"%s","deps":[],"cksum":"%s","features":{},"yanked":true}\n' \
	"$version" "$digest" >"$registry3/seed"
python3 "$fixture_root/scripts/test-registry.py" "$registry3" 0 dummy-token >/dev/null 2>&1 &
registry_pid=$!
for _ in $(seq 1 50); do
	[[ -f "$registry3/ready" ]] && break
	sleep 0.1
done
port="$(cat "$registry3/ready")"
index="sparse+http://127.0.0.1:$port/index/"
api="http://127.0.0.1:$port"
dl="http://127.0.0.1:$port/dl"
rm -f "$fixture_dir/yanked-provider.log"
export SYSPRIMS_CARGO_PROVIDER_LOG="$fixture_dir/yanked-provider.log"
expect_fail_with "sysprims-core@$version is yanked on crates.io" publish </dev/null
[[ ! -e "$fixture_dir/yanked-provider.log" ]] || {
	echo 'error: a yanked version reached the credential provider' >&2
	exit 1
}

echo '[ok] PDR-0011 Cargo credential-provider and zero-compile publication controls passed'
