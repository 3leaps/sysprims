#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Synthetic controls for scripts/release-verify-registries.py and
# scripts/pack-npm-redirector.sh against a loopback HTTP server across all 8
# @3leaps/sysprims* npm packages.
set -euo pipefail
unset GITHUB_ACTIONS CI SYSPRIMS_NPM_REGISTRY_URL SYSPRIMS_SIGSTORE_TEST_ROOT

root="$(cd "$(dirname "$0")/.." && pwd -P)"
scratch="$(mktemp -d)"
mkdir -p "$scratch/http"
python3 -u -m http.server -d "$scratch/http" -b 127.0.0.1 0 >"$scratch/server.log" 2>&1 &
server=$!
trap 'rc=$?; [[ "$rc" -ne 0 && -f "$scratch/out" ]] && cat "$scratch/out" >&2; kill "$server" 2>/dev/null || true; rm -rf "$scratch"; exit "$rc"' EXIT
for _ in $(seq 1 50); do
	grep -q 'Serving HTTP' "$scratch/server.log" && break
	sleep 0.1
done
port="$(sed -n 's/.*port \([0-9]*\).*/\1/p' "$scratch/server.log" | head -1)"
[[ -n "$port" ]] || {
	echo "error: test server did not start" >&2
	exit 1
}

version="0.3.0"
commit="0123456789abcdef0123456789abcdef01234567"
npm_base="http://127.0.0.1:$port/npm"
test_root_file="$scratch/test-trust-root.json"

python3 -I -B - "$scratch/http" "$version" "$commit" "$npm_base" "$root/scripts/release-verify-registries.py" "$test_root_file" <<'PY'
import base64
import hashlib
import importlib.util
import json
import pathlib
import sys

http = pathlib.Path(sys.argv[1])
version = sys.argv[2]
commit = sys.argv[3]
npm_base = sys.argv[4]
mod_path = sys.argv[5]
test_root_path = pathlib.Path(sys.argv[6])

spec = importlib.util.spec_from_file_location("rvr", mod_path)
rvr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rvr)


