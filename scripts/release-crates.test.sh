#!/usr/bin/env bash
# The ordered crate plan, and the publication guards that refuse to run from
# anywhere but a clean detached checkout of the annotated release tag.
set -euo pipefail
# shellcheck source=scripts/release-test-fixture.sh
source "$(dirname "$0")/release-test-fixture.sh"
PYTHONDONTWRITEBYTECODE=1 python3 - "$fixture_root/scripts" <<'PY'
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
expect_fail_with 'canonical vX.Y.Z' dry SYSPRIMS_RELEASE_TAG=9.9.9
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
for body in 'does not describe|{"version":{"crate":"other","num":"0.3.0","checksum":"'"$sum"'"}}' \
	'does not describe|{"version":{"crate":"sysprims-core","num":"0.2.9","checksum":"'"$sum"'"}}' \
	'no valid checksum|{"version":{"crate":"sysprims-core","num":"0.3.0","checksum":"short"}}' \
	'no valid checksum|{"version":{"crate":"sysprims-core","num":"0.3.0"}}' 'Expecting value|not json'; do
	reason="${body%%|*}"
	body="${body#*|}"
	expect_fail_with "$reason" bash -c "printf '%s' '$body' | python3 '$fixture_root/scripts/release-crates.py' registry-checksum sysprims-core 0.3.0"
done
# A failing plan producer stops the run before any crate is touched.
grep -q "plan_output=\"\$(python3 \"\$tree/scripts/release-crates.py\" paths)\" || die" "$fixture_root/scripts/release-crates-dry-run.sh"
# Behaviour on the publish path, with the network and cargo stubbed on PATH.
# The cargo stub behaves like cargo where it matters: it packages the crate
# source it finds in the current directory and writes the .crate under
# ${CARGO_TARGET_DIR:-./target}/package. Whatever bytes it packages are the
# bytes published, so the tests can see which tree and which path were used.
pub="$fixture_dir/pub"
fixture_repo "$pub" "$version" publish 0
cp "$fixture_root/scripts/release-crates-dry-run.sh" "$fixture_root/scripts/release-crates.py" \
	"$fixture_root/scripts/release-export-tree.py" "$pub/scripts/"
printf 'sysprims-core\n' >"$pub/config/release/publishable-crates.txt"
mkdir -p "$pub/crates/sysprims-core/src"
printf '[package]\nname = "sysprims-core"\npublish = true\n' >"$pub/crates/sysprims-core/Cargo.toml"
printf 'pub fn tagged() {}\n' >"$pub/crates/sysprims-core/src/lib.rs"
printf 'target/\n/.variant/\n' >"$pub/.gitignore"
fixture_commit "$pub"
git -C "$pub" tag -a -m "Release $version" "v$version"
git -C "$pub" checkout -q --detach "v$version"
bin="$fixture_dir/pub-bin"
mkdir -p "$bin"
# A deterministic stand-in for cargo's .crate: gzip'd tar of the crate's
# sources plus .cargo_vcs_info.json (commit from the current git HEAD, the
# crate's path), refusing a dirty tree as cargo does without --allow-dirty.
cat >"$fixture_dir/fake-crate.py" <<'PY'
import gzip, io, json, subprocess, sys, tarfile
root, out, version, source = sys.argv[1:5]
def blob(path):
    if source == "tree":
        return open(f"{root}/{path}", "rb").read()
    return subprocess.run(["git", "-C", root, "cat-file", "blob", f"{source}:{path}"], capture_output=True, check=True).stdout
rev = "HEAD" if source == "tree" else source
commit = subprocess.run(["git", "-C", root, "rev-parse", rev], capture_output=True, text=True, check=True).stdout.strip()
if source == "tree" and subprocess.run(["git", "-C", root, "status", "--porcelain"], capture_output=True, text=True).stdout:
    sys.exit("error: files in the working directory contain changes that were not yet committed into git")
files = {
    "Cargo.toml": blob("crates/sysprims-core/Cargo.toml"),
    "src/lib.rs": blob("crates/sysprims-core/src/lib.rs"),
    ".cargo_vcs_info.json": json.dumps({"git": {"sha1": __import__("os").environ.get("FAKE_VCS_SHA", commit)},
                                        "path_in_vcs": __import__("os").environ.get("FAKE_VCS_PATH", "crates/sysprims-core")}).encode(),
}
raw = io.BytesIO()
with tarfile.open(fileobj=raw, mode="w", format=tarfile.USTAR_FORMAT) as archive:
    for name, data in sorted(files.items()):
        info = tarfile.TarInfo(f"sysprims-core-{version}/{name}")
        info.size, info.mtime, info.mode = len(data), 0, 0o644
        archive.addfile(info, io.BytesIO(data))
