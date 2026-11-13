#!/usr/bin/env bash
# Every action and reusable workflow is pinned by full commit SHA, every Docker
# action by image digest, and each pinned line carries a comment naming the
# version or channel the pin was taken from. `uses` values are read from the
# parsed workflow (scripts/workflow_yaml.py), so quoting and flow style do not
# hide one. Runner images and Rust toolchain versions are governed by
# config/ci/pins.toml and scripts/ci-pins.py.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
python3 -B -P -E - "$root/scripts" "$root/.github/workflows" "$root" <<'PY'
import importlib.util
import pathlib
import re
import sys

sys.path.insert(0, sys.argv[1])
import workflow_yaml as wy  # noqa: E402

spec = importlib.util.spec_from_file_location(
    'ci_pins', pathlib.Path(sys.argv[1]) / 'ci-pins.py'
)
ci_pins = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci_pins)

SHA_REF = re.compile(r'[\w.-]+/[\w./-]+@[0-9a-f]{40}')
DOCKER_REF = re.compile(r'docker://[\w./:-]+@sha256:[0-9a-f]{64}')
COMMENT = re.compile(r'#\s*(v?\d[\w.-]*|stable|beta|nightly)\s*$')


def uses_values(doc):
    for job_id, job in wy.jobs(doc).items():
        if not isinstance(job, dict):
            raise wy.WorkflowError(f'job {job_id!r} is not a mapping')
        if 'uses' in job:
            yield f'job {job_id}', job['uses']
        for index, step in enumerate(wy.steps(job)):
            if not isinstance(step, dict):
                raise wy.WorkflowError(f'job {job_id} step {index} is not a mapping')
            if 'uses' in step:
                yield f'job {job_id} step {index}', step['uses']


def check(name, text):
    try:
        doc = wy.load_text(text, name)
        found = list(uses_values(doc))
    except wy.WorkflowError as error:
        return [str(error)]
    errors = []
    lines = text.split('\n')
    for where, value in found:
        ref = str(value)
        if ref.startswith('./'):
            continue
        if ref.startswith('docker://'):
            if not DOCKER_REF.fullmatch(ref):
                errors.append(f'{name}: {where}: Docker action not pinned by digest: {ref}')
            continue
        if not SHA_REF.fullmatch(ref):
            errors.append(f'{name}: {where}: action not pinned by full SHA: {ref}')
            continue
        # The pin's own line carries the version comment.
        carriers = [line for line in lines if ref in line and not line.lstrip().startswith('#')]
        if not carriers or not all(COMMENT.search(line.split(ref, 1)[1]) for line in carriers):
            errors.append(f'{name}: {where}: pinned action lacks a version comment: {ref}')
    return errors


directory = pathlib.Path(sys.argv[2])
texts = {path.name: path.read_text(encoding='utf-8') for path in sorted(directory.glob('*.y*ml'))}
if not texts:
    raise SystemExit('error: no workflow files found')
errors = [e for name, text in texts.items() for e in check(name, text)]
if errors:
    raise SystemExit('error: ' + '\n'.join(errors))

sha = 'actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1'
base = texts['ci.yml']
job = '\n  pinctl:\n    runs-on: ubuntu-24.04\n    steps:\n'
SHA = 'not pinned by full SHA'
COMMENTED = 'lacks a version comment'
DOCKER = 'Docker action not pinned by digest'
PARSE = 'not accepted in workflows'
controls = [
    ('tag ref', base + job + '      - uses: actions/checkout@v7\n', SHA),
    ('branch ref', base + job + '      - uses: actions/checkout@main\n', SHA),
    ('short SHA', base + job + '      - uses: actions/checkout@3d3c42e\n', SHA),
    ('no comment', base + job + f'      - uses: {sha}\n', COMMENTED),
    ('free-text comment', base + job + f'      - uses: {sha}  # pinned\n', COMMENTED),
    ('quoted key', base + job + '      - "uses": actions/checkout@main\n', SHA),
    ('quoted value', base + job + '      - uses: "actions/checkout@main"\n', SHA),
    ('flow-style step', base + job + '      - {uses: actions/checkout@main}\n', SHA),
    ('flow-style steps list', base + '\n  pinctl:\n    runs-on: ubuntu-24.04\n    steps: [{uses: actions/checkout@main}]\n', SHA),
    ('flow-style reusable job', base + '\n  pinctl: {uses: org/repo/.github/workflows/x.yml@main}\n', SHA),
    ('reusable job tag ref', base + '\n  pinctl:\n    uses: org/repo/.github/workflows/x.yml@v1\n', SHA),
    ('docker tag', base + job + '      - uses: docker://alpine:latest\n', DOCKER),
    ('docker untagged', base + job + '      - uses: docker://alpine\n', DOCKER),
    ('alias', base + job + f'      - uses: &a {sha}  # v7\n      - uses: *a\n', PARSE),
]
for label, text, expected in controls:
    found = check('ci.yml', text)
    if not any(expected in error for error in found):
        raise SystemExit(f'error: pin mutation control did not fail as intended: {label} (expected {expected!r}, got {found})')

