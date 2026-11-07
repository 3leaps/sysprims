#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Read one version's entry from a sparse registry index.

Usage: release-index-entry.py INDEX_URL CRATE VERSION
INDEX_URL is a sparse index base such as https://index.crates.io/. Prints the
cksum of the version's last index line (the one Cargo uses) and exits 0 when
that line is not yanked; exits 1 when it is absent; exits 3 when that line is
yanked; exits 2 on any other failure. Reads the index only: no
crate is downloaded and nothing is authenticated.
"""

import json
import sys
import urllib.error
import urllib.request

USER_AGENT = "sysprims-release-verify/1.0 (+https://github.com/3leaps/sysprims)"


def index_path(name):
    name = name.lower()
    if len(name) <= 2:
        return f"{len(name)}/{name}"
    if len(name) == 3:
        return f"3/{name[0]}/{name}"
    return f"{name[:2]}/{name[2:4]}/{name}"


def main():
    if len(sys.argv) != 4:
        print("usage: release-index-entry.py INDEX_URL CRATE VERSION", file=sys.stderr)
        return 2
    base, crate, version = sys.argv[1:]
    base = base.removeprefix("sparse+").rstrip("/") + "/"
    request = urllib.request.Request(
        base + index_path(crate),
        headers={"User-Agent": USER_AGENT},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            body = response.read().decode("utf-8")
    except urllib.error.HTTPError as error:
        if error.code in (404, 410):
            return 1
        print(f"error: index request failed: HTTP {error.code}", file=sys.stderr)
        return 2
    except (urllib.error.URLError, OSError, UnicodeDecodeError) as error:
        print(f"error: index request failed: {error}", file=sys.stderr)
        return 2
    # Cargo keeps the last line for a version (a yank or edit is appended), so
    # the last matching line is authoritative.
    found = None
    for line in body.splitlines():
        if not line.strip():
            continue
        entry = json.loads(line)
        if (
            not isinstance(entry, dict)
            or not isinstance(entry.get("name"), str)
            or not isinstance(entry.get("vers"), str)
        ):
            raise ValueError("an index line is not a version entry")
        if entry["name"].lower() == crate.lower() and entry["vers"] == version:
            if not isinstance(entry.get("cksum"), str):
                raise ValueError("cksum is not a string")
            found = entry
    if found is None:
        return 1
    if found.get("yanked"):
        # Absent for resolution, but the version exists: it can never be
        # published again.
        print("yanked", file=sys.stderr)
        return 3
    print(found["cksum"])
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, KeyError) as error:
        print(f"error: malformed index entry: {error}", file=sys.stderr)
        sys.exit(2)
