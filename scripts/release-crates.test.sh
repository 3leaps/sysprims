#!/usr/bin/env bash
# The ordered crate plan, and the publication guards that refuse to run from
# anywhere but a clean detached checkout of the annotated release tag.
set -euo pipefail
# shellcheck source=scripts/release-test-fixture.sh
source "$(dirname "$0")/release-test-fixture.sh"
python3 - "$fixture_root/scripts" <<'PY'
import importlib.util
import pathlib
import sys
spec = importlib.util.spec_from_file_location('rc', pathlib.Path(sys.argv[1]) / 'release-crates.py')
rc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rc)

def pkg(name, deps=(), publish=None):
    return {'name': name, 'publish': publish, 'dependencies': [{'name': d} for d in deps]}

def manifests(names, flag=True):
    return {n: {'package': {'publish': flag}} for n in names}

order = ['sysprims-core', 'sysprims-signal', 'sysprims-session', 'sysprims-proc', 'sysprims-timeout']
graph = [pkg('sysprims-core'), pkg('sysprims-signal', ['sysprims-core']), pkg('sysprims-session', ['sysprims-core']),
         pkg('sysprims-proc', ['sysprims-core', 'sysprims-signal']),
         pkg('sysprims-timeout', ['sysprims-core', 'sysprims-proc', 'sysprims-signal', 'sysprims-session']),
         pkg('sysprims-cli', ['sysprims-core'], publish=[]), pkg('sysprims-ffi', ['sysprims-core'], publish=[])]
rc.validate(graph, order, manifests(order))

def rejects(packages, names, mans, needle):
    try:
        rc.validate(packages, names, mans)
    except ValueError as error:
        assert needle in str(error), (needle, str(error))
        return
    raise SystemExit(f'expected rejection: {needle}')

rejects(graph, order + ['sysprims-recurrence'], manifests(order), 'not in this workspace: sysprims-recurrence')
swapped = ['sysprims-core', 'sysprims-proc', 'sysprims-signal', 'sysprims-session', 'sysprims-timeout']
rejects(graph, swapped, manifests(swapped), 'sysprims-proc must follow sysprims-signal')
rejects(graph, order[:-1], manifests(order[:-1]), 'missing from the list: sysprims-timeout')
rejects(graph, order, {**manifests(order), 'sysprims-core': {'package': {'publish': False}}}, 'explicitly set publish = true')
leaky = graph[:-1] + [pkg('sysprims-ffi', ['sysprims-core'])]
rejects(leaky, order, manifests(order), 'missing from the list: sysprims-ffi')
internal = [p if p['name'] != 'sysprims-timeout' else pkg('sysprims-timeout', ['sysprims-cli']) for p in graph]
rejects(internal, order, manifests(order), 'depends on unpublished workspace crate sysprims-cli')
with_rec = graph + [pkg('sysprims-recurrence')]
rc.validate(with_rec, order + ['sysprims-recurrence'], manifests(order + ['sysprims-recurrence']))

plan = pathlib.Path(sys.argv[1]).parent / 'config/release/publishable-crates.txt'
names = rc.read_plan(plan)
assert names[:5] == order and names[-1] == 'sysprims-recurrence', names
for bad in ('sysprims-core\n\nsysprims-proc\n', 'sysprims-core\nsysprims-core\n', ' sysprims-core\n', 'other-crate\n'):
    tmp = pathlib.Path(sys.argv[1]).parent / '.release-crates-test-plan'
    try:
        tmp.write_text(bad)
        try:
            rc.read_plan(tmp)
        except ValueError:
            continue
        raise SystemExit(f'expected plan rejection for {bad!r}')
    finally:
        tmp.unlink(missing_ok=True)
PY

# Publication guards (no network: the order check runs after them).
version=1.2.3
repo="$fixture_dir/repo"
fixture_repo "$repo" "$version" publish 0
cp "$fixture_root/scripts/release-crates-dry-run.sh" "$repo/scripts/"
fixture_commit "$repo"
dry() { (cd "$repo" && env "$@" ./scripts/release-crates-dry-run.sh); }
expect_fail_with 'SYSPRIMS_RELEASE_TAG=vX.Y.Z is required' dry -u SYSPRIMS_RELEASE_TAG
expect_fail_with 'does not match VERSION' dry SYSPRIMS_RELEASE_TAG=v9.9.9
git -C "$repo" tag -a -m "Release $version" "v$version"
expect_fail_with 'HEAD must be detached' dry SYSPRIMS_RELEASE_TAG="v$version"
git -C "$repo" checkout -q --detach HEAD
git -C "$repo" commit -q --allow-empty -m later
expect_fail_with 'HEAD is not the release tag commit' dry SYSPRIMS_RELEASE_TAG="v$version"
git -C "$repo" checkout -q --detach "v$version"
git -C "$repo" tag -d "v$version" >/dev/null
git -C "$repo" tag "v$version"
expect_fail_with 'must be an annotated tag' dry SYSPRIMS_RELEASE_TAG="v$version"
git -C "$repo" tag -d "v$version" >/dev/null
git -C "$repo" tag -a -m "Release $version" "v$version"
touch "$repo/dirty"
expect_fail_with 'clean checkout required' dry SYSPRIMS_RELEASE_TAG="v$version"
rm "$repo/dirty"
expect_fail_with 'never CI' bash -c "cd '$repo' && CI=true SYSPRIMS_RELEASE_TAG=v$version ./scripts/release-crates-dry-run.sh --publish"
# The publish path reads crates.io directly and treats a crate as published
# only when the index checksum equals the locally packaged crate; there is no
# alternate lookup command it would honour.
if grep -nE 'SYSPRIMS_CARGO_INFO|cargo info' "$fixture_root/scripts/release-crates-dry-run.sh"; then
	echo 'error: the publish path must not use a substitutable index lookup' >&2
	exit 1
