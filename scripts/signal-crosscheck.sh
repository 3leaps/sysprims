#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Byte-for-byte agreement between crates/sysprims-signal and
# github.com/3leaps/sysprims/signal over testdata/conformance/signal/vectors.json,
# live Rust/Go ProcessHandle start_identity parity on macOS and Linux, and
# standalone pure-Go out-of-tree module checks.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
work="$(mktemp -d "${TMPDIR:-/tmp}/sysprims-signal-xcheck.XXXXXX")"
child_pid=""
cleanup() {
	if [ -n "$child_pid" ]; then
		kill "$child_pid" 2>/dev/null || true
		wait "$child_pid" 2>/dev/null || true
	fi
	rm -rf "$work"
}
trap cleanup EXIT

cargo run --quiet -p sysprims-signal --example signal-diff >"${work}/rust.ndjson"
(cd signal && GOWORK=off CGO_ENABLED=0 go run ./internal/difftool) >"${work}/go.ndjson"

if ! cmp -s "${work}/rust.ndjson" "${work}/go.ndjson"; then
	echo "error: signal NDJSON differs between Rust and Go" >&2
	diff -u "${work}/rust.ndjson" "${work}/go.ndjson" | head -n 40 >&2
	exit 1
fi
echo "[ok] signal NDJSON identical between Rust and pure Go ($(wc -l <"${work}/rust.ndjson" | tr -d ' ') lines)"

if [ -f "bindings/typescript/sysprims/dist-test/test/signal.test.js" ] && command -v node >/dev/null 2>&1; then
	SIGNAL_CONFORMANCE_VECTORS="${ROOT}/testdata/conformance/signal/vectors.json" \
		SIGNAL_NDJSON_OUT="${work}/ts.ndjson" \
		node --test "bindings/typescript/sysprims/dist-test/test/signal.test.js" >/dev/null
	if ! cmp -s "${work}/rust.ndjson" "${work}/ts.ndjson"; then
		echo "error: signal NDJSON differs between Rust and TypeScript" >&2
		diff -u "${work}/rust.ndjson" "${work}/ts.ndjson" | head -n 40 >&2
		exit 1
	fi
	echo "[ok] signal NDJSON identical between Rust and TypeScript ($(wc -l <"${work}/ts.ndjson" | tr -d ' ') lines)"
fi

case "$(uname -s)" in
Darwin | Linux)
	sleep_bin=""
	for candidate in /bin/sleep /usr/bin/sleep; do
		if [ -x "$candidate" ]; then
			sleep_bin="$candidate"
			break
		fi
	done
	if [ -z "$sleep_bin" ]; then
		echo "error: neither /bin/sleep nor /usr/bin/sleep found" >&2
		exit 1
	fi

	cargo build --quiet -p sysprims-signal --example signal-diff
	GOWORK=off CGO_ENABLED=0 go build -o "${work}/go-signal-diff" ./signal/internal/difftool

	"$sleep_bin" 30 &
	child_pid=$!

	"${ROOT}/target/debug/examples/signal-diff" --live-pid "$child_pid" >"${work}/live-rust.json"
	"${work}/go-signal-diff" --live-pid "$child_pid" >"${work}/live-go.json"
	if ! cmp -s "${work}/live-rust.json" "${work}/live-go.json"; then
		echo "error: live ProcessHandle JSON differs between Rust and Go" >&2
		diff -u "${work}/live-rust.json" "${work}/live-go.json" >&2
		exit 1
	fi

	kill "$child_pid" 2>/dev/null || true
	wait "$child_pid" 2>/dev/null || true
	child_pid=""
	echo "[ok] live OS child ProcessHandle start_identity and binding identical between Rust and pure Go"
	;;
esac

# Out-of-tree standalone root Go module check (no go.work, no Rust workspace relative paths).
mkdir -p "${work}/oot"
cp go.mod go.sum "${work}/oot/"
cp -R syserror "${work}/oot/syserror"
cp -R signal "${work}/oot/signal"
(
	cd "${work}/oot"
	GOWORK=off \
		CGO_ENABLED=0 \
		SIGNAL_CONFORMANCE_VECTORS="${ROOT}/testdata/conformance/signal/vectors.json" \
		go test -count=1 ./syserror/... ./signal/... >/dev/null
	GOWORK=off CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build ./syserror/... ./signal/...
	GOWORK=off CGO_ENABLED=0 GOOS=darwin GOARCH=arm64 go build ./syserror/... ./signal/...
	GOWORK=off CGO_ENABLED=0 GOOS=windows GOARCH=amd64 go build ./syserror/... ./signal/...
)
echo "[ok] standalone pure-Go syserror + signal module tests and cross-OS builds passed"
