#!/usr/bin/env bash
# Every workflow is read-only at the top level, and no workflow creates, edits
# or publishes a GitHub release. Write permissions exist only on two reviewed
# jobs, each with exactly the scopes it needs:
#   typescript-npm-publish.yml publish: id-token (npm trusted publishing), only
#     behind the publish-npm environment
#   go-bindings.yml update-go-bindings: contents + pull-requests (opens the
#     prebuilt-library pull request). Its steps may use only the actions listed
#     below; the token reaches only the pull-request action's `with.token`,
#     checkout does not persist credentials, and no shell step or environment
#     names a token, so repository code run in that job never holds the write
#     token.
# Security boundary: permission and credential confinement (no `secrets.*`
# anywhere in any workflow except the single `create-pull-request` `with.token`,
# and no `github`/`secrets` expressions anywhere else in the write job). Bare
# (non-`${{ }}`) `if:` conditions are boolean predicates only and are not
# expression-scanned. `RELEASE_WRITERS` and `WRITE_JOB_FORBIDDEN_RUN` are
# defence-in-depth lint checks, not a complete shell parser.
# Workflows are parsed as YAML (scripts/workflow_yaml.py). Negative controls
# are raw workflow text run through the same loader and checks.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
python3 -B -P -E - "$root/scripts" "$root/.github/workflows" <<'PY'
import pathlib
import re
import sys

sys.path.insert(0, sys.argv[1])
import workflow_yaml as wy  # noqa: E402

ALLOWED_WRITE = {
    ('typescript-npm-publish.yml', 'publish'): {'id-token'},
    ('go-bindings.yml', 'update-go-bindings'): {'contents', 'pull-requests'},
}
REQUIRED_ENVIRONMENT = {('typescript-npm-publish.yml', 'publish'): 'publish-npm'}
# Actions a write job may run, by repository (the pin lint fixes the SHA), and
# the one action that may receive the token.
WRITER_ACTIONS = {
    ('go-bindings.yml', 'update-go-bindings'): {
        'actions/checkout', 'actions/download-artifact', 'peter-evans/create-pull-request',
    },
}
TOKEN_ACTIONS = {('go-bindings.yml', 'update-go-bindings'): {'peter-evans/create-pull-request'}}
# Expressions in a write job may read only these contexts outside the one
# token-receiving `with.token`; `secrets` and `github` (which holds the token,
# by name, by index, or serialised whole) never appear.
SAFE_CONTEXTS = {'needs', 'matrix', 'inputs', 'env', 'steps', 'runner', 'job', 'strategy', 'vars'}
TOP_IDENTIFIER = re.compile(r"(?<![\w.])([A-Za-z_][\w-]*)")
LITERALS = {'true', 'false', 'null'}
FUNCTIONS = {'contains', 'startsWith', 'endsWith', 'format', 'join', 'toJSON', 'fromJSON', 'hashFiles',
             'success', 'always', 'cancelled', 'failure'}


def expressions(text):
    """Bodies of every ${{ ... }} in text. Single-quoted strings (with '' as an
    escaped quote) may contain braces; an unterminated expression is returned
    whole so it is still inspected."""
    found = []
    i = 0
    while True:
        start = text.find('${{', i)
        if start < 0:
            return found
        j = start + 3
        quoted = False
        while j < len(text):
            if quoted:
                if text.startswith("''", j):
                    j += 2
                    continue
                if text[j] == "'":
                    quoted = False
            elif text[j] == "'":
                quoted = True
            elif text.startswith('}}', j):
                break
            j += 1
        found.append(text[start + 3:j])
        i = j + 2


def unsafe_expressions(value):
    """Expressions in value that read anything outside SAFE_CONTEXTS."""
    if value is None:
        return []
    if isinstance(value, dict):
        return [expr for k, v in value.items() for expr in (unsafe_expressions(k) + unsafe_expressions(v))]
    if isinstance(value, list):
        return [expr for item in value for expr in unsafe_expressions(item)]
    text = value if isinstance(value, str) else repr(value)
    bad = []
    for expression in expressions(text):
        body = re.sub(r"'(?:[^']|'')*'", "''", expression)
        for match in TOP_IDENTIFIER.finditer(body):
            name = match.group(1)
            if name in LITERALS:
                continue
            if name in FUNCTIONS and re.match(r'\s*\(', body[match.end():]):
                continue
            if name not in SAFE_CONTEXTS:
                bad.append(expression.strip())
                break
    return bad


