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
echo 'verify step ran' >&2
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
expect_fail_with 'verify step ran' "$fixture_root/scripts/install-release-anchors.sh" "$stage" "$fixture_dir/dest" "$racer"
[[ ! -e "$fixture_dir/dest/expected-fingerprints.txt" && ! -e "$fixture_dir/dest/expected-fingerprints.ndjson" ]]
printf 'unrelated\n' >"$fixture_dir/dest/other"
"$fixture_root/scripts/install-release-anchors.sh" "$stage" "$fixture_dir/dest"
[[ "$(cat "$fixture_dir/dest/other")" == unrelated ]]
# Exclusive writes go through the created descriptor. A symlink at the
# destination is refused and its target is never written; a write that fails
# part way leaves no file behind.
helper="$fixture_root/scripts/write-exclusive.py"
mkdir -p "$fixture_dir/ex"
printf 'victim\n' >"$fixture_dir/ex/victim"
ln -s "$fixture_dir/ex/victim" "$fixture_dir/ex/link"
status=0
python3 "$helper" "$stage/expected-fingerprints.txt" "$fixture_dir/ex/link" 2>/dev/null || status=$?
[[ "$status" == 3 && "$(cat "$fixture_dir/ex/victim")" == victim && -L "$fixture_dir/ex/link" ]]
head -c 300000 /dev/zero >"$fixture_dir/ex/big"
status=0
[[ "$(wc -c <"$fixture_dir/ex/big")" -eq 300000 ]]
(ulimit -f 64 && python3 "$helper" "$fixture_dir/ex/big" "$fixture_dir/ex/partial") 2>"$fixture_dir/ex/partial.err" || status=$?
grep -q "error: writing $fixture_dir/ex/partial failed:" "$fixture_dir/ex/partial.err" || {
	echo 'error: the helper partial-write control did not reach the failing write' >&2
	exit 1
}
[[ "$status" != 0 && ! -e "$fixture_dir/ex/partial" ]] || {
	echo 'error: a failed exclusive write left a file behind' >&2
	exit 1
}
python3 "$helper" "$stage/expected-fingerprints.txt" "$fixture_dir/ex/ok" >/dev/null
cmp "$stage/expected-fingerprints.txt" "$fixture_dir/ex/ok"
# Failures before the create report 3 (nothing created), so a caller never
# claims, and later removes, a file it did not make.
for args in "" "$fixture_dir/ex/absent-src $fixture_dir/ex/new" \
	"$stage/expected-fingerprints.txt $fixture_dir/ex/no-such-dir/file"; do
	status=0
	# shellcheck disable=SC2086 # Word splitting builds the argument list.
	python3 "$helper" $args 2>/dev/null || status=$?
	[[ "$status" == 3 ]] || {
		echo "error: write-exclusive.py returned $status, not 3, for a pre-create failure ($args)" >&2
		exit 1
	}
done
[[ ! -e "$fixture_dir/ex/new" ]]
# The installers never open a destination a second time by path.
for script in install-release-anchors.sh release-export-pin.sh; do
	# shellcheck disable=SC2016 # A literal pattern matched in the scripts.
	if grep -nE '(cat|printf)[^|]*>[[:space:]]*"\$(dest|path)' "$fixture_root/scripts/$script"; then
		echo "error: $script writes a destination through a second open" >&2
		exit 1
	fi
done
# Through the installer: a failing write mid-pair removes what this run made.
# The oversized stage is built first, with no limit; only the installer runs
# under the file-size limit, and the failure must be the ndjson write itself.
partial="$fixture_dir/partial"
mkdir -p "$partial" "$fixture_dir/bigstage"
cp "$stage/expected-fingerprints.txt" "$fixture_dir/bigstage/"
head -c 300000 /dev/zero >"$fixture_dir/bigstage/expected-fingerprints.ndjson"
[[ "$(wc -c <"$fixture_dir/bigstage/expected-fingerprints.ndjson")" -eq 300000 ]]
status=0
(ulimit -f 64 && "$fixture_root/scripts/install-release-anchors.sh" "$fixture_dir/bigstage" "$partial") \
	>"$fixture_dir/partial.out" 2>&1 || status=$?
if [[ "$status" == 0 ]] || ! grep -q "error: writing $partial/expected-fingerprints.ndjson failed:" "$fixture_dir/partial.out"; then
	echo 'error: the partial-write control did not reach the failing anchor write' >&2
	cat "$fixture_dir/partial.out" >&2
	exit 1
fi
[[ -z "$(ls -A "$partial")" ]] || {
	echo "error: a partially installed anchor pair survived: $(ls -A "$partial")" >&2
	exit 1
}
# An unexpected helper exit (here a crash after creating the file) is not
# ownership: the installer does not claim or remove that file, and still
# removes the anchor it did create.
mkdir -p "$fixture_dir/stubbed"
cp "$fixture_root/scripts/install-release-anchors.sh" "$fixture_dir/stubbed/"
cat >"$fixture_dir/stubbed/write-exclusive.py" <<PY
#!/usr/bin/env python3
import os, subprocess, sys
if sys.argv[1] != "--remove" and sys.argv[2].endswith(".ndjson"):
    open(sys.argv[2], "x").write("crashed")
    os.kill(os.getpid(), 9)
