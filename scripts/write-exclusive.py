#!/usr/bin/env python3
"""Create DEST exclusively and write SRC's bytes through that same descriptor.

Usage: write-exclusive.py SRC DEST
       write-exclusive.py --remove DEST TOKEN

DEST is opened with O_CREAT|O_EXCL|O_NOFOLLOW, so an existing file or symlink
at DEST refuses the write, and the bytes go through the descriptor that was
created, never through a second open of the path. On success it prints an
ownership token, DEV:INO:SIZE:SHA256 of the file it created and the bytes it
wrote.
Exit codes: 0 written, and DEST is the caller's (identified by the token);
3 nothing was created (DEST existed, or any failure before the create); any
other status (a write error, a crash, a signal) leaves ownership unknown, and
the caller must not claim DEST. On a write error the helper rolls back what
it wrote itself, as --remove does.

--remove never deletes bytes this caller did not write. It moves DEST aside
to an unpredictable sibling name (renameat through a directory descriptor
opened O_DIRECTORY|O_NOFOLLOW), then checks the captured object: a regular
file with the token's device, inode, size and SHA-256. Only then is it
unlinked. Otherwise it is linked back to DEST (link never replaces an existing
name) and the sibling removed; if another file appeared at DEST meanwhile,
the sibling is kept and named. A reused inode number with other bytes fails
the hash; byte-identical content is exactly what rollback would remove.

Residual: DEST is briefly absent while it is captured, and a swap of DEST's
parent directory would need directory-descriptor operations on every step.
Both need a same-user process already acting in the checkout. Rollback is
best-effort under concurrent modification by the same user, and exact for
"never delete bytes we did not write".
"""

import hashlib
import os
import re
import secrets
import stat
import sys

TOKEN = re.compile(r"[0-9]+:[0-9]+:[0-9]+:[0-9a-f]{64}")


def token(info, data):
    return f"{info.st_dev}:{info.st_ino}:{len(data)}:{hashlib.sha256(data).hexdigest()}"


def captured_matches(directory, name, expected):
    """Whether the captured object is a regular file matching the token."""
    try:
        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=directory)
    except OSError:
        return False
    with os.fdopen(fd, "rb") as handle:
        info = os.fstat(handle.fileno())
        if not stat.S_ISREG(info.st_mode):
            return False
        return token(info, handle.read()) == expected


def remove(dest, expected):
    if not TOKEN.fullmatch(expected):
        sys.exit(f"error: malformed ownership token for {dest}; left in place")
    parent, name = os.path.split(os.path.abspath(dest))
    directory = os.open(parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        aside = f".{name}.rollback-{secrets.token_hex(16)}"
        try:
            os.rename(name, aside, src_dir_fd=directory, dst_dir_fd=directory)
        except FileNotFoundError:
            return
        if captured_matches(directory, aside, expected):
            os.unlink(aside, dir_fd=directory)
            return
        try:
            os.link(
                aside,
                name,
                src_dir_fd=directory,
                dst_dir_fd=directory,
                follow_symlinks=False,
            )
        except FileExistsError:
            print(
                f"warning: {dest} is not the file this run created, and another file "
                f"appeared there; the captured file is kept at {os.path.join(parent, aside)}",
                file=sys.stderr,
            )
            return
        os.unlink(aside, dir_fd=directory)
        print(
            f"warning: {dest} is not the file this run created; left in place",
            file=sys.stderr,
        )
    finally:
        os.close(directory)


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
    done = 0
    try:
        while done < len(data):
            done += os.write(fd, data[done:])
        os.fsync(fd)
    except OSError as error:
        os.close(fd)
        remove(dest, token(created, data[:done]))
        sys.exit(f"error: writing {dest} failed: {error}")
    print(token(created, data), flush=True)
    os.close(fd)


if __name__ == "__main__":
    main()
