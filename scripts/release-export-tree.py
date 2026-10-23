#!/usr/bin/env python3
"""Export a commit's tree as its exact blob bytes, verified file by file.

Usage: release-export-tree.py REPO COMMIT DEST

Writes every blob of COMMIT under DEST (which must not exist) straight from
`git cat-file`, with no checkout semantics: no autocrlf, eol or filter
conversion, no export-ignore or export-subst, no sparse-checkout. Then checks
that DEST holds exactly the tree's paths, that every regular file's
`git hash-object --no-filters` equals its blob id and its executable bit
matches the tree mode. Any difference refuses. Symlinks (mode 120000) and
submodules (gitlinks) are refused: packaging follows a symlink to whatever it
names, which the blob hash cannot vouch for. The export is also checked to
hold no symlink at all before anything reads it.
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
        if kind != "blob" or mode not in ("100644", "100755"):
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
        fd = os.open(
            target,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
            0o755 if mode == "100755" else 0o644,
        )
        with os.fdopen(fd, "wb") as handle:
            handle.write(content)
    verify(repo, entries, root)
    attach_commit(repo, commit, root)


def attach_commit(repo, commit, root):
    """Make the export a git checkout of COMMIT without rewriting any file.

    The commit is fetched from the local repository (no network) and the index
    is built from its tree; read-tree never touches the working files, so the
    verified bytes stay as written. Cargo then records the commit in
    .cargo_vcs_info.json, and, since nothing passes --allow-dirty, refuses to
    package if any file differs from its blob: a second, independent check.
    """

    def local(*args):
        result = subprocess.run(
            [
                "git",
                "-C",
                str(root),
                "-c",
                "core.autocrlf=false",
                "-c",
                "core.sparseCheckout=false",
                *args,
            ],
            capture_output=True,
            check=False,
        )
        if result.returncode != 0:
            raise SystemExit(
                f"error: git {' '.join(args)} in the export failed: {result.stderr.decode(errors='replace').strip()}"
            )
        return result.stdout.decode()

    full = git(repo, "rev-parse", "--verify", f"{commit}^{{commit}}").decode().strip()
    local("init", "-q")
    local("config", "core.autocrlf", "false")
    local("config", "core.sparseCheckout", "false")
    local("fetch", "-q", "--no-tags", str(pathlib.Path(repo).resolve()), full)
    local("update-ref", "HEAD", "FETCH_HEAD")
    if local("rev-parse", "HEAD").strip() != full:
        raise SystemExit("error: the export's HEAD is not the tagged commit")
    local("read-tree", "HEAD")
    local("update-index", "-q", "--refresh")
    status = local("status", "--porcelain", "--untracked-files=all", "--ignored=no")
    if status:
        raise SystemExit(f"error: the export differs from the tagged commit:\n{status}")


def verify(repo, entries, root):
    found = set()
    for current, dirs, files in os.walk(root):
        # The repository metadata attach_commit adds is not exported content.
        if pathlib.Path(current) == root and ".git" in dirs and ".git" not in entries:
            dirs.remove(".git")
        for name in dirs + files:
            if (pathlib.Path(current) / name).is_symlink():
                raise SystemExit(
                    f"error: the export contains a symlink: {(pathlib.Path(current) / name).relative_to(root)}"
                )
        for name in files:
            found.add(str((pathlib.Path(current) / name).relative_to(root)))
    if found != set(entries):
        missing = sorted(set(entries) - found)[:5]
        extra = sorted(found - set(entries))[:5]
        raise SystemExit(
            f"error: exported file set differs from the tree (missing {missing}, extra {extra})"
        )
    paths = list(entries)
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


if __name__ == "__main__":
    main()
