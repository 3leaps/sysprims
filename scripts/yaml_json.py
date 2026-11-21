#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Read YAML as JSON through the yamlprims CLI (standard library only).

The workflow checkers load YAML with `yamlprims dump --plain -`: YAML 1.2 Core
Schema, so `on` is the string "on". The trust anchor is the binary digests
committed in config/yamlprims-cli.txt, not a path: SYSPRIMS_YAMLPRIMS_BIN
(an absolute path, never a PATH lookup) only says where to read the binary
from. Its bytes are copied into a private directory, and the copy is used only
if its SHA-256 equals a committed digest for a platform this host runs; only
the copy is ever executed. The copy must then identify as yamlprims, meet the
minimum version, produce the exact bytes of a fixed probe document and refuse
a duplicate key (a compatibility check, not an identity one).

Each load runs the copy with an argument list (never a shell), the YAML on
stdin, a deadline on the whole run and a cap on stdout. Every failure of the
reader (resolution, digest, identity, exit status, timeout, size, malformed
JSON, a duplicate key) raises ReaderError, which is deliberately not a
ValueError, so a caller can never count a reader failure as a policy decision.

The checkers load this file by path after removing their own directory and
the working directory from sys.path; run directly, it does the same itself.

Usage (for scripts/release-yamlprims.sh, CI and the doctor):
  yaml_json.py identify   authenticate and probe the binary; print its source
  yaml_json.py version    authenticate; print "yamlprims X.Y.Z" (no minimum)
"""

import os
import sys

if __name__ == "__main__":
    _untrusted = {
        os.path.realpath(p)
        for p in (os.path.dirname(os.path.abspath(__file__)), os.getcwd())
    }
    sys.path[:] = [
        p
        for p in sys.path
        if p not in ("", ".") and os.path.realpath(p) not in _untrusted
    ]
    sys.dont_write_bytecode = True

import atexit  # noqa: E402
import hashlib  # noqa: E402
import json  # noqa: E402
import platform  # noqa: E402
import re  # noqa: E402
import shutil  # noqa: E402
import signal  # noqa: E402
import subprocess  # noqa: E402
import tempfile  # noqa: E402
import threading  # noqa: E402
import time  # noqa: E402

ENV = "SYSPRIMS_YAMLPRIMS_BIN"
CONFIG = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "config", "yamlprims-cli.txt"
)
PLATFORMS = (
    "darwin-arm64",
    "linux-amd64",
    "linux-arm64",
    "windows-amd64",
    "windows-arm64",
)
MAX_BINARY = 64 * 1024 * 1024
MAX_OUTPUT = 4 * 1024 * 1024
MAX_INPUT = 4 * 1024 * 1024
# Seconds for one run of the binary, start to exit, including every byte of
# its output. Tests lower it by assignment; it is not read from the
# environment.
TIMEOUT = 30
# Seconds for the first run of each private copy (the version probe). A
# freshly written executable can be assessed by the OS on its first run
# (macOS scans new code before it starts), which a loaded host can stretch
# past TIMEOUT. Later runs of the same copy get TIMEOUT.
FIRST_RUN_TIMEOUT = 120
# Releases through 0.1.1 have no version flag: `--version` prints the usage
# text and exits 2. Such a binary is read as 0.1.0, the oldest release; which
# release it is comes from its committed digest, not from this probe.
FIRST_RELEASE = (0, 1, 0)
USAGE = b"usage: yamlprims "
PROBE_IN = (
    b'on: push\ny: yes\nz: [1, -2, 2.5, true, null, "x\\u00e9"]\na: {q: 1, b: 2}\n'
)
PROBE_OUT = (
    '{"on":"push","y":"yes","z":[1,-2,2.5,true,null,"xé"],"a":{"q":1,"b":2}}\n'.encode(
        "utf-8"
    )
)
HOWTO = f"set {ENV} to the absolute path of the yamlprims CLI from a release archive pinned in config/yamlprims-cli.txt"


class ReaderError(Exception):
    """The YAML reader failed or refused the input. Not a ValueError: it is
    never a policy decision."""


class Refused(ReaderError):
    """yamlprims read the input and refused it (exit status 1): a YAML error
    in the input, such as a syntax error, a duplicate key or a merge key. The
    message is the reader's own first diagnostic line."""