sys.exit(subprocess.call([sys.executable, "$helper", *sys.argv[1:]]))
PY
crash="$fixture_dir/crash"
mkdir "$crash"
expect_fail_with 'status 137.*left in place' "$fixture_dir/stubbed/install-release-anchors.sh" "$stage" "$crash"
[[ ! -e "$crash/expected-fingerprints.txt" && "$(cat "$crash/expected-fingerprints.ndjson")" == crashed ]] || {
	echo 'error: the installer claimed a file of unknown ownership, or kept its own' >&2
	exit 1
}
# Rollback is by identity: an anchor this run created, replaced by another
# file before the rollback, is left alone.
replacer="$fixture_dir/replacer.sh"
cat >"$replacer" <<'SH'
#!/usr/bin/env bash
rm "$REPLACE_DIR/expected-fingerprints.txt"
printf 'replacement\n' >"$REPLACE_DIR/expected-fingerprints.txt"
echo 'replacer ran' >&2
exit 1
SH
chmod +x "$replacer"
replaced="$fixture_dir/replaced"
mkdir "$replaced"
expect_fail_with 'replacer ran' env REPLACE_DIR="$replaced" "$fixture_root/scripts/install-release-anchors.sh" "$stage" "$replaced" "$replacer"
[[ "$(cat "$replaced/expected-fingerprints.txt")" == replacement && ! -e "$replaced/expected-fingerprints.ndjson" ]] || {
	echo 'error: the rollback removed a file that replaced the one it created, or kept its own' >&2
	exit 1
}
# The helper's ownership token and its rollback. Each case asserts that a
# file this run did not write survives with its bytes.
own="$fixture_dir/own"
mkdir "$own"
printf 'ours\n' >"$fixture_dir/ours.src"
python3 "$helper" "$fixture_dir/ours.src" "$own/ours" >"$own.token"
token="$(cat "$own.token")"
[[ "$token" =~ ^[0-9]+:[0-9]+:5:[0-9a-f]{64}$ && "${token##*:}" == "$(shasum -a 256 <"$fixture_dir/ours.src" | awk '{print $1}')" ]]
# Ours, unchanged: removed, and nothing is left beside it.
python3 "$helper" --remove "$own/ours" "$token"
[[ -z "$(ls -A "$own")" ]]
python3 "$helper" "$fixture_dir/ours.src" "$own/ours" >"$own.token"
token="$(cat "$own.token")"
# (a) Replaced by a different file (another inode): left in place, bytes intact.
rm "$own/ours"
printf 'foreign-a\n' >"$own/ours"
python3 "$helper" --remove "$own/ours" "$token" 2>"$own.err"
[[ "$(cat "$own/ours")" == foreign-a && "$(ls -A "$own")" == ours ]] || {
	echo 'error: rollback removed or moved a replacement file' >&2
	exit 1
}
grep -q 'left in place' "$own.err"
# (b) The same inode number holding other bytes (inode reuse, forced by a
# token that names this file's device and inode with the bytes we wrote).
rm "$own/ours"
printf 'evil\n' >"$own/ours"
here="$(python3 -c 'import os, sys; i = os.lstat(sys.argv[1]); print(f"{i.st_dev}:{i.st_ino}")' "$own/ours")"
reused="$here:${token#*:*:}"
# The token matches the file on device, inode and (same length) size; only
# the hash of the bytes tells them apart.
[[ "$reused" == "$here:5:"* && "$(wc -c <"$own/ours" | tr -d ' ')" == 5 ]]
python3 "$helper" --remove "$own/ours" "$reused" 2>"$own.err"
[[ "$(cat "$own/ours")" == evil && "$(ls -A "$own")" == ours ]] || {
	echo 'error: rollback removed a file whose inode matched but whose bytes did not' >&2
	exit 1
}
# (c) A third file appears at the destination between capture and link-back:
# both foreign files survive, the captured one under the sibling name the
# warning gives.
cat >"$fixture_dir/race-link.py" <<'PY'
import os, runpy, sys
real_link = os.link
def link(src, dst, **kwargs):
    with open(os.path.join(os.path.dirname(sys.argv[2]), dst), "x") as handle:
        handle.write("third\n")
    return real_link(src, dst, **kwargs)
