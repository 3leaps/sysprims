#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Build the Python binding as an abi3 wheel and source distribution with the
# hash-pinned maturin, install the wheel into a throwaway virtual environment,
# and verify:
#
# 0. the Python manifest version matches the repository VERSION (or retained
#    version when surfaces.python.disposition == "skip" in the release plan),
#    and the wheel is cp310-abi3 carrying LICENSE-MIT, LICENSE-APACHE, NOTICE,
#    and py.typed with zero build-host paths;
# 1. bindings/python/tests pass from outside the source tree under `python -I`
#    with zero skipped tests;
# 2. recurrence/v0 corpus output (34 accept + 38 reject vectors) matches Rust's
#    `crates/sysprims-recurrence` byte-for-byte;
# 3. `maturin sdist` produces `sysprims-<py_version>.tar.gz` deterministically
#    under SOURCE_DATE_EPOCH with zero build-host paths, safe regular/dir-only
#    members, and builds offline in isolation without the monorepo checkout.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd -P)"
python="${PYTHON:-python3}"
work="$(mktemp -d "${TMPDIR:-/tmp}/sysprims-python.XXXXXX")"
trap 'rm -rf "${work}"' EXIT
cd "${root}"

if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
	SOURCE_DATE_EPOCH="$(git log -1 --format=%ct HEAD)"
fi
case "${SOURCE_DATE_EPOCH}" in
'' | *[!0-9]*)
	echo "python-check: invalid SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}" >&2
	exit 1
	;;
esac
export SOURCE_DATE_EPOCH

for license in LICENSE-MIT LICENSE-APACHE NOTICE; do
	cmp -s "${license}" "bindings/python/${license}" || {
		echo "python-check: bindings/python/${license} differs from ${license}" >&2
		exit 1
	}
done

"${python}" -I -c 'import sys; sys.exit(sys.version_info < (3, 10))' || {
	echo "python-check: Python 3.10 or later required" >&2
	exit 1
}

"${python}" -I -m venv "${work}/venv"
venv_python="${work}/venv/bin/python"
[ -x "${venv_python}" ] || venv_python="${work}/venv/Scripts/python.exe"
"${venv_python}" -I -m pip install --quiet --disable-pip-version-check --require-hashes \
	--only-binary :all: -r config/python-build-requirements.txt

resolve_python_version() {
	local tree_root="$1"
	"${venv_python}" -I - "${tree_root}" <<'PY'
import json
import pathlib
import sys

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib  # type: ignore[no-redef]

root = pathlib.Path(sys.argv[1])
repo_version = (root / "VERSION").read_text(encoding="utf-8").strip()
pyproject = tomllib.loads((root / "bindings/python/pyproject.toml").read_text(encoding="utf-8"))
cargo_toml = tomllib.loads((root / "bindings/python/Cargo.toml").read_text(encoding="utf-8"))
py_ver = pyproject.get("project", {}).get("version")
cargo_ver = cargo_toml.get("package", {}).get("version")
if not isinstance(py_ver, str) or not py_ver or py_ver != cargo_ver:
    raise SystemExit(
        f"python-check: bindings/python version mismatch (pyproject={py_ver!r}, Cargo.toml={cargo_ver!r})"
    )

expected = repo_version
plan_path = root / "docs" / "releases" / f"v{repo_version}.json"
if plan_path.is_file():
    plan = json.loads(plan_path.read_text(encoding="utf-8"))
    py_surface = (plan.get("surfaces") or {}).get("python") or {}
    if py_surface.get("disposition") == "skip":
        retained = py_surface.get("retained_version")
        if not isinstance(retained, str) or not retained:
            raise SystemExit("python-check: python skip plan missing retained_version")
        expected = retained

if py_ver != expected:
    raise SystemExit(
        f"python-check: bindings/python version {py_ver!r} != expected {expected!r} (VERSION={repo_version!r})"
    )
print(py_ver)
PY
}

version="$(resolve_python_version "${root}")"

