#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Extract the yamlprims binary from a release archive (CI only).

Usage: ci-extract-yamlprims.py ARCHIVE MEMBER OUT SHA256

Reads exactly MEMBER (a regular file) from ARCHIVE (.tar.gz or .zip), refuses
it unless its SHA-256 is SHA256 (the committed binary digest for the runner
platform, config/yamlprims-cli.txt), and writes it to OUT, which must not
exist. Nothing else in the archive is read or written. Standard library only.
"""

import os
import sys

_untrusted = {
    os.path.realpath(p)
    for p in (os.path.dirname(os.path.abspath(__file__)), os.getcwd())
}
sys.path[:] = [
    p for p in sys.path if p not in ("", ".") and os.path.realpath(p) not in _untrusted
]
import hashlib  # noqa: E402
import re  # noqa: E402
import tarfile  # noqa: E402
import zipfile  # noqa: E402

MAX_BINARY = 64 * 1024 * 1024


def extract(archive, member):
    if archive.endswith(".zip"):
        with zipfile.ZipFile(archive) as zf:
            try:
                info = zf.getinfo(member)
            except KeyError:
                sys.exit(f"error: {os.path.basename(archive)} has no member {member}")
            if info.is_dir() or info.file_size > MAX_BINARY:
                sys.exit(
                    f"error: {member} in {os.path.basename(archive)} is not a binary"
                )
            return zf.read(info)
    with tarfile.open(archive, "r:gz") as tf:
        try:
            info = tf.getmember(member)
        except KeyError:
            sys.exit(f"error: {os.path.basename(archive)} has no member {member}")
        if not info.isreg() or info.size > MAX_BINARY:
            sys.exit(
                f"error: {member} in {os.path.basename(archive)} is not a regular file"
            )
        return tf.extractfile(info).read()


def main():
    if len(sys.argv) != 5 or not re.fullmatch(r"[0-9a-f]{64}", sys.argv[4]):
        sys.exit("usage: ci-extract-yamlprims.py ARCHIVE MEMBER OUT SHA256")
    archive, member, out, want = sys.argv[1:]
    try:
        data = extract(archive, member)
    except (OSError, tarfile.TarError, zipfile.BadZipFile) as error:
        sys.exit(f"error: cannot read {os.path.basename(archive)}: {error}")
    got = hashlib.sha256(data).hexdigest()
    if got != want:
        sys.exit(f"error: {member} sha256 {got} != committed {want}")
    try:
        with open(out, "xb") as handle:
            handle.write(data)
        os.chmod(out, 0o755)
    except OSError as error:
        sys.exit(f"error: cannot write {out}: {error.strerror or error}")


if __name__ == "__main__":
    main()