with open(out, "wb") as handle:
    with gzip.GzipFile(fileobj=handle, mode="wb", mtime=0, filename="") as gz:
        gz.write(raw.getvalue())
PY
cat >"$bin/cargo" <<'SH'
#!/usr/bin/env bash
[[ -z "${CARGO_CALLS:-}" ]] || echo "$1" >>"$CARGO_CALLS"
case "$1" in
metadata) printf '{"packages":[{"name":"sysprims-core","publish":null,"dependencies":[],"manifest_path":"%s/crates/sysprims-core/Cargo.toml"}]}' "$PWD" ;;
package)
	out="${CARGO_TARGET_DIR:-$PWD/target}/package"
	mkdir -p "$out"
	python3 "$FAKE_CRATE" "$PWD" "$out/sysprims-core-$(cat VERSION).crate" "$(cat VERSION)" tree
	;;
publish) echo "UPLOAD $PWD" >>"$UPLOAD_LOG" ;;
*) exit 1 ;;
esac
SH
cat >"$bin/curl" <<SH
#!/usr/bin/env bash
printf '{"version":{"crate":"sysprims-core","num":"$version","checksum":"%s"}}\n200' "\$(cat "$fixture_dir/index-sum")"
SH
chmod +x "$bin/cargo" "$bin/curl"
# The checksum of the crate built from the tagged commit's own bytes.
tag_sum="$(python3 "$fixture_dir/fake-crate.py" "$pub" "$fixture_dir/expected.crate" "$version" "v$version^{commit}" &&
	shasum -a 256 "$fixture_dir/expected.crate" | awk '{print $1}')"
publish_run() {
	(cd "$pub" && env PATH="$bin:$PATH" FAKE_CRATE="$fixture_dir/fake-crate.py" UPLOAD_LOG="$fixture_dir/uploads" SYSPRIMS_RELEASE_TAG="v$version" "$@" \
		./scripts/release-crates-dry-run.sh --publish)
}
printf '%s' "$tag_sum" >"$fixture_dir/index-sum"
out="$(publish_run)"
[[ "$out" == *"already on crates.io with these contents; skipping"* ]]
printf '%s' "$(printf 'b%.0s' $(seq 64))" >"$fixture_dir/index-sum"
expect_fail_with 'on crates.io with different contents' publish_run
[[ ! -e "$fixture_dir/uploads" ]]
printf '%s' "$tag_sum" >"$fixture_dir/index-sum"

# A packaged crate that records another commit or another path is refused
# before its checksum is trusted.
expect_fail_with 'does not record the tagged commit and its path' publish_run FAKE_VCS_SHA="$(printf 'a%.0s' $(seq 40))"
expect_fail_with 'does not record the tagged commit and its path' publish_run FAKE_VCS_PATH=sysprims-core

# Hidden working-tree edits are refused before packaging.
lib="crates/sysprims-core/src/lib.rs"
git -C "$pub" update-index --skip-worktree "$lib"
printf 'pub fn tampered() {}\n' >"$pub/$lib"
[[ -z "$(git -C "$pub" status --porcelain)" ]]
expect_fail_with 'skip-worktree or assume-unchanged' publish_run
git -C "$pub" update-index --no-skip-worktree "$lib"
git -C "$pub" checkout -q -- "$lib"
git -C "$pub" update-index --assume-unchanged "$lib"
printf 'pub fn tampered() {}\n' >"$pub/$lib"
[[ -z "$(git -C "$pub" status --porcelain)" ]]
expect_fail_with 'skip-worktree or assume-unchanged' publish_run
git -C "$pub" update-index --no-assume-unchanged "$lib"
git -C "$pub" checkout -q -- "$lib"

# The packaging layer on its own: a test-local copy of the script without the
# index-flag refusal still publishes the tag's bytes from its blob export,
# so a hidden edit cannot reach the package even if that refusal were gone.
# (In an ignored directory one level down, so the script finds this repository.)
mkdir -p "$pub/.variant"
sed '/index entries hide working-tree changes/d' "$pub/scripts/release-crates-dry-run.sh" >"$pub/.variant/no-flag-check.sh"
chmod +x "$pub/.variant/no-flag-check.sh"
git -C "$pub" update-index --skip-worktree "$lib"
printf 'pub fn tampered() {}\n' >"$pub/$lib"
printf '%s' "$tag_sum" >"$fixture_dir/index-sum"
out="$(cd "$pub" && env PATH="$bin:$PATH" FAKE_CRATE="$fixture_dir/fake-crate.py" UPLOAD_LOG="$fixture_dir/uploads" SYSPRIMS_RELEASE_TAG="v$version" \
	./.variant/no-flag-check.sh --publish)"