def secrets_expressions(value):
    """Expressions in value that read the secrets context (in any job or workflow)."""
    if value is None:
        return []
    if isinstance(value, dict):
        return [expr for k, v in value.items() for expr in (secrets_expressions(k) + secrets_expressions(v))]
    if isinstance(value, list):
        return [expr for item in value for expr in secrets_expressions(item)]
    text = value if isinstance(value, str) else repr(value)
    bad = []
    for expression in expressions(text):
        body = re.sub(r"'(?:[^']|'')*'", "''", expression)
        for match in TOP_IDENTIFIER.finditer(body):
            if match.group(1).lower() == 'secrets':
                bad.append(expression.strip())
                break
    return bad


def fold_continuations(text):
    """Join shell line continuations before scanning for release-writing commands."""
    return re.sub(r'\\\r?\n[ \t]*', ' ', str(text))


# Defence-in-depth lint checks (the boundary is permission and token/secret
# confinement above, not shell parsing).
RELEASE_WRITERS = re.compile(
    r'(?i)softprops/|action-gh-release|ncipollo/release|actions/create-release|'
    r'\bgh\b[^\n]*\b(?:release|alias)\b|'
    r'\b(?:create|update|delete)Release\b|'
    r'releases/(?:tags|latest)|'
    r'\bgh\s+api\b[^\n]*\breleases\b[^\n]*(?:\s-[fFX]\b|--(?:field|raw-field|input|method)\b)|'
    r'\bgh\s+api\b[^\n]*(?:\s-[fFX]\b|--(?:field|raw-field|input|method)\b)[^\n]*\breleases\b|'
    r'\bcurl\b[^\n]*(?:\s-X\s*|--request[ =])(?:POST|PATCH|PUT|DELETE)[^\n]*/releases\b|'
    r'\bcurl\b[^\n]*/releases\b[^\n]*(?:\s-X\s*|--request[ =])(?:POST|PATCH|PUT|DELETE)|'
    r'\bcurl\b[^\n]*/releases\b[^\n]*\s(?:-d|--data(?:-\w+)?|-F|--form)\b'
)
# Defence-in-depth: a contents:write job runs only local file/manifest
# preparation; network CLIs and remote git mutations have no business in its
# shell steps.
WRITE_JOB_FORBIDDEN_RUN = re.compile(
    r'(?i)(?:^|[\s;&|(`$=])(?:[^\s;&|()`\'"]*/)?(?:'
    r'gh|curl|wget|git\s+(?:push|tag|remote)\b'
    r')(?:\s|$)'
)


class Refused(Exception):
    pass


def scopes(permissions, where):
    """Resolved (scope -> level). Anything Actions would read as write is write."""
    if permissions is None:
        return {}
    if isinstance(permissions, str):
        if permissions == 'read-all':
            return {'*': 'read'}
        if permissions == 'write-all':
            return {'*': 'write'}
        raise Refused(f'{where}: unrecognised permissions value {permissions!r}')
    if not isinstance(permissions, dict):
        raise Refused(f'{where}: permissions must be a mapping or read-all/write-all')
    out = {}
    for scope, level in permissions.items():
        if not isinstance(scope, str) or level not in ('read', 'write', 'none'):
            raise Refused(f'{where}: unrecognised permission {scope!r}: {level!r}')
        out[scope] = level
    return out


