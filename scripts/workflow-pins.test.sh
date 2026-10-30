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