repo_root = pathlib.Path(sys.argv[3])
pins = ci_pins.load_pins(repo_root)
ci_pins.run_check(repo_root)

marked_job = '\n  pinctl:\n    runs-on: ubuntu-24.04  # pin:runner=linux-x64\n    steps:\n'
ci_pin_controls = [
    (
        'moving runner label',
        base + '\n  pinctl:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo ok\n',
        'unapproved or moving runner label',
    ),
    (
        'unmarked hosted runner',
        base + '\n  pinctl:\n    runs-on: ubuntu-24.04\n    steps:\n      - run: echo ok\n',
        'lacks # pin:runner=<key> marker',
    ),
    (
        'expression runs-on',
        base + '\n  pinctl:\n    runs-on: ${{ vars.RUNNER }}\n    steps:\n      - run: echo ok\n',
        'expression-valued runs-on cannot be checked',
    ),
    (
        'matrix moving runner',
        base
        + '\n  pinctl:\n    runs-on: ${{ matrix.os }}\n    strategy:\n      matrix:\n        os:\n          - ubuntu-latest\n    steps:\n      - run: echo ok\n',
        'unapproved or moving runner label',
    ),
    (
        'dtolnay rust-toolchain',
        base
        + marked_job
        + '      - uses: dtolnay/rust-toolchain@6bed0761d98439e5a578e2877258200ad565ba87  # stable\n',
        'third-party Rust toolchain action is forbidden',
    ),
    (
        'floating rust channel',
        base + marked_job + '      - run: cargo +stable check\n',
        'floating Rust channel in run step',
    ),
    (
        'direct rustup target add',
        base + marked_job + '      - run: rustup target add x86_64-pc-windows-gnu\n',
        'direct rustup invocation in workflow step',
    ),
    (
        'setup-rust drifted version',
        base
        + marked_job
        + '      - uses: ./.github/actions/setup-rust\n        with:\n          toolchain: 1.97.0  # pin:rust=stable\n',
        'rust pin \'stable\' is \'1.97.0\'',
    ),
    (
        'setup-rust missing marker',
        base
        + marked_job
        + '      - uses: ./.github/actions/setup-rust\n        with:\n          toolchain: 1.98.1\n',
        'lacks # pin:rust=<key> marker',
    ),
    (
        'setup-rust flow-style missing marker',
        base
        + marked_job
        + '      - {uses: ./.github/actions/setup-rust, with: {toolchain: 1.98.1}}\n',
        'lacks # pin:rust=<key> marker',
    ),
    (
        'flow-style runner list missing marker',
        base
        + '\n  pinctl:\n    runs-on: [ubuntu-24.04]\n    steps:\n      - run: echo ok\n',
        'lacks # pin:runner=<key> marker',
    ),
    (
        'custom matrix axis unmarked hosted runner',
        base
        + marked_job
        + '      - run: echo ok\n    strategy:\n      matrix:\n        custom_runner:\n          - ubuntu-24.04\n',
        'lacks # pin:runner=<key> marker',
    ),
    (
        'flow-style setup-rust with decoy env marker',
        base
        + marked_job
        + '      - name: decoy\n        env:\n          DECOY: 1.98.1  # pin:rust=stable\n        run: echo ok\n'
        + '      - {uses: ./.github/actions/setup-rust, with: {toolchain: 1.98.1}}\n',
        'lacks # pin:rust=<key> marker',
    ),
    (
        'orphan runner pin marker on non-runner line',
        base
        + marked_job
        + '      - name: decoy\n        env:\n          DECOY: ubuntu-24.04  # pin:runner=linux-x64\n        run: echo ok\n',
        'not attached to a runner consumer',
    ),
    (
        'env RUSTUP_TOOLCHAIN floating channel',
        base
        + marked_job
        + '      - env:\n          RUSTUP_TOOLCHAIN: nightly\n        run: cargo check\n',
        'env.RUSTUP_TOOLCHAIN must come from config/ci/pins.toml',
    ),
    (
        'quoted cargo floating channel',
        base + marked_job + '      - run: cargo +"nightly" check\n',
        'floating Rust channel in run step',
    ),
    (
        'double-quoted plus cargo floating channel',
        base + marked_job + '      - run: cargo "+nightly" check\n',
        'floating Rust channel in run step',
    ),
    (
        'single-quoted plus cargo floating channel',
        base + marked_job + "      - run: cargo '+nightly' check\n",
        'floating Rust channel in run step',
    ),
    (
        'same-line flow setup-rust decoy',
        base
        + marked_job
        + '      - {uses: ./.github/actions/setup-rust, with: {toolchain: 1.98.1}, env: {DECOY: 1.98.1  # pin:rust=stable\n        }}\n',
        'lacks # pin:rust=<key> marker',
    ),
    (
        'same-line flow runs-on decoy',
        base
        + '\n  pinctl: {runs-on: ubuntu-24.04, env: {DECOY: ubuntu-24.04  # pin:runner=linux-x64\n  }, steps: [{run: echo ok}]}\n',
        'lacks # pin:runner=<key> marker',
    ),
    (
        'same-line flow matrix decoy',
        base
        + '\n  pinctl:\n    runs-on: ${{ matrix.os }}\n    strategy: {matrix: {other: [ubuntu-24.04], os: [ubuntu-24.04  # pin:runner=linux-x64\n    ]}}\n    steps:\n      - run: echo ok\n',
        'lacks # pin:runner=<key> marker',
    ),
    (
        'node20 checkout pin',
        base
        + marked_job
        + '      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262  # v4\n',
        'deprecated Node 20',
    ),
]
for label, text, expected in ci_pin_controls:
    found = ci_pins.check_workflow_text('ci.yml', text, pins)
    if not any(expected in error for error in found):
        raise SystemExit(
            f'error: ci-pins mutation control did not fail as intended: {label} '
            f'(expected {expected!r}, got {found})'
        )