[[ "$out" == *"already on crates.io with these contents; skipping"* ]]
git -C "$pub" update-index --no-skip-worktree "$lib"
git -C "$pub" checkout -q -- "$lib"

# A caller's CARGO_TARGET_DIR, and a stale .crate at the default path whose
# checksum matches the index, are both ignored: the hash is of the crate just
# packaged from the tagged commit into a fresh directory.
printf '%s' "$(printf 'c%.0s' $(seq 64))" >"$fixture_dir/index-sum"
mkdir -p "$pub/target/package" "$fixture_dir/caller-target/package"
printf 'stale\n' >"$pub/target/package/sysprims-core-$version.crate"
printf 'stale\n' >"$fixture_dir/caller-target/package/sysprims-core-$version.crate"
stale_sum="$(shasum -a 256 "$pub/target/package/sysprims-core-$version.crate" | awk '{print $1}')"
printf '%s' "$stale_sum" >"$fixture_dir/index-sum"
expect_fail_with 'on crates.io with different contents' publish_run CARGO_TARGET_DIR="$fixture_dir/caller-target"
expect_fail_with 'on crates.io with different contents' publish_run CARGO_BUILD_TARGET_DIR="$fixture_dir/caller-target"
expect_fail_with 'on crates.io with different contents' publish_run
printf '%s' "$tag_sum" >"$fixture_dir/index-sum"
publish_run CARGO_TARGET_DIR="$fixture_dir/caller-target" >/dev/null
# Checkout settings never change the packaged bytes: with autocrlf on, an
# eol=crlf attribute, export-ignore/export-subst attributes and a sparse
# checkout configured, publication still packages the tag's exact blobs (the
# index checksum of the tag's bytes matches) or refuses; it never packages
# different bytes.
printf '%s' "$tag_sum" >"$fixture_dir/index-sum"
git -C "$pub" checkout -q main
printf 'crates/** text eol=crlf\ncrates/sysprims-core/src/lib.rs export-subst\ncrates/sysprims-core/Cargo.toml export-ignore\n' >"$pub/.gitattributes"
# shellcheck disable=SC2016 # A literal export-subst marker.
printf '$Format:%%H$ pub fn tagged() {}\n' >"$pub/crates/sysprims-core/src/lib.rs"
fixture_commit "$pub" attributes
git -C "$pub" tag -d "v$version" >/dev/null
git -C "$pub" tag -a -m "Release $version" "v$version"
git -C "$pub" checkout -q --detach "v$version"
tag_sum="$(python3 "$fixture_dir/fake-crate.py" "$pub" "$fixture_dir/expected.crate" "$version" "v$version^{commit}" &&
	shasum -a 256 "$fixture_dir/expected.crate" | awk '{print $1}')"
printf '%s' "$tag_sum" >"$fixture_dir/index-sum"
for setting in plain autocrlf sparse; do
	case "$setting" in
	autocrlf) git -C "$pub" config core.autocrlf true ;;
	sparse)
		git -C "$pub" config core.autocrlf false
		git -C "$pub" sparse-checkout set --no-cone /config /scripts /VERSION /docs /.gitignore /.gitattributes
		;;
	esac
	out="$(publish_run 2>&1)" || {
		[[ "$out" == *"cannot export the tagged commit"* || "$out" == *"skip-worktree or assume-unchanged"* ]] || {
			echo "error: publication under $setting failed for an unexpected reason: $out" >&2
			exit 1
		}
		continue
	}
	[[ "$out" == *"already on crates.io with these contents; skipping"* ]] || {
		echo "error: publication under $setting packaged bytes other than the tag's: $out" >&2
		exit 1
	}
