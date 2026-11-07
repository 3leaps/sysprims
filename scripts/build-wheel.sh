#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Build the Python binding's release wheel (or sdist) for one target, verify it
# carries no build-host paths, and run the binding test suite and recurrence
# corpus against native wheels.
#
#   build-wheel.sh <target|sdist> <out-dir>
#
# Supported targets:
#   - x86_64-unknown-linux-gnu   (inside digest-pinned manylinux2014 container)
#   - aarch64-unknown-linux-gnu  (inside digest-pinned manylinux2014 container)
#   - aarch64-apple-darwin
#   - x86_64-apple-darwin        (native or cross-compiled from aarch64-apple-darwin)
#   - x86_64-pc-windows-msvc
#   - sdist                      (sysprims-<version>.tar.gz)
set -euo pipefail

target="${1:?target required}"
out="${2:?output directory required}"
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "${root}"

toolchain="$(awk '
	/^\[rust\]/ { in_rust = 1; next }
	/^\[/ { in_rust = 0 }
	in_rust && $1 == "msrv" {
		gsub(/["\r\n]/, "", $3)
		print $3
		exit
	}
' config/ci/pins.toml)"
[ -n "${toolchain}" ] || {
	echo "build-wheel: missing rust.msrv in config/ci/pins.toml" >&2
	exit 1
}
version="$(tr -d ' \t\r\n' <VERSION)"
python="${PYTHON:-python3}"
work="$(mktemp -d "${TMPDIR:-/tmp}/sysprims-wheel.XXXXXX")"
cleanup_wheel() {
	local rc=$?
	rm -rf "${work}"
	exit "${rc}"
}
[ "${BUILD_WHEEL_KEEP:-}" = 1 ] || trap cleanup_wheel EXIT

for license in LICENSE-MIT LICENSE-APACHE NOTICE; do
	cmp -s "${license}" "bindings/python/${license}" || {
		echo "build-wheel: bindings/python/${license} differs from ${license}" >&2
		exit 1
	}
done

platform=native
compat=()
target_args=()
can_exec_wheel=1

case "${target}" in
x86_64-unknown-linux-gnu | aarch64-unknown-linux-gnu)
	platform=linux
	[ -f /etc/os-release ] && grep -q 'CentOS Linux' /etc/os-release && [ "$(ldd --version | head -1 | awk '{print $NF}')" = 2.17 ] || {
		echo "build-wheel: Linux wheels build only inside the manylinux2014 image" >&2
		exit 1
	}
	[ "$(uname -m)" = "${target%%-*}" ] || {
		echo "build-wheel: host is not ${target}" >&2
		exit 1
	}
	name="rustup-init-1.29.1-${target}"
	want_sha="$(awk -v n="${name}" '$2 == n { print $1 }' config/ci-tools.sha256)"
	[ -n "${want_sha}" ] || {
		echo "build-wheel: no pin for ${name}" >&2
		exit 1
	}
	curl --proto '=https' --tlsv1.2 --fail --silent --show-error --retry 3 --output "${work}/rustup-init" \
		"https://static.rust-lang.org/rustup/archive/1.29.1/${target}/rustup-init"
	got_sha="$(sha256sum "${work}/rustup-init" | cut -c1-64)"
	[ "${got_sha}" = "${want_sha}" ] || {
		echo "build-wheel: rustup-init checksum ${got_sha} != ${want_sha}" >&2
		exit 1
	}
	chmod +x "${work}/rustup-init"
	"${work}/rustup-init" -y -q --no-modify-path --profile minimal --default-toolchain "${toolchain}" >/dev/null
	export PATH="${HOME}/.cargo/bin:${PATH}"
	python=/opt/python/cp310-cp310/bin/python
	compat=(--compatibility manylinux2014)
	;;
aarch64-apple-darwin | x86_64-pc-windows-msvc)
	platform=native
	;;
x86_64-apple-darwin)
	host_triple="$(rustc -vV | awk '/^host:/{print $2}')"
	target_args=(--target x86_64-apple-darwin)
	if [ "${host_triple}" != "x86_64-apple-darwin" ]; then
		platform=cross-darwin
		can_exec_wheel=0
	fi
	;;
