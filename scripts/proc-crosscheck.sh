#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Byte-for-byte agreement between crates/sysprims-proc and
# github.com/3leaps/sysprims/proc over testdata/conformance/proc/vectors.json,
# TypeScript NDJSON parity, and standalone pure-Go out-of-tree module checks.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
work="$(mktemp -d "${TMPDIR:-/tmp}/sysprims-proc-xcheck.XXXXXX")"
cleanup() {
	rm -rf "$work"
}
trap cleanup EXIT

cargo run --quiet -p sysprims-proc --example proc-diff >"${work}/rust.ndjson"
(cd proc && GOWORK=off CGO_ENABLED=0 go run ./internal/difftool) >"${work}/go.ndjson"

if ! cmp -s "${work}/rust.ndjson" "${work}/go.ndjson"; then
	echo "error: proc NDJSON differs between Rust and Go" >&2
	diff -u "${work}/rust.ndjson" "${work}/go.ndjson" | head -n 40 >&2
	exit 1
fi
echo "[ok] proc NDJSON identical between Rust and pure Go ($(wc -l <"${work}/rust.ndjson" | tr -d ' ') lines)"

if [ -f "bindings/typescript/sysprims/dist-test/test/proc.test.js" ] && command -v node >/dev/null 2>&1; then
	PROC_CONFORMANCE_VECTORS="${ROOT}/testdata/conformance/proc/vectors.json" \
		PROC_NDJSON_OUT="${work}/ts.ndjson" \
		node --test "bindings/typescript/sysprims/dist-test/test/proc.test.js" >/dev/null
	if ! cmp -s "${work}/rust.ndjson" "${work}/ts.ndjson"; then
		echo "error: proc NDJSON differs between Rust and TypeScript" >&2
		diff -u "${work}/rust.ndjson" "${work}/ts.ndjson" | head -n 40 >&2
		exit 1
	fi
	echo "[ok] proc NDJSON identical between Rust and TypeScript ($(wc -l <"${work}/ts.ndjson" | tr -d ' ') lines)"
fi

# Out-of-tree standalone root Go module check (no go.work, no Rust workspace relative paths).
mkdir -p "${work}/oot"
cp go.mod go.sum "${work}/oot/"
cp -R evidence "${work}/oot/evidence"
cp -R procobserve "${work}/oot/procobserve"
cp -R syserror "${work}/oot/syserror"
cp -R signal "${work}/oot/signal"
cp -R session "${work}/oot/session"
cp -R timeout "${work}/oot/timeout"
cp -R proc "${work}/oot/proc"
(
	cd "${work}/oot"
	GOWORK=off \
		CGO_ENABLED=0 \
		PROC_CONFORMANCE_VECTORS="${ROOT}/testdata/conformance/proc/vectors.json" \
		go test -count=1 ./syserror/... ./signal/... ./proc/... >/dev/null
	GOWORK=off CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build ./syserror/... ./signal/... ./session/... ./timeout/... ./proc/...
	GOWORK=off CGO_ENABLED=0 GOOS=darwin GOARCH=arm64 go build ./syserror/... ./signal/... ./session/... ./timeout/... ./proc/...
	GOWORK=off CGO_ENABLED=0 GOOS=windows GOARCH=amd64 go build ./syserror/... ./signal/... ./session/... ./timeout/... ./proc/...
)
echo "[ok] standalone pure-Go syserror + signal + session + timeout + proc module tests and cross-OS builds passed"