done
git -C "$pub" sparse-checkout disable
# The exporter itself: a tampered export (an extra, a missing or a changed
# file) is refused by its verification step.
exported="$fixture_dir/exported"
python3 "$fixture_root/scripts/release-export-tree.py" "$pub" "v$version" "$exported"
cmp <(git -C "$pub" cat-file blob "v$version:crates/sysprims-core/src/lib.rs") "$exported/crates/sysprims-core/src/lib.rs"
grep -q 'Format:%H' "$exported/crates/sysprims-core/src/lib.rs"
[[ -f "$exported/crates/sysprims-core/Cargo.toml" ]]
expect_fail_with 'already exists' python3 "$fixture_root/scripts/release-export-tree.py" "$pub" "v$version" "$exported"
PYTHONDONTWRITEBYTECODE=1 python3 - "$fixture_root/scripts" "$pub" "v$version" "$exported" <<'PY'
import importlib.util, pathlib, sys
spec = importlib.util.spec_from_file_location("ex", pathlib.Path(sys.argv[1]) / "release-export-tree.py")
ex = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ex)
repo, commit, root = sys.argv[2], sys.argv[3], pathlib.Path(sys.argv[4])
entries = ex.tree_entries(repo, commit)
(git_dir,) = [p.resolve() for p in root.parent.glob(f"{root.name}.gitdir.*")]
def refused(change, undo, needle):
    change()
    try:
        ex.verify(repo, entries, root, git_dir)
    except SystemExit as error:
        assert needle in str(error), (needle, str(error))
    else:
        raise SystemExit(f"expected refusal: {needle}")
    finally:
        undo()
lib = root / "crates/sysprims-core/src/lib.rs"
original = lib.read_bytes()
refused(lambda: lib.write_bytes(original.replace(b"\n", b"\r\n")), lambda: lib.write_bytes(original), "differs from its blob")
extra = root / "crates/sysprims-core/src/extra.rs"
refused(lambda: extra.write_text("x"), lambda: extra.unlink(), "file set differs")
refused(lambda: lib.rename(root / "moved"), lambda: (root / "moved").rename(lib), "file set differs")
refused(lambda: lib.chmod(0o755), lambda: lib.chmod(0o644), "wrong mode")
# Defence in depth: a symlink in the export, to a file or a directory, is
# refused even if it somehow got there.
link = root / "crates/sysprims-core/src/link.rs"
refused(lambda: link.symlink_to("lib.rs"), lambda: link.unlink(), "contains a symlink")
up = root / "crates/sysprims-core/up"
refused(lambda: up.symlink_to(".."), lambda: up.unlink(), "contains a symlink")
# The same predicate applies to what is found in the export, at any depth.
for name in (".Git", ".git.", "git~1", "a:b", "\u200c.git"):
    planted = root / "crates/sysprims-core" / name
    refused(lambda: planted.write_text("gitdir: /elsewhere\n"), lambda: planted.unlink(), "contains a git path")
# The export's .git must be exactly the gitfile this run wrote.
gitfile = root / ".git"
written = gitfile.read_bytes()
refused(lambda: gitfile.write_bytes(b"gitdir: /elsewhere\n"), lambda: gitfile.write_bytes(written), "not the gitfile this run wrote")
refused(lambda: gitfile.unlink(), lambda: gitfile.write_bytes(written), "not the gitfile this run wrote")
ex.verify(repo, entries, root, git_dir)
PY
# Symlinks in the tagged tree are refused before anything is written, whatever
# they point at: cargo would follow them to bytes no blob hash covers.
links="$fixture_dir/links"
for kind in relative-escape absolute dir dangling chain; do
	rm -rf "$links" "$fixture_dir/links-export"
	git init -q "$links"
	mkdir -p "$links/crates/sysprims-core/src"
	printf 'pub fn tagged() {}\n' >"$links/crates/sysprims-core/src/lib.rs"
	case "$kind" in
	relative-escape) ln -s ../../../../outside "$links/crates/sysprims-core/src/extra.rs" ;;
	absolute) ln -s /etc/hosts "$links/crates/sysprims-core/src/extra.rs" ;;
	dir) ln -s ../../.. "$links/crates/sysprims-core/src/up" ;;
	dangling) ln -s missing.rs "$links/crates/sysprims-core/src/extra.rs" ;;
	chain)
		ln -s lib.rs "$links/crates/sysprims-core/src/one.rs"
		ln -s one.rs "$links/crates/sysprims-core/src/two.rs"
		;;
	esac
	git -C "$links" add -A
	git -C "$links" -c user.name=t -c user.email=t@example.invalid commit -q -m "$kind"
	[[ "$(git -C "$links" ls-tree -r HEAD | grep -c '^120000 ')" -ge 1 ]]
	expect_fail_with 'unsupported tree entry 120000' python3 "$fixture_root/scripts/release-export-tree.py" "$links" HEAD "$fixture_dir/links-export"
	[[ ! -e "$fixture_dir/links-export/crates" ]]
