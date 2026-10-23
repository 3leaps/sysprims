#!/usr/bin/env python3
"""Create DEST exclusively and write SRC's bytes through that same descriptor.

Usage: write-exclusive.py SRC DEST
       write-exclusive.py --remove DEST DEV:INO

DEST is opened with O_CREAT|O_EXCL|O_NOFOLLOW, so an existing file or symlink
at DEST refuses the write, and the bytes go through the descriptor that was
created, never through a second open of the path. On success the created
file's identity is printed as DEV:INO. On a write failure the file is removed
if the path still names the inode this call created.
Exit codes: 0 written, and DEST is the caller's (identified by DEV:INO);
3 nothing was created (DEST existed, or any failure before the create); any
other status (a write error, a crash, a signal) leaves ownership unknown, and
the caller must not claim DEST.

--remove unlinks DEST only if it is still the inode DEV:INO, so a rollback
never removes a file that replaced the one this caller created.
"""

import os
import sys


def remove(dest, identity):
    try:
        current = os.lstat(dest)
    except FileNotFoundError:
        return
    if f"{current.st_dev}:{current.st_ino}" == identity:
        os.unlink(dest)
    else:
        print(
            f"warning: {dest} is no longer the file this run created; left in place",
            file=sys.stderr,
        )


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "--remove":
        remove(*sys.argv[2:])
        return
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
    print(f"{created.st_dev}:{created.st_ino}")


if __name__ == "__main__":
    main()
