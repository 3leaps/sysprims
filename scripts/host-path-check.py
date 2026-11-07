#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Fail if a built artifact embeds a build-host path.

Usage: host-path-check.py ARTIFACT PATH...

ARTIFACT is a wheel (every member is read) or any other file. Each member is
searched as raw bytes and also with its JSON string escapes (\\uXXXX, \\\\,
\\/) and URL percent-encoding decoded, alone and together, so a path written
as Jane%20Doe, J\\u00f6rg or J%C3%B6rg is still found. Each PATH is a
build-host directory (the checkout, the Cargo and rustup homes, $HOME). A
match is that directory followed by a separator. A directory of two or more
components (/home/someone, C:\\Users\\someone) matches anywhere. A
single-component directory (a container root such as /src) matches only at
the start of an absolute path, not preceded by a path or name character, so it
does not match the /src/ inside a remapped library path
(library/alloc/src/string.rs). "/" alone is ignored.
"""

import re
import sys
import tarfile
import zipfile

# The start of an absolute path: right after file:// (as in an SBOM's
# path+file:///... URL), or after any byte that is not a path or name
# character (a NUL in a string table, a quote in JSON, a space).
NOT_PRECEDED = rb"(?:(?<=file://)|(?<![A-Za-z0-9._~/\\-]))"
# One path separator as it appears in an artifact: a slash, a backslash, a
# JSON-escaped backslash (\\) or a URL-encoded one (%2F, %5C).
SEPARATOR = rb"(?:/|\\\\|\\|%2[Ff]|%5[Cc])+"


def needles(paths):
    """One pattern per directory: its components joined by any separator
    form, so C:\\Users\\someone also matches C:\\\\Users\\\\someone in JSON and
    file:///C:/Users/someone in a URL; a drive letter matches in either case."""
    out = []
    for path in paths:
        parts = [c for c in re.split(r"[/\\]", path) if c]
        if not parts:
            continue
        drive = re.fullmatch(r"([A-Za-z]):", parts[0])
        if drive:
            head = rb"(?:/)?[" + drive.group(1).upper().encode() + drive.group(1).lower().encode() + rb"]:"
            names = parts[1:]
        else:
            head = b""
            names = parts
        if not names:
            continue
        body = SEPARATOR.join(re.escape(n.encode()) for n in names)
        lead = head + SEPARATOR if head else SEPARATOR
        # A single named component (a container root such as /src) counts
        # only at the start of a path; two or more match anywhere.
        prefix = NOT_PRECEDED if len(names) < 2 and not head else b""
        out.append(re.compile(prefix + lead + body + SEPARATOR))
    return out


# A JSON string escape: one backslash, then a \uXXXX code or one of \\ \/ \" \b
# \f \n \r \t. A \u surrogate pair decodes to its one supplementary character.
JSON_ESCAPE = re.compile(rb"\\(u[0-9A-Fa-f]{4}|[\\/\"bfnrt])")
JSON_SURROGATE_PAIR = re.compile(rb"\\u(d[89ab][0-9a-f]{2})\\u(d[c-f][0-9a-f]{2})", re.I)
PERCENT = re.compile(rb"%([0-9A-Fa-f]{2})")
SIMPLE = {b"\\": b"\\", b"/": b"/", b'"': b'"', b"b": b"\b", b"f": b"\f", b"n": b"\n", b"r": b"\r", b"t": b"\t"}


def json_unescape(data):
    """Decode JSON string escapes in place; anything else is left as is."""

    def pair(m):
        hi, lo = int(m.group(1), 16), int(m.group(2), 16)
        return chr(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)).encode("utf-8", "surrogatepass")

    def one(m):
        e = m.group(1)
        if e[:1] == b"u":
            return chr(int(e[1:], 16)).encode("utf-8", "surrogatepass")
        return SIMPLE[e]

    return JSON_ESCAPE.sub(one, JSON_SURROGATE_PAIR.sub(pair, data))


def percent_decode(data):
    return PERCENT.sub(lambda m: bytes([int(m.group(1), 16)]), data)


def views(data):
    """The member's bytes and its JSON-decoded, URL-decoded and doubly decoded
    forms (both orders), without duplicates."""
    seen = []
    for view in (
        data,
        json_unescape(data),
        percent_decode(data),
        percent_decode(json_unescape(data)),
        json_unescape(percent_decode(data)),
    ):
        if view not in seen:
            seen.append(view)
    return seen


def members(artifact):
    if zipfile.is_zipfile(artifact):
        with zipfile.ZipFile(artifact) as z:
            for name in z.namelist():
                yield name, z.read(name)
    elif tarfile.is_tarfile(artifact):
        with tarfile.open(artifact, "r:*") as t:
            for member in t.getmembers():
                yield f"{member.name}#header", f"{member.name}\n{member.linkname}".encode("utf-8", "surrogateescape")
                if member.isfile():
                    extracted = t.extractfile(member)
                    if extracted is not None:
                        yield member.name, extracted.read()
    else:
        with open(artifact, "rb") as f:
            yield artifact.replace("\\", "/").rsplit("/", 1)[-1], f.read()


def main(argv):
    if len(argv) < 3:
        sys.exit("usage: host-path-check.py ARTIFACT PATH...")
    patterns = needles(argv[2:])
    for name, data in members(argv[1]):
        for view in views(data):
            for pattern in patterns:
                m = pattern.search(view)
                if m:
                    found = view[m.start():m.end() + 40].decode("ascii", "backslashreplace")
                    sys.exit(f"{name} embeds a build-host path: {found}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