done
# On the publish path the same refusal stops the run before cargo is called.
git -C "$pub" checkout -q main
ln -s /etc/hosts "$pub/crates/sysprims-core/src/hosts.rs"
fixture_commit "$pub" link
git -C "$pub" tag -d "v$version" >/dev/null
git -C "$pub" tag -a -m "Release $version" "v$version"
git -C "$pub" checkout -q --detach "v$version"
rm -f "$fixture_dir/cargo-calls"
expect_fail_with 'unsupported tree entry 120000' publish_run CARGO_CALLS="$fixture_dir/cargo-calls"
[[ ! -e "$fixture_dir/cargo-calls" && ! -e "$fixture_dir/uploads" ]]
git -C "$pub" checkout -q main
git -C "$pub" rm -q "crates/sysprims-core/src/hosts.rs"
fixture_commit "$pub" unlink
git -C "$pub" tag -d "v$version" >/dev/null
git -C "$pub" tag -a -m "Release $version" "v$version"
git -C "$pub" checkout -q --detach "v$version"
# A tree path that a filesystem may resolve to `.git` is refused before
# anything is written, and the repository it points at is left untouched:
# its HEAD, refs and config are compared before and after. The crafted trees
# are built with mktree, since git add refuses these paths.
# Test-local exporter variants live beside the helper they call.
variants="$fixture_dir/variants"
mkdir -p "$variants"
cp "$fixture_root/scripts/write-exclusive.py" "$variants/"
victim="$fixture_dir/victim"
git init -q "$victim"
git -C "$victim" -c user.name=t -c user.email=t@example.invalid commit -q --allow-empty -m victim
victim_state() { (cd "$victim/.git" && cat HEAD config && git -C "$victim" for-each-ref && ls && shasum index); }
victim_state >"$fixture_dir/victim.before"
crafted="$fixture_dir/crafted"
git init -q "$crafted"
lib_blob="$(printf 'pub fn tagged() {}\n' | git -C "$crafted" hash-object -w --stdin)"
gitfile_blob="$(printf 'gitdir: %s/.git\n' "$victim" | git -C "$crafted" hash-object -w --stdin)"
# crafted_commit <tree line or empty>: a commit holding lib.rs and that entry.
crafted_commit() {
	local tree
	tree="$({
		printf '100644 blob %s\tlib.rs\n' "$lib_blob"
		[[ -z "$1" ]] || printf '%s\n' "$1"
	} | git -C "$crafted" mktree)"
	git -C "$crafted" -c user.name=t -c user.email=t@example.invalid commit-tree -m crafted "$tree"
}
# crates/sysprims-core/.git, a gitfile beside a crate: cargo would find it
# before the export root's, walking up from the crate.
crate_tree="$(printf '100644 blob %s\t.git\n100644 blob %s\tCargo.toml\n' "$gitfile_blob" "$lib_blob" | git -C "$crafted" mktree)"
crates_tree="$(printf '040000 tree %s\tsysprims-core\n' "$crate_tree" | git -C "$crafted" mktree)"
# One fixture per class: case, ZWNJ, ZWJ, BOM, compatibility forms that only
# NFKC folds (full-width g, superscript t), trailing dot, trailing space,
# both, an 8.3 short name, and an NTFS stream.
for entry in ".git" ".Git" \
	".g"$'\xe2\x80\x8c'"it" ".g"$'\xe2\x80\x8d'"it" $'\xef\xbb\xbf'".git" \
	"."$'\xef\xbd\x87'"it" ".gi"$'\xe1\xb5\x97' \
	".git." ".git " ".GIT. ." "git~1" "GIT~12" ".git::\$INDEX_ALLOCATION"; do
	rm -rf "$fixture_dir/crafted-export" "$fixture_dir"/crafted-export.gitdir.*
	expect_fail_with 'names a git directory' python3 "$fixture_root/scripts/release-export-tree.py" "$crafted" \
		"$(crafted_commit "100644 blob $gitfile_blob	$entry")" "$fixture_dir/crafted-export"
	[[ ! -e "$fixture_dir/crafted-export" ]] && ! compgen -G "$fixture_dir/crafted-export.gitdir.*" >/dev/null
	victim_state | cmp - "$fixture_dir/victim.before" || {
		echo "error: exporting a tree with $(printf %q "$entry") changed another repository" >&2
		exit 1
	}