os.link = link
sys.argv = sys.argv[1:]
runpy.run_path(sys.argv[0], run_name="__main__")
PY
python3 "$fixture_dir/race-link.py" "$helper" --remove "$own/ours" "$token" 2>"$own.err"
kept="$(sed -n 's/.* it is kept at //p' "$own.err")"
[[ -n "$kept" && "$(cat "$kept")" == evil && "$(cat "$own/ours")" == third ]] || {
	echo 'error: a race at link-back lost a file this run did not write' >&2
	cat "$own.err" >&2
	exit 1
}
[[ "$(find "$own" -mindepth 1 | wc -l | tr -d ' ')" == 2 ]]
# Non-regular replacements are mismatches and are never opened: a directory
# and a FIFO at the destination each end up back there, intact, or at the
# path the warning names. The FIFO run is under a watchdog, so a blocking open
# fails the suite instead of hanging it.
# run_bounded <seconds> <command...>: the command's status, or 124 if killed.
run_bounded() {
	local limit="$1" pid watchdog status=0
	shift
	"$@" &
	pid=$!
	(sleep "$limit" && kill -9 "$pid" 2>/dev/null) &
	watchdog=$!
	wait "$pid" || status=$?
	kill "$watchdog" 2>/dev/null || true
	wait "$watchdog" 2>/dev/null || true
	[[ "$status" != 137 ]] || status=124
	return "$status"
}
for kind in directory fifo; do
	odd="$fixture_dir/odd-$kind"
	mkdir "$odd"
	python3 "$helper" "$fixture_dir/ours.src" "$odd/ours" >"$odd.token"
	rm "$odd/ours"
	case "$kind" in
	directory)
		mkdir "$odd/ours"
		printf 'inside\n' >"$odd/ours/content"
		;;
	fifo) mkfifo "$odd/ours" ;;
	esac
	status=0
	run_bounded 10 python3 "$helper" --remove "$odd/ours" "$(cat "$odd.token")" 2>"$odd.err" || status=$?
	[[ "$status" == 0 ]] || {
		echo "error: rollback over a $kind exited $status (124: it hung)" >&2
		cat "$odd.err" >&2
		exit 1
	}
	# The destination was free, so the object is back there, intact, and
	# nothing is left under a rollback name.
	case "$kind" in
	directory) [[ -d "$odd/ours" && ! -L "$odd/ours" && "$(cat "$odd/ours/content")" == inside ]] ;;
	fifo) [[ -p "$odd/ours" ]] ;;
	esac && [[ "$(find "$odd" -mindepth 1 -maxdepth 1 | wc -l | tr -d ' ')" == 1 ]] || {
		echo "error: rollback did not put back a $kind it did not create" >&2
		cat "$odd.err" >&2
		exit 1
	}
	grep -q 'left in place' "$odd.err"
done
# The same protocol, instrumented: a non-regular captured object is never
# opened; an unexpected exception while checking is a mismatch, not a crash;
# and a non-regular object is never renamed over a file that appeared at the
# destination meanwhile (it is kept, and its path named).
PYTHONDONTWRITEBYTECODE=1 python3 - "$helper" "$fixture_dir/instrumented" <<'PY'
import contextlib, importlib.util, io, os, pathlib, sys
spec = importlib.util.spec_from_file_location("wx", sys.argv[1])
wx = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wx)
base = pathlib.Path(sys.argv[2])
base.mkdir()
(base / "ours.src").write_bytes(b"ours\n")

def owned(name):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        sys.argv = ["write-exclusive.py", str(base / "ours.src"), str(base / name)]
        wx.main()
    os.unlink(base / name)
    return out.getvalue().strip()

def removing(name, token):
    err = io.StringIO()
    with contextlib.redirect_stderr(err):
        wx.remove(str(base / name), token)
    return err.getvalue()

# Never opened: every open of a rollback name is recorded.
real_open, opened = os.open, []
def recording_open(path, flags, *args, **kwargs):
    if ".rollback-" in str(path):
        opened.append(path)
    return real_open(path, flags, *args, **kwargs)
os.open = recording_open
for kind, make in (("dir", os.mkdir), ("fifo", os.mkfifo)):
    token = owned(kind)
    make(base / kind)
    removing(kind, token)
    assert not opened, f"a captured {kind} was opened: {opened}"
os.open = real_open

# An unexpected exception during the check is a mismatch: the file is put back.
token = owned("raises")
(base / "raises").write_bytes(b"evil\n")
real_fstat = os.fstat
def failing_fstat(fd):
    raise ValueError("injected")
os.fstat = failing_fstat
try:
    message = removing("raises", token)
finally:
    os.fstat = real_fstat
assert (base / "raises").read_bytes() == b"evil\n", "a checking error lost the file"
assert "left in place" in message, message

# No clobber: a file appears at the destination just before a captured FIFO
# is restored; both survive, the FIFO at the named path.
token = owned("contested")
os.mkfifo(base / "contested")
real_restore = wx.restore
def contested_restore(directory, aside, name):
    (base / name).write_bytes(b"third\n")
    return real_restore(directory, aside, name)
wx.restore = contested_restore
message = removing("contested", token)
wx.restore = real_restore
# (Type first: reading a FIFO left there would block.)
assert (base / "contested").is_file() and (base / "contested").read_bytes() == b"third\n", "the restore replaced a file that appeared"
kept = message.rsplit("it is kept at ", 1)[-1].strip()
assert "it is kept at " in message and pathlib.Path(kept).is_fifo(), message
PY
# A malformed token removes nothing.
expect_fail_with 'malformed ownership token' python3 "$helper" --remove "$own/ours" 0:0
[[ "$(cat "$own/ours")" == third ]]
echo '[ok] anchor and checker-binding controls passed'
