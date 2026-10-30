#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Verify that npm serves the exact signed release packages and Sigstore provenance.

Queries the npm registry JSON API (`https://registry.npmjs.org/<pkg>/<version>`)
and the npm Sigstore attestation endpoint for all 8 `@3leaps/sysprims*` packages
(`@3leaps/sysprims` plus the 7 N-API platform packages), verifying:
  - exact version and gitHead (when --commit is given)
  - dist.integrity (sha512) and dist.shasum (sha1), plus downloaded tarball bytes
    when dist.tarball is present
  - cryptographic Sigstore provenance bundle (Fulcio P-384 certificate chain,
    OIDC workflow extensions for 3leaps/sysprims, Rekor P-256 SET / RFC 6962
    Merkle inclusion proof and checkpoint, DSSE P-256 signature, and SLSA v1
    in-toto statement binding the package sha512)

Usage:
  release-verify-registries.py [--expect-provenance | --bootstrap-no-provenance] --version <version> [--commit <sha>]
  release-verify-registries.py [--expect-provenance | --bootstrap-no-provenance] <version> [npm|all]
"""

import base64
import calendar
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path

NPM_PACKAGES = (
    "@3leaps/sysprims-darwin-arm64",
    "@3leaps/sysprims-linux-arm64-gnu",
    "@3leaps/sysprims-linux-arm64-musl",
    "@3leaps/sysprims-linux-x64-gnu",
    "@3leaps/sysprims-linux-x64-musl",
    "@3leaps/sysprims-win32-arm64-msvc",
    "@3leaps/sysprims-win32-x64-msvc",
    "@3leaps/sysprims",
)

EXPECTED_REPO = "3leaps/sysprims"
EXPECTED_WORKFLOW_PATHS = (
    ".github/workflows/typescript-npm-publish.yml",
    ".github/workflows/registry-publish.yml",
)
EXPECTED_OIDC_ISSUER = b"https://token.actions.githubusercontent.com"
DSSE_PAYLOAD_TYPE = "application/vnd.in-toto+json"
SLSA_PREDICATE_V1 = "https://slsa.dev/provenance/v1"

# X.509 and Fulcio OIDs (DER value bytes of OBJECT IDENTIFIER TLV, tag 0x06):
# - 2.5.29.17 subjectAltName: 55 1d 11
# - 2.5.29.19 basicConstraints: 55 1d 13
# - 1.3.6.1.4.1.57264.1.1  Fulcio OIDC Issuer (v1): 2b 06 01 04 01 83 bf 30 01 01
# - 1.3.6.1.4.1.57264.1.5  Fulcio GitHub Workflow Repository (v1): 2b 06 01 04 01 83 bf 30 01 05
# - 1.3.6.1.4.1.57264.1.6  Fulcio GitHub Workflow Ref (v1): 2b 06 01 04 01 83 bf 30 01 06
# - 1.3.6.1.4.1.57264.1.8  Fulcio Issuer (v2): 2b 06 01 04 01 83 bf 30 01 08
# - 1.3.6.1.4.1.57264.1.9  Fulcio Build Signer URI: 2b 06 01 04 01 83 bf 30 01 09
# - 1.3.6.1.4.1.57264.1.12 Fulcio Source Repository URI: 2b 06 01 04 01 83 bf 30 01 0c
# - 1.3.6.1.4.1.57264.1.14 Fulcio Source Repository Ref: 2b 06 01 04 01 83 bf 30 01 0e
OID_EXT_SAN = bytes.fromhex("551d11")
OID_EXT_BASIC_CONSTRAINTS = bytes.fromhex("551d13")
OID_FULCIO_ISSUER_V1 = bytes.fromhex("2b0601040183bf300101")
OID_FULCIO_WORKFLOW_REPO = bytes.fromhex("2b0601040183bf300105")
OID_FULCIO_WORKFLOW_REF = bytes.fromhex("2b0601040183bf300106")
OID_FULCIO_ISSUER_V2 = bytes.fromhex("2b0601040183bf300108")
OID_FULCIO_BUILD_SIGNER_URI = bytes.fromhex("2b0601040183bf300109")
OID_FULCIO_SOURCE_REPO_URI = bytes.fromhex("2b0601040183bf30010c")
OID_FULCIO_SOURCE_REPO_REF = bytes.fromhex("2b0601040183bf30010e")

# AlgorithmIdentifier OIDs:
# - ecdsa-with-SHA256 (1.2.840.10045.4.3.2): 06 08 2a 86 48 ce 3d 04 03 02
# - ecdsa-with-SHA384 (1.2.840.10045.4.3.3): 06 08 2a 86 48 ce 3d 04 03 03
# - id-ecPublicKey (1.2.840.10045.2.1) + prime256v1 (1.2.840.10045.3.1.7)
ALG_ECDSA_SHA256 = bytes.fromhex("06082a8648ce3d040302")
ALG_ECDSA_SHA384 = bytes.fromhex("06082a8648ce3d040303")
SPKI_P256_PREFIX = bytes.fromhex(
    "3059301306072a8648ce3d020106082a8648ce3d03010703420004"
)

# NIST P-256 (secp256r1) parameters (FIPS 186-4):
P256_P = 0xFFFFFFFF00000001000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFF
P256_A = P256_P - 3
P256_B = 0x5AC635D8AA3A93E7B3EBBD55769886BC651D06B0CC53B0F63BCE3C3E27D2604B
P256_N = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551
P256_G = (
    0x6B17D1F2E12C4247F8BCE6E563A440F277037D812DEB33A0F4A13945D898C296,
    0x4FE342E2FE1A7F9B8EE7EB4A7C0F9E162BCE33576B315ECECBB6406837BF51F5,
)

# NIST P-384 (secp384r1) parameters (FIPS 186-4):
P384_P = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFFFF0000000000000000FFFFFFFF
P384_A = P384_P - 3
P384_B = 0xB3312FA7E23EE7E4988E056BE3F82D19181D9C6EFE8141120314088F5013875AC656398D8A2ED19D2A85C8EDD3EC2AEF
P384_N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFC7634D81F4372DDF581A0DB248B0A77AECEC196ACCC52973
P384_G = (
    0xAA87CA22BE8B05378EB1C71EF320AD746E1D3B628BA79B9859F741E082542A385502F25DBF55296C3A545E3872760AB7,
    0x3617DE4A96262C6F5D9E98BF9292DC29F8F41DBD289A147CE9DA3113B5F0B8C00A60B1CE1D7E819D7A431D7C90EA0E5F,
)


@dataclass(frozen=True)
class SigstoreTrustRoot:
    # Sigstore Production Fulcio Intermediate CA (sigstore-intermediate, O=sigstore.dev)
    # and Rekor v1 transparency log (rekor.sigstore.dev) from Sigstore TUF trusted_root.json.
    fulcio_issuer_der: bytes
    fulcio_p384_q: tuple[int, int]
    rekor_log_id_hex: str
    rekor_log_id_b64: str
    rekor_p256_q: tuple[int, int]


DEFAULT_TRUST_ROOT = SigstoreTrustRoot(
    fulcio_issuer_der=bytes.fromhex(
        "303731153013060355040a130c73696773746f72652e646576311e301c0603550403131573696773746f72652d696e7465726d656469617465"
    ),
    fulcio_p384_q=(
        0xF11552FF2B07F8D3AFB836723C866D8A581417D3656AB62901DF473F5BC1047D54E4257BECB492EECD19887E2713B1EF,
        0xEE9B52E8BBEF47F49393BF7C2D580CCCB949E077887C5DED1D269EC4B718A52012AF5912D0DFD1801273FFD8D60A25E7,
    ),
    rekor_log_id_hex="c0d23d6ad406973f9559f3ba2d1ca01f84147d8ffc5b8445c224f98b9591801d",
    rekor_log_id_b64="wNI9atQGlz+VWfO6LRygH4QUfY/8W4RFwiT5i5WRgB0=",
    rekor_p256_q=(
        0xD86D98FB6B5A6DD4D5E41706881231D1AF5F005C2B9016E62D21AD92CE0BDEA5,
        0xFAC98634CEE7C19E10BC52BFE2CB9E468563FFF40FDB6362E10B7D0CF7E458B7,
    ),
)


def fail(reason: str) -> None:
    print(f"error: {reason}", file=sys.stderr)
    sys.exit(1)


def is_ci_env() -> bool:
    for var in ("GITHUB_ACTIONS", "CI"):
        val = os.environ.get(var, "").strip().lower()
        if val and val not in ("0", "false"):
            return True
    return False


def active_trust_root() -> SigstoreTrustRoot:
    override = os.environ.get("SYSPRIMS_SIGSTORE_TEST_ROOT")
    if not override:
        return DEFAULT_TRUST_ROOT
    if is_ci_env():
        fail("SYSPRIMS_SIGSTORE_TEST_ROOT is not accepted under GitHub Actions or CI")
    data = json.loads(Path(override).read_text(encoding="utf-8"))
    return SigstoreTrustRoot(
        fulcio_issuer_der=bytes.fromhex(data["fulcio_issuer_der_hex"]),
        fulcio_p384_q=(
            int(data["fulcio_p384_qx"], 16),
            int(data["fulcio_p384_qy"], 16),
        ),
        rekor_log_id_hex=data["rekor_log_id_hex"],
        rekor_log_id_b64=data["rekor_log_id_b64"],
        rekor_p256_q=(int(data["rekor_p256_qx"], 16), int(data["rekor_p256_qy"], 16)),
    )


def ec_on_curve(pt: tuple[int, int] | None, p: int, a: int, b: int) -> bool:
    if pt is None:
        return False
    x, y = pt
    if not (0 <= x < p and 0 <= y < p):
        return False
    return (y * y - (x * x * x + a * x + b)) % p == 0


def ec_add(
    p1: tuple[int, int] | None,
    p2: tuple[int, int] | None,
    p: int,
    a: int,
) -> tuple[int, int] | None:
    if p1 is None:
        return p2
    if p2 is None:
        return p1
    x1, y1 = p1
    x2, y2 = p2
    if x1 == x2:
        if (y1 + y2) % p == 0:
            return None
        lam = (3 * x1 * x1 + a) * pow(2 * y1, -1, p) % p
    else:
        lam = (y2 - y1) * pow((x2 - x1) % p, -1, p) % p
    x3 = (lam * lam - x1 - x2) % p
    y3 = (lam * (x1 - x3) - y1) % p
    return (x3, y3)


def ec_mul(
    k: int, pt: tuple[int, int] | None, p: int, a: int
) -> tuple[int, int] | None:
    res: tuple[int, int] | None = None
    addend = pt
    while k > 0:
        if k & 1:
            res = ec_add(res, addend, p, a)
        addend = ec_add(addend, addend, p, a)
        k >>= 1
    return res


def p256_mul(k: int, pt: tuple[int, int] | None) -> tuple[int, int] | None:
    return ec_mul(k, pt, P256_P, P256_A)


def p384_mul(k: int, pt: tuple[int, int] | None) -> tuple[int, int] | None:
    return ec_mul(k, pt, P384_P, P384_A)


def read_der_tlv(data: bytes, offset: int = 0) -> tuple[int, bytes, int]:
    if offset + 2 > len(data):
        fail("truncated ASN.1 DER header")
    tag = data[offset]
    length = data[offset + 1]
    pos = offset + 2
    if length & 0x80:
        num_bytes = length & 0x7F
        if num_bytes == 0 or num_bytes > 4 or pos + num_bytes > len(data):
            fail("invalid ASN.1 DER length prefix")
        length = int.from_bytes(data[pos : pos + num_bytes], "big")
        pos += num_bytes
    end = pos + length
    if end > len(data):
        fail("ASN.1 DER element extends past buffer")
    return tag, data[pos:end], end


def parse_der_int(body: bytes) -> int:
    if not body or (body[0] & 0x80):
        fail("ASN.1 INTEGER must be positive")
    if len(body) > 1 and body[0] == 0x00 and not (body[1] & 0x80):
        fail("ASN.1 INTEGER has non-minimal leading zero padding")
    return int.from_bytes(body, "big")


def parse_ecdsa_sig_der(sig_der: bytes, label: str) -> tuple[int, int]:
    tag, seq, end = read_der_tlv(sig_der, 0)
    if tag != 0x30 or end != len(sig_der):
        fail(f"{label}: ECDSA signature is not a valid ASN.1 SEQUENCE")
    t_r, r_bytes, next_pos = read_der_tlv(seq, 0)
    t_s, s_bytes, seq_end = read_der_tlv(seq, next_pos)
    if t_r != 0x02 or t_s != 0x02 or seq_end != len(seq):
        fail(f"{label}: ECDSA signature must contain exactly two ASN.1 INTEGERs")
    return parse_der_int(r_bytes), parse_der_int(s_bytes)


def ecdsa_verify_digest(
    pub_q: tuple[int, int],
    digest: bytes,
    sig_der: bytes,
    label: str,
    *,
    p: int,
    a: int,
    b: int,
    n: int,
    g: tuple[int, int],
) -> None:
    if not ec_on_curve(pub_q, p, a, b):
        fail(f"{label}: EC public key point is not on the curve")
    r, s = parse_ecdsa_sig_der(sig_der, label)
    if not (1 <= r < n and 1 <= s < n):
        fail(f"{label}: ECDSA (r, s) out of curve order range")
    e = int.from_bytes(digest, "big")
    n_bits = n.bit_length()
    d_bits = len(digest) * 8
    if d_bits > n_bits:
        e >>= d_bits - n_bits
    w = pow(s, -1, n)
    u1 = (e * w) % n
    u2 = (r * w) % n
    pt = ec_add(ec_mul(u1, g, p, a), ec_mul(u2, pub_q, p, a), p, a)
    if pt is None or (pt[0] % n) != r:
        fail(f"{label}: cryptographic ECDSA signature verification failed")


def ecdsa_p256_verify_sha256(
    pub_q: tuple[int, int], message: bytes, sig_der: bytes, label: str
) -> None:
    ecdsa_verify_digest(
        pub_q,
        hashlib.sha256(message).digest(),
        sig_der,
        label,
        p=P256_P,
        a=P256_A,
        b=P256_B,
        n=P256_N,
        g=P256_G,
    )


def ecdsa_p384_verify_sha384(
    pub_q: tuple[int, int], message: bytes, sig_der: bytes, label: str
) -> None:
    ecdsa_verify_digest(
        pub_q,
        hashlib.sha384(message).digest(),
        sig_der,
        label,
        p=P384_P,
        a=P384_A,
        b=P384_B,
        n=P384_N,
        g=P384_G,
    )


def parse_asn1_time_unix(tag: int, body: bytes, label: str) -> int:
    try:
        text = body.decode("ascii")
    except UnicodeDecodeError as err:
        raise ValueError("non-ASCII ASN.1 time") from err
    if not text.endswith("Z"):
        fail(f"{label}: ASN.1 time must be UTC ('Z'): {text!r}")
    core = text[:-1]
    if tag == 0x17:  # UTCTime: YYMMDDHHMMSSZ
        if len(core) != 12 or not core.isdigit():
            fail(f"{label}: malformed UTCTime {text!r}")
        yy = int(core[0:2])
        year = 2000 + yy if yy < 50 else 1900 + yy
        rest = core[2:]
    elif tag == 0x18:  # GeneralizedTime: YYYYMMDDHHMMSSZ
        if len(core) != 14 or not core.isdigit():
            fail(f"{label}: malformed GeneralizedTime {text!r}")
        year = int(core[0:4])
        rest = core[4:]
    else:
        fail(f"{label}: unsupported ASN.1 time tag 0x{tag:02x}")
    month = int(rest[0:2])
    day = int(rest[2:4])
    hour = int(rest[4:6])
    minute = int(rest[6:8])
    second = int(rest[8:10])
    return int(calendar.timegm((year, month, day, hour, minute, second, 0, 0, 0)))


def parse_and_verify_fulcio_leaf_cert(
    cert_der: bytes,
    expected_san_uris: tuple[bytes, ...],
    version: str,
    trust_root: SigstoreTrustRoot,
    label: str,
) -> tuple[tuple[int, int], int, int]:
    """Parse X.509v3 leaf certificate DER, verify Fulcio CA signature and OIDC extensions."""
    tag, cert_seq, end = read_der_tlv(cert_der, 0)
    if tag != 0x30 or end != len(cert_der):
        fail(f"{label}: certificate is not a valid ASN.1 DER SEQUENCE")

    tbs_start = 0
    t_tbs, tbs_body, pos_alg = read_der_tlv(cert_seq, tbs_start)
    if t_tbs != 0x30:
        fail(f"{label}: missing TBSCertificate SEQUENCE")
    tbs_raw = cert_seq[tbs_start:pos_alg]

    t_sig_alg, sig_alg_body, pos_sig = read_der_tlv(cert_seq, pos_alg)
    t_sig_val, sig_val_body, cert_end = read_der_tlv(cert_seq, pos_sig)
    if t_sig_alg != 0x30 or t_sig_val != 0x03 or cert_end != len(cert_seq):
        fail(f"{label}: malformed Certificate signature fields")
    if not sig_alg_body.startswith(ALG_ECDSA_SHA384):
        fail(f"{label}: Fulcio leaf certificate must be signed with ecdsa-with-SHA384")
    if not sig_val_body or sig_val_body[0] != 0x00:
        fail(f"{label}: malformed BIT STRING in Certificate signatureValue")
    ca_sig_der = sig_val_body[1:]

    ecdsa_p384_verify_sha384(
        trust_root.fulcio_p384_q,
        tbs_raw,
        ca_sig_der,
        f"{label} Fulcio CA certificate signature",
    )

    pos = 0
    t0, v0, next_pos = read_der_tlv(tbs_body, pos)
    if t0 != 0xA0 or v0 != b"\x02\x01\x02":
        fail(f"{label}: TBSCertificate must be X.509 v3")
    pos = next_pos

    t_ser, _, pos = read_der_tlv(tbs_body, pos)  # serialNumber
    t_alg, tbs_alg, pos = read_der_tlv(tbs_body, pos)  # signature
    if t_ser != 0x02 or t_alg != 0x30 or tbs_alg != sig_alg_body:
        fail(f"{label}: TBSCertificate signature algorithm mismatch")

    issuer_start = pos
    t_iss, _, pos = read_der_tlv(tbs_body, pos)  # issuer
    issuer_raw = tbs_body[issuer_start:pos]
    if t_iss != 0x30 or issuer_raw != trust_root.fulcio_issuer_der:
        fail(f"{label}: certificate issuer does not match pinned Sigstore Fulcio CA")

    t_val, val_body, pos = read_der_tlv(tbs_body, pos)  # validity
    if t_val != 0x30:
        fail(f"{label}: malformed TBSCertificate Validity")
    t_nb, nb_body, val_pos = read_der_tlv(val_body, 0)
    t_na, na_body, val_end = read_der_tlv(val_body, val_pos)
    if val_end != len(val_body):
        fail(f"{label}: trailing bytes in TBSCertificate Validity")
    not_before = parse_asn1_time_unix(t_nb, nb_body, f"{label} notBefore")
    not_after = parse_asn1_time_unix(t_na, na_body, f"{label} notAfter")

    t_sub, _, pos = read_der_tlv(tbs_body, pos)  # subject
    if t_sub != 0x30:
        fail(f"{label}: malformed TBSCertificate Subject")

    spki_start = pos
    t_spki, _, pos = read_der_tlv(tbs_body, pos)  # subjectPublicKeyInfo
    spki_raw = tbs_body[spki_start:pos]
    if (
        t_spki != 0x30
        or len(spki_raw) != len(SPKI_P256_PREFIX) + 64
        or not spki_raw.startswith(SPKI_P256_PREFIX)
    ):
        fail(f"{label}: TBSCertificate SPKI is not an uncompressed P-256 EC public key")
    xy = spki_raw[len(SPKI_P256_PREFIX) :]
    leaf_q = (int.from_bytes(xy[:32], "big"), int.from_bytes(xy[32:], "big"))
    if not ec_on_curve(leaf_q, P256_P, P256_A, P256_B):
        fail(f"{label}: leaf certificate P-256 public key is not on secp256r1")

    exts_raw: bytes | None = None
    while pos < len(tbs_body):
        t_field, field_body, pos = read_der_tlv(tbs_body, pos)
        if t_field == 0xA3:
            exts_raw = field_body
            break
    if exts_raw is None:
        fail(f"{label}: TBSCertificate has no X.509v3 extensions")

    t_ext_seq, ext_seq_body, ext_seq_end = read_der_tlv(exts_raw, 0)
    if t_ext_seq != 0x30 or ext_seq_end != len(exts_raw):
        fail(f"{label}: malformed X.509v3 Extensions SEQUENCE")

    exts: dict[bytes, bytes] = {}
    epos = 0
    while epos < len(ext_seq_body):
        t_ext, ext_body, epos = read_der_tlv(ext_seq_body, epos)
        if t_ext != 0x30:
            fail(f"{label}: malformed Extension entry")
        t_oid, oid_bytes, inner = read_der_tlv(ext_body, 0)
        if t_oid != 0x06:
            fail(f"{label}: Extension missing OID")
        t_next, next_body, inner = read_der_tlv(ext_body, inner)
        if t_next == 0x01:  # critical BOOLEAN
            t_next, next_body, inner = read_der_tlv(ext_body, inner)
        if t_next != 0x04 or inner != len(ext_body):
            fail(f"{label}: Extension missing OCTET STRING extnValue")
        if oid_bytes in exts:
            fail(f"{label}: duplicate X.509 extension OID {oid_bytes.hex()}")
        exts[oid_bytes] = next_body

    bc_bytes = exts.get(OID_EXT_BASIC_CONSTRAINTS)
    if bc_bytes is not None:
        t_bc, bc_seq, bc_end = read_der_tlv(bc_bytes, 0)
        if t_bc != 0x30 or bc_end != len(bc_bytes):
            fail(f"{label}: malformed basicConstraints extension")
        if bc_seq:
            t_ca, ca_val, _ = read_der_tlv(bc_seq, 0)
            if t_ca == 0x01 and ca_val != b"\x00":
                fail(
                    f"{label}: leaf certificate must not have basicConstraints cA=TRUE"
                )

    san_bytes = exts.get(OID_EXT_SAN)
    if san_bytes is None:
        fail(f"{label}: certificate missing subjectAltName extension")
    t_san, san_seq, san_end = read_der_tlv(san_bytes, 0)
    if t_san != 0x30 or san_end != len(san_bytes):
        fail(f"{label}: malformed subjectAltName SEQUENCE")
    san_uris: list[bytes] = []
    spos = 0
    while spos < len(san_seq):
        t_gn, gn_body, spos = read_der_tlv(san_seq, spos)
        if t_gn == 0x86:  # uniformResourceIdentifier [6] IMPLICIT IA5String
            san_uris.append(gn_body)
    matched_san = next((u for u in expected_san_uris if u in san_uris), None)
    if matched_san is None:
        fail(
            f"{label}: certificate missing expected workflow SAN URI (got {[u.decode('ascii', errors='replace') for u in san_uris]})"
        )

    issuer_v1 = exts.get(OID_FULCIO_ISSUER_V1)
    issuer_v2 = exts.get(OID_FULCIO_ISSUER_V2)
    if issuer_v1 != EXPECTED_OIDC_ISSUER and issuer_v2 != (
        b"\x0c" + bytes([len(EXPECTED_OIDC_ISSUER)]) + EXPECTED_OIDC_ISSUER
    ):
        fail(f"{label}: Fulcio OIDC issuer extension mismatch")

    expected_ref = f"refs/tags/v{version}".encode("ascii")
    expected_repo_bytes = EXPECTED_REPO.encode("ascii")
    expected_source_uri = f"https://github.com/{EXPECTED_REPO}".encode("ascii")

    def require_ext_match(
        oid: bytes, allowed_raw: tuple[bytes, ...], name: str
    ) -> bool:
        val = exts.get(oid)
        if val is None:
            return False
        if val in allowed_raw:
            return True
        t_str, str_body, str_end = read_der_tlv(val, 0)
        if t_str in (0x0C, 0x16) and str_end == len(val) and str_body in allowed_raw:
            return True
        fail(f"{label}: Fulcio {name} extension mismatch: {val!r}")
        return False

    has_repo = require_ext_match(
        OID_FULCIO_WORKFLOW_REPO, (expected_repo_bytes,), "repository"
    ) | require_ext_match(
        OID_FULCIO_SOURCE_REPO_URI, (expected_source_uri,), "source repository URI"
    )
    has_ref = require_ext_match(
        OID_FULCIO_WORKFLOW_REF, (expected_ref,), "workflow ref"
    ) | require_ext_match(
        OID_FULCIO_SOURCE_REPO_REF, (expected_ref,), "source repo ref"
    )
    has_signer = require_ext_match(
        OID_FULCIO_BUILD_SIGNER_URI, expected_san_uris, "build signer URI"
    )
    if not (has_repo and has_ref and has_signer):
        fail(
            f"{label}: certificate missing required Fulcio GitHub workflow OIDC extensions"
        )

    return leaf_q, not_before, not_after


def verify_rekor_merkle_inclusion(
    leaf_bytes: bytes,
    log_index: int,
    tree_size: int,
    hashes_b64: list,
    expected_root: bytes,
    label: str,
) -> None:
    """RFC 6962 §2.1.3 Merkle audit path inclusion proof verification."""
    if log_index < 0 or tree_size <= 0 or log_index >= tree_size:
        fail(
            f"{label}: invalid Rekor inclusionProof index={log_index} treeSize={tree_size}"
        )
    fn = log_index
    sn = tree_size - 1
    cur = hashlib.sha256(b"\x00" + leaf_bytes).digest()
    for h_b64 in hashes_b64:
        if not isinstance(h_b64, str):
            fail(f"{label}: non-string hash in Rekor inclusionProof")
        peer = base64.b64decode(h_b64)
        if len(peer) != 32:
            fail(f"{label}: Rekor inclusionProof hash is not 32 bytes")
        if sn == 0:
            fail(f"{label}: Rekor inclusionProof has extra audit path hashes")
        if (fn & 1) == 1 or fn == sn:
            cur = hashlib.sha256(b"\x01" + peer + cur).digest()
            while (fn & 1) == 0 and fn != 0:
                fn >>= 1
                sn >>= 1
        else:
            cur = hashlib.sha256(b"\x01" + cur + peer).digest()
        fn >>= 1
        sn >>= 1
    if sn != 0 or cur != expected_root:
        fail(f"{label}: Rekor RFC 6962 inclusionProof Merkle root mismatch")


def verify_rekor_tlog_entry(
    entry: dict,
    cert_der: bytes,
    dsse_sig: bytes,
    stmt_bytes: bytes,
    not_before: int,
    not_after: int,
    trust_root: SigstoreTrustRoot,
    label: str,
) -> None:
    """Cryptographically verify a Rekor transparency log entry against the pinned Rekor key."""
    log_id = (entry.get("logId") or {}).get("keyId")
    if log_id not in (trust_root.rekor_log_id_b64, trust_root.rekor_log_id_hex):
        fail(f"{label}: untrusted or missing Rekor logId.keyId: {log_id!r}")

    integrated_time = entry.get("integratedTime")
    if isinstance(integrated_time, str) and integrated_time.isdigit():
        integrated_time = int(integrated_time)
    if not isinstance(integrated_time, int) or integrated_time <= 0:
        fail(f"{label}: invalid Rekor integratedTime: {integrated_time!r}")
    if not (not_before <= integrated_time <= not_after):
        fail(
            f"{label}: Rekor integratedTime ({integrated_time}) is outside certificate validity [{not_before}, {not_after}]"
        )

    log_index = entry.get("logIndex")
    if isinstance(log_index, str) and log_index.isdigit():
        log_index = int(log_index)
    if not isinstance(log_index, int) or log_index < 0:
        fail(f"{label}: invalid Rekor logIndex: {log_index!r}")

    canon_b64 = entry.get("canonicalizedBody")
    if not isinstance(canon_b64, str) or not canon_b64:
        fail(f"{label}: missing Rekor canonicalizedBody")
    body_bytes = base64.b64decode(canon_b64)
    body_doc = json.loads(body_bytes.decode("utf-8"))

    stmt_sha256_hex = hashlib.sha256(stmt_bytes).hexdigest()
    kind = body_doc.get("kind")
    spec = body_doc.get("spec") or {}
    if kind == "intoto":
        content = spec.get("content") or {}
        payload_hash = content.get("payloadHash") or {}
        env = content.get("envelope") or {}
        sigs = env.get("signatures") or []
    elif kind == "dsse":
        payload_hash = spec.get("payloadHash") or {}
        sigs = spec.get("signatures") or []
    else:
        fail(f"{label}: unsupported Rekor entry kind {kind!r}")

    if (
        payload_hash.get("algorithm") != "sha256"
        or payload_hash.get("value") != stmt_sha256_hex
    ):
        fail(
            f"{label}: Rekor entry payloadHash does not match in-toto statement sha256"
        )
    if not isinstance(sigs, list) or not sigs:
        fail(f"{label}: Rekor entry has no signatures in spec")

    def decode_pem_or_der_cert(raw_b64: str) -> bytes:
        raw = base64.b64decode(raw_b64)
        if b"-----BEGIN CERTIFICATE-----" in raw:
            lines = [
                line.strip()
                for line in raw.decode("ascii", errors="ignore").splitlines()
                if line.strip() and not line.strip().startswith("-----")
            ]
            return base64.b64decode("".join(lines))
        return raw

    def matches_dsse_sig(sig_field: str) -> bool:
        try:
            raw = base64.b64decode(sig_field)
        except Exception:
            return False
        if raw == dsse_sig:
            return True
        try:
            return base64.b64decode(raw) == dsse_sig
        except Exception:
            return False

    matched_binding = False
    for s_entry in sigs:
        if not isinstance(s_entry, dict):
            continue
        cert_field = s_entry.get("verifier") or s_entry.get("publicKey")
        sig_field = s_entry.get("signature") or s_entry.get("sig")
        if not isinstance(cert_field, str) or not isinstance(sig_field, str):
            continue
        if decode_pem_or_der_cert(cert_field) == cert_der and matches_dsse_sig(
            sig_field
        ):
            matched_binding = True
            break
    if not matched_binding:
        fail(
            f"{label}: Rekor entry spec.signatures does not bind leaf certificate and DSSE signature"
        )

    promise = entry.get("inclusionPromise") or {}
    set_b64 = promise.get("signedEntryTimestamp")
    if not isinstance(set_b64, str) or not set_b64:
        fail(
            f"{label}: Rekor tlog entry is missing or invalid inclusionPromise.signedEntryTimestamp"
        )
    set_payload = json.dumps(
        {
            "body": canon_b64,
            "integratedTime": integrated_time,
            "logID": trust_root.rekor_log_id_hex,
            "logIndex": log_index,
        },
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    ecdsa_p256_verify_sha256(
        trust_root.rekor_p256_q,
        set_payload,
        base64.b64decode(set_b64),
        f"{label} Rekor SET",
    )

    proof = entry.get("inclusionProof") or {}
    if proof:
        root_b64 = proof.get("rootHash")
        hashes = proof.get("hashes")
        tree_size = proof.get("treeSize")
        proof_index = proof.get("logIndex", log_index)
        if isinstance(tree_size, str) and tree_size.isdigit():
            tree_size = int(tree_size)
        if isinstance(proof_index, str) and proof_index.isdigit():
            proof_index = int(proof_index)
        checkpoint = (proof.get("checkpoint") or {}).get("envelope")
        if (
            not isinstance(root_b64, str)
            or not isinstance(hashes, list)
            or not isinstance(tree_size, int)
            or not isinstance(proof_index, int)
            or proof_index != log_index
            or not isinstance(checkpoint, str)
        ):
            fail(f"{label}: incomplete Rekor inclusionProof")
        root_bytes = base64.b64decode(root_b64)
        if len(root_bytes) != 32:
            fail(f"{label}: Rekor inclusionProof rootHash must decode to 32 bytes")
        verify_rekor_merkle_inclusion(
            body_bytes,
            log_index,
            tree_size,
            hashes,
            root_bytes,
            label,
        )
        parts = checkpoint.split("\n\n", 1)
        if len(parts) != 2:
            fail(f"{label}: malformed Rekor checkpoint envelope")
        note_body, sig_section = parts
        note_lines = note_body.splitlines()
        if len(note_lines) < 3 or not note_lines[1].isdigit():
            fail(f"{label}: malformed Rekor checkpoint note body")
        if (
            int(note_lines[1]) != tree_size
            or base64.b64decode(note_lines[2]) != root_bytes
        ):
            fail(f"{label}: Rekor checkpoint root hash does not match inclusionProof")
        signed_note = (note_body + "\n").encode("utf-8")
        key_hint = bytes.fromhex(trust_root.rekor_log_id_hex[:8])
        verified_cp = False
        for line in sig_section.splitlines():
            line = line.strip()
            if not line.startswith("\u2014 "):
                continue
            tokens = line.split()
            if len(tokens) < 3:
                continue
            raw_sig = base64.b64decode(tokens[-1])
            if len(raw_sig) > 4 and raw_sig[:4] == key_hint:
                ecdsa_p256_verify_sha256(
                    trust_root.rekor_p256_q,
                    signed_note,
                    raw_sig[4:],
                    f"{label} Rekor checkpoint",
                )
                verified_cp = True
                break
        if not verified_cp:
            fail(
                f"{label}: Rekor checkpoint contains no signature from pinned Rekor log key"
            )


def verify_sigstore_attestation(
    cert_b64: str,
    tlog_entries: list,
    statement_b64: str,
    sig_b64: str,
    expected_subject_name: str,
    expected_digest_alg: str,
    expected_digest_hex: str,
    version: str,
    trust_root: SigstoreTrustRoot,
    label: str,
) -> None:
    """Verify Fulcio certificate, Rekor transparency log entry, DSSE signature, and SLSA statement."""
    if not isinstance(cert_b64, str) or not cert_b64:
        fail(f"{label} missing verification certificate")
    if not isinstance(tlog_entries, list) or not tlog_entries:
        fail(f"{label} missing Rekor transparency log entries")
    if not isinstance(statement_b64, str) or not statement_b64:
        fail(f"{label} missing in-toto statement payload")
    if not isinstance(sig_b64, str) or not sig_b64:
        fail(f"{label} missing DSSE signature")

    cert_der = base64.b64decode(cert_b64)
    expected_san_uris = tuple(
        f"https://github.com/{EXPECTED_REPO}/{wf}@refs/tags/v{version}".encode("ascii")
        for wf in EXPECTED_WORKFLOW_PATHS
    )
    leaf_q, not_before, not_after = parse_and_verify_fulcio_leaf_cert(
        cert_der, expected_san_uris, version, trust_root, label
    )

    stmt_bytes = base64.b64decode(statement_b64)
    dsse_sig = base64.b64decode(sig_b64)
    pae = (
        f"DSSEv1 {len(DSSE_PAYLOAD_TYPE)} {DSSE_PAYLOAD_TYPE} {len(stmt_bytes)} ".encode(
            "ascii"
        )
        + stmt_bytes
    )
    ecdsa_p256_verify_sha256(leaf_q, pae, dsse_sig, f"{label} DSSE PAE")

    for entry in tlog_entries:
        if not isinstance(entry, dict):
            fail(f"{label}: Rekor transparency entry is not an object")
        verify_rekor_tlog_entry(
            entry,
            cert_der,
            dsse_sig,
            stmt_bytes,
            not_before,
            not_after,
            trust_root,
            label,
        )

    stmt = json.loads(stmt_bytes.decode("utf-8"))
    if stmt.get("_type") != "https://in-toto.io/Statement/v1":
        fail(f"{label} statement _type mismatch: {stmt.get('_type')!r}")
    pred_type = stmt.get("predicateType")
    if pred_type != SLSA_PREDICATE_V1:
        fail(f"{label} statement predicateType mismatch: {pred_type!r}")
    subjects = stmt.get("subject") or []
    matched = [
        s
        for s in subjects
        if isinstance(s, dict)
        and s.get("name") == expected_subject_name
        and (s.get("digest") or {}).get(expected_digest_alg) == expected_digest_hex
    ]
    if not matched:
        fail(
            f"{label} statement subject mismatch (expected {expected_subject_name} {expected_digest_alg}={expected_digest_hex}): {subjects!r}"
        )

    predicate = stmt.get("predicate")
    if not isinstance(predicate, dict):
        fail(f"{label} SLSA statement predicate must be an object")
    wf = (
        (predicate.get("buildDefinition") or {})
        .get("externalParameters", {})
        .get("workflow")
    )
    if isinstance(wf, dict):
        if (
            wf.get("repository") != f"https://github.com/{EXPECTED_REPO}"
            or wf.get("path") not in EXPECTED_WORKFLOW_PATHS
            or wf.get("ref") != f"refs/tags/v{version}"
        ):
            fail(f"{label} SLSA workflow parameters mismatch: {wf!r}")


def validate_base_url(url: str, default: str, env_var: str) -> str:
    if url == default:
        return url
    if is_ci_env():
        fail(f"{env_var} is not accepted under GitHub Actions or CI")
    parsed = urllib.parse.urlsplit(url)
    if (
        parsed.scheme != "http"
        or parsed.hostname != "127.0.0.1"
        or parsed.port is None
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
    ):
        fail(
            f"{env_var} override must be http://127.0.0.1:<port>[/path] without userinfo, query, or fragment: {url}"
        )
    return url.rstrip("/")


def poll_config() -> tuple[int, float]:
    attempts = int(os.environ.get("SYSPRIMS_REGISTRY_POLL_ATTEMPTS", "6"))
    sleep_s = float(os.environ.get("SYSPRIMS_REGISTRY_POLL_SLEEP", "5"))
    if attempts < 1 or attempts > 30 or sleep_s < 0 or sleep_s > 60:
        fail("invalid SYSPRIMS_REGISTRY_POLL_ATTEMPTS or SYSPRIMS_REGISTRY_POLL_SLEEP")
    return attempts, sleep_s


def fetch_bytes(url: str, label: str, accept: str = "application/json") -> bytes:
    attempts, sleep_s = poll_config()
    last_error = "unavailable"
    for attempt in range(1, attempts + 1):
        req = urllib.request.Request(
            url,
            headers={
                "Accept": accept,
                "User-Agent": "sysprims-release-verify/1.0",
            },
        )
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                return resp.read()
        except (urllib.error.HTTPError, urllib.error.URLError) as err:
            last_error = str(err)
            retryable = isinstance(err, urllib.error.URLError) or (
                isinstance(err, urllib.error.HTTPError)
                and err.code in (404, 429, 500, 502, 503, 504)
            )
            if attempt < attempts and retryable:
                time.sleep(sleep_s)
                continue
            break
    fail(f"{label} ({url}) unavailable after {attempts} attempt(s): {last_error}")
    return b""


def fetch_json(url: str, label: str) -> dict:
    raw = fetch_bytes(url, label)
    try:
        data = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as err:
        fail(f"{label} ({url}) returned invalid JSON: {err}")
    if not isinstance(data, dict):
        fail(f"{label} ({url}) did not return a JSON object")
    return data


def verify_npm(
    version: str,
    commit: str | None,
    require_provenance: bool,
    trust_root: SigstoreTrustRoot,
) -> None:
    default_base = "https://registry.npmjs.org"
    base = validate_base_url(
        os.environ.get("SYSPRIMS_NPM_REGISTRY_URL", default_base),
        default_base,
        "SYSPRIMS_NPM_REGISTRY_URL",
    )

    for pkg in NPM_PACKAGES:
        doc = fetch_json(f"{base}/{pkg}/{version}", f"npm {pkg} {version}")
        if doc.get("name") != pkg or doc.get("version") != version:
            fail(f"npm metadata name/version mismatch for {pkg}@{version}")

        if commit is not None:
            git_head = doc.get("gitHead")
            if (
                not isinstance(git_head, str)
                or not re.fullmatch(r"[0-9a-f]{40}", git_head)
                or git_head != commit
            ):
                fail(
                    f"npm gitHead mismatch for {pkg}@{version}: {git_head!r} != {commit}"
                )

        dist = doc.get("dist") or {}
        integrity = dist.get("integrity")
        shasum = dist.get("shasum")
        if not isinstance(integrity, str) or not integrity.startswith("sha512-"):
            fail(
                f"npm dist.integrity missing or not sha512 for {pkg}@{version}: {integrity!r}"
            )
        try:
            sha512_raw = base64.b64decode(
                integrity.removeprefix("sha512-"), validate=True
            )
        except Exception:
            sha512_raw = b""
        if len(sha512_raw) != 64:
            fail(f"npm dist.integrity must decode to 64 bytes for {pkg}@{version}")
        expected_sha512_hex = sha512_raw.hex()

        if not isinstance(shasum, str) or not re.fullmatch(r"[0-9a-f]{40}", shasum):
            fail(
                f"npm dist.shasum missing or malformed for {pkg}@{version}: {shasum!r}"
            )

        tarball_url = dist.get("tarball")
        if tarball_url is not None:
            if not isinstance(tarball_url, str) or not tarball_url.startswith(
                f"{base}/"
            ):
                fail(
                    f"npm dist.tarball URL invalid for {pkg}@{version}: {tarball_url!r}"
                )
            tgz_bytes = fetch_bytes(
                tarball_url,
                f"npm tarball {pkg}@{version}",
                accept="application/octet-stream",
            )
            actual_sha512_hex = hashlib.sha512(tgz_bytes).hexdigest()
            actual_sha1_hex = hashlib.sha1(tgz_bytes).hexdigest()
            if actual_sha512_hex != expected_sha512_hex:
                fail(f"npm tarball sha512 mismatch for {pkg}@{version}")
            if actual_sha1_hex != shasum:
                fail(f"npm tarball shasum mismatch for {pkg}@{version}")

        attestations = dist.get("attestations")
        if require_provenance:
            if not isinstance(attestations, dict) or not attestations.get("url"):
                fail(
                    f"npm Sigstore provenance missing dist.attestations for {pkg}@{version}"
                )
            att_url = attestations.get("url")
            prov = attestations.get("provenance") or {}
            if not isinstance(att_url, str) or not att_url.startswith(
                f"{base}/-/npm/v1/attestations/"
            ):
                fail(
                    f"npm Sigstore provenance attestation invalid URL for {pkg}@{version}: {att_url!r}"
                )
            if prov.get("predicateType") != SLSA_PREDICATE_V1:
                fail(
                    f"npm Sigstore provenance attestation invalid predicateType for {pkg}@{version}: {prov!r}"
                )
            att_doc = fetch_json(
                att_url, f"npm Sigstore provenance for {pkg}@{version}"
            )
            att_list = att_doc.get("attestations") or []
            slsa_atts = [
                a
                for a in att_list
                if isinstance(a, dict) and a.get("predicateType") == SLSA_PREDICATE_V1
            ]
            if not slsa_atts:
                fail(
                    f"npm Sigstore attestation endpoint returned no attestations for {pkg}@{version}"
                )
            bundle = slsa_atts[0].get("bundle") or {}
            vm = bundle.get("verificationMaterial") or {}
            cert_b64 = (vm.get("certificate") or {}).get("rawBytes")
            tlog_entries = vm.get("tlogEntries") or []
            dsse = bundle.get("dsseEnvelope") or {}
            if dsse.get("payloadType") != DSSE_PAYLOAD_TYPE:
                fail(
                    f"npm DSSE payloadType mismatch for {pkg}@{version}: {dsse.get('payloadType')!r}"
                )
            sigs = dsse.get("signatures") or []
            sig_b64 = sigs[0].get("sig") if sigs and isinstance(sigs[0], dict) else None
            purl = f"pkg:npm/{pkg.replace('@', '%40')}@{version}"
            verify_sigstore_attestation(
                cert_b64,
                tlog_entries,
                dsse.get("payload"),
                sig_b64,
                purl,
                "sha512",
                expected_sha512_hex,
                version,
                trust_root,
                f"npm {pkg}@{version}",
            )

    prov_note = (
        " + Sigstore provenance"
        if require_provenance
        else " (bootstrap: no provenance)"
    )
    print(
        f"[ok] npm: {len(NPM_PACKAGES)} @3leaps/sysprims* packages@{version} match expected digests{prov_note}"
    )


def main() -> None:
    args = sys.argv[1:]
    require_provenance = True
    seen_expect = False
    seen_bootstrap = False
    version: str | None = None
    commit: str | None = None
    positional: list[str] = []

    i = 0
    while i < len(args):
        arg = args[i]
        if arg == "--expect-provenance":
            seen_expect = True
            require_provenance = True
            i += 1
        elif arg == "--bootstrap-no-provenance":
            seen_bootstrap = True
            require_provenance = False
            i += 1
        elif arg == "--version" and i + 1 < len(args):
            version = args[i + 1]
            i += 2
        elif arg == "--commit" and i + 1 < len(args):
            commit = args[i + 1]
            i += 2
        elif arg.startswith("-"):
            fail(f"unknown option: {arg}")
        else:
            positional.append(arg)
            i += 1

    if seen_expect and seen_bootstrap:
        fail("cannot combine --expect-provenance and --bootstrap-no-provenance")
    if seen_bootstrap and is_ci_env():
        fail("--bootstrap-no-provenance is not accepted under GitHub Actions or CI")

    scope = "npm"
    if version is None:
        if len(positional) in (1, 2):
            version = positional[0]
            if len(positional) == 2:
                scope = positional[1]
        else:
            fail(
                "usage: release-verify-registries.py [--expect-provenance | --bootstrap-no-provenance] --version <version> [--commit <sha>]"
            )
    elif positional:
        if len(positional) == 1:
            scope = positional[0]
        else:
            fail("unexpected positional arguments")

    if not re.match(r"^[0-9]+\.[0-9]+\.[0-9]+$", version):
        fail(f"version ({version}) must be X.Y.Z")
    if commit is not None and not re.fullmatch(r"[0-9a-f]{40}", commit):
        fail(f"commit ({commit}) must be a 40-hex SHA")
    if scope not in ("npm", "all"):
        fail(f"unknown registry scope: {scope} (expected npm or all)")

    trust_root = active_trust_root()
    verify_npm(version, commit, require_provenance, trust_root)


if __name__ == "__main__":
    main()