done
# Names that only resemble .git are exported normally.
rm -rf "$fixture_dir/crafted-export"
python3 "$fixture_root/scripts/release-export-tree.py" "$crafted" \
	"$(crafted_commit "100644 blob $lib_blob	.gitignore")" "$fixture_dir/crafted-export"
[[ -f "$fixture_dir/crafted-export/.gitignore" ]]
rm -rf "$fixture_dir/crafted-export"
expect_fail_with "names a git directory: 'crates/sysprims-core/.git'" python3 "$fixture_root/scripts/release-export-tree.py" \
	"$crafted" "$(crafted_commit "040000 tree $crates_tree	crates")" "$fixture_dir/crafted-export"
[[ ! -e "$fixture_dir/crafted-export" ]]
victim_state | cmp - "$fixture_dir/victim.before" || {
	echo 'error: exporting a crafted tree changed another repository' >&2
	exit 1
}
# Past the tree check (a test-local exporter without it), the gitfile is
# still never used: the export scan refuses it before any git command runs.
sed 's/^        if any(names_git_dir(part) for part in name.split("\/")):$/        if False:/' \
	"$fixture_root/scripts/release-export-tree.py" >"$variants/no-name-check.py"
grep -q '^        if False:$' "$variants/no-name-check.py"
rm -rf "$fixture_dir/crafted-export" "$fixture_dir"/crafted-export.gitdir.*
expect_fail_with 'export contains a git path: .git' python3 "$variants/no-name-check.py" "$crafted" \
	"$(crafted_commit "100644 blob $gitfile_blob	.git")" "$fixture_dir/crafted-export"
victim_state | cmp - "$fixture_dir/victim.before"
# With both name checks gone, git still runs only on the exporter's own git
# directory (explicit GIT_DIR, nothing discovered), which refuses the path
# when it builds the index; the victim is untouched.
sed 's/^def names_git_dir(component):$/def names_git_dir(component):\n    return False/' \
	"$fixture_root/scripts/release-export-tree.py" >"$variants/no-git-names.py"
grep -q '^    return False$' "$variants/no-git-names.py"
rm -rf "$fixture_dir/crafted-export" "$fixture_dir"/crafted-export.gitdir.*
expect_fail_with "invalid path '.git'" python3 "$variants/no-git-names.py" "$crafted" \
	"$(crafted_commit "100644 blob $gitfile_blob	.git")" "$fixture_dir/crafted-export"
victim_state | cmp - "$fixture_dir/victim.before"
# A `.git` directory symlink placed in the export is refused by the symlink
# scan, which sees `.git` before anything is skipped.
PYTHONDONTWRITEBYTECODE=1 python3 - "$fixture_root/scripts" "$crafted" "$(crafted_commit "")" "$fixture_dir/linked" "$victim/.git" <<'PY'
import importlib.util, os, pathlib, sys
spec = importlib.util.spec_from_file_location("ex", pathlib.Path(sys.argv[1]) / "release-export-tree.py")
ex = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ex)
repo, commit, root, victim = sys.argv[2], sys.argv[3], pathlib.Path(sys.argv[4]), sys.argv[5]
root.mkdir()
(root / "lib.rs").write_text("pub fn tagged() {}\n")
os.symlink(victim, root / ".git")
try:
    ex.verify(repo, ex.tree_entries(repo, commit), root)
except SystemExit as error:
    assert "contains a symlink: .git" in str(error), str(error)
else:
    raise SystemExit("expected the .git symlink to be refused")
PY
victim_state | cmp - "$fixture_dir/victim.before"
# The attach never follows the export: with a `.git` symlink planted after
# verification, every git command still runs on the exporter's own directory,
# and its gitfile cannot be created over the link.
sed 's/^    git_dir = attach_commit(repo, commit, root)$/    os.symlink(sys.argv[4], root \/ ".git"); git_dir = attach_commit(repo, commit, root)/' \
	"$fixture_root/scripts/release-export-tree.py" >"$variants/planted.py"