def check(name, doc, text):
    errors = []
    try:
        top = scopes(doc.get('permissions'), f'{name}: workflow')
        if top.get('contents') != 'read' or any(level == 'write' for level in top.values()):
            errors.append(f'{name}: workflow-level permissions must be read-only and include contents: read')
        for key, val in doc.items():
            if key != 'jobs' and (secrets_expressions(key) or secrets_expressions(val)):
                errors.append(f'{name}: workflow {key} reads a token context')
        for job_id, job in wy.jobs(doc).items():
            if not isinstance(job, dict):
                raise Refused(f'{name}: job {job_id!r} is not a mapping')
            where = f'{name}: job {job_id}'
            if 'uses' in job:
                errors.append(f'{where}: reusable workflow calls are not accepted')
            granted = {scope for scope, level in scopes(job.get('permissions'), where).items() if level == 'write'}
            allowed = ALLOWED_WRITE.get((name, job_id), set())
            if granted - allowed:
                errors.append(f'{where}: unreviewed write permission {sorted(granted - allowed)}')
            environment = REQUIRED_ENVIRONMENT.get((name, job_id))
            if environment and granted:
                env = job.get('environment')
                env = env.get('name') if isinstance(env, dict) else env
                if env != environment:
                    errors.append(f'{where}: write permission outside environment {environment}')
            writer_actions = WRITER_ACTIONS.get((name, job_id))
            if granted and writer_actions is not None:
                # Inherited by or wrapping the job: every workflow-level key outside jobs.
                for key, val in doc.items():
                    if key != 'jobs' and (unsafe_expressions(key) or unsafe_expressions(val)):
                        errors.append(f'{where}: workflow {key} reads a token context inherited by a write job')
                # Scan every job-level key generically outside steps.
                for key, val in job.items():
                    if key != 'steps' and (unsafe_expressions(key) or unsafe_expressions(val)):
                        errors.append(f'{where}: job {key} reads a token context in a write job')
                token_actions = TOKEN_ACTIONS.get((name, job_id), set())
                token_steps = 0
                for step in wy.steps(job):
                    if not isinstance(step, dict):
                        raise Refused(f'{where}: step is not a mapping')
                    uses = step.get('uses')
                    repo = str(uses).split('@', 1)[0] if uses is not None else None
                    if repo is not None and repo not in writer_actions:
                        errors.append(f'{where}: action {uses} is not allowed in a write job')
                    if repo == 'actions/checkout' and (step.get('with') or {}).get('persist-credentials') is not False:
                        errors.append(f'{where}: checkout must set persist-credentials: false in a write job')
                    if 'run' in step and WRITE_JOB_FORBIDDEN_RUN.search(fold_continuations(step.get('run'))):
                        errors.append(f'{where}: forbidden command in a write-job run step')
                    is_token_step = repo in token_actions
                    if is_token_step:
                        token_steps += 1
                    for key, value in step.items():
                        if unsafe_expressions(key):
                            errors.append(f'{where}: step {key} reads a token context in a write job')
                        if key == 'with' and is_token_step:
                            if not isinstance(value, dict):
                                raise Refused(f'{where}: step with is not a mapping')
                            for wk, wv in value.items():
                                if wk == 'token':
                                    if not isinstance(wv, str) or wv.strip() != '${{ secrets.GITHUB_TOKEN }}':
                                        errors.append(f'{where}: step with.token reads a token context in a write job')
                                elif unsafe_expressions(wk) or unsafe_expressions(wv):
                                    errors.append(f'{where}: step with.{wk} reads a token context in a write job')
                            continue
                        if unsafe_expressions(value):
                            errors.append(f'{where}: step {key} reads a token context in a write job')
                if token_steps > 1:
                    errors.append(f'{where}: multiple token-receiving steps are not allowed in a write job')
            else:
                for key, val in job.items():
                    if key != 'steps' and (secrets_expressions(key) or secrets_expressions(val)):
                        errors.append(f'{where}: job {key} reads a token context')
                for step in wy.steps(job):
                    if secrets_expressions(step):
                        errors.append(f'{where}: step reads a token context')
            for step in wy.steps(job):
                if isinstance(step, dict) and 'run' in step:
                    if RELEASE_WRITERS.search(fold_continuations(step.get('run'))):
                        errors.append(f'{name}: release-creating or release-editing construct present')
    except (Refused, wy.WorkflowError) as error:
        errors.append(str(error))
    if RELEASE_WRITERS.search(fold_continuations(text)):
        errors.append(f'{name}: release-creating or release-editing construct present')
    return errors