def fail(reason):
    raise ReaderError(reason)


def read_config(path=CONFIG):
    """(minimum version, {platform: binary sha256}) from the committed file."""
    minimum, digests = [], {}
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            line = line.rstrip("\n")
            match = re.fullmatch(r"minimum ([0-9]+)\.([0-9]+)\.([0-9]+)", line)
            if match:
                minimum.append(tuple(int(part) for part in match.groups()))
            match = re.fullmatch(r"binary ([a-z0-9-]+) ([0-9a-f]{64})", line)
            if match:
                if match.group(1) not in PLATFORMS or match.group(1) in digests:
                    fail(
                        f"{path}: unknown or duplicate binary platform {match.group(1)}"
                    )
                digests[match.group(1)] = match.group(2)
    if len(minimum) != 1:
        fail(f"{path} must declare exactly one 'minimum X.Y.Z' line")
    if set(digests) != set(PLATFORMS):
        fail(
            f"{path} must declare one binary digest for each of {', '.join(PLATFORMS)}"
        )
    return minimum[0], digests


def pinned_release(path=CONFIG):
    """The 'release X.Y.Z' version the committed file pins for CI, or None."""
    try:
        with open(path, encoding="utf-8") as handle:
            found = [
                line.split()[1]
                for line in handle
                if re.fullmatch(r"release [0-9]+\.[0-9]+\.[0-9]+", line.rstrip("\n"))
            ]
    except OSError:
        return None
    return found[0] if len(found) == 1 else None


def host_platforms():
    """The release platforms whose binaries this host runs."""
    machine = platform.machine().lower()
    if sys.platform == "darwin":
        return {"darwin-arm64"} if machine in ("arm64", "aarch64") else set()
    if sys.platform.startswith("linux"):
        return {
            {
                "x86_64": "linux-amd64",
                "amd64": "linux-amd64",
                "aarch64": "linux-arm64",
                "arm64": "linux-arm64",
            }.get(machine)
        } - {None}
    if sys.platform == "win32":
        # An x64 interpreter on Windows on Arm reports AMD64; the OS does not.
        arches = {machine} | {
            os.environ.get(v, "").lower()
            for v in ("PROCESSOR_ARCHITECTURE", "PROCESSOR_ARCHITEW6432")
        }
        if "arm64" in arches:
            return {"windows-arm64", "windows-amd64"}
        return {"windows-amd64"} if arches & {"amd64", "x86_64"} else set()
    return set()


def _kill(proc):
    try:
        if os.name == "posix":
            os.killpg(proc.pid, signal.SIGKILL)
        else:
            proc.kill()
    except OSError:
        pass
    try:
        proc.wait(timeout=1)
    except subprocess.TimeoutExpired:
        pass


def run(binary, args, data, timeout=None):
    """Run binary with args and data on stdin; return (status, stdout, stderr
    head). The whole run, including reading stdout, ends at timeout seconds
    (TIMEOUT by default) whatever holds the pipe open; stdout above
    MAX_OUTPUT fails."""
    limit = TIMEOUT if timeout is None else timeout
    if len(data) > MAX_INPUT:
        fail(f"YAML input is larger than {MAX_INPUT} bytes")
    deadline = time.monotonic() + limit
    with tempfile.TemporaryFile() as stdin, tempfile.TemporaryFile() as stderr:
        stdin.write(data)
        stdin.seek(0)
        try:
            proc = subprocess.Popen(
                [binary, *args],
                stdin=stdin,
                stdout=subprocess.PIPE,
                stderr=stderr,
                close_fds=True,
                start_new_session=(os.name == "posix"),
            )
        except OSError as error:
            fail(f"cannot run {ENV}: {error.strerror or error}")
        chunks, state = [], {"size": 0, "over": False}

        def pump():
            try:
                while True:
                    chunk = proc.stdout.read1(65536)
                    if not chunk:
                        return
                    state["size"] += len(chunk)
                    if state["size"] > MAX_OUTPUT:
                        state["over"] = True
                        return
                    chunks.append(chunk)
            except (OSError, ValueError):
                return

        # The reader thread is abandoned at the deadline (a descendant may
        # hold the pipe open forever); the run ends regardless.
        reader = threading.Thread(target=pump, daemon=True)
        reader.start()
        reader.join(max(0.0, deadline - time.monotonic()))
        if reader.is_alive():
            _kill(proc)
            fail(f"yamlprims did not finish within {limit} seconds")
        if state["over"]:
            _kill(proc)
            fail(f"yamlprims output is larger than {MAX_OUTPUT} bytes")
        try:
            status = proc.wait(timeout=max(0.0, deadline - time.monotonic()))
        except subprocess.TimeoutExpired:
            _kill(proc)
            fail(f"yamlprims did not finish within {limit} seconds")
        stderr.seek(0)
        err = stderr.read(2048).decode("utf-8", "replace").strip().splitlines()
    return status, b"".join(chunks), (err[0] if err else "")


