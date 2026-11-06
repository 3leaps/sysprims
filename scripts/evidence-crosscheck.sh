#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Byte-for-byte agreement between crates/sysprims-evidence and
# github.com/3leaps/sysprims/evidence, plus standalone pure-Go module checks.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
work="$(mktemp -d "${TMPDIR:-/tmp}/sysprims-evidence-xcheck.XXXXXX")"
trap 'rm -rf "$work"' EXIT

cargo run --quiet -p sysprims-evidence --example evidence-diff >"${work}/rust.ndjson"
(cd evidence && GOWORK=off CGO_ENABLED=0 go run ./internal/difftool) >"${work}/go.ndjson"

if ! cmp -s "${work}/rust.ndjson" "${work}/go.ndjson"; then
	echo "error: evidence NDJSON differs between Rust and Go" >&2
	diff -u "${work}/rust.ndjson" "${work}/go.ndjson" | head -n 40 >&2
	exit 1
fi
echo "[ok] evidence NDJSON identical ($(wc -l <"${work}/rust.ndjson" | tr -d ' ') lines)"

# Out-of-tree standalone root Go module check (no go.work, no Rust workspace relative paths).
mkdir -p "${work}/oot"
cp go.mod go.sum "${work}/oot/"
cp -R evidence "${work}/oot/evidence"
(
	cd "${work}/oot"
	GOWORK=off \
		CGO_ENABLED=0 \
		EVIDENCE_CORPUS_DIR="${ROOT}/crates/sysprims-evidence/tests/identity-evidence-v0" \
		EVIDENCE_PARITY_CASES="${ROOT}/crates/sysprims-evidence/tests/parity-cases.json" \
		go test -count=1 ./evidence/... >/dev/null
	GOWORK=off CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build ./evidence/...
	GOWORK=off CGO_ENABLED=0 GOOS=darwin GOARCH=arm64 go build ./evidence/...
	GOWORK=off CGO_ENABLED=0 GOOS=windows GOARCH=amd64 go build ./evidence/...
)
echo "[ok] standalone pure-Go evidence module tests and cross-OS builds passed"
