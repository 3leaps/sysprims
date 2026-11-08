#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Byte-for-byte agreement between crates/sysprims-evidence and
# github.com/3leaps/sysprims/evidence, live Rust/Go OS collector parity on
# macOS and Linux, and standalone pure-Go out-of-tree module checks.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
work="$(mktemp -d "${TMPDIR:-/tmp}/sysprims-evidence-xcheck.XXXXXX")"
child_pid=""
cleanup() {
	if [ -n "$child_pid" ]; then
		kill "$child_pid" 2>/dev/null || true
		wait "$child_pid" 2>/dev/null || true
	fi
	rm -rf "$work"
}
trap cleanup EXIT

cargo run --quiet -p sysprims-evidence --example evidence-diff >"${work}/rust.ndjson"
(cd evidence && GOWORK=off CGO_ENABLED=0 go run ./internal/difftool) >"${work}/go.ndjson"

if ! cmp -s "${work}/rust.ndjson" "${work}/go.ndjson"; then
	echo "error: evidence NDJSON differs between Rust and Go" >&2
	diff -u "${work}/rust.ndjson" "${work}/go.ndjson" | head -n 40 >&2
	exit 1
fi
echo "[ok] evidence NDJSON identical ($(wc -l <"${work}/rust.ndjson" | tr -d ' ') lines)"

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

	cargo build --quiet -p sysprims-proc --example observe-process
	GOWORK=off CGO_ENABLED=0 go build -o "${work}/go-observecli" ./procobserve/internal/observecli

	"$sleep_bin" 30 &
	child_pid=$!

	fixed_ts="2026-11-06T12:00:00Z"
	"${ROOT}/target/debug/examples/observe-process" \
		--pid "$child_pid" \
		--observed-at "$fixed_ts" >"${work}/live-default-rust.json"
	"${work}/go-observecli" \
		--pid "$child_pid" \
		--observed-at "$fixed_ts" >"${work}/live-default-go.json"
	if ! cmp -s "${work}/live-default-rust.json" "${work}/live-default-go.json"; then
		echo "error: live default observation JSON differs between Rust and Go" >&2
		diff -u "${work}/live-default-rust.json" "${work}/live-default-go.json" >&2
		exit 1
	fi

	"${ROOT}/target/debug/examples/observe-process" \
		--pid "$child_pid" \
		--observed-at "$fixed_ts" \
		--all-context >"${work}/live-full-rust.json"
	"${work}/go-observecli" \
		--pid "$child_pid" \
		--observed-at "$fixed_ts" \
		--all-context >"${work}/live-full-go.json"
	if ! cmp -s "${work}/live-full-rust.json" "${work}/live-full-go.json"; then
		echo "error: live full-context observation JSON differs between Rust and Go" >&2
		diff -u "${work}/live-full-rust.json" "${work}/live-full-go.json" >&2
		exit 1
	fi

	kill "$child_pid" 2>/dev/null || true
	wait "$child_pid" 2>/dev/null || true
	child_pid=""
	echo "[ok] live OS child observation canonical JSON identical between Rust and pure Go"
	;;
esac

# Out-of-tree standalone root Go module check (no go.work, no Rust workspace relative paths).
mkdir -p "${work}/oot"
cp go.mod go.sum "${work}/oot/"
cp -R evidence "${work}/oot/evidence"
cp -R procobserve "${work}/oot/procobserve"
(
	cd "${work}/oot"
	GOWORK=off \
		CGO_ENABLED=0 \
		EVIDENCE_CORPUS_DIR="${ROOT}/crates/sysprims-evidence/tests/identity-evidence-v0" \
		EVIDENCE_PARITY_CASES="${ROOT}/crates/sysprims-evidence/tests/parity-cases.json" \
		go test -count=1 ./evidence/... ./procobserve/... >/dev/null
	GOWORK=off CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build ./evidence/... ./procobserve/...
	GOWORK=off CGO_ENABLED=0 GOOS=darwin GOARCH=arm64 go build ./evidence/... ./procobserve/...
	GOWORK=off CGO_ENABLED=0 GOOS=windows GOARCH=amd64 go build ./evidence/... ./procobserve/...
)
echo "[ok] standalone pure-Go evidence + procobserve module tests and cross-OS builds passed"
