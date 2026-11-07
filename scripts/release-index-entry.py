#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Read one version's entry from a sparse registry index.

Usage: release-index-entry.py [--allow-http] [--max-bytes N] [--max-line-bytes N] [--timeout SEC] INDEX_URL CRATE VERSION
INDEX_URL is a sparse index base such as https://index.crates.io/. Prints the
cksum of the version's last index line (the one Cargo uses) and exits 0 when
that line is not yanked; exits 1 when it is absent; exits 3 when that line is
yanked; exits 2 on any other failure. Reads the index only: no
crate is downloaded and nothing is authenticated.
"""

from __future__ import annotations

import http.client
import json
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

USER_AGENT = "sysprims-release-verify/1.0 (+https://github.com/3leaps/sysprims)"
CRATE_NAME_RE = re.compile(r"^[A-Za-z0-9_-]{1,64}$")
DEFAULT_MAX_BYTES = 16 * 1024 * 1024
DEFAULT_MAX_LINE_BYTES = 512 * 1024
DEFAULT_TIMEOUT_SEC = 30.0
CHUNK_BYTES = 65536
MAX_REDIRECTS = 5


class IndexFetchError(Exception):
    pass


def index_path(name: str) -> str:
    if not CRATE_NAME_RE.fullmatch(name):
        raise IndexFetchError(f"invalid crate name: {name!r}")
    lower = name.lower()
    if len(lower) <= 2:
        return f"{len(lower)}/{lower}"
    if len(lower) == 3:
        return f"3/{lower[0]}/{lower}"
    return f"{lower[:2]}/{lower[2:4]}/{lower}"


def validate_index_url(url: str, *, allow_http: bool) -> str:
    parsed = urllib.parse.urlsplit(url)
    allowed_schemes = ("https", "http") if allow_http else ("https",)
    if parsed.scheme not in allowed_schemes:
        if parsed.scheme == "http" and not allow_http:
            raise IndexFetchError("plain HTTP sparse-index URL requires --allow-http")
        raise IndexFetchError(f"unsupported sparse-index URL scheme: {parsed.scheme!r}")
    if not parsed.netloc or parsed.hostname is None:
        raise IndexFetchError("sparse-index URL is missing a host")
    if parsed.username is not None or parsed.password is not None:
        raise IndexFetchError("sparse-index URL must not include credentials")
    if parsed.query or parsed.fragment:
        raise IndexFetchError("sparse-index URL must not include query or fragment")
    return url


class _SafeRedirectHandler(urllib.request.HTTPRedirectHandler):
    def __init__(self, *, allow_http: bool) -> None:
        super().__init__()
        self._allow_http = allow_http
        self._redirects = 0

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        self._redirects += 1
        if self._redirects > MAX_REDIRECTS:
            raise urllib.error.URLError("too many sparse-index redirects")
        src_scheme = urllib.parse.urlsplit(req.full_url).scheme
        resolved = urllib.parse.urljoin(req.full_url, newurl)
        dst = urllib.parse.urlsplit(resolved)
        if dst.scheme not in ("https", "http"):
            raise urllib.error.URLError(
                f"unsupported redirect URL scheme: {dst.scheme!r}"
            )
        if src_scheme == "https" and dst.scheme != "https":
            raise urllib.error.URLError("refusing HTTPS-to-HTTP sparse-index redirect")
        if dst.scheme == "http" and not self._allow_http:
            raise urllib.error.URLError("refusing plain HTTP sparse-index redirect")
        if dst.username is not None or dst.password is not None:
            raise urllib.error.URLError("redirect URL must not include credentials")
        return super().redirect_request(req, fp, code, msg, headers, resolved)


def _set_socket_timeout(response: object, remaining: float) -> None:
    fp = getattr(response, "fp", None)
    raw = getattr(fp, "raw", None)
    sock = getattr(raw, "_sock", None)
    if sock is not None and hasattr(sock, "settimeout"):
        sock.settimeout(remaining)


def read_bounded_lines(
    response: http.client.HTTPResponse,
    *,
    deadline: float,
    max_bytes: int,
    max_line_bytes: int,
) -> list[str]:
    content_length_hdr = response.headers.get("Content-Length")
    transfer_encoding = (response.headers.get("Transfer-Encoding") or "").lower()
    expected_length: int | None = None
    if content_length_hdr is not None and "chunked" not in transfer_encoding:
        try:
            expected_length = int(content_length_hdr.strip())
        except ValueError as exc:
            raise IndexFetchError(
                f"invalid Content-Length header: {content_length_hdr!r}"
            ) from exc
        if expected_length < 0:
            raise IndexFetchError(
                f"negative Content-Length header: {content_length_hdr!r}"
            )
        if expected_length > max_bytes:
            raise IndexFetchError(
                f"sparse-index Content-Length {expected_length} exceeds limit {max_bytes}"
            )

    total = 0
    pending = bytearray()
    lines: list[str] = []

    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("sparse-index request exceeded overall deadline")
        _set_socket_timeout(response, remaining)
        chunk = response.read(min(CHUNK_BYTES, max_bytes - total + 1))
        if not chunk:
            break
        total += len(chunk)
        if total > max_bytes:
            raise IndexFetchError(
                f"sparse-index response exceeds {max_bytes} byte limit"
            )
        pending.extend(chunk)
        while True:
            nl = pending.find(b"\n")
            if nl == -1:
                if len(pending) > max_line_bytes:
                    raise IndexFetchError(
                        f"sparse-index line exceeds {max_line_bytes} byte limit"
                    )
                break
            raw_line = bytes(pending[:nl])
            del pending[: nl + 1]
            if raw_line.endswith(b"\r"):
                raw_line = raw_line[:-1]
            if len(raw_line) > max_line_bytes:
                raise IndexFetchError(
                    f"sparse-index line exceeds {max_line_bytes} byte limit"
                )
            lines.append(raw_line.decode("utf-8"))

    # Python's HTTPResponse.read(amt) returns b"" on premature EOF without
    # raising IncompleteRead; check both response.length and Content-Length.
    rem_len = getattr(response, "length", None)
    if isinstance(rem_len, int) and rem_len > 0:
        raise IndexFetchError(
            f"truncated sparse-index response ({rem_len} byte(s) unread)"
        )
    if expected_length is not None and total != expected_length:
        raise IndexFetchError(
            f"truncated sparse-index response (got {total} of {expected_length} bytes)"
        )
    if pending:
        raw_tail = bytes(pending[:-1] if pending.endswith(b"\r") else pending)
        if len(raw_tail) > max_line_bytes:
            raise IndexFetchError(
                f"sparse-index line exceeds {max_line_bytes} byte limit"
            )
        lines.append(raw_tail.decode("utf-8"))

    return lines


def parse_args(argv: list[str]) -> tuple[bool, int, int, float, str, str, str]:
    allow_http = False
    max_bytes = DEFAULT_MAX_BYTES
    max_line_bytes = DEFAULT_MAX_LINE_BYTES
    timeout_sec = DEFAULT_TIMEOUT_SEC
    idx = 0
    while idx < len(argv) and argv[idx].startswith("--"):
        flag = argv[idx]
        if flag == "--allow-http":
            allow_http = True
            idx += 1
        elif flag == "--max-bytes" and idx + 1 < len(argv):
            max_bytes = int(argv[idx + 1])
            idx += 2
        elif flag == "--max-line-bytes" and idx + 1 < len(argv):
            max_line_bytes = int(argv[idx + 1])
            idx += 2
        elif flag == "--timeout" and idx + 1 < len(argv):
            timeout_sec = float(argv[idx + 1])
            idx += 2
        else:
            raise IndexFetchError(
                "usage: release-index-entry.py [--allow-http] [--max-bytes N] "
                "[--max-line-bytes N] [--timeout SEC] INDEX_URL CRATE VERSION"
            )
    rest = argv[idx:]
    if len(rest) != 3 or max_bytes < 1 or max_line_bytes < 1 or timeout_sec <= 0:
        raise IndexFetchError(
            "usage: release-index-entry.py [--allow-http] [--max-bytes N] "
            "[--max-line-bytes N] [--timeout SEC] INDEX_URL CRATE VERSION"
        )
    return (
        allow_http,
        max_bytes,
        max_line_bytes,
        timeout_sec,
        rest[0],
        rest[1],
        rest[2],
    )


def main() -> int:
    try:
        (
            allow_http,
            max_bytes,
            max_line_bytes,
            timeout_sec,
            base,
            crate,
            version,
        ) = parse_args(sys.argv[1:])
        base = base.removeprefix("sparse+").rstrip("/") + "/"
        validate_index_url(base, allow_http=allow_http)
        target_url = base + index_path(crate)
        validate_index_url(target_url, allow_http=allow_http)
    except (IndexFetchError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    deadline = time.monotonic() + timeout_sec
    opener = urllib.request.build_opener(_SafeRedirectHandler(allow_http=allow_http))
    request = urllib.request.Request(
        target_url,
        headers={"User-Agent": USER_AGENT, "Accept-Encoding": "identity"},
    )
    try:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("sparse-index request exceeded overall deadline")
        with opener.open(request, timeout=remaining) as response:
            lines = read_bounded_lines(
                response,
                deadline=deadline,
                max_bytes=max_bytes,
                max_line_bytes=max_line_bytes,
            )
    except urllib.error.HTTPError as error:
        if error.code in (404, 410):
            return 1
        print(f"error: index request failed: HTTP {error.code}", file=sys.stderr)
        return 2
    except (
        IndexFetchError,
        urllib.error.URLError,
        http.client.HTTPException,
        TimeoutError,
        OSError,
        UnicodeDecodeError,
    ) as error:
        print(f"error: index request failed: {error}", file=sys.stderr)
        return 2

    # Cargo keeps the last line for a version (a yank or edit is appended), so
    # the last matching line is authoritative.
    found = None
    for line in lines:
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