def source_path():
    """The file SYSPRIMS_YAMLPRIMS_BIN names, symlinks resolved (never a
    PATH lookup). Where it lives does not matter: its digest does."""
    candidate = os.environ.get(ENV, "")
    if not candidate:
        fail(f"{ENV} is required: {HOWTO}")
    if not os.path.isabs(candidate):
        fail(f"{ENV} must be an absolute path: {HOWTO}")
    resolved = os.path.realpath(candidate)
    if not os.path.isfile(resolved):
        fail(f"{ENV} must name a regular file: {HOWTO}")
    return resolved


_private = None


def authenticate(source):
    """Copy source into a private directory, and return the copy's path if its
    SHA-256 is a committed digest for a platform this host runs. The bytes
    hashed are the bytes written, so a later change to source is never run."""
    global _private
    _, digests = read_config()
    platforms = sorted(host_platforms())
    allowed = {digests[p] for p in platforms}
    if not allowed:
        fail(
            f"no yamlprims release binary for this host ({sys.platform} {platform.machine()})"
        )
    try:
        with open(source, "rb") as handle:
            data = handle.read(MAX_BINARY + 1)
    except OSError as error:
        fail(f"cannot read {ENV}: {error.strerror or error}")
    if len(data) > MAX_BINARY:
        fail(f"{ENV} is larger than {MAX_BINARY} bytes")
    got = hashlib.sha256(data).hexdigest()
    if got not in allowed:
        release = pinned_release()
        hint = (
            f"; this checkout pins yamlprims {release}: install that release to a "
            f"versioned path (for example ~/.local/share/yamlprims/{release}/yamlprims) "
            f"and set {ENV} to it. Post-tag release steps use the pin in the tag's "
            f"own config/yamlprims-cli.txt"
            if release
            else ""
        )
        fail(
            f"{ENV} is not a released yamlprims binary for this host: sha256 {got} "
            f"matches no binary digest for {', '.join(platforms)} in config/yamlprims-cli.txt"
            f"{hint}"
        )
    if _private is None:
        _private = tempfile.mkdtemp(prefix="sysprims-yamlprims.")
        atexit.register(shutil.rmtree, _private, True)
    os.chmod(_private, 0o700)
    fd, copy = tempfile.mkstemp(
        prefix="yamlprims-", suffix=".exe" if os.name == "nt" else "", dir=_private
    )
    with os.fdopen(fd, "wb") as handle:
        handle.write(data)
    os.chmod(copy, 0o700)
    return copy


def read_version(binary):
    """The version the binary reports, as a tuple; it must identify as
    yamlprims."""
    status, out, err = run(binary, ["--version"], b"", FIRST_RUN_TIMEOUT)
    match = (
        re.fullmatch(rb"yamlprims ([0-9]+)\.([0-9]+)\.([0-9]+)\n", out)
        if status == 0
        else None
    )
    if match:
        return tuple(int(part) for part in match.groups())
    if status == 2 and not out and err.encode("utf-8").startswith(USAGE):
        return FIRST_RELEASE
    fail(f"{ENV} does not identify as yamlprims")