fi
# shellcheck disable=SC2016 # Literal text matched in the script under test.
grep -q 'die "$crate@$version is on crates.io with different contents' "$fixture_root/scripts/release-crates-dry-run.sh"
sum="$(printf 'a%.0s' $(seq 64))"
[[ "$(printf '{"version":{"crate":"sysprims-core","num":"0.3.0","checksum":"%s"}}' "$sum" |
	python3 "$fixture_root/scripts/release-crates.py" registry-checksum sysprims-core 0.3.0)" == "$sum" ]]
for body in '{"version":{"crate":"other","num":"0.3.0","checksum":"'"$sum"'"}}' \
	'{"version":{"crate":"sysprims-core","num":"0.2.9","checksum":"'"$sum"'"}}' \
	'{"version":{"crate":"sysprims-core","num":"0.3.0","checksum":"short"}}' \
	'{"version":{"crate":"sysprims-core","num":"0.3.0"}}' 'not json'; do
	expect_fail bash -c "printf '%s' '$body' | python3 '$fixture_root/scripts/release-crates.py' registry-checksum sysprims-core 0.3.0"
done
# A failing plan producer stops the run before any crate is touched.
grep -q "plan_output=\"\$(python3 \"\$root/scripts/release-crates.py\" paths)\" || die" "$fixture_root/scripts/release-crates-dry-run.sh"
# Behaviour on the publish path, with the network and cargo stubbed on PATH:
# an index checksum equal to the local crate skips; any other checksum stops.
pub="$fixture_dir/pub"
fixture_repo "$pub" "$version" publish 0
cp "$fixture_root/scripts/release-crates-dry-run.sh" "$fixture_root/scripts/release-crates.py" "$pub/scripts/"
printf 'sysprims-core\n' >"$pub/config/release/publishable-crates.txt"
mkdir -p "$pub/crates/sysprims-core"
printf '[package]\nname = "sysprims-core"\npublish = true\n' >"$pub/crates/sysprims-core/Cargo.toml"
printf 'target/\n' >"$pub/.gitignore"
fixture_commit "$pub"
git -C "$pub" tag -a -m "Release $version" "v$version"
git -C "$pub" checkout -q --detach "v$version"
bin="$fixture_dir/pub-bin"
mkdir -p "$bin"
printf 'crate bytes\n' >"$fixture_dir/core.crate"
local_sum="$(shasum -a 256 "$fixture_dir/core.crate" | awk '{print $1}')"
cat >"$bin/cargo" <<SH
#!/usr/bin/env bash
case "\$1" in
metadata) printf '{"packages":[{"name":"sysprims-core","publish":null,"dependencies":[],"manifest_path":"$pub/crates/sysprims-core/Cargo.toml"}]}' ;;
package) mkdir -p "$pub/target/package" && cp "$fixture_dir/core.crate" "$pub/target/package/sysprims-core-$version.crate" ;;
publish) echo "UPLOAD \$*" >>"$fixture_dir/uploads"; exit 0 ;;
*) exit 1 ;;
esac
SH
cat >"$bin/curl" <<SH
#!/usr/bin/env bash
printf '{"version":{"crate":"sysprims-core","num":"$version","checksum":"%s"}}\n200' "\$(cat "$fixture_dir/index-sum")"
SH
chmod +x "$bin/cargo" "$bin/curl"
publish_run() { (cd "$pub" && env PATH="$bin:$PATH" SYSPRIMS_RELEASE_TAG="v$version" ./scripts/release-crates-dry-run.sh --publish); }
printf '%s' "$local_sum" >"$fixture_dir/index-sum"
out="$(publish_run)"
[[ "$out" == *"already on crates.io with these contents; skipping"* ]]
printf '%s' "$(printf 'b%.0s' $(seq 64))" >"$fixture_dir/index-sum"
expect_fail_with 'on crates.io with different contents' publish_run
[[ ! -e "$fixture_dir/uploads" ]]
echo '[ok] crate plan and publication guard controls passed'
