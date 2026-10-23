#!/usr/bin/env python3
"""Create DEST exclusively and write SRC's bytes through that same descriptor.

Usage: write-exclusive.py SRC DEST

DEST is opened with O_CREAT|O_EXCL|O_NOFOLLOW, so an existing file or symlink
at DEST refuses the write, and the bytes go through the descriptor that was
created, never through a second open of the path. On a write failure the file
is removed if the path still names the inode this call created.
Exit codes: 0 written; 3 nothing was created (DEST existed, or any failure
before the create); any other status means DEST was created by this call
(callers treat it as theirs to remove; the helper also removes it itself on a
write error).
"""

import os
import sys


def main():
    if len(sys.argv) != 3:
        print("usage: write-exclusive.py SRC DEST", file=sys.stderr)
        sys.exit(3)
    src, dest = sys.argv[1:]
    try:
        with open(src, "rb") as handle:
            data = handle.read()
    except OSError as error:
        print(f"error: cannot read {src}: {error}", file=sys.stderr)
        sys.exit(3)
    try:
        fd = os.open(dest, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
    except FileExistsError:
        print(f"error: {dest} already exists", file=sys.stderr)
        sys.exit(3)
    except OSError as error:
        print(f"error: cannot create {dest}: {error}", file=sys.stderr)
        sys.exit(3)
    created = os.fstat(fd)
    try:
        view = memoryview(data)
        while view:
            written = os.write(fd, view)
            view = view[written:]
        os.fsync(fd)
    except OSError as error:
        os.close(fd)
        try:
            current = os.lstat(dest)
            if (current.st_dev, current.st_ino) == (created.st_dev, created.st_ino):
                os.unlink(dest)
        except FileNotFoundError:
            pass
        sys.exit(f"error: writing {dest} failed: {error}")
    os.close(fd)


if __name__ == "__main__":
    main()