def check_text(name, text):
    try:
        doc = wy.load_text(text, name)
    except wy.WorkflowError as error:
        return [str(error)]
    return check(name, doc, text)


directory = pathlib.Path(sys.argv[2])
texts = {path.name: path.read_text(encoding='utf-8') for path in sorted(directory.glob('*.y*ml'))}
if not texts:
    raise SystemExit('error: no workflow files found')
errors = [e for name, text in texts.items() for e in check_text(name, text)]
if errors:
    raise SystemExit('error: ' + '\n'.join(errors))


def job_start(text, job_id):
    match = re.search(rf'(?m)^  {re.escape(job_id)}:\n', text)
    if not match:
        raise SystemExit(f'error: control fixture cannot find job {job_id}')
    return match.end()


def insert_job_key(name, job_id, lines):
    text = texts[name]
    at = job_start(text, job_id)
    return text[:at] + lines + text[at:]


def replace_once(name, old, new):
    text = texts[name]
    if text.count(old) != 1:
        raise SystemExit(f'error: control fixture: {old!r} not unique in {name}')
    return text.replace(old, new)


def append_step(name, job_id, step):
    """Append a step to the end of a job (block style, 6-space list items)."""
    text = texts[name]
    start = job_start(text, job_id)
    nxt = re.search(r'(?m)^  [\w-]+:\n', text[start:])
    end = start + nxt.start() if nxt else len(text)
    body = text[start:end].rstrip('\n') + '\n' + step
    return text[:start] + body + ('\n' if nxt else '') + text[end:]


rel = next(iter(wy.load_text(texts['release.yml'], 'release.yml')['jobs']))
top = 'permissions:\n  contents: read\n'
TOP = 'workflow-level permissions must be read-only'
JOB = 'unreviewed write permission'
UNREC = 'unrecognised permission'
TOKEN = 'reads a token context'
WRITER = 'release-creating or release-editing construct'
PARSE = 'not accepted in workflows'


def writer_job(command):
    """A valid read-only job whose one step runs command."""
    return replace_once('ci.yml', '\njobs:\n', '\njobs:\n  writer:\n    runs-on: ubuntu-24.04\n    steps:\n      - run: ' + command + '\n')


