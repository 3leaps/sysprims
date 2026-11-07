#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Download the published GitHub Release for SYSPRIMS_RELEASE_TAG, verify every
# signed asset, digest, anchor, public key, signature, and embedded package
# metadata against the tagged commit, and stage the verified directory (or the
# subset requested by --pypi-only / --npm-only).
#
# Usage: release-stage-published.sh [--pypi-only | --npm-only] [dist-dir]
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
# shellcheck source=scripts/release-tag-common.sh
source "$root/scripts/release-tag-common.sh"
# shellcheck source=scripts/release-decernor.sh
source "$root/scripts/release-decernor.sh"
die() {
	echo "error: $*" >&2
	exit 1
}

mode="all"
case "${1:-}" in
--pypi-only)
	mode="pypi"
	shift
	;;
--npm-only)
	mode="npm"
	shift
	;;
--*)
	die 'usage: release-stage-published.sh [--pypi-only | --npm-only] [dist-dir]'
	;;
esac
[[ $# -le 1 ]] || die 'usage: release-stage-published.sh [--pypi-only | --npm-only] [dist-dir]'

tag_version
tag="$SYSPRIMS_RELEASE_TAG"
default_out="$root/dist/published"
if [[ "$mode" == pypi ]]; then
	default_out="$root/dist/published-pypi"
elif [[ "$mode" == npm ]]; then
	default_out="$root/dist/published-npm"
fi
out="${1:-$default_out}"
anchor="${out%/}.anchor"
[[ ! -e "$out" && ! -L "$out" ]] || die "$out already exists; run make release-clean first"
[[ ! -e "$anchor" && ! -L "$anchor" ]] || die "$anchor already exists; run make release-clean first"

resolve_release_decernor
RELEASE_DECERNOR_BIN="$RELEASE_DECERNOR_BIN" "$root/scripts/verify-pinned-tag.sh" --published >/dev/null
tag_object="$(git rev-parse --verify "refs/tags/$tag")"
commit="$(git rev-parse "${tag_object}^{commit}")"
version="$(git cat-file blob "$commit:VERSION" | tr -d ' \t\r\n')"
[[ "v$version" == "$tag" ]] || die "tagged VERSION ($version) does not match $tag"

work="$(mktemp -d "${TMPDIR:-/tmp}/sysprims-published.XXXXXX")"
trap 'rm -rf "$work"' EXIT
chmod 700 "$work"
mkdir -m 700 "$work/assets"

if [[ -n "${SYSPRIMS_PUBLISHED_RELEASE_METADATA_FILE:-}" ]]; then
	cp "$SYSPRIMS_PUBLISHED_RELEASE_METADATA_FILE" "$work/release.json"
else
	gh api "repos/3leaps/sysprims/releases/tags/$tag" >"$work/release.json"
fi

if [[ -n "${SYSPRIMS_REMOTE_ASSET_DIR:-}" ]]; then
	cp "$SYSPRIMS_REMOTE_ASSET_DIR"/* "$work/assets/"
else
	gh release download "$tag" --repo 3leaps/sysprims --dir "$work/assets"
fi

node --input-type=module - "$root" "$work/assets" "$work/release.json" "$version" "$commit" "$mode" >"$work/surfaces.json" <<'EOF'
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { lstatSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import {
  NPM_TARBALL_PACKAGES,
  PYTHON_WHEEL_TAGS,
  assertInventory,
  inventoryForCommit,
  surfacesAtCommit,
  verifyChecksums,
  verifyTaggedAssets,
} from "./scripts/release-integrity.mjs";

const [, , root, assetsDir, releaseJsonPath, version, commit, mode] = process.argv;
const tag = `v${version}`;
const fail = (msg) => { console.error(`error: ${msg}`); process.exit(1); };
const sha256 = (p) => createHash("sha256").update(readFileSync(p)).digest("hex");

const release = JSON.parse(readFileSync(releaseJsonPath, "utf8"));
if (release.tag_name !== tag) fail(`published release tag_name (${release.tag_name}) does not match ${tag}`);
if (release.target_commitish !== commit) fail(`published release target_commitish (${release.target_commitish}) does not match tagged commit ${commit}`);
if (release.draft !== false) fail(`release ${tag} is still a draft; run make release-publish first`);
if (release.immutable !== true) fail(`published release ${tag} is not immutable (immutable=${release.immutable})`);

for (const entry of readdirSync(assetsDir)) {
  const st = lstatSync(join(assetsDir, entry));
  if (st.isSymbolicLink() || !st.isFile()) fail(`downloaded asset is not a regular file: ${entry}`);
}

const surfaces = surfacesAtCommit(root, commit, version);
const { base, signed } = inventoryForCommit(root, commit, version);
assertInventory(assetsDir, signed, "published release");

const byName = new Map((release.assets ?? []).map((a) => [a.name, a]));
if (byName.size !== signed.length) fail("published release asset count mismatch");
for (const name of signed) {
  const asset = byName.get(name);
  if (!asset) fail(`published release metadata missing asset: ${name}`);
  const digest = asset.digest?.replace(/^sha256:/, "");
  if (!/^[0-9a-f]{64}$/.test(digest ?? "")) fail(`published release asset has no SHA-256 digest: ${name}`);
  if (digest !== sha256(join(assetsDir, name))) fail(`published release digest mismatch for ${name}`);
}

verifyTaggedAssets(root, assetsDir, version);
for (const [blobPath, assetName] of [
  ["docs/security/release-signing-keys.asc", "sysprims-release-signing-key.asc"],
  ["docs/security/sysprims-minisign.pub", "sysprims-minisign.pub"],
]) {
  const blob = spawnSync("git", ["cat-file", "blob", `${commit}:${blobPath}`], { cwd: root }).stdout;
  if (!blob?.length) fail(`committed ${blobPath} is empty in ${commit}`);
  if (!readFileSync(join(assetsDir, assetName)).equals(blob)) {
    fail(`${assetName} in the published release differs from ${commit}:${blobPath}`);
  }
}
verifyChecksums(assetsDir, base);

const pyPublish = surfaces?.python?.disposition === "publish";
const tsPublish = surfaces?.python !== undefined && surfaces?.typescript?.disposition === "publish";
if (mode === "pypi" && !pyPublish) fail(`release plan at ${commit} does not publish the Python surface`);
if (mode === "npm" && !tsPublish) fail(`release plan at ${commit} does not attach npm tarballs`);

const pypiFiles = pyPublish
  ? [...PYTHON_WHEEL_TAGS.map((w) => `sysprims-${version}-${w}.whl`), `sysprims-${version}.tar.gz`].sort()
  : [];
const npmPackages = tsPublish
  ? NPM_TARBALL_PACKAGES.map(([prefix, pkgName]) => ({ file: `${prefix}-${version}.tgz`, name: pkgName }))
  : [];

console.log(JSON.stringify({ pyPublish, tsPublish, pypiFiles, npmPackages, base }));
EOF

"$root/scripts/verify-public-keys.sh" "$work/assets" >/dev/null
"$root/scripts/verify-signatures.sh" "$work/assets" >/dev/null

python3 -I - "$work/assets" "$version" "$work/surfaces.json" <<'PY'
import json
import pathlib
import re
import sys
import tarfile
import zipfile

assets = pathlib.Path(sys.argv[1])
version = sys.argv[2]
meta = json.loads(pathlib.Path(sys.argv[3]).read_text(encoding="utf-8"))


def parse_rfc822_headers(text: str) -> dict[str, str]:
    headers: dict[str, str] = {}
    for line in text.splitlines():
        if not line.strip():
            break
        key, sep, val = line.partition(": ")
        if sep and key not in headers:
            headers[key] = val
    return headers


def validate_member_path(
    raw: str,
    allowed_roots: set[str],
    seen_folded: set[str],
    archive_label: str,
    *,
    is_dir: bool = False,
) -> None:
    if (
        not raw
        or "\\" in raw
        or raw.startswith("/")
        or (not is_dir and raw.endswith("/"))
        or any(ord(ch) < 32 for ch in raw)
        or re.match(r"^[A-Za-z]:", raw)
    ):
        sys.exit(f"error: unsafe archive entry path in {archive_label}: {raw!r}")
    trimmed = raw.rstrip("/") if is_dir else raw
    parts = trimmed.split("/")
    if (
        not trimmed
        or any(p in ("", ".", "..") or re.match(r"^[A-Za-z]:", p) for p in parts)
        or parts[0] not in allowed_roots
        or (not is_dir and len(parts) < 2)
    ):
        sys.exit(f"error: unexpected archive entry path in {archive_label}: {raw!r}")
    folded = "/".join(p.casefold() for p in parts)
    if folded in seen_folded:
        sys.exit(f"error: duplicate or case-colliding archive entry in {archive_label}: {raw!r}")
    seen_folded.add(folded)


if meta["pyPublish"]:
    wheels = [f for f in meta["pypiFiles"] if f.endswith(".whl")]
    if len(wheels) != 5:
        sys.exit(f"error: expected 5 wheels for {version}, found {len(wheels)}")
    wheel_roots = {
        "sysprims",
        "sysprims.libs",
        f"sysprims-{version}.dist-info",
        f"sysprims-{version}.data",
    }
    meta_member = f"sysprims-{version}.dist-info/METADATA"
    for wheel_name in wheels:
        wheel_path = assets / wheel_name
        seen_wheel_folded: set[str] = set()
        with zipfile.ZipFile(wheel_path) as zf:
            for info in zf.infolist():
                if info.is_dir():
                    sys.exit(f"error: directory entry in wheel {wheel_name}: {info.filename!r}")
                unix_mode = info.external_attr >> 16
                if unix_mode and (unix_mode & 0o170000) not in (0, 0o100000):
                    sys.exit(f"error: non-regular or symlink entry in wheel {wheel_name}: {info.filename!r}")
                validate_member_path(info.filename, wheel_roots, seen_wheel_folded, wheel_name)
            if meta_member not in zf.namelist():
                sys.exit(f"error: {wheel_name} missing {meta_member}")
            headers = parse_rfc822_headers(zf.read(meta_member).decode("utf-8"))
        if headers.get("Name") != "sysprims" or headers.get("Version") != version:
            sys.exit(f"error: {wheel_name} metadata mismatch: {headers}")

    sdist_name = f"sysprims-{version}.tar.gz"
    sdist_roots = {f"sysprims-{version}"}
    pkg_info_member = f"sysprims-{version}/PKG-INFO"
    seen_sdist_folded: set[str] = set()
    with tarfile.open(assets / sdist_name, "r:gz") as tf:
        for member in tf.getmembers():
            if (
                member.type not in (tarfile.REGTYPE, tarfile.AREGTYPE, tarfile.DIRTYPE)
                or member.issym()
                or member.islnk()
                or member.isfifo()
                or member.ischr()
                or member.isblk()
                or member.isdev()
            ):
                sys.exit(f"error: non-regular entry in {sdist_name}: {member.name!r}")
            validate_member_path(
                member.name,
                sdist_roots,
                seen_sdist_folded,
                sdist_name,
                is_dir=member.isdir(),
            )
        extracted = tf.extractfile(pkg_info_member)
        if extracted is None:
            sys.exit(f"error: {sdist_name} missing {pkg_info_member}")
        headers = parse_rfc822_headers(extracted.read().decode("utf-8"))
    if headers.get("Name") != "sysprims" or headers.get("Version") != version:
        sys.exit(f"error: {sdist_name} PKG-INFO mismatch: {headers}")

if meta["tsPublish"]:
    if len(meta["npmPackages"]) != 8:
        sys.exit(f"error: expected 8 npm tarballs for {version}, found {len(meta['npmPackages'])}")
    npm_roots = {"package"}
    for entry in meta["npmPackages"]:
        tgz_path = assets / entry["file"]
        seen_tgz_folded: set[str] = set()
        with tarfile.open(tgz_path, "r:gz") as tf:
            for member in tf.getmembers():
                if (
                    member.type not in (tarfile.REGTYPE, tarfile.AREGTYPE, tarfile.DIRTYPE)
                    or member.issym()
                    or member.islnk()
                    or member.isfifo()
                    or member.ischr()
                    or member.isblk()
                    or member.isdev()
                ):
                    sys.exit(f"error: non-regular entry in {tgz_path.name}: {member.name!r}")
                validate_member_path(
                    member.name,
                    npm_roots,
                    seen_tgz_folded,
                    tgz_path.name,
                    is_dir=member.isdir(),
                )
            extracted = tf.extractfile("package/package.json")
            if extracted is None:
                sys.exit(f"error: {tgz_path.name} missing package/package.json")
            pkg = json.loads(extracted.read().decode("utf-8"))
        repo_url = (pkg.get("repository") or {}).get("url")
        if (
            pkg.get("name") != entry["name"]
            or pkg.get("version") != version
            or repo_url != "git+https://github.com/3leaps/sysprims.git"
        ):
            sys.exit(f"error: {tgz_path.name} package.json mismatch: {pkg}")
PY

stage_src="$work/assets"
anchor_list="base"
if [[ "$mode" == pypi ]]; then
	mkdir -m 700 "$work/subset"
	anchor_list="pypiFiles"
	while IFS= read -r file; do
		cp -p "$work/assets/$file" "$work/subset/$file"
	done < <(python3 -I -c 'import json,sys; print("\n".join(json.load(open(sys.argv[1]))["pypiFiles"]))' "$work/surfaces.json")
	stage_src="$work/subset"
elif [[ "$mode" == npm ]]; then
	mkdir -m 700 "$work/subset"
	anchor_list="npmFiles"
	while IFS= read -r file; do
		cp -p "$work/assets/$file" "$work/subset/$file"
	done < <(python3 -I -c 'import json,sys; print("\n".join(sorted(e["file"] for e in json.load(open(sys.argv[1]))["npmPackages"])))' "$work/surfaces.json")
	stage_src="$work/subset"
fi

{
	printf 'tag=%s\nobject=%s\ncommit=%s\n' "$tag" "$tag_object" "$commit"
	python3 -I - "$stage_src" "$work/surfaces.json" "$anchor_list" <<'PY'
import hashlib
import json
import pathlib
import sys

stage_dir = pathlib.Path(sys.argv[1])
meta = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
which = sys.argv[3]
if which == "pypiFiles":
    names = sorted(meta["pypiFiles"])
elif which == "npmFiles":
    names = sorted(e["file"] for e in meta["npmPackages"])
else:
    names = sorted(meta["base"])
for name in names:
    digest = hashlib.sha256((stage_dir / name).read_bytes()).hexdigest()
    print(f"file={digest} {name}")
PY
} >"$work/anchor"

mkdir -p "$(dirname "$out")"
mv "$stage_src" "$out"
"$root/scripts/write-exclusive.py" "$work/anchor" "$anchor"
echo "[ok] verified published release $tag staged in $out ($anchor)"
