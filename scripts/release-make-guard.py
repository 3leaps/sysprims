#!/usr/bin/env python3
"""Which Makefile targets read the release tag, and are they all guarded?

A target reads the tag when its recipe mentions SYSPRIMS_RELEASE_TAG, runs a
script that reads it (directly or through another script it runs), or depends
on or sub-makes a target that does. Every such target must list
release-require-tag as its first prerequisite, and none may appear in
RELEASE_TAG_FREE_TARGETS. Prints the tag-reading targets; exits 1 on a gap.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
TAG = "SYSPRIMS_RELEASE_TAG"


def script_readers():
    scripts = {}
    for path in (ROOT / "scripts").iterdir():
        if (
            path.suffix in (".sh", ".mjs", ".py")
            and ".test." not in path.name
            and path.is_file()
        ):
            scripts[path.name] = path.read_text(errors="replace")
    readers = {name for name, text in scripts.items() if TAG in text}
    changed = True
    while changed:
        changed = False
        for name, text in scripts.items():
            if name not in readers and any(other in text for other in readers):
                readers.add(name)
                changed = True
    return readers


def parse_makefile():
    text = (ROOT / "Makefile").read_text()
    text = re.sub(r"\\\n", " ", text)
    rules, current = {}, None
    for line in text.split("\n"):
        match = re.match(r"^([A-Za-z][A-Za-z0-9_.-]*):(?!=)([^#]*)", line)
        if match and not line.startswith("\t"):
            current = match.group(1)
            rules.setdefault(current, {"prereqs": match.group(2).split(), "recipe": []})
        elif line.startswith("\t") and current:
            rules[current]["recipe"].append(line)
        elif line.strip() and not line.startswith("#"):
            current = None
    free = re.search(r"^RELEASE_TAG_FREE_TARGETS\s*:=\s*(.*)$", text, re.M)
    serial = re.search(r"^\.NOTPARALLEL:\s*(.*)$", text, re.M)
    return (
        rules,
        set(free.group(1).split()) if free else set(),
        set(serial.group(1).split()) if serial else None,
    )


def main():
    readers = script_readers()
    rules, free, serial = parse_makefile()
    direct = set()
    edges = {}
    for name, rule in rules.items():
        # Echoed help text is not a use; expansions and script runs are.
        recipe = "\n".join(
            line for line in rule["recipe"] if not re.match(r"^\t@?echo\b", line)
        )
        if (
            f"$({TAG})" in recipe
            or f"${{{TAG}}}" in recipe
            or any(script in recipe for script in readers)
        ):
            direct.add(name)
        sub_makes = re.findall(r"\$\(MAKE\)\s+([a-z][a-z0-9-]*)", recipe)
        edges[name] = [p for p in rule["prereqs"] + sub_makes if p in rules]
    reading = set(direct)
    changed = True
    while changed:
        changed = False
        for name, deps in edges.items():
            if name not in reading and any(dep in reading for dep in deps):
                reading.add(name)
                changed = True
    reading.discard("release-require-tag")
    errors = []
    for name in sorted(reading):
        if (
            not rules[name]["prereqs"]
            or rules[name]["prereqs"][0] != "release-require-tag"
        ):
            errors.append(
                f"{name} reads the release tag but does not start with release-require-tag"
            )
        if name in free:
            errors.append(
                f"{name} is listed in RELEASE_TAG_FREE_TARGETS but reads the release tag"
            )
        if serial is None or name not in serial:
            errors.append(f"{name} reads the release tag but is not in .NOTPARALLEL")
    for name in sorted(free):
        if name not in rules:
            errors.append(f"RELEASE_TAG_FREE_TARGETS names unknown target {name}")
    for line in errors:
        print(f"error: {line}", file=sys.stderr)
    print("\n".join(sorted(reading)))
    sys.exit(1 if errors else 0)


if __name__ == "__main__":
    main()
