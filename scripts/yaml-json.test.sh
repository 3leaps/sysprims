#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Controls for the workflow checkers' YAML reader (scripts/yaml_json.py) and
# the CI extraction of its binary (scripts/ci-extract-yamlprims.py).
#
# The binary is trusted by its committed SHA-256 only, so stub binaries can
# never pass through SYSPRIMS_YAMLPRIMS_BIN: the shell controls below show
# they are refused. The reader's own failure modes, its compatibility checks
# and the workflow loader's handling of reader failures are exercised by a
# Python harness that loads the modules by path and points the already
# authenticated binary at stubs in a scratch directory (no environment
# variable or file in the repository can do that).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
real="${SYSPRIMS_YAMLPRIMS_BIN:-}"
[[ "$real" == /* && -f "$real" ]] || {
	echo 'error: the YAML reader test needs SYSPRIMS_YAMLPRIMS_BIN (absolute path to the yamlprims CLI)' >&2
	exit 1
}
scratch="$(cd "$(mktemp -d)" && pwd -P)"
trap 'rm -rf "$scratch"' EXIT
pin() { (cd "$root" && python3 -B -E scripts/release-workflow-pin.py check) >"$scratch/out" 2>&1; }
perm() { (cd "$root" && python3 -B -P -E scripts/ci-pins.py check) >"$scratch/out" 2>&1; }
accepted() {
	SYSPRIMS_YAMLPRIMS_BIN="$1" pin || {
		echo "error: refused: $1" >&2
		cat "$scratch/out" >&2
		exit 1
	}
}
refused() {
	local expected="$1"
	shift
	if SYSPRIMS_YAMLPRIMS_BIN="$*" pin; then
		echo "error: accepted: $expected" >&2
		exit 1
	fi
	grep -qF -- "$expected" "$scratch/out" || {
		echo "error: rejection did not mention: $expected" >&2
		cat "$scratch/out" >&2
		exit 1
	}
	# A YAML reader failure is never reported as a digest or policy failure.
	! grep -q 'differs from its reviewed digest\|must declare\|grants more\|not valid UTF-8 YAML\|Traceback' "$scratch/out" || {
		cat "$scratch/out" >&2
		exit 1
	}
}

# The real CLI passes both checkers and reports its version.
accepted "$real"
SYSPRIMS_YAMLPRIMS_BIN="$real" perm || {
	cat "$scratch/out" >&2
	exit 1
}
SYSPRIMS_YAMLPRIMS_BIN="$real" python3 -I -B "$root/scripts/yaml_json.py" version | grep -qE '^yamlprims [0-9]+\.[0-9]+\.[0-9]+$'
# The minimum is declared once for the reader and once for the doctor; they agree.
[[ "$(sed -n 's/^minimum //p' "$root/config/yamlprims-cli.txt")" == "$(awk '/^  yamlprims:/ {t=1} t && /minimum_version:/ {print $2; exit}' "$root/.goneat/tools.yaml")" ]] || {
	echo 'error: yamlprims minimum differs between config/yamlprims-cli.txt and .goneat/tools.yaml' >&2
	exit 1
}

# The path is not the trust anchor: the same bytes anywhere pass (a copy, a
# symlink, a symlinked parent directory, a copy without the execute bit,
# since only a private copy is run); other bytes fail wherever they are.
mkdir -p "$scratch/real" "$scratch/bin"
cp "$real" "$scratch/real/yamlprims"
ln -s "$scratch/real/yamlprims" "$scratch/bin/link"
ln -s "$scratch/real" "$scratch/linkdir"
accepted "$scratch/real/yamlprims"
accepted "$scratch/bin/link"
accepted "$scratch/linkdir/yamlprims"
cp "$real" "$scratch/noexec"
chmod -x "$scratch/noexec"
accepted "$scratch/noexec"
cp "$real" "$scratch/tampered"
printf '\0' >>"$scratch/tampered"
refused 'is not a released yamlprims binary for this host' "$scratch/tampered"
# The refusal names the pinned release and a versioned install path.
pinned="$(sed -n 's/^release \([0-9][0-9.]*\)$/\1/p' "$root/config/yamlprims-cli.txt")"
[[ -n "$pinned" ]] || {
	echo 'error: config/yamlprims-cli.txt pins no release' >&2
	exit 1
}
refused "this checkout pins yamlprims $pinned: install that release to a versioned path (for example ~/.local/share/yamlprims/$pinned/yamlprims)" "$scratch/tampered"
printf '#!/usr/bin/env bash\nexec %q "$@"\n' "$real" >"$scratch/wrapper"
chmod +x "$scratch/wrapper"
refused 'is not a released yamlprims binary for this host' "$scratch/wrapper"
ln -s "$scratch/wrapper" "$scratch/bin/wrapper-link"
refused 'is not a released yamlprims binary for this host' "$scratch/bin/wrapper-link"

# Resolution: unset (a yamlprims on PATH does not stand in), relative,
# missing, a directory.
ln -s "$real" "$scratch/bin/yamlprims"
(
	unset SYSPRIMS_YAMLPRIMS_BIN
	export PATH="$scratch/bin:$PATH"
	if pin; then
		echo 'error: accepted without SYSPRIMS_YAMLPRIMS_BIN' >&2
		exit 1
	fi
	grep -q 'SYSPRIMS_YAMLPRIMS_BIN is required: set SYSPRIMS_YAMLPRIMS_BIN to the absolute path' "$scratch/out" || {
		cat "$scratch/out" >&2
		exit 1
	}
)
refused 'must be an absolute path' yamlprims
refused 'must be an absolute path' bin/yamlprims
refused 'must name a regular file' "$scratch/missing"
refused 'must name a regular file' "$scratch/bin"

# Run directly without -I, the helper still imports only the standard
# library: a module planted beside it or in the working directory is never
# imported (the helper strips both from sys.path itself).
mkdir -p "$scratch/direct/scripts" "$scratch/direct/config" "$scratch/direct/cwd"
cp "$root/scripts/yaml_json.py" "$scratch/direct/scripts/"
cp "$root/config/yamlprims-cli.txt" "$scratch/direct/config/"
for module in json hashlib platform shutil tempfile subprocess threading; do
	for dir in "$scratch/direct/scripts" "$scratch/direct/cwd"; do
		printf 'raise SystemExit("SHADOW_%s_IMPORTED")\n' "$module" >"$dir/$module.py"
	done
done
(cd "$scratch/direct/cwd" && SYSPRIMS_YAMLPRIMS_BIN="$real" python3 -B -E ../scripts/yaml_json.py version) >"$scratch/out" 2>&1 || true
if grep -q 'SHADOW_' "$scratch/out" || ! grep -qxE 'yamlprims [0-9]+\.[0-9]+\.[0-9]+' "$scratch/out"; then
	echo 'error: the helper run directly imported a planted module' >&2
	cat "$scratch/out" >&2
	exit 1
fi

# The shell resolver used by the ceremony targets takes the same path.
(
	# shellcheck source=scripts/release-yamlprims.sh
	source "$root/scripts/release-yamlprims.sh"
	SYSPRIMS_YAMLPRIMS_BIN="$scratch/bin/link" resolve_release_yamlprims
	if SYSPRIMS_YAMLPRIMS_BIN="$scratch/wrapper" resolve_release_yamlprims 2>"$scratch/out"; then
		echo 'error: the shell resolver accepted an unpinned binary' >&2
		exit 1
	fi
	grep -q 'is not a released yamlprims binary' "$scratch/out"
)

# The reader, its compatibility checks, and the workflow loader.
mkdir "$scratch/stubs"
python3 -I -B - "$root" "$real" "$scratch" <<'PY'
import importlib.util
import os
import shutil
import stat
import sys
import time

root, real, scratch = sys.argv[1:]
stubs = os.path.join(scratch, "stubs")


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


ym = module("yaml_json_under_test", os.path.join(root, "scripts", "yaml_json.py"))
assert not issubclass(ym.ReaderError, ValueError), "a reader failure must not be a ValueError"
count = 0


def stub(name, body):
    path = os.path.join(stubs, name)
    with open(path, "w") as handle:
        handle.write("#!/usr/bin/env bash\n" + body + "\n")
    os.chmod(path, 0o755)
    return path


def expect(substring, call):
    global count
    try:
        call()
    except ym.ReaderError as error:
        if substring not in str(error):
            raise SystemExit(f"error: reader failure did not mention {substring!r}: {error}")
        count += 1
        return
    raise SystemExit(f"error: accepted, expected reader failure: {substring}")


def loaded_by(binary, data=b"on: push\n"):
    def call():
        ym._resolved = (binary, (0, 1, 0))
        try:
            ym.load(data, "w.yml")
        finally:
            ym._resolved = None
    return call


# Each way the binary can misbehave on a workflow fails closed with its cause.
expect("w.yml: yamlprims refused it: -:1:1: stub refusal", loaded_by(stub("refuse", 'echo "-:1:1: stub refusal" >&2; exit 1')))
expect("w.yml: yamlprims usage or I/O error: stub usage", loaded_by(stub("usage", 'echo "stub usage" >&2; exit 2')))
expect("w.yml: yamlprims exited with status 70", loaded_by(stub("crash", "exit 70")))
expect("malformed yamlprims output", loaded_by(stub("malformed", 'printf "{\\"permissions\\":\\n"')))
expect("yamlprims output is not one JSON line", loaded_by(stub("unterminated", 'printf "{}"')))
expect("yamlprims output is not one JSON line", loaded_by(stub("lines", 'printf "{}\\n{}\\n"')))
expect("yamlprims output is not one JSON line", loaded_by(stub("empty", "exit 0")))
expect("duplicate key 'permissions' in yamlprims output", loaded_by(stub(
    "duplicate", 'printf "{\\"permissions\\":{\\"contents\\":\\"write\\"},\\"permissions\\":{\\"contents\\":\\"read\\"}}\\n"')))
expect("non-finite number NaN in yamlprims output", loaded_by(stub("nonfinite", 'printf "{\\"a\\":NaN}\\n"')))
expect(f"yamlprims output is larger than {ym.MAX_OUTPUT} bytes", loaded_by(stub("oversize", 'yes "{}" | head -c 6000000; exit 0')))
expect(f"YAML input is larger than {ym.MAX_INPUT} bytes", loaded_by(real, b"# " + b"x" * ym.MAX_INPUT + b"\n"))
expect("cannot run SYSPRIMS_YAMLPRIMS_BIN", loaded_by(os.path.join(stubs, "absent")))

# The deadline bounds the whole run: a binary that never exits, and one whose
# detached descendant (its own session, beyond the process group kill) keeps
# stdout open after the binary itself has exited.
ym.TIMEOUT = 1
for name, body in (
    ("sleeper", "cat >/dev/null; sleep 20"),
    ("orphan", "cat >/dev/null; python3 -c 'import os, time; os.setsid(); time.sleep(20)' & echo '{}'; exit 0"),
):
    started = time.monotonic()
    expect("yamlprims did not finish within 1 seconds", loaded_by(stub(name, body)))
    if time.monotonic() - started > 4:
        raise SystemExit(f"error: the {name} run was not bounded by the timeout")
# A timed-out run leaves no descendant in its process group behind.
pidfile = os.path.join(scratch, "child.pid")
expect("yamlprims did not finish within 1 seconds", loaded_by(stub(
    "parent", f"cat >/dev/null; sleep 20 & echo $! >{pidfile}; wait")))
time.sleep(0.2)
try:
    os.kill(int(open(pidfile).read()), 0)
    raise SystemExit("error: a descendant of the timed-out reader is still running")
except ProcessLookupError:
    pass
# The first run of a copy (the version probe) has its own budget, so a slow
# first start is not a reader failure; it is still bounded.
slow_start = stub("slowstart", f'sleep 2\nexec "{real}" "$@"')
ym.FIRST_RUN_TIMEOUT = 5
if ym.read_version(slow_start) < (0, 1, 0):
    raise SystemExit("error: a slow first run did not report a version")
ym.FIRST_RUN_TIMEOUT = 1
expect("yamlprims did not finish within 1 seconds", lambda: ym.read_version(slow_start))
ym.FIRST_RUN_TIMEOUT = 120
ym.TIMEOUT = 30

# Compatibility checks after a digest match (the copy is a stub here): another
# program, an older version, contract drift, a duplicate key accepted.
def resolved_with(binary):
    def call():
        saved = ym.authenticate
        ym.authenticate = lambda source: binary
        try:
            ym._resolved = None
            os.environ[ym.ENV] = real
            ym.resolve()
        finally:
            ym.authenticate = saved
            ym._resolved = None
    return call


expect("does not identify as yamlprims", resolved_with(stub("other", 'echo "other 1.0.0"')))
expect("yamlprims >= 0.1.0 required (have 0.0.9)", resolved_with(stub("old", 'echo "yamlprims 0.0.9"')))
expect("does not produce the yamlprims plain JSON contract", resolved_with(stub(
    "drift", f'[[ "$1" == --version ]] && exec "{real}" "$@"\ncat >/dev/null\nprintf "{{\\"on\\":\\"push\\"}}\\n"')))
expect("does not refuse a duplicate key", resolved_with(stub(
    "lenient", f'[[ "$1" == --version ]] && exec "{real}" "$@"\nin="$(cat)"\n[[ "$in" == "a: 1"* ]] && {{ echo "{{\\"a\\":2}}"; exit 0; }}\nexec "{real}" "$@" <<<"$in"')))
# Only the version probe gets the first-run budget: resolve's contract
# probes, which follow it, keep TIMEOUT.
ym.TIMEOUT = 1
for name, first in (("slowprobe", "on:"), ("slowdup", "a: 1")):
    expect("yamlprims did not finish within 1 seconds", resolved_with(stub(
        name, f'[[ "$1" == --version ]] && exec "{real}" "$@"\nin="$(cat)"\n[[ "$in" == "{first}"* ]] && sleep 2\nexec "{real}" "$@" <<<"$in"')))
ym.TIMEOUT = 30

# Check and use are one copy: only a private 0700 copy is run, so replacing
# the source after it was authenticated changes nothing.
source = os.path.join(scratch, "source-bin")
shutil.copyfile(real, source)
os.environ[ym.ENV] = source
ym._resolved = None
binary, _ = ym.resolve()
if os.path.realpath(binary) == os.path.realpath(source) or os.path.dirname(binary) == os.path.dirname(source):
    raise SystemExit("error: the reader runs the source path, not a private copy")
if stat.S_IMODE(os.stat(os.path.dirname(binary)).st_mode) != 0o700:
    raise SystemExit("error: the private copy's directory is not 0700")
shutil.copyfile(os.path.join(stubs, "crash"), source)
if ym.load(b"on: push\n", "w.yml") != {"on": "push"}:
    raise SystemExit("error: the replaced source was run")
ym._resolved = None

# The workflow loader reports a YAML refusal in the input as a WorkflowError
# (a lint finding) and every failure of the reader itself as ReaderError,
# never as a finding.
wy = module("workflow_yaml_under_test", os.path.join(root, "scripts", "workflow_yaml.py"))
os.environ[wy.yaml_json.ENV] = real


def workflow_refused(substring, text):
    global count
    try:
        wy.load_text(text, "w.yml")
    except wy.WorkflowError as error:
        if substring not in str(error):
            raise SystemExit(f"error: workflow refusal did not mention {substring!r}: {error}")
        count += 1
        return
    raise SystemExit(f"error: workflow accepted: {substring}")


def loader_failed(substring, text, binary=None, timeout=30):
    global count
    wy.yaml_json.TIMEOUT = timeout
    if binary is not None:
        wy.yaml_json._resolved = (binary, (0, 1, 0))
    try:
        wy.load_text(text, "w.yml")
    except wy.WorkflowError as error:
        raise SystemExit(f"error: a reader failure was reported as a workflow finding: {error}")
    except wy.ReaderError as error:
        if substring not in str(error):
            raise SystemExit(f"error: reader failure did not mention {substring!r}: {error}")
        count += 1
        return
    finally:
        wy.yaml_json._resolved = None
        wy.yaml_json.TIMEOUT = 30
    raise SystemExit(f"error: the loader accepted a reader failure: {substring}")


base = "on: push\npermissions:\n  contents: read\njobs:\n  a:\n    runs-on: x\n    steps:\n      - run: echo\n"
if wy.load_text(base, "w.yml")["on"] != "push":
    raise SystemExit("error: the loader did not read on as a string key")
workflow_refused("duplicate key", base + "permissions:\n  contents: write\n")
workflow_refused("merge keys are not accepted", base.replace("  contents: read\n", "  <<: {contents: write}\n"))
workflow_refused("aliases and merge keys are not accepted", base + "x: &p {contents: write}\ny: *p\n")
workflow_refused("explicit tags are not accepted", base + "x: !!str write\n")
workflow_refused("explicit tags are not accepted", base + "x: !custom write\n")
workflow_refused("not valid YAML", base + "x: [\n")
workflow_refused("exactly one YAML document", base + "---\n" + base)
workflow_refused("workflow is not a mapping", "- a\n")
loader_failed("YAML input is larger than", base + "# " + "x" * wy.yaml_json.MAX_INPUT + "\n")
loader_failed("yamlprims usage or I/O error", base, os.path.join(stubs, "usage"))
loader_failed("yamlprims exited with status 70", base, os.path.join(stubs, "crash"))
loader_failed("not a yamlprims-value/1 documents envelope", base, stub("plain", 'printf "{\\"on\\":\\"push\\"}\\n"'))
loader_failed("yamlprims did not finish within 1 seconds", base, os.path.join(stubs, "sleeper"), 1)
print(f"[ok] {count} reader failure controls")
PY

# CI extraction: only the named member, a regular file, with the committed
# digest; anything else fails with a message, never a traceback.
extract() { python3 -I -B "$root/scripts/ci-extract-yamlprims.py" "$@" >"$scratch/out" 2>&1; }
extract_refused() {
	local expected="$1"
	shift
	if extract "$@"; then
		echo "error: extraction accepted: $expected" >&2
		exit 1
	fi
	grep -qF -- "$expected" "$scratch/out" && ! grep -q Traceback "$scratch/out" || {
		echo "error: extraction rejection did not mention: $expected" >&2
		cat "$scratch/out" >&2
		exit 1
	}
}
digest="$(shasum -a 256 "$real" | cut -c1-64)"
mkdir -p "$scratch/arc/dot" "$scratch/arc/bare"
cp "$real" "$scratch/arc/dot/yamlprims"
(cd "$scratch/arc/dot" && tar -czf "$scratch/dot.tar.gz" ./yamlprims)
(cd "$scratch/arc/dot" && tar -czf "$scratch/bare.tar.gz" yamlprims)
extract "$scratch/dot.tar.gz" ./yamlprims "$scratch/x1" "$digest" || {
	cat "$scratch/out" >&2
	exit 1
}
cmp -s "$real" "$scratch/x1"
extract_refused 'bare.tar.gz has no member ./yamlprims' "$scratch/bare.tar.gz" ./yamlprims "$scratch/x2" "$digest"
extract_refused "sha256 $digest != committed $(printf '%064d' 0)" "$scratch/dot.tar.gz" ./yamlprims "$scratch/x3" "$(printf '%064d' 0)"
(cd "$scratch/arc" && ln -s dot/yamlprims link && tar -czf "$scratch/link.tar.gz" ./link)
extract_refused 'is not a regular file' "$scratch/link.tar.gz" ./link "$scratch/x4" "$digest"
python3 -I -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1], "w").write(sys.argv[2], "other.exe")' "$scratch/w.zip" "$real"
extract_refused 'w.zip has no member yamlprims.exe' "$scratch/w.zip" yamlprims.exe "$scratch/x5" "$digest"
printf 'not an archive' >"$scratch/junk.tar.gz"
extract_refused 'cannot read junk.tar.gz' "$scratch/junk.tar.gz" ./yamlprims "$scratch/x6" "$digest"
extract_refused 'File exists' "$scratch/dot.tar.gz" ./yamlprims "$scratch/x1" "$digest"
echo '[ok] YAML reader authentication, resolution, failure, timeout and extraction controls'