_resolved = None


def resolve():
    """(path of the authenticated private copy, version), cached: digest,
    then identity, minimum version and the plain JSON contract."""
    global _resolved
    if _resolved is not None:
        return _resolved
    binary = authenticate(source_path())
    minimum, _ = read_config()
    version = read_version(binary)
    if version < minimum:
        fail(
            f"yamlprims >= {'.'.join(map(str, minimum))} required (have {'.'.join(map(str, version))})"
        )
    status, out, err = run(binary, ["dump", "--plain", "-"], PROBE_IN)
    if status != 0 or out != PROBE_OUT:
        fail(f"{ENV} does not produce the yamlprims plain JSON contract")
    status, out, err = run(binary, ["dump", "--plain", "-"], b"a: 1\na: 2\n")
    if status != 1 or out:
        fail(f"{ENV} does not refuse a duplicate key")
    _resolved = (binary, version)
    return _resolved


def _unique(pairs):
    seen = {}
    for key, value in pairs:
        if key in seen:
            fail(f"duplicate key {key!r} in yamlprims output")
        seen[key] = value
    return seen


def _constant(name):
    fail(f"non-finite number {name} in yamlprims output")


def load(data, label="YAML input"):
    """Parse YAML text or bytes to plain JSON values (dicts keep key order)."""
    if isinstance(data, str):
        data = data.encode("utf-8")
    binary, _ = resolve()
    status, out, err = run(binary, ["dump", "--plain", "-"], data)
    if status == 1:
        fail(f"{label}: yamlprims refused it: {err or 'no reason given'}")
    if status == 2:
        fail(f"{label}: yamlprims usage or I/O error: {err or 'no reason given'}")
    if status != 0:
        fail(f"{label}: yamlprims exited with status {status}")
    if not out.endswith(b"\n") or b"\n" in out[:-1]:
        fail(f"{label}: yamlprims output is not one JSON line")
    try:
        return json.loads(
            out.decode("utf-8"), object_pairs_hook=_unique, parse_constant=_constant
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"{label}: malformed yamlprims output: {error}")


def load_wire(data, label="YAML input"):
    """Parse YAML text or bytes to the yamlprims-value/1 documents list: each
    node keeps its kind, its source span ([start_byte, start_line,
    start_col, end_byte, end_line, end_col], lines and columns 1-based,
    columns in Unicode scalar values), alias or merge provenance (via) and
    explicit tag position (tag_start)."""
    if isinstance(data, str):
        data = data.encode("utf-8")
    binary, _ = resolve()
    status, out, err = run(binary, ["dump", "-"], data)
    if status == 1:
        raise Refused(f"{label}: yamlprims refused it: {err or 'no reason given'}")
    if status == 2:
        fail(f"{label}: yamlprims usage or I/O error: {err or 'no reason given'}")
    if status != 0:
        fail(f"{label}: yamlprims exited with status {status}")
    if not out.endswith(b"\n") or b"\n" in out[:-1]:
        fail(f"{label}: yamlprims output is not one JSON line")
    try:
        envelope = json.loads(
            out.decode("utf-8"), object_pairs_hook=_unique, parse_constant=_constant
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"{label}: malformed yamlprims output: {error}")
    if (
        not isinstance(envelope, dict)
        or envelope.get("format") != "yamlprims-value"
        or envelope.get("version") != 1
        or not isinstance(envelope.get("documents"), list)
    ):
        fail(f"{label}: yamlprims output is not a yamlprims-value/1 documents envelope")
    return envelope["documents"]


def main():
    if sys.argv[1:] == ["identify"]:
        resolve()
        print(source_path())
    elif sys.argv[1:] == ["version"]:
        # For the doctor, which compares the version with its own minimum.
        print(
            "yamlprims " + ".".join(map(str, read_version(authenticate(source_path()))))
        )
    else:
        fail("usage: yaml_json.py identify | version")


if __name__ == "__main__":
    try:
        main()
    except (ReaderError, OSError) as error:
        sys.exit(f"error: {error}")
