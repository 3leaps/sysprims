#!/usr/bin/env bash
# Every action and reusable workflow is pinned by full commit SHA, every Docker
# action by image digest, and each pinned line carries a comment naming the
# version or channel the pin was taken from. `uses` values are read from the
# parsed workflow (scripts/workflow_yaml.py), so quoting and flow style do not
# hide one. Runner images and the Rust toolchain channel are governed
# separately.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
python3 -B -P -E - "$root/scripts" "$root/.github/workflows" <<'PY'
import pathlib
import re
import sys

sys.path.insert(0, sys.argv[1])
import workflow_yaml as wy  # noqa: E402

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
print(f'[ok] {len(texts)} workflows: every action SHA- or digest-pinned with a version comment; {len(controls)} raw-text controls refused')
PY
(cd "$root" && python3 -B -E scripts/release-workflow-pin.py check) >/dev/null
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
repo="$scratch/repo"
mkdir -p "$repo/.github/workflows" "$repo/.github/actions/sample" "$repo/config/release"
cp "$root"/.github/workflows/*.yml "$repo/.github/workflows/"
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
	(cd "$repo" && shasum -a 256 .github/workflows/*.yml) >"$repo/config/release/release-workflow.sha256"
}
pin check
pin verify-commit HEAD
printf '# unreviewed\n' >>"$repo/.github/workflows/release.yml"
refused 'differs from its reviewed digest' check
git -C "$repo" checkout -q -- .github
printf 'name: x\n' >"$repo/.github/actions/sample/action.yml"
refused 'action.yml is not pinned' check
rm -rf "$repo/.github/actions"
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
sed_in_place 's/^    environment: publish-npm$/    environment: other-env/' "$repo/.github/workflows/typescript-npm-publish.yml"
pin_with
refused 'grants more than read' check
git -C "$repo" checkout -q -- .
sed_in_place 's/^  workflow_dispatch:$/  push:\n  workflow_dispatch:/' "$repo/.github/workflows/typescript-npm-publish.yml"
pin_with
refused 'must trigger on workflow_dispatch only' check
git -C "$repo" checkout -q -- .
mkdir "$scratch/shadow"
cp "$root/scripts/release-workflow-pin.py" "$scratch/shadow/"
printf 'raise SystemExit("shadow yaml imported")\n' >"$scratch/shadow/yaml.py"
(cd "$repo" && python3 -B -E "$scratch/shadow/release-workflow-pin.py" check) >"$scratch/out" 2>&1 || true
if grep -q 'shadow' "$scratch/out"; then
	echo 'error: a yaml.py beside the pin checker was imported' >&2
	exit 1
fi
grep -q 'every workflow matches' "$scratch/out" || {
	cat "$scratch/out" >&2
	exit 1
}
printf '# uncommitted\n' >>"$repo/.github/workflows/release.yml"
pin verify-commit HEAD
git -C "$repo" add .github/workflows/release.yml
git -C "$repo" commit -qm unreviewed
git -C "$repo" checkout -q HEAD~1 -- .github/workflows/release.yml
refused 'differs from its reviewed digest' verify-commit HEAD
echo '[ok] workflow digest pins and verify-commit controls passed'