grep -q 'os.symlink(sys.argv\[4\]' "$variants/planted.py"
sed -i.bak 's/^    repo, commit, dest = sys.argv\[1:\]$/    repo, commit, dest = sys.argv[1:4]/; s/^    if len(sys.argv) != 4:$/    if len(sys.argv) != 5:/' "$variants/planted.py"
rm -rf "$fixture_dir/crafted-export" "$fixture_dir"/crafted-export.gitdir.*
expect_fail_with 'File exists|exists' python3 "$variants/planted.py" "$crafted" "$(crafted_commit "")" "$fixture_dir/crafted-export" "$victim/.git"
victim_state | cmp - "$fixture_dir/victim.before" || {
	echo 'error: the attach step acted on a repository named by the export' >&2
	exit 1
}
# Inherited git variables steer nothing: with GIT_DIR, GIT_WORK_TREE,
# GIT_INDEX_FILE and a GIT_CONFIG_* override aimed at the victim, the export
# reads the crafted repository, attaches to its own git directory, and leaves
# the victim untouched.
rm -rf "$fixture_dir/crafted-export"
env GIT_DIR="$victim/.git" GIT_WORK_TREE="$victim" GIT_INDEX_FILE="$victim/.git/index" \
	GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0=/nonexistent \
	python3 "$fixture_root/scripts/release-export-tree.py" "$crafted" "$(crafted_commit "")" "$fixture_dir/crafted-export"
cmp <(printf 'pub fn tagged() {}\n') "$fixture_dir/crafted-export/lib.rs"
[[ "$(git -C "$fixture_dir/crafted-export" rev-parse --absolute-git-dir)" == "$fixture_dir"/crafted-export.gitdir.* ]]
victim_state | cmp - "$fixture_dir/victim.before" || {
	echo 'error: an inherited git variable steered the export into another repository' >&2
	exit 1
}
# On the publish path: an inherited GIT_DIR/GIT_INDEX_FILE aimed at the victim
# steers nothing (the run still packages the tag's bytes and the victim is
# untouched), and an export whose gitfile names any other directory is
# refused before cargo is called.
tag_sum="$(python3 "$fixture_dir/fake-crate.py" "$pub" "$fixture_dir/expected.crate" "$version" "v$version^{commit}" &&
	shasum -a 256 "$fixture_dir/expected.crate" | awk '{print $1}')"
printf '%s' "$tag_sum" >"$fixture_dir/index-sum"
out="$(publish_run GIT_DIR="$victim/.git" GIT_WORK_TREE="$victim" GIT_INDEX_FILE="$victim/.git/index")"
[[ "$out" == *"already on crates.io with these contents; skipping"* ]]
victim_state | cmp - "$fixture_dir/victim.before" || {
	echo 'error: an inherited git variable steered the publish path into another repository' >&2
	exit 1
}
# (The variants live in the ignored .variant directory, so the checkout stays
# clean; the dry-run copy calls the variant exporter, which writes a gitfile
# naming the victim after its own checks.)
mkdir -p "$pub/.variant"
cp "$pub/scripts/write-exclusive.py" "$pub/.variant/"
python3 - "$pub/scripts/release-export-tree.py" "$pub/.variant/foreign-gitfile.py" "$victim/.git" <<'PY'
import sys
text = open(sys.argv[1]).read()
for old, new in [
    ("    return git_dir\n", f"    (root / '.git').write_text('gitdir: {sys.argv[3]}\\n')\n    return git_dir\n"),
    ("    verify(repo, entries, root, git_dir)\n", ""),
]:
    assert text.count(old) == 1, old
    text = text.replace(old, new)
open(sys.argv[2], "w").write(text)
PY
# shellcheck disable=SC2016 # Literal script text substituted.
sed 's|"$root/scripts/release-export-tree.py"|"$root/.variant/foreign-gitfile.py"|' "$pub/scripts/release-crates-dry-run.sh" \
	>"$pub/.variant/foreign-gitfile.sh"
grep -q 'variant/foreign-gitfile.py' "$pub/.variant/foreign-gitfile.sh"
chmod +x "$pub/.variant/foreign-gitfile.sh"
rm -f "$fixture_dir/cargo-calls"
expect_fail_with "gitfile does not name this run's git directory" bash -c "cd '$pub' && env PATH='$bin:$PATH' \
	CARGO_CALLS='$fixture_dir/cargo-calls' SYSPRIMS_RELEASE_TAG=v$version ./.variant/foreign-gitfile.sh --publish"
