#!/usr/bin/env python3
"""Validate the ordered crates.io publication plan against Cargo metadata.

config/release/publishable-crates.txt lists the published library crates in
registry dependency order. Every crate in the list must exist in the workspace,
set publish = true, and follow every workspace crate it depends on (including
dev dependencies); every other workspace crate must stay unpublished.
"""

import json
import pathlib
import subprocess
import sys

if sys.version_info < (3, 11):
    sys.exit("error: release crate checks require Python 3.11 or newer")

import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent
PLAN = ROOT / "config/release/publishable-crates.txt"
UNPUBLISHED = ("sysprims-cli", "sysprims-ffi", "sysprims-ts-napi")


def read_plan(path=PLAN):
    names = path.read_text().splitlines()
    if not names or any(not n or n != n.strip() or n.startswith("#") for n in names):
        raise ValueError("crate list must contain one bare name per line")
    if len(names) != len(set(names)) or any(
        not n.startswith("sysprims-") for n in names
    ):
        raise ValueError("invalid or duplicate publishable crate name")
    return names


def validate(packages, names, manifests):
    by_name = {p["name"]: p for p in packages}
    if len(by_name) != len(packages):
        raise ValueError("duplicate workspace package")
    missing = [n for n in names if n not in by_name]
    if missing:
        raise ValueError(f"listed crate(s) not in this workspace: {', '.join(missing)}")
    publishable = {p["name"] for p in packages if p.get("publish") != []}
    extra = sorted(publishable - set(names))
    if extra:
        raise ValueError(
            f"publishable workspace crate(s) missing from the list: {', '.join(extra)}"
        )
    for name in names:
        if manifests[name]["package"].get("publish") is not True:
            raise ValueError(f"{name} must explicitly set publish = true")
        for dependency in by_name[name]["dependencies"]:
            dep = dependency["name"]
            if dep in by_name and dep not in names:
                raise ValueError(f"{name} depends on unpublished workspace crate {dep}")
            if dep in names and names.index(dep) >= names.index(name):
                raise ValueError(
                    f"{name} must follow {dep} (including dev dependencies)"
                )
    for name in UNPUBLISHED:
        if name in by_name and by_name[name].get("publish") != []:
            raise ValueError(f"{name} must remain unpublished")


def main():
    names = read_plan()
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"],
            cwd=ROOT,
        )
    )
    packages = metadata["packages"]
    manifests = {
        p["name"]: tomllib.loads(pathlib.Path(p["manifest_path"]).read_text())
        for p in packages
    }
    validate(packages, names, manifests)
    if sys.argv[1:] == ["list"]:
        print("\n".join(names))
    elif sys.argv[1:] == ["paths"]:
        by_name = {p["name"]: p for p in packages}
        for name in names:
            print(
                name,
                pathlib.Path(by_name[name]["manifest_path"]).parent.relative_to(ROOT),
            )
    elif sys.argv[1:] != ["check"]:
        raise ValueError("usage: release-crates.py check|list|paths")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError) as error:
        sys.exit(f"error: {error}")
