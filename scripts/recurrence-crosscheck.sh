#!/usr/bin/env bash
# Byte-for-byte agreement between the Rust recurrence crate and the Go twin.
#
# 1. Both tzdb bundle copies equal each other and their sha256 pin.
# 2. Both corpus harnesses emit one NDJSON line per vector (canonical bytes,
#    digest, text, every query result; reject reasons); the streams must be
#    identical.
# 3. Both differential drivers run the same generated rules; the outputs
#    must be identical.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
work="$(mktemp -d "${TMPDIR:-/tmp}/sysprims-recurrence-xcheck.XXXXXX")"
trap 'rm -rf "$work"' EXIT

rust_bundle="crates/sysprims-recurrence/tzdata/tzif-bundle-2026e"
go_bundle="go/recurrence/tzdata/tzif-bundle-2026e"
cmp "${rust_bundle}.bin" "${go_bundle}.bin"
cmp "${rust_bundle}.sha256" "${go_bundle}.sha256"
(cd "$(dirname "$rust_bundle")" && {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum -c --quiet tzif-bundle-2026e.sha256
	else shasum -a 256 -c --quiet tzif-bundle-2026e.sha256; fi
})
echo "[ok] tz bundle copies match the pin"

RECURRENCE_NDJSON_OUT="${work}/rust.ndjson" cargo test -q -p sysprims-recurrence --test corpus accept_vectors >/dev/null
(cd go/recurrence && RECURRENCE_NDJSON_OUT="${work}/go.ndjson" GOWORK=off CGO_ENABLED=0 go test -count=1 -run '^TestCorpus$' . >/dev/null)
if ! cmp -s "${work}/rust.ndjson" "${work}/go.ndjson"; then
	echo "error: corpus NDJSON differs between Rust and Go" >&2
	diff "${work}/rust.ndjson" "${work}/go.ndjson" | head -20 >&2
	exit 1
fi
echo "[ok] corpus NDJSON identical ($(wc -l <"${work}/rust.ndjson" | tr -d ' ') lines)"

python3 scripts/recurrence-gen-rules.py "${RECURRENCE_XCHECK_SEED:-20261022}" "${RECURRENCE_XCHECK_RULES:-5000}" >"${work}/rules.jsonl"
cargo run -q --release -p sysprims-recurrence --example recurrence-diff <"${work}/rules.jsonl" >"${work}/rust.out"
(cd go/recurrence && GOWORK=off CGO_ENABLED=0 go run ./internal/difftool) <"${work}/rules.jsonl" >"${work}/go.out"
if ! cmp -s "${work}/rust.out" "${work}/go.out"; then
	echo "error: generated-rule outputs differ between Rust and Go" >&2
	diff <(paste -d'\t' "${work}/rules.jsonl" "${work}/rust.out") <(paste -d'\t' "${work}/rules.jsonl" "${work}/go.out") | head -10 >&2
	exit 1
fi
echo "[ok] generated rules identical ($(wc -l <"${work}/rules.jsonl" | tr -d ' ') rules, $(grep -vc '^refused' "${work}/rust.out" || true) expanded)"