[[ ! -e "$fixture_dir/cargo-calls" ]]
victim_state | cmp - "$fixture_dir/victim.before"
# Real cargo, offline, on a tiny crate: the export is attached to the tagged
# commit, so cargo records it in .cargo_vcs_info.json without --allow-dirty,
# and refuses to package if an exported byte differs from its blob.
# No cargo package or publish invocation carries --allow-dirty. (Options come
# before the path: BSD grep treats later arguments as files.)
allow_dirty() {
	grep -rnE --include='*.sh' --include='*.py' --include='*.mjs' \
		'cargo[[:space:]]+(package|publish)[^#]*--allow-dirty|"--allow-dirty"' "$1" | grep -v '\.test\.sh:'
}
if allow_dirty "$fixture_root/scripts"; then
	echo 'error: the package or publish path passes --allow-dirty' >&2
	exit 1
fi
mkdir -p "$fixture_dir/dirty-probe"
printf 'cargo package --locked --allow-dirty -p x\n' >"$fixture_dir/dirty-probe/probe.sh"
allow_dirty "$fixture_dir/dirty-probe" >/dev/null || {
	echo 'error: the --allow-dirty check does not detect a planted use' >&2
	exit 1
}
real="$fixture_dir/real"
fixture_repo "$real" "$version" publish 0
cp "$fixture_root/scripts/"{release-export-tree.py,release-crates.py,write-exclusive.py} "$real/scripts/"
mkdir -p "$real/crates/sysprims-core/src"
printf '[workspace]\nmembers = ["crates/sysprims-core"]\nresolver = "2"\n' >"$real/Cargo.toml"
printf '[package]\nname = "sysprims-core"\nversion = "%s"\nedition = "2021"\nlicense = "MIT"\ndescription = "fixture"\npublish = true\n' "$version" \
	>"$real/crates/sysprims-core/Cargo.toml"
printf 'pub fn tagged() {}\n' >"$real/crates/sysprims-core/src/lib.rs"
printf 'target/\n' >"$real/.gitignore"
(cd "$real" && cargo generate-lockfile --offline -q)
fixture_commit "$real" crate
commit="$(git -C "$real" rev-parse HEAD)"
export_and_package() {
	local tree="$fixture_dir/export-$1"
	rm -rf "$tree" "$tree".gitdir.* "$fixture_dir/target-$1"
	# Explicit returns: this runs under expect_fail, where errexit is off.
	python3 "$2" "$real" "$commit" "$tree" >/dev/null || return 1
	[[ -z "${3:-}" ]] || eval "$3"
	[[ -f "$tree/.git" ]] || return 1
	(cd "$tree" && CARGO_TARGET_DIR="$fixture_dir/target-$1" cargo package --locked --no-verify --offline -q -p sysprims-core)
}
export_and_package clean "$real/scripts/release-export-tree.py"
crate="$fixture_dir/target-clean/package/sysprims-core-$version.crate"
python3 "$real/scripts/release-crates.py" vcs-info "$crate" sysprims-core "$version" "$commit" crates/sysprims-core
expect_fail_with 'records commit' python3 "$real/scripts/release-crates.py" vcs-info "$crate" sysprims-core "$version" \
	"$(printf '0%.0s' $(seq 40))" crates/sysprims-core
expect_fail_with 'records path' python3 "$real/scripts/release-crates.py" vcs-info "$crate" sysprims-core "$version" \
	"$commit" sysprims-core
# A byte changed after the export (a copy of the exporter whose blob checks
# are disabled, then a CRLF rewrite) makes cargo refuse: nothing allows dirty.
python3 - "$real/scripts/release-export-tree.py" "$variants/unverified-export.py" <<'PY'
import sys
text = open(sys.argv[1]).read()
for old, new in [
    ("    verify(repo, entries, root)\n    git_dir = attach_commit(repo, commit, root)\n    verify(repo, entries, root, git_dir)\n",
     "    lib = root / 'crates/sysprims-core/src/lib.rs'\n    lib.write_bytes(lib.read_bytes().replace(b'\\n', b'\\r\\n'))\n    attach_commit(repo, commit, root)\n"),
]:
    assert text.count(old) == 1, old
    text = text.replace(old, new)
open(sys.argv[2], "w").write(text)
PY
expect_fail_with 'export differs from the tagged commit' export_and_package tampered "$variants/unverified-export.py"
# Even past the exporter's own status check, cargo refuses the dirty file.
sed -i.bak -e 's/^    if status:$/    if False:/' -e 's/^    if attached:$/    if False:/' \
	"$variants/unverified-export.py"
[[ "$(grep -c '^    if False:$' "$variants/unverified-export.py")" == 2 ]]
expect_fail_with 'uncommitted|dirty' export_and_package tampered "$variants/unverified-export.py"
echo '[ok] crate plan and publication guard controls passed'