def der_tlv(tag: int, body: bytes) -> bytes:
    n = len(body)
    if n < 128:
        hdr = bytes([tag, n])
    else:
        len_bytes = n.to_bytes((n.bit_length() + 7) // 8, "big")
        hdr = bytes([tag, 0x80 | len(len_bytes)]) + len_bytes
    return hdr + body


def der_int(val: int) -> bytes:
    raw = val.to_bytes(max(1, (val.bit_length() + 7) // 8), "big")
    if raw[0] & 0x80:
        raw = b"\x00" + raw
    return der_tlv(0x02, raw)


def ecdsa_sign(priv_d: int, digest: bytes, n: int, g: tuple[int, int], p: int, a: int, k: int = 11) -> bytes:
    e = int.from_bytes(digest, "big")
    n_bits = n.bit_length()
    d_bits = len(digest) * 8
    if d_bits > n_bits:
        e >>= d_bits - n_bits
    e %= n
    r_pt = rvr.ec_mul(k, g, p, a)
    assert r_pt is not None
    r = r_pt[0] % n
    s = (pow(k, -1, n) * (e + r * priv_d)) % n
    return der_tlv(0x30, der_int(r) + der_int(s))


ca_priv_d = 13
ca_pub = rvr.p384_mul(ca_priv_d, rvr.P384_G)
assert ca_pub is not None

rekor_priv_d = 19
rekor_pub = rvr.p256_mul(rekor_priv_d, rvr.P256_G)
assert rekor_pub is not None
rekor_spki = (
    rvr.SPKI_P256_PREFIX
    + rekor_pub[0].to_bytes(32, "big")
    + rekor_pub[1].to_bytes(32, "big")
)
rekor_log_id_bytes = hashlib.sha256(rekor_spki).digest()
rekor_log_id_hex = rekor_log_id_bytes.hex()
rekor_log_id_b64 = base64.b64encode(rekor_log_id_bytes).decode("ascii")

leaf_priv_d = 7
leaf_pub = rvr.p256_mul(leaf_priv_d, rvr.P256_G)
assert leaf_pub is not None
leaf_spki = (
    rvr.SPKI_P256_PREFIX
    + leaf_pub[0].to_bytes(32, "big")
    + leaf_pub[1].to_bytes(32, "big")
)

issuer_der = rvr.DEFAULT_TRUST_ROOT.fulcio_issuer_der
test_root_path.write_text(
    json.dumps(
        {
            "fulcio_issuer_der_hex": issuer_der.hex(),
            "fulcio_p384_qx": hex(ca_pub[0]),
            "fulcio_p384_qy": hex(ca_pub[1]),
            "rekor_log_id_hex": rekor_log_id_hex,
            "rekor_log_id_b64": rekor_log_id_b64,
            "rekor_p256_qx": hex(rekor_pub[0]),
            "rekor_p256_qy": hex(rekor_pub[1]),
        }
    ),
    encoding="utf-8",
)


def make_ext(oid: bytes, val_bytes: bytes, critical: bool = False) -> bytes:
    body = der_tlv(0x06, oid)
    if critical:
        body += der_tlv(0x01, b"\xff")
    body += der_tlv(0x04, val_bytes)
    return der_tlv(0x30, body)


def make_test_cert_b64(
    ver: str,
    repo: str = "3leaps/sysprims",
    workflow_path: str = ".github/workflows/typescript-npm-publish.yml",
    signer_d: int = ca_priv_d,
    subject_cn: bytes = b"sigstore-leaf",
    include_workflow_oids: bool = True,
) -> tuple[str, bytes]:
    san_uri = f"https://github.com/{repo}/{workflow_path}@refs/tags/v{ver}".encode("ascii")
    san_ext_val = der_tlv(0x30, der_tlv(0x86, san_uri))
    bc_ext_val = der_tlv(0x30, b"")
    exts_list = [
        make_ext(rvr.OID_EXT_BASIC_CONSTRAINTS, bc_ext_val, critical=True),
        make_ext(rvr.OID_EXT_SAN, san_ext_val, critical=True),
        make_ext(rvr.OID_FULCIO_ISSUER_V1, rvr.EXPECTED_OIDC_ISSUER),
        make_ext(rvr.OID_FULCIO_ISSUER_V2, der_tlv(0x0C, rvr.EXPECTED_OIDC_ISSUER)),
    ]
    if include_workflow_oids:
        exts_list.extend(
            [
                make_ext(rvr.OID_FULCIO_WORKFLOW_REPO, repo.encode("ascii")),
                make_ext(rvr.OID_FULCIO_WORKFLOW_REF, f"refs/tags/v{ver}".encode("ascii")),
                make_ext(rvr.OID_FULCIO_BUILD_SIGNER_URI, der_tlv(0x0C, san_uri)),
                make_ext(rvr.OID_FULCIO_SOURCE_REPO_URI, der_tlv(0x0C, f"https://github.com/{repo}".encode("ascii"))),
                make_ext(rvr.OID_FULCIO_SOURCE_REPO_REF, der_tlv(0x0C, f"refs/tags/v{ver}".encode("ascii"))),
            ]
        )
    exts = der_tlv(0xA3, der_tlv(0x30, b"".join(exts_list)))
    alg = der_tlv(0x30, rvr.ALG_ECDSA_SHA384)
    validity = der_tlv(0x30, der_tlv(0x17, b"231114221000Z") + der_tlv(0x17, b"231114222000Z"))
    subj = der_tlv(
        0x30,
        der_tlv(0x31, der_tlv(0x30, der_tlv(0x06, bytes.fromhex("550403")) + der_tlv(0x0C, subject_cn))),
    )
    tbs = der_tlv(
        0x30,
        der_tlv(0xA0, b"\x02\x01\x02")
        + der_int(1001)
        + alg
        + issuer_der
        + validity
        + subj
        + leaf_spki
        + exts,
    )
    ca_sig = ecdsa_sign(
        signer_d,
        hashlib.sha384(tbs).digest(),
        rvr.P384_N,
        rvr.P384_G,
        rvr.P384_P,
        rvr.P384_A,
    )
    cert_der = der_tlv(0x30, tbs + alg + der_tlv(0x03, b"\x00" + ca_sig))
    return base64.b64encode(cert_der).decode("ascii"), cert_der


def sign_dsse_pae(payload_bytes: bytes) -> tuple[str, bytes]:
    pae = (
        f"DSSEv1 {len(rvr.DSSE_PAYLOAD_TYPE)} {rvr.DSSE_PAYLOAD_TYPE} {len(payload_bytes)} ".encode("ascii")
        + payload_bytes
    )
    sig_der = ecdsa_sign(
        leaf_priv_d,
        hashlib.sha256(pae).digest(),
        rvr.P256_N,
        rvr.P256_G,
        rvr.P256_P,
        rvr.P256_A,
    )
    return base64.b64encode(sig_der).decode("ascii"), sig_der


def make_tlog_entry(
    cert_der: bytes,
    sig_der: bytes,
    payload_bytes: bytes,
    integrated_time: int = 1700000000,
) -> dict:
    payload_sha256 = hashlib.sha256(payload_bytes).hexdigest()
    cert_b64_der = base64.b64encode(cert_der).decode("ascii")
    pem_block = (
        "-----BEGIN CERTIFICATE-----\n"
        + "\n".join(cert_b64_der[i : i + 64] for i in range(0, len(cert_b64_der), 64))
        + "\n-----END CERTIFICATE-----\n"
    ).encode("ascii")
    cert_b64_pem = base64.b64encode(pem_block).decode("ascii")
    sig_b64 = base64.b64encode(sig_der).decode("ascii")
    body = {
        "apiVersion": "0.0.1",
        "kind": "dsse",
        "spec": {
            "payloadHash": {"algorithm": "sha256", "value": payload_sha256},
            "signatures": [{"signature": sig_b64, "verifier": cert_b64_pem}],
        },
    }
    body_bytes = json.dumps(body, separators=(",", ":"), sort_keys=True).encode("utf-8")
    canon_b64 = base64.b64encode(body_bytes).decode("ascii")
    idx_val = 1

    set_payload = json.dumps(
        {
            "body": canon_b64,
            "integratedTime": integrated_time,
            "logID": rekor_log_id_hex,
            "logIndex": idx_val,
        },
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    set_sig_der = ecdsa_sign(
        rekor_priv_d,
        hashlib.sha256(set_payload).digest(),
        rvr.P256_N,
        rvr.P256_G,
        rvr.P256_P,
        rvr.P256_A,
    )

    sibling_hash = hashlib.sha256(b"\x00sibling-leaf").digest()
    leaf_hash = hashlib.sha256(b"\x00" + body_bytes).digest()
    root_bytes = hashlib.sha256(b"\x01" + sibling_hash + leaf_hash).digest()
    tree_size = 2
    note_body = f"rekor.sigstore.dev - 1\n{tree_size}\n{base64.b64encode(root_bytes).decode('ascii')}"
    cp_sig_der = ecdsa_sign(
        rekor_priv_d,
        hashlib.sha256((note_body + "\n").encode("utf-8")).digest(),
        rvr.P256_N,
        rvr.P256_G,
        rvr.P256_P,
        rvr.P256_A,
    )
    key_hint = rekor_log_id_bytes[:4]
    cp_sig_b64 = base64.b64encode(key_hint + cp_sig_der).decode("ascii")
    envelope = f"{note_body}\n\n\u2014 rekor.sigstore.dev {cp_sig_b64}\n"

    return {
        "logIndex": idx_val,
        "logId": {"keyId": rekor_log_id_b64},
        "integratedTime": integrated_time,
        "inclusionPromise": {"signedEntryTimestamp": base64.b64encode(set_sig_der).decode("ascii")},
        "inclusionProof": {
            "logIndex": idx_val,
            "rootHash": base64.b64encode(root_bytes).decode("ascii"),
            "treeSize": tree_size,
            "hashes": [base64.b64encode(sibling_hash).decode("ascii")],
            "checkpoint": {"envelope": envelope},
        },
        "canonicalizedBody": canon_b64,
    }


def make_slsa_statement(subj_name: str, alg: str, digest_hex: str, ver: str) -> bytes:
    stmt = {
        "_type": "https://in-toto.io/Statement/v1",
        "subject": [{"name": subj_name, "digest": {alg: digest_hex}}],
        "predicateType": "https://slsa.dev/provenance/v1",
        "predicate": {
            "buildDefinition": {
                "externalParameters": {
                    "workflow": {
                        "repository": "https://github.com/3leaps/sysprims",
                        "path": ".github/workflows/typescript-npm-publish.yml",
                        "ref": f"refs/tags/v{ver}",
                    }
                }
            }
        },
    }
    return json.dumps(stmt, separators=(",", ":")).encode("utf-8")


cert_b64, cert_der = make_test_cert_b64(version)

npm_att_dir = http / "npm" / "-" / "npm" / "v1" / "attestations" / "@3leaps"
npm_att_dir.mkdir(parents=True, exist_ok=True)
tarballs_dir = http / "npm" / "tarballs"
tarballs_dir.mkdir(parents=True, exist_ok=True)

for pkg in rvr.NPM_PACKAGES:
    short = pkg.split("/", 1)[1]
    tgz_data = f"npm-tarball-{pkg}-{version}\n".encode("utf-8")
    tgz_file = f"{short}-{version}.tgz"
    (tarballs_dir / tgz_file).write_bytes(tgz_data)
    tgz_sha512 = hashlib.sha512(tgz_data).hexdigest()
    integrity = "sha512-" + base64.b64encode(hashlib.sha512(tgz_data).digest()).decode("ascii")
    shasum = hashlib.sha1(tgz_data).hexdigest()

    pkg_dir = http / "npm" / "@3leaps" / short
    pkg_dir.mkdir(parents=True, exist_ok=True)
    npm_doc = {
        "name": pkg,
        "version": version,
        "gitHead": commit,
        "dist": {
            "integrity": integrity,
            "shasum": shasum,
            "tarball": f"{npm_base}/tarballs/{tgz_file}",
            "attestations": {
                "url": f"{npm_base}/-/npm/v1/attestations/@3leaps%2F{short}@{version}",
                "provenance": {"predicateType": "https://slsa.dev/provenance/v1"},
            },
        },
    }
    (pkg_dir / version).write_text(json.dumps(npm_doc), encoding="utf-8")

    purl = f"pkg:npm/%403leaps/{short}@{version}"
    stmt_bytes = make_slsa_statement(purl, "sha512", tgz_sha512, version)
    sig_b64, sig_der = sign_dsse_pae(stmt_bytes)
    att_doc = {
        "attestations": [
            {
                "predicateType": "https://slsa.dev/provenance/v1",
                "bundle": {
                    "mediaType": "application/vnd.dev.sigstore.bundle+json;version=0.1",
                    "verificationMaterial": {
                        "certificate": {"rawBytes": cert_b64},
                        "tlogEntries": [make_tlog_entry(cert_der, sig_der, stmt_bytes)],
                    },
                    "dsseEnvelope": {
                        "payloadType": "application/vnd.in-toto+json",
                        "payload": base64.b64encode(stmt_bytes).decode("ascii"),
                        "signatures": [{"sig": sig_b64}],
                    },
                },
            }
        ]
    }
    (npm_att_dir / f"{short}@{version}").write_text(json.dumps(att_doc), encoding="utf-8")

# Negative-control attestation fixtures for @3leaps/sysprims
root_tgz_sha512 = hashlib.sha512(f"npm-tarball-@3leaps/sysprims-{version}\n".encode("utf-8")).hexdigest()
good_stmt = make_slsa_statement(f"pkg:npm/%403leaps/sysprims@{version}", "sha512", root_tgz_sha512, version)
good_sig_b64, good_sig_der = sign_dsse_pae(good_stmt)


def write_att(filename: str, c_b64: str, tlog_entry: dict, stmt_b: bytes, s_b64: str) -> None:
    (http.parent / filename).write_text(
        json.dumps(
            {
                "attestations": [
                    {
                        "predicateType": "https://slsa.dev/provenance/v1",
                        "bundle": {
                            "mediaType": "application/vnd.dev.sigstore.bundle+json;version=0.1",
                            "verificationMaterial": {
                                "certificate": {"rawBytes": c_b64},
                                "tlogEntries": [tlog_entry],
                            },
                            "dsseEnvelope": {
                                "payloadType": "application/vnd.in-toto+json",
                                "payload": base64.b64encode(stmt_b).decode("ascii"),
                                "signatures": [{"sig": s_b64}],
                            },
                        },
                    }
                ]
            }
        ),
        encoding="utf-8",
    )


bad_subj_stmt = make_slsa_statement(f"pkg:npm/%403leaps/sysprims@{version}", "sha512", "00" * 64, version)
bad_subj_sig_b64, bad_subj_sig_der = sign_dsse_pae(bad_subj_stmt)
write_att("att.wrong-subj.json", cert_b64, make_tlog_entry(cert_der, bad_subj_sig_der, bad_subj_stmt), bad_subj_stmt, bad_subj_sig_b64)

evil_cert_b64, evil_cert_der = make_test_cert_b64(version, repo="evil/sysprims")
write_att("att.wrong-cert.json", evil_cert_b64, make_tlog_entry(evil_cert_der, good_sig_der, good_stmt), good_stmt, good_sig_b64)

suffix_cert_b64, suffix_cert_der = make_test_cert_b64(f"{version}0", include_workflow_oids=False)
write_att("att.san-suffix.json", suffix_cert_b64, make_tlog_entry(suffix_cert_der, good_sig_der, good_stmt), good_stmt, good_sig_b64)

untrusted_cert_b64, untrusted_cert_der = make_test_cert_b64(version, signer_d=29)
write_att("att.untrusted-ca.json", untrusted_cert_b64, make_tlog_entry(untrusted_cert_der, good_sig_der, good_stmt), good_stmt, good_sig_b64)

write_att("att.expired-cert.json", cert_b64, make_tlog_entry(cert_der, good_sig_der, good_stmt, integrated_time=1700009999), good_stmt, good_sig_b64)
PY

npm_meta_file="$scratch/http/npm/@3leaps/sysprims/$version"
npm_att_file="$scratch/http/npm/-/npm/v1/attestations/@3leaps/sysprims@$version"
cp "$npm_meta_file" "$scratch/npm.pristine.json"
cp "$npm_att_file" "$scratch/npm-att.pristine.json"

run_verify() {
	SYSPRIMS_NPM_REGISTRY_URL="$npm_base" \
		SYSPRIMS_SIGSTORE_TEST_ROOT="$test_root_file" \
		SYSPRIMS_REGISTRY_POLL_ATTEMPTS=2 \
		SYSPRIMS_REGISTRY_POLL_SLEEP=0.05 \
		python3 -I -B "$root/scripts/release-verify-registries.py" --version "$version" --commit "$commit" "$@" >"$scratch/out" 2>&1
}

refused() {
	local pattern="$1"
	shift
	if run_verify "$@"; then
		echo "error: accepted, expected failure matching '$pattern'" >&2
		cat "$scratch/out" >&2
		exit 1
	fi
	grep -Eq -- "$pattern" "$scratch/out" || {
		echo "error: failed for unexpected reason (want '$pattern')" >&2
		cat "$scratch/out" >&2
		exit 1
	}
}

run_verify

# Without SYSPRIMS_SIGSTORE_TEST_ROOT, synthetic test CA certs must be rejected by the pinned production Fulcio CA root!
if SYSPRIMS_NPM_REGISTRY_URL="$npm_base" \
	SYSPRIMS_REGISTRY_POLL_ATTEMPTS=1 \
	python3 -I -B "$root/scripts/release-verify-registries.py" --version "$version" --commit "$commit" >"$scratch/out" 2>&1; then
	echo "error: accepted synthetic CA certificate against production Sigstore trust root" >&2
	exit 1
fi
grep -q "Fulcio CA certificate signature: cryptographic ECDSA signature verification failed" "$scratch/out"

# Wrong gitHead, dist.integrity, dist.shasum, tarball bytes, and missing/invalid Sigstore provenance.
jq '.gitHead = "ffffffffffffffffffffffffffffffffffffffff"' "$scratch/npm.pristine.json" >"$npm_meta_file"
refused "npm gitHead mismatch"
jq '.dist.integrity = "sha512-AAAA"' "$scratch/npm.pristine.json" >"$npm_meta_file"
refused "npm dist.integrity must decode to 64 bytes"
jq '.dist.shasum = "0000000000000000000000000000000000000000"' "$scratch/npm.pristine.json" >"$npm_meta_file"
refused "npm tarball shasum mismatch"
jq 'del(.dist.attestations)' "$scratch/npm.pristine.json" >"$npm_meta_file"
refused "npm Sigstore provenance missing dist.attestations"
SYSPRIMS_NPM_REGISTRY_URL="$npm_base" \
	python3 -I -B "$root/scripts/release-verify-registries.py" --bootstrap-no-provenance --version "$version" --commit "$commit" >/dev/null
cp "$scratch/npm.pristine.json" "$npm_meta_file"

# Missing, tampered, or mismatched npm Sigstore attestation bundle.
rm -f "$npm_att_file"
refused "npm Sigstore provenance for .* unavailable"
jq '.attestations = []' "$scratch/npm-att.pristine.json" >"$npm_att_file"
refused "attestation endpoint returned no attestations"
jq '.attestations[0].bundle.dsseEnvelope.signatures[0].sig = "MEUCIQD/////////////////////////////////////////AAIgf/////////////////////////////////////////8="' \
	"$scratch/npm-att.pristine.json" >"$npm_att_file"
refused "(ECDSA|signature verification failed)"
jq '.attestations[0].bundle.verificationMaterial.tlogEntries = []' \
	"$scratch/npm-att.pristine.json" >"$npm_att_file"
refused "missing Rekor transparency log entries"
jq 'del(.attestations[0].bundle.verificationMaterial.tlogEntries[0].inclusionPromise) | .attestations[0].bundle.verificationMaterial.tlogEntries[0].integratedTime = 1700000000' \
	"$scratch/npm-att.pristine.json" >"$npm_att_file"
refused "missing or invalid inclusionPromise"
jq '.attestations[0].bundle.verificationMaterial.tlogEntries[0].inclusionProof.rootHash = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="' \
	"$scratch/npm-att.pristine.json" >"$npm_att_file"
refused "inclusionProof Merkle root mismatch"
jq '.attestations[0].bundle.verificationMaterial.tlogEntries[0].logId.keyId = "untrusted-log"' \
	"$scratch/npm-att.pristine.json" >"$npm_att_file"
refused "untrusted or missing Rekor logId.keyId"
cp "$scratch/att.wrong-subj.json" "$npm_att_file"
refused "statement subject mismatch"
cp "$scratch/att.wrong-cert.json" "$npm_att_file"
refused "certificate missing expected workflow SAN URI"
cp "$scratch/att.san-suffix.json" "$npm_att_file"
refused "certificate missing expected workflow SAN URI"
cp "$scratch/att.untrusted-ca.json" "$npm_att_file"
refused "Fulcio CA certificate signature: cryptographic ECDSA signature verification failed"
cp "$scratch/att.expired-cert.json" "$npm_att_file"
refused "outside certificate validity"
cp "$scratch/npm-att.pristine.json" "$npm_att_file"

# Conflicting flags and CI environment protections.
if python3 -I -B "$root/scripts/release-verify-registries.py" --expect-provenance --bootstrap-no-provenance --version "$version" >"$scratch/out" 2>&1; then
	echo "error: accepted conflicting provenance flags" >&2
	exit 1
fi
grep -q "cannot combine --expect-provenance and --bootstrap-no-provenance" "$scratch/out"
if GITHUB_ACTIONS=true python3 -I -B "$root/scripts/release-verify-registries.py" --bootstrap-no-provenance --version "$version" >"$scratch/out" 2>&1; then
	echo "error: accepted --bootstrap-no-provenance under GITHUB_ACTIONS" >&2
	exit 1
fi
grep -q -- "--bootstrap-no-provenance is not accepted under GitHub Actions or CI" "$scratch/out"
if GITHUB_ACTIONS=true SYSPRIMS_NPM_REGISTRY_URL="$npm_base" \
	python3 -I -B "$root/scripts/release-verify-registries.py" --version "$version" >"$scratch/out" 2>&1; then
	echo "error: accepted SYSPRIMS_NPM_REGISTRY_URL under GITHUB_ACTIONS" >&2
	exit 1
fi
grep -q "SYSPRIMS_NPM_REGISTRY_URL is not accepted under GitHub Actions" "$scratch/out"
if CI=true SYSPRIMS_SIGSTORE_TEST_ROOT="$test_root_file" \
	python3 -I -B "$root/scripts/release-verify-registries.py" --version "$version" >"$scratch/out" 2>&1; then
	echo "error: accepted SYSPRIMS_SIGSTORE_TEST_ROOT under CI" >&2
	exit 1
fi
grep -q "SYSPRIMS_SIGSTORE_TEST_ROOT is not accepted under GitHub Actions or CI" "$scratch/out"
if SYSPRIMS_NPM_REGISTRY_URL="http://127.0.0.1:$port@attacker.example/npm" \
	SYSPRIMS_SIGSTORE_TEST_ROOT="$test_root_file" \
	python3 -I -B "$root/scripts/release-verify-registries.py" --version "$version" >"$scratch/out" 2>&1; then
	echo "error: accepted userinfo loopback bypass in npm registry URL" >&2
	exit 1
fi
grep -q "must be http://127.0.0.1:<port>" "$scratch/out"

if command -v npm >/dev/null 2>&1; then
	"$root/scripts/pack-npm-redirector.sh" "$scratch/redirector" "$version" >/dev/null
	[[ -s "$scratch/redirector/sysprims-$version.tgz" ]]
fi

echo '[ok] registry verification and bare-name redirector controls passed'