# Self-test the retained-version resolver for disposition == "skip" and mismatch rejection.
mkdir -p "${work}/skip-fixture/bindings/python" "${work}/skip-fixture/docs/releases"
printf '9.9.9\n' >"${work}/skip-fixture/VERSION"
cp bindings/python/pyproject.toml "${work}/skip-fixture/bindings/python/pyproject.toml"
cp bindings/python/Cargo.toml "${work}/skip-fixture/bindings/python/Cargo.toml"
cat >"${work}/skip-fixture/docs/releases/v9.9.9.json" <<EOF
{"surfaces":{"python":{"disposition":"skip","retained_version":"${version}"}}}
EOF
[ "$(resolve_python_version "${work}/skip-fixture")" = "${version}" ] || {
	echo "python-check: retained-version skip fixture did not resolve ${version}" >&2
	exit 1
}
cat >"${work}/skip-fixture/docs/releases/v9.9.9.json" <<'EOF'
{"surfaces":{"python":{"disposition":"publish"}}}
EOF
if resolve_python_version "${work}/skip-fixture" >/dev/null 2>&1; then
	echo "python-check: publish fixture unexpectedly accepted mismatched Python version" >&2
	exit 1
fi

here="$(pwd -P)"
cargo_home="${CARGO_HOME:-${HOME}/.cargo}"
rustup_home="${RUSTUP_HOME:-${HOME}/.rustup}"
home="${HOME}"
rustflags=""
for pair in "${here}=/sysprims" "${cargo_home}=/cargo" "${rustup_home}=/rustup"; do
	dir="${pair%=*}"
	to="${pair##*=}"
	if command -v cygpath >/dev/null 2>&1; then
		rustflags+=" --remap-path-prefix=$(cygpath -w "${dir}")=${to} --remap-path-prefix=$(cygpath -m "${dir}")=${to}"
	else
		rustflags+=" --remap-path-prefix=${dir}=${to}"
	fi
done
if command -v cygpath >/dev/null 2>&1; then
	here="$(cygpath -m "${here}")"
	cargo_home="$(cygpath -m "${cargo_home}")"
	rustup_home="$(cygpath -m "${rustup_home}")"
	home="$(cygpath -m "${home}")"
fi
case "$(rustc -vV | awk '/^host:/{print $2}')" in
*-windows-msvc) rustflags+=" -Clink-arg=/PDBALTPATH:%_PDB%" ;;
esac

MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}" \
	MSYS2_ENV_CONV_EXCL=RUSTFLAGS \
	RUSTFLAGS="${rustflags}" \
	"${venv_python}" -I -m maturin build --release --locked --quiet \
	--manifest-path bindings/python/Cargo.toml --interpreter "${venv_python}" --out "${work}/wheels"

MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}" \
	MSYS2_ENV_CONV_EXCL=RUSTFLAGS \
	RUSTFLAGS="${rustflags}" \
	"${venv_python}" -I -m maturin build --release --locked --quiet \
	--manifest-path bindings/python/Cargo.toml --interpreter "${venv_python}" --out "${work}/wheels-repeat"

wheels=("${work}"/wheels/sysprims-*.whl)
wheels_repeat=("${work}"/wheels-repeat/sysprims-*.whl)
[ "${#wheels[@]}" -eq 1 ] && [ -f "${wheels[0]}" ] || {
	echo "python-check: expected one wheel" >&2
	exit 1
}
[ "${#wheels_repeat[@]}" -eq 1 ] && cmp -s "${wheels[0]}" "${wheels_repeat[0]}" || {
	echo "python-check: repeat maturin wheel build under SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} was not deterministic" >&2
	exit 1
}
case "${wheels[0]##*/}" in
"sysprims-${version}-cp310-abi3-"*) ;;
*)
	echo "python-check: wheel is not sysprims-${version}-cp310-abi3-*: ${wheels[0]##*/}" >&2
	exit 1
	;;
esac

"${venv_python}" -I scripts/host-path-check.py "${wheels[0]}" \
	"${here}" "${cargo_home}" "${rustup_home}" "${home}" || {
	echo "python-check: the wheel embeds a build-host path" >&2
	exit 1
}