go_writer = ('    permissions:\n      contents: write\n      pull-requests: write\n')
controls = [
    ('top-level write', 'release.yml', replace_once('release.yml', top, 'permissions:\n  contents: write\n'), TOP),
    ('top-level missing', 'ci.yml', replace_once('ci.yml', top, ''), TOP),
    ('top-level write-all', 'ci.yml', replace_once('ci.yml', top, 'permissions: write-all\n'), TOP),
    ('top-level quoted write-all', 'ci.yml', replace_once('ci.yml', top, 'permissions: "write-all"\n'), TOP),
    ('top-level extra write scope', 'ci.yml', replace_once('ci.yml', top, top + '  packages: write\n'), TOP),
    ('top-level flow mapping write', 'ci.yml', replace_once('ci.yml', top, 'permissions: { contents: write }\n'), TOP),
    ('job write-all', 'release.yml', insert_job_key('release.yml', rel, '    permissions: write-all\n'), JOB),
    ('job quoted value', 'release.yml', insert_job_key('release.yml', rel, '    permissions:\n      contents: "write"\n'), JOB),
    ('job quoted key', 'release.yml', insert_job_key('release.yml', rel, '    "permissions":\n      "contents": write\n'), JOB),
    ('job commented key', 'release.yml', insert_job_key('release.yml', rel, '    permissions: # note\n      contents: write\n'), JOB),
    ('job flow mapping', 'release.yml', insert_job_key('release.yml', rel, '    permissions: {contents: write}\n'), JOB),
    ('job deeper indentation', 'release.yml', insert_job_key('release.yml', rel, '    permissions:\n         contents: write\n'), JOB),
    ('job unknown level', 'release.yml', insert_job_key('release.yml', rel, '    permissions:\n      contents: admin\n'), UNREC),
    ('flow-style job', 'release.yml', replace_once('release.yml', '\njobs:\n', '\njobs:\n  sneak: {runs-on: ubuntu-24.04, permissions: write-all, steps: [{run: echo}]}\n'), JOB),
    ('alias inherits writer permissions', 'go-bindings.yml',
     replace_once('go-bindings.yml', go_writer, go_writer.replace('permissions:\n', 'permissions: &w\n', 1))
     + '  other:\n    runs-on: ubuntu-24.04\n    permissions: *w\n    steps:\n      - run: echo\n', PARSE),
    ('merge key', 'ci.yml', replace_once('ci.yml', top, 'permissions:\n  <<: {contents: write}\n'), PARSE),
    ('duplicate permissions key', 'ci.yml', replace_once('ci.yml', top, top + 'permissions:\n  contents: write\n'), 'duplicate key'),
    ('reusable workflow job', 'release.yml', replace_once('release.yml', '\njobs:\n', '\njobs:\n  reuse:\n    uses: org/repo/.github/workflows/x.yml@0123456789012345678901234567890123456789  # v1\n'), 'reusable workflow calls'),
    ('npm publish loses environment', 'typescript-npm-publish.yml', replace_once('typescript-npm-publish.yml', '    environment: publish-npm\n', ''), 'outside environment'),
    ('npm publish gains contents write', 'typescript-npm-publish.yml', replace_once('typescript-npm-publish.yml', '      contents: read\n      actions: read\n      id-token: write\n', '      contents: write\n      actions: read\n      id-token: write\n'), JOB),
    ('go PR job gains a scope', 'go-bindings.yml', replace_once('go-bindings.yml', '      pull-requests: write\n', '      pull-requests: write\n      actions: write\n'), JOB),
    ('go PR job run reads secrets', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', '      - run: echo ${{ secrets.GITHUB_TOKEN }}\n'), TOKEN),
    ('go PR job step env github.token', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', '      - run: make x\n        env:\n          GH_TOKEN: ${{ github.token }}\n'), TOKEN),
    ('go PR job env secrets', 'go-bindings.yml', insert_job_key('go-bindings.yml', 'update-go-bindings', '    env:\n      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}\n'), TOKEN),
    ('go PR job bracket secret', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', "      - run: echo ${{ secrets['GITHUB_TOKEN'] }}\n"), TOKEN),
    ('go PR job bracket github token', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', "      - run: echo ${{ github['token'] }}\n"), TOKEN),
    ('go PR job serialised github', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', "      - run: echo '${{ toJSON(github) }}'\n"), TOKEN),
    ('go PR job serialised secrets', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', "      - run: echo '${{ toJSON(secrets) }}'\n"), TOKEN),
    ('go PR job format() secrets', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', "      - run: echo ${{ format('{0}', secrets.GITHUB_TOKEN) }}\n"), TOKEN),
    ('go PR job format() github token', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', "      - run: echo ${{ format('{0}}', github.token) }}\n"), TOKEN),
    ('go PR job secrets||empty', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', "      - run: echo '${{ toJSON(secrets||'') }}'\n"), TOKEN),
    ('go PR job github||0', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', "      - run: echo '${{ toJSON(github||0) }}'\n"), TOKEN),
    ('go PR job secrets&&1', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', "      - run: echo '${{ toJSON(secrets&&1) }}'\n"), TOKEN),
    ('go PR job github!=1', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', "      - run: echo '${{ toJSON(github!=1) }}'\n"), TOKEN),
    ('go PR job format(secrets||0)', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', "      - run: echo ${{ format('{0}', secrets||0) }}\n"), TOKEN),
    ('go PR job format(toJSON(secrets||empty))', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', "      - run: echo ${{ format('{0}', toJSON(secrets||'')) }}\n"), TOKEN),
    ('go PR job secrets via env of step with', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', "      - run: make x\n        env:\n          T: ${{ toJSON(secrets) }}\n"), TOKEN),
    ('go PR job environment.url reads secrets', 'go-bindings.yml', insert_job_key('go-bindings.yml', 'update-go-bindings', '    environment:\n      name: x\n      url: ${{ secrets.GITHUB_TOKEN }}\n'), TOKEN),
    ('go PR job concurrency reads secrets', 'go-bindings.yml', insert_job_key('go-bindings.yml', 'update-go-bindings', '    concurrency: ${{ secrets.GITHUB_TOKEN }}\n'), TOKEN),
    ('go PR job timeout-minutes reads github', 'go-bindings.yml', insert_job_key('go-bindings.yml', 'update-go-bindings', '    timeout-minutes: ${{ github.token }}\n'), TOKEN),
    ('go PR job continue-on-error reads secrets', 'go-bindings.yml', insert_job_key('go-bindings.yml', 'update-go-bindings', '    continue-on-error: ${{ secrets.GITHUB_TOKEN }}\n'), TOKEN),
    ('go PR job create-pull-request with.title reads token', 'go-bindings.yml', replace_once('go-bindings.yml', '          title: "feat(bindings): update Go prebuilt libs for v${{ needs.validate.outputs.version }}"\n', '          title: "${{ secrets.GITHUB_TOKEN }}"\n'), TOKEN),
    ('go PR job create-pull-request with.body-path reads secrets', 'go-bindings.yml', replace_once('go-bindings.yml', '          delete-branch: true\n', '          delete-branch: true\n          body-path: ${{ toJSON(secrets) }}\n'), TOKEN),
    ('go PR job create-pull-request with.token altered', 'go-bindings.yml', replace_once('go-bindings.yml', '          token: ${{ secrets.GITHUB_TOKEN }}\n', '          token: ${{ toJSON(secrets) }}\n'), TOKEN),
    ('go PR job second create-pull-request step', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', '      - uses: peter-evans/create-pull-request@5f6978faf089d4d20b00c7766989d076bb2fc7f1  # v8\n        with:\n          token: ${{ secrets.GITHUB_TOKEN }}\n'), 'multiple token-receiving steps'),
    ('go PR job run step invokes curl', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', '      - run: curl -fsSL https://example.com\n'), 'forbidden command in a write-job run step'),
    ('go PR job run step invokes git push', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', '      - run: git push origin HEAD\n'), 'forbidden command in a write-job run step'),
    ('go PR job run step git continuation push', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', '      - run: |\n          git \\\n            push origin HEAD\n'), 'forbidden command in a write-job run step'),
    ('workflow env inherited by writer', 'go-bindings.yml', replace_once('go-bindings.yml', "  CARGO_TERM_COLOR: always\n", "  CARGO_TERM_COLOR: always\n  GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}\n"), TOKEN),
    ('workflow non-env key inherited by writer', 'go-bindings.yml', replace_once('go-bindings.yml', top, top + 'run-name: ${{ github.token }}\n'), TOKEN),
    ('go PR job dict key reads token', 'go-bindings.yml', insert_job_key('go-bindings.yml', 'update-go-bindings', '    env:\n      "${{ github.token }}": x\n'), TOKEN),
    ('go PR job bare function name as identifier', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', '      - run: echo ${{ format }}\n'), TOKEN),
    ('read-only job env reads secret PAT', 'ci.yml', writer_job('echo ok\n    env:\n      GH_TOKEN: ${{ secrets.RELEASE_PAT }}'), TOKEN),
    ('read-only step with reads secret PAT', 'ci.yml', writer_job('echo ok\n        with:\n          token: ${{ secrets.RELEASE_PAT }}'), TOKEN),
    ('read-only workflow top-level env reads secret', 'ci.yml', replace_once('ci.yml', "  CARGO_TERM_COLOR: always\n", "  CARGO_TERM_COLOR: always\n  GH_TOKEN: ${{ secrets.RELEASE_PAT }}\n"), TOKEN),
    ('read-only workflow run-name reads secret', 'ci.yml', replace_once('ci.yml', top, top + 'run-name: ${{ secrets.RELEASE_PAT }}\n'), TOKEN),
    ('read-only job concurrency reads secret', 'release.yml', insert_job_key('release.yml', rel, '    concurrency: ${{ secrets.RELEASE_PAT }}\n'), TOKEN),
    ('read-only job dict key reads secret', 'release.yml', insert_job_key('release.yml', rel, '    env:\n      "${{ secrets.RELEASE_PAT }}": x\n'), TOKEN),
    ('read-only job list value reads secret', 'release.yml', insert_job_key('release.yml', rel, '    needs:\n      - ${{ secrets.RUNNER }}\n'), TOKEN),
    ('read-only step uppercase SECRETS context', 'ci.yml', writer_job('echo ${{ SECRETS.RELEASE_PAT }}'), TOKEN),
    ('go PR job checkout persists credentials', 'go-bindings.yml', replace_once('go-bindings.yml', '          fetch-depth: 0\n          persist-credentials: false\n      - name: Download all FFI artifacts', '          fetch-depth: 0\n      - name: Download all FFI artifacts'), 'persist-credentials: false'),
    ('go PR job token to download action', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', '      - uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c  # v8\n        with:\n          github-token: ${{ secrets.GITHUB_TOKEN }}\n'), TOKEN),
    ('go PR job unlisted action', 'go-bindings.yml', append_step('go-bindings.yml', 'update-go-bindings', '      - uses: actions/github-script@0123456789012345678901234567890123456789  # v7\n'), 'not allowed in a write job'),
    ('release action', 'release.yml', replace_once('release.yml', '\njobs:\n', '\njobs:\n  writer:\n    runs-on: ubuntu-24.04\n    steps:\n      - uses: softprops/action-gh-release@3bb12739c298aeb8a4eeaf626c5b8d85266b0e65  # v2\n'), WRITER),
    ('gh release create', 'ci.yml', writer_job('gh release create v1.2.3'), WRITER),
    ('gh release flag before create', 'ci.yml', writer_job('gh release --repo 3leaps/sysprims create v9.9.9 --draft'), WRITER),
    ('gh -R before release create', 'ci.yml', writer_job('gh -R 3leaps/sysprims release create v9.9.9 --draft'), WRITER),
    ('gh release continuation line', 'ci.yml', writer_job('|\n          gh release \\\n            create v9.9.9 --draft'), WRITER),
    ('gh release variable subcommand', 'ci.yml', writer_job('sub=create; gh release $sub v9.9.9'), WRITER),
    ('gh alias set', 'ci.yml', writer_job("gh alias set rel 'release create'"), WRITER),
    ('gh api graphql createRelease', 'ci.yml', writer_job("gh api graphql -f query='mutation { createRelease }'"), WRITER),
    ('gh release upload', 'ci.yml', writer_job('gh release upload v1.2.3 x'), WRITER),
    ('gh api implicit POST', 'ci.yml', writer_job('gh api repos/o/r/releases -f tag_name=v1.2.3 -F draft=true'), WRITER),
    ('gh api continuation POST', 'ci.yml', writer_job('|\n          gh api repos/o/r/releases \\\n            -X POST'), WRITER),
    ('gh api --method=POST', 'ci.yml', writer_job('gh api --method=POST repos/o/r/releases'), WRITER),
    ('gh api --input', 'ci.yml', writer_job('gh api repos/o/r/releases --input body.json'), WRITER),
    ('curl --request POST', 'ci.yml', writer_job('curl --request POST https://api.github.com/repos/o/r/releases'), WRITER),
    ('curl -X POST', 'ci.yml', writer_job('curl -X POST https://api.github.com/repos/o/r/releases'), WRITER),
    ('curl continuation -X POST', 'ci.yml', writer_job('|\n          curl -X POST \\\n            https://api.github.com/repos/o/r/releases'), WRITER),
    ('curl --data', 'ci.yml', writer_job('curl https://api.github.com/repos/o/r/releases --data @b.json'), WRITER),
]
for label, name, text, expected in controls:
    found = check_text(name, text)
    if not any(expected in error for error in found):
        raise SystemExit(f'error: permission mutation control did not fail as intended: {label} (expected {expected!r}, got {found})')
print(f'[ok] {len(texts)} workflows read-only; writes only on reviewed jobs; {len(controls)} raw-text controls refused')
PY
