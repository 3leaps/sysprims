#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Build the Python binding as an abi3 wheel and source distribution with the
# hash-pinned maturin, install the wheel into a throwaway virtual environment,
# and verify:
#
# 0. the wheel is cp310-abi3 and carries LICENSE-MIT, LICENSE-APACHE, NOTICE,
#    and py.typed with zero build-host paths;
# 1. bindings/python/tests pass from outside the source tree under `python -I`
#    with zero skipped tests;
# 2. recurrence/v0 corpus output (34 accept + 38 reject vectors) matches Rust's
#    `crates/sysprims-recurrence` byte-for-byte;
# 3. `maturin sdist` produces `sysprims-<version>.tar.gz` with zero build-host
#    paths and builds offline in isolation without the monorepo checkout.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd -P)"
python="${PYTHON:-python3}"
work="$(mktemp -d "${TMPDIR:-/tmp}/sysprims-python.XXXXXX")"
trap 'rm -rf "${work}"' EXIT
cd "${root}"

version="$(tr -d ' \t\r\n' <VERSION)"

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

wheels=("${work}"/wheels/sysprims-*.whl)
[ "${#wheels[@]}" -eq 1 ] && [ -f "${wheels[0]}" ] || {
	echo "python-check: expected one wheel" >&2
	exit 1
}
case "${wheels[0]##*/}" in
*-cp310-abi3-*) ;;
*)
	echo "python-check: wheel is not abi3 for 3.10: ${wheels[0]##*/}" >&2
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
(
	cd "${work}" &&
		SYSPRIMS_RECURRENCE_V0_DIR="${corpus}" \
			RECURRENCE_NDJSON_OUT="${work}/py.results" \
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

# Build and verify source distribution (sdist) isolation + offline buildability.
"${venv_python}" -I -m maturin sdist \
	--manifest-path bindings/python/Cargo.toml --out "${work}/sdist" >/dev/null
sdist="${work}/sdist/sysprims-${version}.tar.gz"
[ -s "${sdist}" ] || {
	echo "python-check: expected sdist ${sdist}" >&2
	exit 1
}
"${venv_python}" -I scripts/host-path-check.py "${sdist}" \
	"${here}" "${cargo_home}" "${rustup_home}" "${home}" || {
	echo "python-check: the sdist embeds a build-host path" >&2
	exit 1
}
mkdir -p "${work}/sdist-src"
"${venv_python}" -I - "${sdist}" "${work}/sdist-src" <<'PY'
import pathlib, sys, tarfile

archive = pathlib.Path(sys.argv[1])
dest = pathlib.Path(sys.argv[2]).resolve()
with tarfile.open(archive, "r:gz") as tf:
    names = tf.getnames()
    if not any(n.endswith("/PKG-INFO") for n in names):
        raise SystemExit("python-check: sdist is missing PKG-INFO")
    for member in tf.getmembers():
        target = (dest / member.name).resolve()
        if dest not in target.parents and target != dest:
            raise SystemExit(f"python-check: unsafe sdist path {member.name!r}")
    tf.extractall(dest, filter="data")
PY
sdist_dir="${work}/sdist-src/sysprims-${version}"
[ -f "${sdist_dir}/pyproject.toml" ] || sdist_dir="$(find "${work}/sdist-src" -mindepth 1 -maxdepth 1 -type d | head -1)"
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
