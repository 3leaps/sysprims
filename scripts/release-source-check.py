#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Check the tagged release source before any Cargo command runs on it.

Cargo and rustup act on files in the tree they run in: toolchain selectors,
.cargo configuration, build scripts, and manifest paths that point elsewhere.
Rather than enumerate what each could do, the release source must contain none
of them (except the exact unpublished FFI/N-API binding crates allowlisted
below). The commit is read with git plumbing only; no file from it executes.

Usage: release-source-check.py <commit> [--repo DIR]
       release-source-check.py <commit> --repo DIR --worktree TREE
         also require every tracked file in TREE to equal its tagged blob
Refuses, anywhere in the commit's tree:
  - a symlink or a submodule;
  - rust-toolchain or rust-toolchain.toml;
  - any path under a .cargo directory;
  - a build.rs outside the allowlisted unpublished FFI/N-API binding crates;
  - a Cargo.toml whose [package] build, readme, license-file, include or
    exclude, or whose [lib]/[[bin]]/[[test]]/[[bench]]/[[example]] paths, are
    absolute or climb out with "..";
  - a [package] build key other than false on any published or crates/* crate,
    or a procedural-macro target;
  - readme, license-file, include, exclude or build inherited from the
    workspace, and escaping [workspace.package] values;
  - a workspace member outside crates/ other than the allowlisted unpublished
    FFI/N-API binding crates;
  - a dependency (any table, including target-specific and workspace) from
    git or another registry, or with a path that is not a crate in the tree;
    [patch], [replace] and package.workspace.
Requires every crate in config/release/publishable-crates.txt (read from the
commit) at crates/<name>/Cargo.toml naming that package.
"""

import pathlib
import re
import subprocess
import sys
import tomllib

TARGET_TABLES = ("lib", "bin", "test", "bench", "example")
ALLOWED_UNPUBLISHED_MEMBERS = {
    "bindings/typescript/sysprims/native",
    "ffi/sysprims-ffi",
}
ALLOWED_BUILD_SCRIPTS = {
    "bindings/typescript/sysprims/native/build.rs",
}
ALLOWED_BUILD_MANIFESTS = {
    "bindings/typescript/sysprims/native/Cargo.toml",
}


def git(repo, *args):
    return subprocess.run(
        ["git", "-C", repo, *args], check=True, capture_output=True
    ).stdout


def entries(repo, commit):
    raw = git(repo, "ls-tree", "-r", "-z", "--full-tree", commit)
    for record in raw.split(b"\0"):
        if not record:
            continue
        meta, path = record.split(b"\t", 1)
        mode, kind, _ = meta.split(b" ")
        yield mode.decode(), kind.decode(), path.decode("utf-8", "surrogateescape")


def contained(value):
    path = pathlib.PurePosixPath(value)
    return (
        not path.is_absolute()
        and ".." not in path.parts
        and "\\" not in value
        and not re.match(r"[A-Za-z]:", value)
    )


PATH_KEYS = ("readme", "license-file", "include", "exclude", "build")


def check_paths(path, table, label):
    """readme/license-file/include/exclude are relative paths that stay put;
    build is refused on release crates; none of them may be inherited."""
    for key in PATH_KEYS:
        if key not in table:
            continue
        value = table[key]
        if isinstance(value, dict):
            raise ValueError(
                f"{path}: {label}.{key} may not be inherited from the workspace"
            )
        if key == "build":
            if value is not False and not (
                label == "package"
                and path in ALLOWED_BUILD_MANIFESTS
                and value == "build.rs"
            ):
                raise ValueError(
                    f"{path}: a build script is not allowed in the release source"
                )
        elif key in ("readme", "license-file"):
            if not isinstance(value, (str, bool)) or (
                isinstance(value, str) and not contained(value)
            ):
                raise ValueError(f"{path}: {label}.{key} must stay inside the crate")
        else:
            if not isinstance(value, list) or any(
                not isinstance(v, str) or not contained(v) for v in value
            ):
                raise ValueError(
                    f"{path}: {label}.{key} entries must stay inside the crate"
                )


DEPENDENCY_TABLES = (
    "dependencies",
    "dev-dependencies",
    "dev_dependencies",
    "build-dependencies",
    "build_dependencies",
)


def dependency_tables(data):
    """Every dependency table in a manifest, with a label."""
    for name in DEPENDENCY_TABLES:
        if name in data:
            yield name, data[name]
    if "dependencies" in data.get("workspace", {}):
        yield "workspace.dependencies", data["workspace"]["dependencies"]
    for target, table in data.get("target", {}).items():
        for name in DEPENDENCY_TABLES:
            if name in table:
                yield f"target.{target}.{name}", table[name]


def check_dependencies(path, data, manifests):
    """Dependencies come from crates.io or from a crate in this tree: no git
    source, no other registry, and a path only to a manifest in the tree."""
    for key in ("patch", "replace"):
        if key in data:
            raise ValueError(f"{path}: [{key}] is not allowed in the release source")
    if "workspace" in data.get("package", {}):
        raise ValueError(
            f"{path}: package.workspace is not allowed in the release source"
        )
    base = pathlib.PurePosixPath(path).parent
    for label, table in dependency_tables(data):
        if not isinstance(table, dict):
            raise ValueError(f"{path}: [{label}] must be a table")
        for name, spec in table.items():
            if not isinstance(spec, dict):
                continue  # a bare version requirement: crates.io
            for key in (
                "git",
                "registry",
                "registry-index",
                "registry_index",
                "branch",
                "tag",
                "rev",
            ):
                if key in spec:
                    raise ValueError(
                        f"{path}: {label}.{name} uses {key}; only crates.io or a path in the tree"
                    )
            if "path" in spec:
                value = spec["path"]
                if (
                    not isinstance(value, str)
                    or pathlib.PurePosixPath(value).is_absolute()
                    or "\\" in value
                    or re.match(r"[A-Za-z]:", value)
                ):
                    raise ValueError(
                        f"{path}: {label}.{name} path must stay inside the tree"
                    )
                target = pathlib.PurePosixPath(base, value.rstrip("/"), "Cargo.toml")
                parts = []
                for part in target.parts:
                    if part == "..":
                        if not parts:
                            raise ValueError(
                                f"{path}: {label}.{name} path must stay inside the tree"
                            )
                        parts.pop()
                    elif part != ".":
                        parts.append(part)
                if "/".join(parts) not in manifests:
                    raise ValueError(
                        f"{path}: {label}.{name} path must name a crate in the tree"
                    )


def check_manifest(path, data):
    if "project" in data:
        # [project] is Cargo's legacy alias for [package]; checks read
        # [package], so the alias is refused rather than half-checked.
        raise ValueError(
            f"{path}: the legacy [project] table is not allowed; use [package]"
        )
    check_paths(path, data.get("package", {}), "package")
    check_paths(path, data.get("workspace", {}).get("package", {}), "workspace.package")
    for table in TARGET_TABLES:
        targets = data.get(table, [])
        for target in [targets] if isinstance(targets, dict) else targets:
            value = target.get("path")
            if isinstance(value, str) and not contained(value):
                raise ValueError(f"{path}: [{table}] path must stay inside the crate")
            # A procedural macro runs inside the compiler during Cargo's
            # package verification; the release source carries none.
            kinds = []
            for key in ("crate-type", "crate_type"):
                value = target.get(key, [])
                kinds += [value] if isinstance(value, str) else list(value)
            if (
                target.get("proc-macro")
                or target.get("proc_macro")
                or "proc-macro" in kinds
            ):
                raise ValueError(
                    f"{path}: a procedural macro is not allowed in the release source"
                )
    members = data.get("workspace", {}).get("members", [])
    for member in members:
        if (
            not isinstance(member, str)
            or not contained(member)
            or (
                not member.startswith("crates/")
                and member not in ALLOWED_UNPUBLISHED_MEMBERS
            )
        ):
            raise ValueError(f"{path}: workspace members must be under crates/")


def is_unpublished(manifest_data, workspace_publish_default):
    pub = manifest_data.get("package", {}).get("publish")
    if pub is False or pub == []:
        return True
    if pub == {"workspace": True} and (
        workspace_publish_default is False or workspace_publish_default == []
    ):
        return True
    return False


def check(repo, commit):
    manifests = []
    folded_paths = {}
    for mode, kind, path in entries(repo, commit):
        parts = pathlib.PurePosixPath(path).parts
        # Names are compared case-folded: macOS and Windows filesystems are
        # case-insensitive, so .CARGO/config.toml is .cargo/config.toml there.
        folded = [part.casefold() for part in parts]
        name = folded[-1]
        if "/".join(folded) in folded_paths:
            raise ValueError(
                f"{path}: collides with {folded_paths['/'.join(folded)]} on a case-insensitive filesystem"
            )
        folded_paths["/".join(folded)] = path
        if mode == "120000":
            raise ValueError(f"{path}: symlinks are not allowed in the release source")
        if kind == "commit":
            raise ValueError(
                f"{path}: submodules are not allowed in the release source"
            )
        if name in ("rust-toolchain", "rust-toolchain.toml"):
            raise ValueError(
                f"{path}: a toolchain selector is not allowed in the release source"
            )
        if ".cargo" in folded:
            raise ValueError(
                f"{path}: Cargo configuration is not allowed in the release source"
            )
        if name == "build.rs":
            if path not in ALLOWED_BUILD_SCRIPTS or parts[-1] != "build.rs":
                raise ValueError(
                    f"{path}: a build script is not allowed in the release source"
                )
        if name == "cargo.toml":
            if parts[-1] != "Cargo.toml":
                raise ValueError(f"{path}: a manifest must be named exactly Cargo.toml")
            manifests.append(path)
    parsed = {}
    for path in manifests:
        data = tomllib.loads(git(repo, "cat-file", "blob", f"{commit}:{path}").decode())
        check_manifest(path, data)
        check_dependencies(path, data, set(manifests))
        parsed[path] = data
    ws_publish = (
        parsed.get("Cargo.toml", {})
        .get("workspace", {})
        .get("package", {})
        .get("publish")
    )
    for member in ALLOWED_UNPUBLISHED_MEMBERS:
        path = f"{member}/Cargo.toml"
        if path in parsed and not is_unpublished(parsed[path], ws_publish):
            raise ValueError(
                f"{path}: allowlisted FFI/binding crate must set publish = false"
            )
    plan = (
        git(repo, "cat-file", "blob", f"{commit}:config/release/publishable-crates.txt")
        .decode()
        .splitlines()
    )
    for crate in plan:
        path = f"crates/{crate}/Cargo.toml"
        if parsed.get(path, {}).get("package", {}).get("name") != crate:
            raise ValueError(f"published crate {crate} must be at {path}")


def check_worktree(repo, commit, tree):
    """Every tracked file in the checked-out tree is byte-identical to its blob
    in the commit, so checkout attributes or filters rewrote nothing."""
    root = pathlib.Path(tree)
    expected = {}
    for mode, kind, path in entries(repo, commit):
        if kind == "blob":
            expected[path] = git(repo, "rev-parse", f"{commit}:{path}").decode().strip()
    paths = list(expected)
    hashed = (
        subprocess.run(
            ["git", "-C", repo, "hash-object", "--no-filters", "--stdin-paths"],
            input="\n".join(str(root / p) for p in paths).encode(),
            check=True,
            capture_output=True,
        )
        .stdout.decode()
        .split()
    )
    if len(hashed) != len(paths):
        raise ValueError("could not hash the checked-out release source")
    changed = [p for p, h in zip(paths, hashed) if h != expected[p]]
    if changed:
        raise ValueError(
            f"checked-out bytes differ from the tagged blobs: {', '.join(changed[:5])}"
        )


def main():
    args = sys.argv[1:]
    if len(args) == 5 and args[1] == "--repo" and args[3] == "--worktree":
        check(args[2], args[0])
        check_worktree(args[2], args[0], args[4])
        print("[ok] checked-out release source equals the tagged blobs")
        return
    repo = "."
    if len(args) == 3 and args[1] == "--repo":
        repo = args[2]
        args = args[:1]
    if len(args) != 1:
        raise ValueError("usage: release-source-check.py <commit> [--repo DIR]")
    check(repo, args[0])
    print(
        "[ok] release source has no toolchain selector, Cargo configuration, build script, symlink or escaping path"
    )


if __name__ == "__main__":
    try:
        main()
    except (
        ValueError,
        subprocess.CalledProcessError,
        tomllib.TOMLDecodeError,
        UnicodeDecodeError,
    ) as error:
        detail = (
            str(error) if isinstance(error, ValueError) else "unreadable release source"
        )
        sys.exit("error: " + detail)
