#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# The sparse-index reader: listed (0), absent (1), yanked (3), explicit
# User-Agent header, and every other outcome, including malformed rows, an
# error (2) that is never read as absent.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
scratch="$(mktemp -d)"
python3 -u - "$scratch" >"$scratch/server.log" 2>&1 <<'PY' &
import http.server
import json
import os
import sys

root = sys.argv[1]
ua_log = os.path.join(root, "ua.log")


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=root, **kwargs)

    def do_GET(self):
        with open(ua_log, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(self.headers.get("User-Agent")) + "\n")
        return super().do_GET()

    def log_message(self, fmt, *args):
        pass


server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
print(f"Serving HTTP on 127.0.0.1 port {server.server_address[1]} ...", flush=True)
server.serve_forever()
PY
server=$!
trap 'kill "$server" 2>/dev/null || true; rm -rf "$scratch"' EXIT
for _ in $(seq 1 50); do
	grep -q 'Serving HTTP' "$scratch/server.log" && break
	sleep 0.1
done
[[ -n "$(sed -n "s/.*port \([0-9]*\).*/\1/p" "$scratch/server.log")" ]] || {
	echo "error: test server did not start" >&2
	exit 1
}
port="$(sed -n 's/.*port \([0-9]*\).*/\1/p' "$scratch/server.log" | head -1)"
base="sparse+http://127.0.0.1:$port/"
mkdir -p "$scratch/ab/cd" "$scratch/3/a" "$scratch/2" "$scratch/1"
expect() {
	local want="$1" name="$2" body="$3" got=0
	printf '%s\n' "$body" >"$scratch/ab/cd/abcdef"
	python3 -I "$root/scripts/release-index-entry.py" "$base" "$name" 1.0.0 >"$scratch/out" 2>/dev/null || got=$?
	[[ "$got" == "$want" ]] || {
		echo "error: $body -> exit $got, expected $want" >&2
		exit 1
	}
}
digest="$(printf '%064d' 7)"
expect 0 abcdef "{\"name\":\"abcdef\",\"vers\":\"1.0.0\",\"cksum\":\"$digest\"}"
[[ "$(cat "$scratch/out")" == "$digest" ]]
grep -q 'sysprims-release-verify/1\.0' "$scratch/ua.log" || {
	echo "error: User-Agent header missing from sparse-index request" >&2
	exit 1
}
expect 0 ABCDEF "{\"name\":\"abcdef\",\"vers\":\"1.0.0\",\"cksum\":\"$digest\"}"
expect 1 abcdef "{\"name\":\"abcdef\",\"vers\":\"0.9.0\",\"cksum\":\"$digest\"}"
expect 3 abcdef "{\"name\":\"abcdef\",\"vers\":\"1.0.0\",\"cksum\":\"$digest\",\"yanked\":true}"
expect 1 abcdefg "{}"
# The last line for a version wins, as in Cargo: a later yank or edit applies.
live="{\"name\":\"abcdef\",\"vers\":\"1.0.0\",\"cksum\":\"$digest\"}"
other="$(printf '%064d' 9)"
expect 3 abcdef "$live
{\"name\":\"abcdef\",\"vers\":\"1.0.0\",\"cksum\":\"$digest\",\"yanked\":true}"
expect 0 abcdef "$live
{\"name\":\"abcdef\",\"vers\":\"1.0.0\",\"cksum\":\"$other\"}"
[[ "$(cat "$scratch/out")" == "$other" ]]
for body in '[]' 'null' '7' '{"name":5,"vers":"1.0.0"}' '{"name":"abcdef","vers":1}' \
	'{"name":"abcdef","vers":"1.0.0","cksum":7}' '{"name":"abcdef","vers":"1.0.0"}' 'not json'; do
	expect 2 abcdef "$body"
done
# Short-name index paths.
printf '{"name":"abc","vers":"1.0.0","cksum":"%s"}\n' "$digest" >"$scratch/3/a/abc"
printf '{"name":"ab","vers":"1.0.0","cksum":"%s"}\n' "$digest" >"$scratch/2/ab"
printf '{"name":"a","vers":"1.0.0","cksum":"%s"}\n' "$digest" >"$scratch/1/a"
for name in a ab abc; do
	python3 -I "$root/scripts/release-index-entry.py" "$base" "$name" 1.0.0 >/dev/null
done
# No server: an error, not absent.
rc=0
python3 -I "$root/scripts/release-index-entry.py" "http://127.0.0.1:9/" abcdef 1.0.0 >/dev/null 2>&1 || rc=$?
[[ "$rc" == 2 ]]
echo '[ok] sparse-index reader: listed, absent, yanked, User-Agent and malformed rows'