sdist)
	platform=sdist
	;;
*)
	echo "build-wheel: unsupported target ${target}" >&2
	exit 2
	;;
esac

rustc --version | grep -q "^rustc ${toolchain} " || {
	echo "build-wheel: rustc is not ${toolchain} (got $(rustc --version))" >&2
	exit 1
}
if [ "${platform}" = "native" ] || [ "${platform}" = "linux" ]; then
	[ "$(rustc -vV | awk '/^host:/{print $2}')" = "${target}" ] || {
		echo "build-wheel: rustc host is not ${target}" >&2
		exit 1
	}
fi

"${python}" -I -m venv "${work}/build"
build_python="${work}/build/bin/python"
[ -x "${build_python}" ] || build_python="${work}/build/Scripts/python.exe"
"${build_python}" -I -m pip install --quiet --disable-pip-version-check --require-hashes \
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

if [ "${target}" = "sdist" ]; then
	"${build_python}" -I -m maturin sdist \
		--manifest-path bindings/python/Cargo.toml --out "${work}/sdist" >/dev/null
	artifact="sysprims-${version}.tar.gz"
	[ -s "${work}/sdist/${artifact}" ] || {
		echo "build-wheel: missing expected sdist ${artifact}" >&2
		exit 1
	}
	"${build_python}" -I scripts/host-path-check.py "${work}/sdist/${artifact}" \
		"${here}" "${cargo_home}" "${rustup_home}" "${home}" || {
		echo "build-wheel: the sdist embeds a build-host path" >&2
		exit 1
	}
	mkdir -p "${work}/sdist-src"
	"${build_python}" -I - "${work}/sdist/${artifact}" "${work}/sdist-src" "${version}" <<'PY'
import email, pathlib, sys, tarfile

archive = pathlib.Path(sys.argv[1])
dest = pathlib.Path(sys.argv[2]).resolve()
want_version = sys.argv[3]
with tarfile.open(archive, "r:gz") as tf:
    pkg_infos = [m for m in tf.getmembers() if m.name.endswith("/PKG-INFO")]
    if len(pkg_infos) != 1:
        raise SystemExit(f"build-wheel: expected 1 PKG-INFO in sdist, got {len(pkg_infos)}")
    raw = tf.extractfile(pkg_infos[0]).read().decode("utf-8")
    msg = email.message_from_string(raw)
    if msg.get("Name") != "sysprims" or msg.get("Version") != want_version:
        raise SystemExit(f"build-wheel: sdist PKG-INFO mismatch: {msg.get('Name')} {msg.get('Version')}")
    for member in tf.getmembers():
        target = (dest / member.name).resolve()
        if dest not in target.parents and target != dest:
            raise SystemExit(f"build-wheel: unsafe sdist path {member.name!r}")
    if hasattr(tarfile, "data_filter"):
        tf.extractall(dest, filter="data")
    else:
        tf.extractall(dest)
PY
	sdist_dir="${work}/sdist-src/sysprims-${version}"
	[ -f "${sdist_dir}/pyproject.toml" ] || sdist_dir="$(find "${work}/sdist-src" -mindepth 1 -maxdepth 1 -type d | head -1)"
	(
		cd "${sdist_dir}" &&
			MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}" \
				CARGO_NET_OFFLINE=true \
				"${build_python}" -I -m maturin build --release --locked --offline --quiet \
				--interpreter "${build_python}" --out "${work}/sdist-wheels"
	)
	mkdir -p "${out}"
	cp "${work}/sdist/${artifact}" "${out}/"
	[ -s "${out}/${artifact}" ] || {
		echo "build-wheel: missing output ${out}/${artifact}" >&2
		exit 1
	}
	echo "[ok] ${artifact} (sdist)"
	exit 0
fi

MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}" \
	MSYS2_ENV_CONV_EXCL=RUSTFLAGS \
	RUSTFLAGS="${rustflags}" \
	"${build_python}" -I -m maturin build --release --locked --quiet \
	${compat[@]+"${compat[@]}"} \
	${target_args[@]+"${target_args[@]}"} \
	--manifest-path bindings/python/Cargo.toml --interpreter "${build_python}" --out "${work}/wheels"

wheels=("${work}"/wheels/sysprims-*.whl)
[ "${#wheels[@]}" -eq 1 ] && [ -f "${wheels[0]}" ] || {
	echo "build-wheel: expected one wheel" >&2
	exit 1
}
wheel="${wheels[0]##*/}"
case "${target}" in
x86_64-unknown-linux-gnu) want="sysprims-${version}-cp310-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl" ;;
aarch64-unknown-linux-gnu) want="sysprims-${version}-cp310-abi3-manylinux_2_17_aarch64.manylinux2014_aarch64.whl" ;;
aarch64-apple-darwin) want="sysprims-${version}-cp310-abi3-macosx_11_0_arm64.whl" ;;
x86_64-apple-darwin) want="sysprims-${version}-cp310-abi3-macosx_11_0_x86_64.whl" ;;
x86_64-pc-windows-msvc) want="sysprims-${version}-cp310-abi3-win_amd64.whl" ;;
esac
[ "${wheel}" = "${want}" ] || {
	echo "build-wheel: built ${wheel}, expected ${want}" >&2
	exit 1
}

"${build_python}" -I scripts/host-path-check.py "${work}/wheels/${wheel}" \
	"${here}" "${cargo_home}" "${rustup_home}" "${home}" || {
	echo "build-wheel: the wheel embeds a build-host path" >&2
	exit 1
}

# Verify wheel archive structure, METADATA, licenses, and py.typed unconditionally.
"${build_python}" -I - "${work}/wheels/${wheel}" "${version}" <<'PY'
import email, sys, zipfile

wheel_path, want_version = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(wheel_path) as zf:
    names = zf.namelist()
    meta_names = [n for n in names if n.endswith(".dist-info/METADATA")]
    if len(meta_names) != 1:
        raise SystemExit(f"build-wheel: expected 1 METADATA in wheel, got {meta_names}")
    msg = email.message_from_string(zf.read(meta_names[0]).decode("utf-8"))
    if msg.get("Name") != "sysprims" or msg.get("Version") != want_version:
        raise SystemExit(f"build-wheel: METADATA mismatch: {msg.get('Name')} {msg.get('Version')}")
    basenames = {n.rsplit("/", 1)[-1] for n in names}
    missing = {"LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "py.typed"} - basenames
    if missing:
        raise SystemExit(f"build-wheel: wheel lacks {sorted(missing)}")
PY

if [ "${can_exec_wheel}" -eq 1 ]; then
	"${python}" -I -m venv "${work}/test"
	test_python="${work}/test/bin/python"
	[ -x "${test_python}" ] || test_python="${work}/test/Scripts/python.exe"
	"${test_python}" -I -m pip install --quiet --disable-pip-version-check --no-index --no-deps "${work}/wheels/${wheel}"
	tests="${root}/bindings/python/tests"
	(
		cd "${work}" &&
			SYSPRIMS_RECURRENCE_V0_DIR="${root}/crates/sysprims-recurrence/tests/recurrence-v0" \
				"${test_python}" -I -m unittest discover -s "${tests}" -t "${tests}"
	) 2>"${work}/unittest.log" || {
		cat "${work}/unittest.log" >&2
		exit 1
	}
	! grep -q 'skipped' "${work}/unittest.log" || {
		echo "build-wheel: tests must not skip" >&2
		exit 1
	}
fi

mkdir -p "${out}"
cp "${work}/wheels/${wheel}" "${out}/"
[ -s "${out}/${wheel}" ] || {
	echo "build-wheel: missing output ${out}/${wheel}" >&2
	exit 1
}
echo "[ok] ${wheel} (${platform})"