"${venv_python}" -I -m pip install --quiet --disable-pip-version-check --no-index --no-deps "${wheels[0]}"
"${venv_python}" -I - "${version}" <<'PY'
import importlib.metadata as md
import sys

want_version = sys.argv[1]
got_version = md.version("sysprims")
if got_version != want_version:
    raise SystemExit(f"python-check: installed version {got_version!r} != {want_version!r}")
files = {str(f).rsplit("/", 1)[-1] for f in md.files("sysprims") or []}
missing = {"LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "py.typed"} - files
if missing:
    raise SystemExit(f"python-check: wheel lacks {sorted(missing)}")
PY

tests="${root}/bindings/python/tests"
corpus="${root}/crates/sysprims-recurrence/tests/recurrence-v0"
sigvec="${root}/testdata/conformance/signal/vectors.json"
sessvec="${root}/testdata/conformance/session/vectors.json"
(
	cd "${work}" &&
		SYSPRIMS_RECURRENCE_V0_DIR="${corpus}" \
			RECURRENCE_NDJSON_OUT="${work}/py.results" \
			SIGNAL_CONFORMANCE_VECTORS="${sigvec}" \
			SIGNAL_NDJSON_OUT="${work}/py.signal.results" \
			SESSION_CONFORMANCE_VECTORS="${sessvec}" \
			SESSION_NDJSON_OUT="${work}/py.session.results" \
			"${venv_python}" -I -m unittest discover -v -s "${tests}" -t "${tests}"
) 2>"${work}/unittest.log" || {
	cat "${work}/unittest.log" >&2
	exit 1
}
if grep -q 'skipped' "${work}/unittest.log"; then
	cat "${work}/unittest.log" >&2
	echo "python-check: the Python tests must not skip" >&2
	exit 1
fi

RECURRENCE_NDJSON_OUT="${work}/rust.results" \
	cargo test --quiet --locked -p sysprims-recurrence --test corpus accept_vectors >/dev/null
if [ ! -s "${work}/py.results" ] || ! cmp -s "${work}/rust.results" "${work}/py.results"; then
	echo "python-check: recurrence corpus results differ (rust vs python):" >&2
	diff -u "${work}/rust.results" "${work}/py.results" | head -30 >&2 || true
	exit 1
fi
echo "python-check: recurrence corpus agrees with Rust ($(wc -l <"${work}/py.results" | tr -d ' ') vectors)" >&2

cargo run --quiet --locked -p sysprims-signal --example signal-diff >"${work}/rust.signal.results"
if [ ! -s "${work}/py.signal.results" ] || ! cmp -s "${work}/rust.signal.results" "${work}/py.signal.results"; then
	echo "python-check: signal conformance results differ (rust vs python):" >&2
	diff -u "${work}/rust.signal.results" "${work}/py.signal.results" | head -30 >&2 || true
	exit 1
fi
echo "python-check: signal conformance agrees with Rust ($(wc -l <"${work}/py.signal.results" | tr -d ' ') vectors)" >&2

cargo run --quiet --locked -p sysprims-session --example session-diff >"${work}/rust.session.results"
if [ ! -s "${work}/py.session.results" ] || ! cmp -s "${work}/rust.session.results" "${work}/py.session.results"; then
	echo "python-check: session conformance results differ (rust vs python):" >&2
	diff -u "${work}/rust.session.results" "${work}/py.session.results" | head -30 >&2 || true
	exit 1
fi
echo "python-check: session conformance agrees with Rust ($(wc -l <"${work}/py.session.results" | tr -d ' ') vectors)" >&2

# Build and verify source distribution (sdist) determinism, isolation + offline buildability.
"${venv_python}" -I -m maturin sdist \
	--manifest-path bindings/python/Cargo.toml --out "${work}/sdist" >/dev/null
"${venv_python}" -I -m maturin sdist \
	--manifest-path bindings/python/Cargo.toml --out "${work}/sdist-repeat" >/dev/null
sdist="${work}/sdist/sysprims-${version}.tar.gz"
sdist_repeat="${work}/sdist-repeat/sysprims-${version}.tar.gz"
[ -s "${sdist}" ] || {
	echo "python-check: expected sdist ${sdist}" >&2
	exit 1
}
cmp -s "${sdist}" "${sdist_repeat}" || {
	echo "python-check: repeat maturin sdist build under SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} was not deterministic" >&2
	exit 1
}
"${venv_python}" -I scripts/host-path-check.py "${sdist}" \
	"${here}" "${cargo_home}" "${rustup_home}" "${home}" || {
	echo "python-check: the sdist embeds a build-host path" >&2
	exit 1
}
mkdir -p "${work}/sdist-src"
"${venv_python}" -I - "${sdist}" "${work}/sdist-src" "${version}" <<'PY'
import os
import pathlib
import re
import sys
import tarfile

archive = pathlib.Path(sys.argv[1])
dest = pathlib.Path(sys.argv[2]).resolve()
want_version = sys.argv[3]
expected_root = f"sysprims-{want_version}"
seen_folded: set[str] = set()

with tarfile.open(archive, "r:gz") as tf:
    members = tf.getmembers()
    if not any(m.name == f"{expected_root}/PKG-INFO" and m.isfile() for m in members):
        raise SystemExit(f"python-check: sdist is missing {expected_root}/PKG-INFO")
    for member in members:
        raw = member.name
        is_dir = member.isdir()
        if (
            member.type not in (tarfile.REGTYPE, tarfile.AREGTYPE, tarfile.DIRTYPE)
            or member.issym()
            or member.islnk()
            or member.isfifo()
            or member.ischr()
            or member.isblk()
            or member.isdev()
        ):
            raise SystemExit(f"python-check: non-regular/link sdist entry {raw!r}")
        if (
            not raw
            or "\\" in raw
            or raw.startswith("/")
            or (not is_dir and raw.endswith("/"))
            or any(ord(ch) < 32 for ch in raw)
            or re.match(r"^[A-Za-z]:", raw)
        ):
            raise SystemExit(f"python-check: unsafe sdist path {raw!r}")
        trimmed = raw.rstrip("/") if is_dir else raw
        parts = trimmed.split("/")
        if (
            not trimmed
            or any(p in ("", ".", "..") or re.match(r"^[A-Za-z]:", p) for p in parts)
            or parts[0] != expected_root
            or (not is_dir and len(parts) < 2)
        ):
            raise SystemExit(f"python-check: unexpected sdist member path {raw!r}")
        folded = "/".join(p.casefold() for p in parts)
        if folded in seen_folded:
            raise SystemExit(f"python-check: duplicate/colliding sdist entry {raw!r}")
        seen_folded.add(folded)
        target = (dest / pathlib.PurePosixPath(*parts)).resolve()
        if dest not in target.parents and target != dest:
            raise SystemExit(f"python-check: sdist path escapes destination: {raw!r}")
        if is_dir:
            target.mkdir(parents=True, exist_ok=True)
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            reader = tf.extractfile(member)
            if reader is None:
                raise SystemExit(f"python-check: unreadable sdist member {raw!r}")
            with open(target, "xb") as out_f:
                out_f.write(reader.read())
            if member.mode & 0o111:
                os.chmod(target, 0o755)
PY
sdist_dir="${work}/sdist-src/sysprims-${version}"
[ -f "${sdist_dir}/pyproject.toml" ] || {
	echo "python-check: missing extracted ${sdist_dir}/pyproject.toml" >&2
	exit 1
}
(
	cd "${sdist_dir}" &&
		MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}" \
			CARGO_NET_OFFLINE=true \
			"${venv_python}" -I -m maturin build --release --locked --offline --quiet \
			--interpreter "${venv_python}" --out "${work}/sdist-wheels"
)
sdist_wheels=("${work}"/sdist-wheels/sysprims-*.whl)
[ "${#sdist_wheels[@]}" -eq 1 ] && [ -f "${sdist_wheels[0]}" ] || {
	echo "python-check: offline wheel build from sdist failed" >&2
	exit 1
}
echo "[ok] python-check"
