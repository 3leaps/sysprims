#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Verify that every workflow and local action matches its reviewed SHA-256 pin.

Every file under `.github/workflows/` and `.github/actions/` is listed in
`config/release/release-workflow.sha256`. Any workflow can run on a tag push
(for example via a plain `on: push`, `on: [push]`, a path filter, or `create`),
and a local composite action is code a workflow executes without a commit SHA
in `uses:`, so the pin covers both directories rather than guessing from
triggers. Every workflow must also declare read-only top-level permissions
including `contents: read`, and every job must be read-only except the two
reviewed workflow_dispatch-only jobs (`typescript-npm-publish.yml` `publish`
and `go-bindings.yml` `update-go-bindings`).

Modes:
  check                 verify the working tree against config/release/release-workflow.sha256
  verify-commit COMMIT  verify the committed tree at COMMIT against COMMIT:config/release/release-workflow.sha256
"""

import hashlib
import re
import subprocess
import sys
from pathlib import Path

# Keep the script directory off sys.path before importing PyYAML so a stray
# scripts/yaml.py cannot shadow the installed module.
sys.path = [
    entry
    for entry in sys.path
    if Path(entry or ".").resolve() != Path(__file__).resolve().parent
]
import yaml  # noqa: E402

PIN_PATH = "config/release/release-workflow.sha256"
PINNED_DIRS = (".github/workflows", ".github/actions")
LINE = re.compile(
    r"^([0-9a-f]{64})  (\.github/(?:workflows|actions)/[A-Za-z0-9._/-]+\.ya?ml)$"
)
RELEASE_MUTATION = re.compile(
    r"(?i)softprops/action-gh-release|ncipollo/release-action|"
    r"actions/create-release|actions/upload-release-asset|"
    r"\bgh\s+release\b|\bgh\s+api\b[^\n]*releases|"
    r"api\.github\.com/repos/[^\s\"']*/releases|\bcreateRelease\b"
)
# Only these two reviewed workflow_dispatch-only jobs may request write scopes:
# - typescript-npm-publish.yml publish: id-token: write behind publish-npm
# - go-bindings.yml update-go-bindings: contents + pull-requests write to open
#   the prebuilt-library PR
ALLOWED_JOB_PERMISSIONS = {
    (".github/workflows/typescript-npm-publish.yml", "publish"): (
        {"contents": "read", "actions": "read", "id-token": "write"},
        "publish-npm",
    ),
    (".github/workflows/go-bindings.yml", "update-go-bindings"): (
        {"contents": "write", "pull-requests": "write"},
        None,
    ),
}
WORKFLOW_DISPATCH_ONLY = {path for path, _ in ALLOWED_JOB_PERMISSIONS}


def fail(reason: str) -> None:
    print(f"error: {reason}", file=sys.stderr)
    sys.exit(1)


def parse_pins(text: str, source: str) -> dict[str, str]:
    pins: dict[str, str] = {}
    if not text or not text.endswith("\n"):
        fail(f"{source} must be non-empty and end with a newline")
    for raw in text.splitlines():
        match = LINE.match(raw)
        if not match:
            fail(f"malformed line in {source}: {raw!r}")
        digest, path = match.groups()
        if ".." in Path(path).parts or path in pins:
            fail(f"invalid or duplicate workflow path in {source}: {path}")
        pins[path] = digest
    if list(pins) != sorted(pins):
        fail(f"{source} must list workflows in lexicographic order")
    if ".github/workflows/release.yml" not in pins:
        fail(f"{source} must pin .github/workflows/release.yml")
    return pins


def require_read_only(rel: str, content: bytes) -> None:
    try:
        text = content.decode("utf-8")
        doc = yaml.safe_load(text)
    except (UnicodeDecodeError, yaml.YAMLError) as error:
        fail(f"{rel} is not valid UTF-8 YAML: {error}")
    if not isinstance(doc, dict):
        fail(f"{rel} must parse to a YAML mapping")
    permissions = doc.get("permissions")
    if (
        not isinstance(permissions, dict)
        or permissions.get("contents") != "read"
        or any(level != "read" for level in permissions.values())
    ):
        fail(
            f"{rel} must declare read-only top-level permissions including contents: read"
        )
    if rel in WORKFLOW_DISPATCH_ONLY:
        on_field = doc.get("on", doc.get(True))
        triggers = (
            list(on_field)
            if isinstance(on_field, dict)
            else ([on_field] if isinstance(on_field, str) else [])
        )
        if triggers != ["workflow_dispatch"]:
            fail(f"{rel} must trigger on workflow_dispatch only")
    jobs = doc.get("jobs")
    if not isinstance(jobs, dict):
        fail(f"{rel} must declare jobs")
    for name, job in jobs.items():
        if not isinstance(job, dict):
            fail(f"{rel} job {name} must be a mapping")
        job_permissions = job.get("permissions")
        if job_permissions is None:
            continue
        allowed = ALLOWED_JOB_PERMISSIONS.get((rel, name))
        if allowed is not None:
            expected_permissions, expected_env = allowed
            environment = job.get("environment")
            env_name = (
                environment.get("name")
                if isinstance(environment, dict)
                else environment
            )
            if job_permissions == expected_permissions and (
                expected_env is None or env_name == expected_env
            ):
                continue
        if not isinstance(job_permissions, dict) or any(
            v not in ("read", "none") for v in job_permissions.values()
        ):
            fail(f"{rel} job {name} grants more than read permissions")
    if rel == ".github/workflows/release.yml" and RELEASE_MUTATION.search(text):
        fail(f"{rel} contains a GitHub Release mutation token")


def verify(pins: dict[str, str], files: dict[str, bytes], label: str) -> None:
    for rel in sorted(set(pins) | set(files)):
        if rel not in pins:
            fail(f"{rel} is not pinned in {PIN_PATH}")
        if rel not in files:
            fail(f"pinned {rel} is missing from {label}")
        actual = hashlib.sha256(files[rel]).hexdigest()
        if actual != pins[rel]:
            fail(
                f"publishing workflow in {label} differs from its reviewed digest: {rel}"
            )
        if rel.startswith(".github/workflows/"):
            require_read_only(rel, files[rel])


def check_working_tree(root: Path) -> None:
    pin_file = root / PIN_PATH
    if not pin_file.is_file() or pin_file.is_symlink():
        fail(f"{PIN_PATH} is missing")
    pins = parse_pins(pin_file.read_text(encoding="utf-8"), PIN_PATH)
    files: dict[str, bytes] = {}
    for directory in PINNED_DIRS:
        base = root / directory
        if not base.exists():
            continue
        for path in sorted(base.rglob("*")):
            if path.is_symlink():
                fail(f"symlink under {directory} is not allowed: {path}")
            if path.is_file() and path.suffix in (".yml", ".yaml"):
                files[path.relative_to(root).as_posix()] = path.read_bytes()
    verify(pins, files, "the working tree")
    print(f"[ok] {len(pins)} pinned workflow/action file(s); every workflow matches")


def git_bytes(root: Path, args: list[str]) -> bytes:
    result = subprocess.run(["git", *args], cwd=root, capture_output=True, check=False)
    if result.returncode != 0:
        fail(result.stderr.decode("utf-8", errors="replace").strip() or "git failed")
    return result.stdout


def verify_commit(root: Path, commit: str) -> None:
    if not re.match(r"^[0-9a-f]{40}$", commit):
        resolved = (
            git_bytes(root, ["rev-parse", "--verify", f"{commit}^{{commit}}"])
            .decode()
            .strip()
        )
        commit = resolved
    pin_entry = (
        git_bytes(root, ["ls-tree", commit, "--", PIN_PATH]).decode().splitlines()
    )
    if len(pin_entry) != 1 or pin_entry[0].split()[0] != "100644":
        fail(f"committed {PIN_PATH} is required in the tagged commit")
    pins = parse_pins(
        git_bytes(root, ["cat-file", "blob", f"{commit}:{PIN_PATH}"]).decode("utf-8"),
        f"{commit}:{PIN_PATH}",
    )
    tree = (
        git_bytes(root, ["ls-tree", "-r", commit, "--", *PINNED_DIRS])
        .decode("utf-8")
        .splitlines()
    )
    files: dict[str, bytes] = {}
    for line in tree:
        meta, rel = line.split("\t", 1)
        mode, kind, _ = meta.split()
        if mode != "100644" or kind != "blob":
            fail(f"non-regular file under .github in {commit}: {rel}")
        if rel.endswith((".yml", ".yaml")):
            files[rel] = git_bytes(root, ["cat-file", "blob", f"{commit}:{rel}"])
    verify(pins, files, "the tagged commit")


def main() -> None:
    root = Path.cwd()
    args = sys.argv[1:]
    if args == ["check"]:
        check_working_tree(root)
    elif len(args) == 2 and args[0] == "verify-commit":
        verify_commit(root, args[1])
    else:
        fail("usage: release-workflow-pin.py <check|verify-commit COMMIT>")


if __name__ == "__main__":
    main()
