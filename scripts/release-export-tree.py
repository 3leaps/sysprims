#!/usr/bin/env python3
"""Export a commit's tree as its exact blob bytes, verified file by file.

Usage: release-export-tree.py REPO COMMIT DEST

Writes every blob of COMMIT under DEST (which must not exist) straight from
`git cat-file`, with no checkout semantics: no autocrlf, eol or filter
conversion, no export-ignore or export-subst, no sparse-checkout. Then checks
that DEST holds exactly the tree's paths, that every regular file's
`git hash-object --no-filters` equals its blob id and its executable bit
matches the tree mode, and that symlinks match their blobs. Any difference
refuses. Submodules (gitlinks) are refused.
"""

import os
import pathlib
import subprocess
import sys


def git(repo, *args, data=None):
    result = subprocess.run(
        ["git", "-C", repo, "-c", "core.autocrlf=false", "-c", "core.eol=lf", *args],
        input=data,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        raise SystemExit(
            f"error: git {' '.join(args)} failed: {result.stderr.decode(errors='replace').strip()}"
        )
    return result.stdout


def tree_entries(repo, commit):
    raw = git(repo, "ls-tree", "-r", "-z", "--full-tree", commit)
    entries = {}
    for record in raw.split(b"\0"):
        if not record:
            continue
        meta, path = record.split(b"\t", 1)
        mode, kind, oid = meta.decode().split()
        if kind != "blob" or mode not in ("100644", "100755", "120000"):
            raise SystemExit(
                f"error: unsupported tree entry {mode} {kind} {path.decode(errors='replace')}"
            )
        entries[path.decode()] = (mode, oid)
    return entries


def read_blobs(repo, oids):
    """Every blob's raw bytes in one `git cat-file --batch` call."""
    raw = git(
        repo, "cat-file", "--batch", data="".join(f"{oid}\n" for oid in oids).encode()
    )
    blobs, offset = {}, 0
    for oid in oids:
        header_end = raw.index(b"\n", offset)
        got, kind, size = raw[offset:header_end].decode().split()
        if got != oid or kind != "blob":
            raise SystemExit(f"error: unexpected object {got} {kind}")
        start = header_end + 1
        blobs[oid] = raw[start : start + int(size)]
        offset = start + int(size) + 1
    return blobs


def main():
    if len(sys.argv) != 4:
        raise SystemExit("usage: release-export-tree.py REPO COMMIT DEST")
    repo, commit, dest = sys.argv[1:]
    dest_path = pathlib.Path(dest)
    if dest_path.exists() or dest_path.is_symlink():
        raise SystemExit(f"error: {dest} already exists")
    entries = tree_entries(repo, commit)
    if not entries:
        raise SystemExit("error: the tree is empty")
    dest_path.mkdir(parents=True)
    root = dest_path.resolve()
    blobs = read_blobs(repo, sorted({oid for _, oid in entries.values()}))
    for path, (mode, oid) in sorted(entries.items()):
        target = root / path
        if root not in target.resolve().parents and target.resolve() != root:
            raise SystemExit(f"error: tree path escapes the export: {path}")
        target.parent.mkdir(parents=True, exist_ok=True)
        content = blobs[oid]
        if mode == "120000":
            os.symlink(content.decode(), target)
        else:
            fd = os.open(
                target,
                os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                0o755 if mode == "100755" else 0o644,
            )
            with os.fdopen(fd, "wb") as handle:
                handle.write(content)
    verify(repo, entries, root)


def verify(repo, entries, root):
    found = set()
    for current, dirs, files in os.walk(root):
        for name in files + [
            d for d in dirs if (pathlib.Path(current) / d).is_symlink()
        ]:
            found.add(str((pathlib.Path(current) / name).relative_to(root)))
    if found != set(entries):
        missing = sorted(set(entries) - found)[:5]
        extra = sorted(found - set(entries))[:5]
        raise SystemExit(
            f"error: exported file set differs from the tree (missing {missing}, extra {extra})"
        )
    paths = [p for p, (mode, _) in entries.items() if mode != "120000"]
    hashed = git(
        repo,
        "hash-object",
        "--no-filters",
        "--stdin-paths",
        data="\n".join(str(root / p) for p in paths).encode() + b"\n",
    )
    for path, oid in zip(paths, hashed.decode().split()):
        if oid != entries[path][1]:
            raise SystemExit(f"error: exported {path} differs from its blob")
        executable = os.stat(root / path).st_mode & 0o111 != 0
        if executable != (entries[path][0] == "100755"):
            raise SystemExit(f"error: exported {path} has the wrong mode")
    for path, (mode, oid) in entries.items():
        if mode == "120000":
            link = os.readlink(root / path).encode()
            if git(repo, "hash-object", "--stdin", data=link).decode().strip() != oid:
                raise SystemExit(
                    f"error: exported symlink {path} differs from its blob"
                )


if __name__ == "__main__":
    main()