bumped_pins = {
    'hosted': dict(pins['hosted'], **{'linux-x64': 'ubuntu-26.04'}),
    'allowlist': set(pins['allowlist']),
    'rust': dict(pins['rust'], stable='1.99.0'),
}
synced_ci, updated_count = ci_pins.sync_text(base, bumped_pins, 'ci.yml')
if updated_count == 0 or 'ubuntu-26.04  # pin:runner=linux-x64' not in synced_ci or '1.99.0  # pin:rust=stable' not in synced_ci:
    raise SystemExit('error: ci-pins sync_text did not propagate bumped runner and rust pins')
post_sync_errors = ci_pins.check_workflow_text('ci.yml', synced_ci, bumped_pins)
if post_sync_errors:
    raise SystemExit(f'error: synced workflow failed ci-pins check: {post_sync_errors}')

print(
    f'[ok] {len(texts)} workflows: every action SHA- or digest-pinned with a version comment; '
    f'{len(controls)} raw-text controls and {len(ci_pin_controls)} CI-pin controls refused'
)
PY
(cd "$root" && python3 -B -E scripts/release-workflow-pin.py check) >/dev/null
(cd "$root" && python3 -B -P -E scripts/ci-pins.py check) >/dev/null
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
repo="$scratch/repo"
mkdir -p "$repo/.github/workflows" "$repo/.github/actions/setup-rust" "$repo/.github/actions/sample" "$repo/config/release"
cp "$root"/.github/workflows/*.yml "$repo/.github/workflows/"
cp "$root/.github/actions/setup-rust/action.yml" "$repo/.github/actions/setup-rust/action.yml"
cp "$root/config/release/release-workflow.sha256" "$repo/config/release/"
git -C "$repo" init -q -b main
git -C "$repo" config user.name fixture
git -C "$repo" config user.email fixture@example.invalid
git -C "$repo" config commit.gpgsign false
git -C "$repo" add -A
git -C "$repo" commit -qm base
sed_in_place() {
	local script="$1" file="$2"
	sed "$script" "$file" >"$file.tmp" && mv "$file.tmp" "$file"
}
pin() { (cd "$repo" && PYTHONDONTWRITEBYTECODE=1 python3 -B -E "$root/scripts/release-workflow-pin.py" "$@") >"$scratch/out" 2>&1; }
refused() {
	if pin "${@:2}"; then
		echo "error: workflow pin accepted: $1" >&2
		exit 1
	fi
	grep -q "$1" "$scratch/out" || {
		cat "$scratch/out" >&2
		exit 1
	}
}
pin_with() {
	(cd "$repo" && LC_ALL=C shasum -a 256 .github/actions/*/*.yml .github/workflows/*.yml) >"$repo/config/release/release-workflow.sha256"
}
pin check
pin verify-commit HEAD
printf '# unreviewed\n' >>"$repo/.github/workflows/release.yml"
refused 'differs from its reviewed digest' check
git -C "$repo" checkout -q -- .github
printf 'name: x\n' >"$repo/.github/actions/sample/action.yml"
refused 'action.yml is not pinned' check
rm -rf "$repo/.github/actions/sample"
sed_in_place '1s/^./0/' "$repo/config/release/release-workflow.sha256"
refused 'differs from its reviewed digest' check
git -C "$repo" checkout -q -- config
printf '# reviewed\n' >>"$repo/.github/workflows/release.yml"
pin_with
pin check
git -C "$repo" checkout -q -- .
printf 'on: push\npermissions:\n  contents: read\njobs: {}\n' >"$repo/.github/workflows/extra.yml"
refused 'extra.yml is not pinned' check
rm "$repo/.github/workflows/extra.yml"
sed_in_place 's/^  contents: read$/  contents: write/' "$repo/.github/workflows/ci.yml"
pin_with
refused 'ci.yml must declare read-only top-level permissions' check
git -C "$repo" checkout -q -- .
sed_in_place 's/^    environment: publish-npm$/    environment: other-env/' "$repo/.github/workflows/registry-publish.yml"
pin_with
refused 'grants more than read' check
git -C "$repo" checkout -q -- .
sed_in_place 's/^    environment: publish-pypi$/    environment: other-env/' "$repo/.github/workflows/registry-publish.yml"
pin_with
refused 'grants more than read' check
git -C "$repo" checkout -q -- .
sed_in_place 's/^  workflow_dispatch:$/  push:\n  workflow_dispatch:/' "$repo/.github/workflows/registry-publish.yml"
pin_with
refused 'must trigger on workflow_dispatch only' check
git -C "$repo" checkout -q -- .
mkdir "$scratch/shadow"
cp "$root/scripts/release-workflow-pin.py" "$root/scripts/ci-pins.py" "$root/scripts/workflow_yaml.py" "$scratch/shadow/"
printf 'raise SystemExit("shadow yaml imported")\n' >"$scratch/shadow/yaml.py"
(cd "$repo" && python3 -B -E "$scratch/shadow/release-workflow-pin.py" check) >"$scratch/out" 2>&1 || true
if grep -q 'shadow yaml imported' "$scratch/out"; then
	echo 'error: a yaml.py beside the pin checker was imported' >&2
	exit 1
fi
grep -q 'every workflow matches' "$scratch/out" || {
	cat "$scratch/out" >&2
	exit 1
}
# Even when PyYAML is unavailable, ci-pins.py fails before restoring its script
# directory to sys.path, so a local scripts/yaml.py never executes.
python3 -B -P -E - "$scratch/shadow/ci-pins.py" "$repo" >"$scratch/out" 2>&1 <<'PY' || true
import importlib.abc, runpy, sys

class BlockInstalledYaml(importlib.abc.MetaPathFinder):
    def find_spec(self, fullname, path, target=None):
        if fullname == "yaml" or fullname.startswith("yaml."):
            raise ModuleNotFoundError("No module named 'yaml'")
        return None

sys.meta_path.insert(0, BlockInstalledYaml())
sys.argv = [sys.argv[1], "--root", sys.argv[2], "check"]
runpy.run_path(sys.argv[0], run_name="__main__")
PY
if grep -q 'shadow yaml imported' "$scratch/out"; then
	echo 'error: ci-pins.py imported local yaml.py when PyYAML was missing' >&2
	exit 1
fi
grep -q 'need PyYAML' "$scratch/out" || {
	cat "$scratch/out" >&2
	exit 1
}

# Symlink refusal in ci-pins.py sync happens before any workflow or digest file
# is modified on disk.
sync_repo="$scratch/sync-repo"
mkdir -p "$sync_repo/.github/workflows" "$sync_repo/.github/actions/setup-rust" "$sync_repo/config/ci" "$sync_repo/config/release"
cp "$root/.github/workflows/ci.yml" "$sync_repo/.github/workflows/ci.yml"
cp "$root/.github/actions/setup-rust/action.yml" "$sync_repo/.github/actions/setup-rust/action.yml"
cp "$root/config/ci/pins.toml" "$sync_repo/config/ci/pins.toml"
cp "$root/config/release/release-workflow.sha256" "$sync_repo/config/release/release-workflow.sha256"
sed_in_place 's/ubuntu-24\.04  # pin:runner=linux-x64/ubuntu-22.04  # pin:runner=linux-x64/' "$sync_repo/.github/workflows/ci.yml"
cp "$sync_repo/.github/workflows/ci.yml" "$scratch/drifted-ci.before"
ln -s /etc/hosts "$sync_repo/.github/actions/evil.yml"
if python3 -B -P -E "$root/scripts/ci-pins.py" --root "$sync_repo" sync >"$scratch/out" 2>&1; then
	echo 'error: ci-pins.py sync accepted symlink under .github/actions' >&2
	exit 1
fi
cmp -s "$scratch/drifted-ci.before" "$sync_repo/.github/workflows/ci.yml" || {
	echo 'error: ci-pins.py sync mutated workflow before refusing symlink' >&2
	exit 1
}
rm "$sync_repo/.github/actions/evil.yml"
rm "$sync_repo/config/release/release-workflow.sha256"
ln -s "$scratch/outside-sha" "$sync_repo/config/release/release-workflow.sha256"
if python3 -B -P -E "$root/scripts/ci-pins.py" --root "$sync_repo" sync >"$scratch/out" 2>&1; then
	echo 'error: ci-pins.py sync accepted symlink at release-workflow.sha256' >&2
	exit 1
fi
[[ ! -e "$scratch/outside-sha" ]] || {
	echo 'error: ci-pins.py sync wrote through release-workflow.sha256 symlink' >&2
	exit 1
}
cmp -s "$scratch/drifted-ci.before" "$sync_repo/.github/workflows/ci.yml" || {
	echo 'error: ci-pins.py sync mutated workflow before refusing digest symlink' >&2
	exit 1
}

# setup-rust composite action input validation refuses leading-zero versions,
# option tokens (--help), and validates every newline-separated token.
action_script="$(
	python3 -B -P -E - "$root/scripts" "$root/.github/actions/setup-rust/action.yml" <<'PY'
import pathlib, sys
sys.path.insert(0, sys.argv[1])
import workflow_yaml as wy
doc = wy.load_text(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"), "action.yml")
print(doc["runs"]["steps"][0]["run"])
PY
)"
stub_bin="$scratch/stub-bin"
mkdir -p "$stub_bin"
cat >"$stub_bin/rustup" <<'SH'
#!/usr/bin/env bash
echo "rustup $*" >>"$RUSTUP_LOG"
SH
cat >"$stub_bin/rustc" <<'SH'
#!/usr/bin/env bash
echo "rustc ${INPUT_TOOLCHAIN} (0000000 2026-01-01)"
SH
cat >"$stub_bin/cargo" <<'SH'
#!/usr/bin/env bash
echo "cargo ${INPUT_TOOLCHAIN}"
SH
chmod +x "$stub_bin/rustup" "$stub_bin/rustc" "$stub_bin/cargo"
run_setup_rust() {
	env PATH="$stub_bin:$PATH" CARGO_HOME="$scratch/empty-cargo" GITHUB_PATH="$scratch/gh_path" GITHUB_ENV="$scratch/gh_env" RUSTUP_LOG="$scratch/rustup.log" \
		INPUT_TOOLCHAIN="$1" INPUT_COMPONENTS="$2" INPUT_TARGETS="$3" bash -c "$action_script" >/dev/null 2>&1
}
if run_setup_rust "01.88.0" "" ""; then
	echo 'error: setup-rust accepted leading-zero toolchain version' >&2
	exit 1
fi
if run_setup_rust "1.88.0" "--help" ""; then
	echo 'error: setup-rust accepted --help in components' >&2
	exit 1
fi
if run_setup_rust "1.88.0" "" "--help"; then
	echo 'error: setup-rust accepted --help in targets' >&2
	exit 1
fi
if run_setup_rust "1.88.0" $'clippy\n--help' ""; then
	echo 'error: setup-rust truncated newline-separated invalid component' >&2
	exit 1
fi
: >"$scratch/rustup.log"
run_setup_rust "1.88.0" $'clippy\nrustfmt' $'x86_64-unknown-linux-gnu\naarch64-apple-darwin'
grep -q 'component add --toolchain 1.88.0 clippy rustfmt' "$scratch/rustup.log"
grep -q 'target add --toolchain 1.88.0 x86_64-unknown-linux-gnu aarch64-apple-darwin' "$scratch/rustup.log"
printf '# uncommitted\n' >>"$repo/.github/workflows/release.yml"
pin verify-commit HEAD
git -C "$repo" add .github/workflows/release.yml
git -C "$repo" commit -qm unreviewed
git -C "$repo" checkout -q HEAD~1 -- .github/workflows/release.yml
refused 'differs from its reviewed digest' verify-commit HEAD
echo '[ok] workflow digest pins and verify-commit controls passed'
