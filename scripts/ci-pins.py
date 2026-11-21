#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Synchronize and verify DRY GitHub Actions runner labels and Rust toolchain pins.

`config/ci/pins.toml` is the single source of truth for:
  - `[runners.hosted]`: explicit GitHub-hosted runner images (for example `ubuntu-24.04`)
  - `[runners].estate_allowlist`: explicit estate-managed runner labels
  - `[rust]`: exact `X.Y.Z` Rust toolchain versions (`msrv`, `stable`)

Workflow lines opt into single-source synchronization with trailing comments:
  - `# pin:runner=<hosted-key>`
  - `# pin:rust=<rust-key>`

Modes:
  sync   propagate `config/ci/pins.toml` to all `# pin:...` lines in
         `.github/workflows/*.yml` and regenerate
         `config/release/release-workflow.sha256`
  check  verify `config/ci/pins.toml` against `Cargo.toml`, verify every
         workflow runner label (including `matrix` expansions) and Rust
         toolchain step is governed by `pins.toml`, and refuse moving runner
         labels, third-party Rust toolchain actions, floating channels, or
         known Node 20 action pins; refuse ${{ }} expressions in workflow
         and local composite action run steps (values reach the shell only
         through env:); require every run line that plainly reaches the
         network, installs a tool, carries a 64-hex digest or calls a
         release-tool installer to appear exactly in
         `config/ci/reviewed-run-lines.txt`
         (and every entry there to still be used, with installers committed
         as executable `scripts/ci-install-<tool>.sh`); and refuse URLs,
         download commands, digests and shell-startup keys in env values.
         The run-line rules target an honest maintainer's mistakes; the
         boundary against a hostile workflow author is the per-file workflow
         digest and review
"""

from __future__ import annotations

import os
import sys

# Keep this script's directory and the working directory off sys.path before
# anything else is imported, so a module planted beside this script (or in the
# working directory) is never imported; only the standard library and the
# workflow reader loaded by path below run.
_untrusted = {
    os.path.realpath(p)
    for p in (os.path.dirname(os.path.abspath(__file__)), os.getcwd())
}
sys.path[:] = [
    p for p in sys.path if p not in ("", ".") and os.path.realpath(p) not in _untrusted
]

import errno  # noqa: E402
import hashlib  # noqa: E402
import importlib.util  # noqa: E402
import re  # noqa: E402
import tomllib  # noqa: E402
from pathlib import Path  # noqa: E402

# The workflow reader: scripts/workflow_yaml.py, which runs the pinned yamlprims
# CLI through scripts/yaml_json.py (standard library only).
SCRIPT_DIR = Path(__file__).resolve().parent
_wy_spec = importlib.util.spec_from_file_location(
    "sysprims_workflow_yaml", SCRIPT_DIR / "workflow_yaml.py"
)
wy = importlib.util.module_from_spec(_wy_spec)
_wy_spec.loader.exec_module(wy)

PINS_REL = "config/ci/pins.toml"
WORKFLOW_SHA_REL = "config/release/release-workflow.sha256"
SETUP_RUST_REL = ".github/actions/setup-rust/action.yml"

SEMVER_RE = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
MARKER_COMMENT_RE = re.compile(r"#\s*pin:(runner|rust)=([A-Za-z0-9_-]+)\s*$")
SYNC_LINE_RE = re.compile(
    r"^(?P<prefix>.*?(?:[:=]\s*|-\s+))"
    r"(?P<token>(?P<quote>['\"]?)(?P<val>[A-Za-z0-9._-]+)(?P=quote))"
    r"(?P<ws>\s+)#\s*pin:(?P<kind>runner|rust)=(?P<key>[A-Za-z0-9_-]+)\s*$"
)
MATRIX_EXPR_RE = re.compile(r"^\$\{\{\s*matrix\.([A-Za-z0-9_-]+)\s*\}\}$")

# Known deprecated Node 20 action commit pins.
NODE20_DENYLIST = {
    "actions/checkout@11d5960a326750d5838078e36cf38b85af677262",
    "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02",
}

FLOATING_RUST_RE = re.compile(
    r"(?:--default-toolchain(?:\s+|=)['\"]?(?:stable|beta|nightly)(?:-[A-Za-z0-9._-]+)?['\"]?(?=$|[\s;|&)])|"
    r"\brustup\s+(?:toolchain\s+install|default|install|run)\s+['\"]?(?:stable|beta|nightly)(?:-[A-Za-z0-9._-]+)?['\"]?(?=$|[\s;|&)])|"
    r"(?:^|[\s;|&(])cargo\s+(?:['\"]\s*)?\+['\"]?(?:stable|beta|nightly)(?:-[A-Za-z0-9._-]+)?['\"]?(?=$|[\s;|&)])|"
    r"\bRUSTUP_TOOLCHAIN=['\"]?(?:stable|beta|nightly)(?:-[A-Za-z0-9._-]+)?['\"]?(?=$|[\s;|&)]))"
)
DIRECT_RUSTUP_RE = re.compile(
    r"(?:^|[;&|\s])rustup\s+(?:toolchain|target|component|default|show|install|update)\b"
)


class PinError(Exception):
    pass


# Run-line review. This gate targets an honest maintainer's mistakes, not an
# adversary who already has workflow write access (that boundary is the
# per-file workflow digest, required review and branch protection). Rules:
#   - Workflow and local composite run steps carry no ${{ }} expressions;
#     values reach the shell only through env:.
#   - A run line that plainly reaches the network, installs a tool, carries a
#     64-hex digest or calls a release-tool installer must equal a line in
#     REVIEWED_LINES_REL (continuations joined, whitespace collapsed).
#   - Env values are plain tokens with no URL, download command or digest,
#     and keys that change shell startup are refused.
# Release tools are installed only by scripts/ci-install-<tool>.sh from
# committed pin files; CI runs those installers on every pull request, so a
# missing or wrong pin fails there for every workflow that reads it.
REVIEWED_LINES_REL = "config/ci/reviewed-run-lines.txt"
SENSITIVE_RE = re.compile(
    r"(?i)(?:\b(?:https?|ftps?|sftp|s3|gs|hkps?)://|releases/download/"
    r"|\b(?:curl|wget|aria2c|scp|sftp|rsync|ssh|nc|ncat|socat|invoke-webrequest"
    r"|invoke-restmethod|iwr|irm|start-bitstransfer|bitsadmin)\b"
    r"|\bgit\b[^\n]*\b(?:clone|fetch|pull|push|submodule|ls-remote|archive)\b|\bgh\b"
    r"|\bcargo\b[^\n]*\b(?:install|binstall|update|fetch|publish|add)\b"
    r"|\bgo\b[^\n]*\b(?:install|get|run)\b|\bgo\b[^\n]*\bmod\b[^\n]*\b(?:download|tidy|vendor)\b"
    r"|\brustup\b|\b(?:docker|podman|nerdctl|npx|uvx|pipx|corepack|dotnet|snap|brew|gem"
    r"|choco|winget|scoop|conda|mamba|dnf|yum|zypper|pacman|apk|svn|hg)\b"
    r"|\b(?:apt|apt-get)\b[^\n]*\b(?:install|update|upgrade|download|source)\b"
    r"|(?:^|[;&|(]|\s)(?:pip|pip3|npm|pnpm|yarn)\s+(?!(?:run|ci|test|pack|freeze|--version|-v)\b)\S"
    # uv resolves and fetches on every subcommand but --version (uv run --with).
    r"|(?:^|[;&|(]|\s)uv\s+(?!--version\b)\S"
    r"|\bbun\b[^\n]*\b(?:install|i|add|x|create|update|upgrade|link)\b|\bbunx\b"
    r"|\b(?:install|save|update)-(?:module|script|package|psresource)\b"
    r"|--keyserver\b|--recv-keys?\b"
    r"|ci-install|(?<![0-9A-Fa-f])[0-9A-Fa-f]{64}(?![0-9A-Fa-f]))"
)
# The shells a step (or defaults.run) may name.
ALLOWED_SHELLS = frozenset({"bash", "sh", "pwsh", "msys2 {0}"})
# A literal env value is a plain token: no quoting, spaces or shell
# metacharacters, so it cannot change the shape of a line it expands into.
ENV_LITERAL_RE = re.compile(r"^[A-Za-z0-9._/+:=@%,-]*$")
HEX64_RE = re.compile(r"(?<![0-9A-Fa-f])[0-9A-Fa-f]{64}(?![0-9A-Fa-f])")
CI_INSTALL_NAME_RE = re.compile(r"ci-install-[A-Za-z0-9_-]+\.sh")
CI_INSTALL_PATH_RE = re.compile(r"^(?:\./)?scripts/(ci-install-[A-Za-z0-9_-]+\.sh)$")
CONTAINER_OPTIONS_RE = re.compile(r"(?:--user[ =][A-Za-z0-9_][A-Za-z0-9_.:-]*)?")
# Variables that make a shell run other code before or around a step.
SHELL_STARTUP_ENV = frozenset(
    {
        "BASH_ENV",
        "ENV",
        "SHELLOPTS",
        "BASHOPTS",
        "PROMPT_COMMAND",
        "PS4",
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "IFS",
        "CDPATH",
        "PATH",
    }
)


def logical_lines(text: str, shell: str | None = None) -> list[str]:
    """Run text as logical lines: a trailing backslash (bash, sh) or backtick
    (PowerShell, joined with a space) continues a line; whitespace collapsed;
    blank lines dropped."""
    pwsh = (shell or "").split(" ")[0].lower() in ("pwsh", "powershell")
    char = "`" if pwsh else "\\"
    out: list[str] = []
    pending = ""
    for raw in text.splitlines():
        raw = raw.rstrip("\r")
        if raw.endswith(char):
            pending += raw[:-1] + (" " if pwsh else "")
            continue
        line = " ".join((pending + raw).split())
        pending = ""
        if line:
            out.append(line)
    if pending.strip():
        out.append(" ".join(pending.split()))
    return out


def load_reviewed_lines(root: Path) -> frozenset[str]:
    path = _check_rel_no_symlinks(root, REVIEWED_LINES_REL)
    if not path.is_file():
        raise PinError(f"missing {REVIEWED_LINES_REL}")
    lines = set()
    for raw in path.read_text(encoding="utf-8").splitlines():
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        line = " ".join(raw.split())
        if line in lines:
            raise PinError(f"{REVIEWED_LINES_REL}: duplicate line: {line[:80]}")
        lines.add(line)
    return frozenset(lines)


def check_shell(shell: str | None, where: str) -> list[str]:
    if shell is None or shell in ALLOWED_SHELLS:
        return []
    return [f"{where}: shell {shell!r} is not one of {sorted(ALLOWED_SHELLS)}"]


def check_env_values(env: dict[str, str], where: str) -> list[str]:
    """Env values are plain tokens (or ${{ }} expressions) with no URL,
    download command or 64-hex digest; shell-startup keys are refused."""
    errors = []
    for key, value in env.items():
        if key in SHELL_STARTUP_ENV or key.startswith("BASH_FUNC_"):
            errors.append(
                f"{where}: env.{key} changes what the shell runs before a step; it is not allowed"
            )
        if "${{" not in value and not ENV_LITERAL_RE.fullmatch(value):
            errors.append(
                f"{where}: env.{key} must be a plain token (no spaces, quotes or shell metacharacters) or a ${{{{ }}}} expression"
            )
        if HEX64_RE.search(value):
            errors.append(
                f"{where}: env.{key} carries an inline SHA-256; keep digests in a committed pin file"
            )
        elif SENSITIVE_RE.search(value):
            errors.append(
                f"{where}: env.{key} carries a URL or download command; fetch release tools with scripts/ci-install-<tool>.sh"
            )
    return errors


def check_run_text(
    run_cmd: str,
    where: str,
    reviewed: frozenset[str],
    used: set[str] | None = None,
    shell: str | None = None,
) -> list[str]:
    """Refuse run-step expressions and unreviewed sensitive lines."""
    errors: list[str] = []
    if "${{" in run_cmd:
        errors.append(
            f"{where}: ${{{{ }}}} expression in a run step; pass the value through env:"
        )
    for line in logical_lines(run_cmd, shell):
        if not SENSITIVE_RE.search(line):
            continue
        if used is not None:
            used.add(line)
        if line not in reviewed:
            errors.append(
                f"{where}: line is not in {REVIEWED_LINES_REL} (network, install, digest or installer): {line[:120]}"
            )
    return errors


def check_reviewed_lines_used(reviewed: frozenset[str], used: set[str]) -> list[str]:
    """Every reviewed line still appears in some run step."""
    return [
        f"{REVIEWED_LINES_REL}: line no longer appears in any run step: {line[:80]}"
        for line in sorted(reviewed - used)
    ]


def check_reviewed_installers(root: Path, reviewed: frozenset[str]) -> list[str]:
    """Each reviewed line that names an installer names it exactly as
    scripts/ci-install-<tool>.sh (optionally ./), committed in HEAD as an
    executable regular file."""
    errors = []
    committed = _committed_installers(root)
    for line in sorted(reviewed):
        if "ci-install" not in line.lower():
            continue
        tokens = [tok for tok in line.split() if "ci-install" in tok.lower()]
        for token in tokens:
            match = CI_INSTALL_PATH_RE.fullmatch(token)
            if match is None or match.group(1) not in committed:
                errors.append(
                    f"{REVIEWED_LINES_REL}: installer {token[:80]} is not scripts/ci-install-<tool>.sh committed in HEAD as an executable regular file"
                )
    return errors


def _committed_installers(root: Path) -> set[str]:
    """Names of scripts/ci-install-*.sh committed in HEAD as regular executables."""
    import subprocess

    try:
        listing = subprocess.run(
            ["git", "-C", str(root), "ls-tree", "HEAD", "--", "scripts/"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return set()
    names = set()
    for row in listing.splitlines():
        meta, _, path = row.partition("\t")
        parts = meta.split()
        name = path.rpartition("/")[2]
        if (
            len(parts) == 3
            and parts[0] == "100755"
            and parts[1] == "blob"
            and CI_INSTALL_NAME_RE.fullmatch(name)
        ):
            names.add(name)
    return names


def _check_rel_no_symlinks(root: Path, rel: str) -> Path:
    current = root
    if current.is_symlink():
        raise PinError(f"symlink root is not allowed: {current}")
    for part in Path(rel).parts:
        current = current / part
        if current.is_symlink():
            raise PinError(f"symlink is not allowed: {rel}")
    return current


def validate_repo_tree_no_symlinks(root: Path) -> None:
    if root.is_symlink():
        raise PinError(f"symlink root is not allowed: {root}")
    for rel in (
        "Cargo.toml",
        "config",
        "config/ci",
        PINS_REL,
        REVIEWED_LINES_REL,
        "config/release",
        WORKFLOW_SHA_REL,
        ".github",
        ".github/actions",
        SETUP_RUST_REL,
        ".github/workflows",
    ):
        _check_rel_no_symlinks(root, rel)

    for sub in (".github/actions", ".github/workflows"):
        base = root / sub
        if not base.exists():
            continue
        for dirpath, dirnames, filenames in os.walk(base, followlinks=False):
            parent = Path(dirpath)
            if parent.is_symlink():
                rel_p = parent.relative_to(root).as_posix()
                raise PinError(f"symlink under {sub} is not allowed: {rel_p}")
            for name in list(dirnames) + list(filenames):
                candidate = parent / name
                if candidate.is_symlink():
                    rel_c = candidate.relative_to(root).as_posix()
                    raise PinError(f"symlink under {sub} is not allowed: {rel_c}")


def _write_text_nofollow(root: Path, rel: str, content: str) -> None:
    target = _check_rel_no_symlinks(root, rel)
    parent_rel = Path(rel).parent.as_posix()
    if parent_rel and parent_rel != ".":
        parent = _check_rel_no_symlinks(root, parent_rel)
        parent.mkdir(parents=True, exist_ok=True)
        _check_rel_no_symlinks(root, parent_rel)
    flags = os.O_WRONLY | os.O_CREAT | os.O_TRUNC | getattr(os, "O_NOFOLLOW", 0)
    data = content.encode("utf-8")
    try:
        fd = os.open(target, flags, 0o644)
    except OSError as exc:
        if exc.errno == errno.ELOOP:
            raise PinError(f"symlink is not allowed: {rel}") from None
        raise PinError(f"cannot write {rel}: {exc}") from None
    try:
        written = 0
        while written < len(data):
            written += os.write(fd, data[written:])
    finally:
        os.close(fd)


def load_pins(root: Path) -> dict:
    pins_path = _check_rel_no_symlinks(root, PINS_REL)
    if not pins_path.is_file():
        raise PinError(f"missing {PINS_REL}")
    try:
        data = tomllib.loads(pins_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise PinError(f"invalid {PINS_REL}: {error}") from None

    runners = data.get("runners")
    if not isinstance(runners, dict):
        raise PinError(f"{PINS_REL}: [runners] table is required")
    hosted = runners.get("hosted")
    if not isinstance(hosted, dict) or not hosted:
        raise PinError(f"{PINS_REL}: [runners.hosted] must be a non-empty table")
    for key, label in hosted.items():
        if not isinstance(label, str) or not label.strip():
            raise PinError(
                f"{PINS_REL}: runners.hosted.{key} must be a non-empty string"
            )
        if label.endswith("-latest") or "${{" in label:
            raise PinError(
                f"{PINS_REL}: runners.hosted.{key} uses a moving or expression label: {label!r}"
            )

    allowlist = runners.get("estate_allowlist", [])
    if not isinstance(allowlist, list) or not all(
        isinstance(item, str) and item.strip() for item in allowlist
    ):
        raise PinError(
            f"{PINS_REL}: runners.estate_allowlist must be a list of strings"
        )

    overlap = set(hosted.values()) & set(allowlist)
    if overlap:
        raise PinError(
            f"{PINS_REL}: hosted labels and estate_allowlist overlap: {sorted(overlap)}"
        )

    rust = data.get("rust")
    if not isinstance(rust, dict) or not rust:
        raise PinError(f"{PINS_REL}: [rust] must be a non-empty table")
    for key in ("msrv", "stable"):
        if key not in rust:
            raise PinError(f"{PINS_REL}: rust.{key} is required")
    for key, version in rust.items():
        if not isinstance(version, str) or not SEMVER_RE.fullmatch(version):
            raise PinError(
                f"{PINS_REL}: rust.{key} must be an exact X.Y.Z version, got {version!r}"
            )

    cargo_toml = _check_rel_no_symlinks(root, "Cargo.toml")
    if cargo_toml.is_file():
        try:
            cargo_data = tomllib.loads(cargo_toml.read_text(encoding="utf-8"))
        except tomllib.TOMLDecodeError as error:
            raise PinError(f"Cargo.toml: {error}") from None
        workspace_msrv = (
            cargo_data.get("workspace", {}).get("package", {}).get("rust-version")
        )
        if workspace_msrv is not None and workspace_msrv != rust["msrv"]:
            raise PinError(
                f"{PINS_REL} rust.msrv ({rust['msrv']}) does not match "
                f"Cargo.toml workspace.package.rust-version ({workspace_msrv})"
            )

    return {
        "hosted": dict(hosted),
        "allowlist": set(allowlist),
        "rust": dict(rust),
        "reviewed": load_reviewed_lines(root),
        "used": set(),
    }


def _map_entries(node: wy.Node | None) -> dict[str, wy.Node]:
    if not isinstance(node, wy.MappingNode):
        return {}
    out: dict[str, wy.Node] = {}
    for k_node, v_node in node.value:
        if isinstance(k_node, wy.ScalarNode):
            out[k_node.value] = v_node
    return out


def _walk_scalars(node: wy.Node | None):
    if isinstance(node, wy.ScalarNode):
        yield node
    elif isinstance(node, wy.SequenceNode):
        for item in node.value:
            yield from _walk_scalars(item)
    elif isinstance(node, wy.MappingNode):
        for _, v_node in node.value:
            yield from _walk_scalars(v_node)


def _resolve_matrix_var_nodes(
    job_id: str, job_node: wy.MappingNode, var: str
) -> list[wy.ScalarNode]:
    job_fields = _map_entries(job_node)
    strategy_node = job_fields.get("strategy")
    if not isinstance(strategy_node, wy.MappingNode):
        raise PinError(f"job {job_id}: uses matrix.{var} without strategy.matrix")
    matrix_node = _map_entries(strategy_node).get("matrix")
    if not isinstance(matrix_node, wy.MappingNode):
        raise PinError(f"job {job_id}: strategy.matrix is not a mapping")
    matrix_fields = _map_entries(matrix_node)
    nodes: list[wy.ScalarNode] = []

    axis_node = matrix_fields.get(var)
    if axis_node is not None:
        if not isinstance(axis_node, wy.SequenceNode) or not axis_node.value:
            raise PinError(
                f"job {job_id}: strategy.matrix.{var} must be a non-empty list"
            )
        for item in axis_node.value:
            if not isinstance(item, wy.ScalarNode) or item.tag not in (
                "tag:yaml.org,2002:str",
            ):
                raise PinError(
                    f"job {job_id}: strategy.matrix.{var} entry is not a string"
                )
            nodes.append(item)

    include_node = matrix_fields.get("include")
    if include_node is not None:
        if not isinstance(include_node, wy.SequenceNode):
            raise PinError(f"job {job_id}: strategy.matrix.include must be a list")
        for entry_node in include_node.value:
            if not isinstance(entry_node, wy.MappingNode):
                raise PinError(
                    f"job {job_id}: strategy.matrix.include entry is not a mapping"
                )
            entry_fields = _map_entries(entry_node)
            if var in entry_fields:
                val_node = entry_fields[var]
                if not isinstance(val_node, wy.ScalarNode) or val_node.tag not in (
                    "tag:yaml.org,2002:str",
                ):
                    raise PinError(
                        f"job {job_id}: strategy.matrix.include.{var} is not a string"
                    )
                nodes.append(val_node)

    if not nodes:
        raise PinError(f"job {job_id}: matrix.{var} resolves to zero runner labels")
    return nodes


def check_workflow_text(
    name: str, text: str, pins: dict, root: Path | None = None
) -> list[str]:
    errors: list[str] = []
    try:
        doc = wy.load_text(text, name)
        wy.jobs(doc)
        root_node = wy.compose(text, name)
    except wy.WorkflowError as error:
        return [str(error)]

    if not isinstance(root_node, wy.MappingNode):
        return [f"{name}: workflow is not a mapping"]

    hosted = pins["hosted"]
    hosted_values = set(hosted.values())
    allowlist = pins["allowlist"]
    rust_pins = pins["rust"]
    rust_values = set(rust_pins.values())

    lines = text.splitlines()
    marked_runner_keys: dict[int, tuple[str, tuple[int, int]]] = {}
    marked_rust_keys: dict[int, tuple[str, tuple[int, int]]] = {}

    # 1. Validate every `# pin:...` marker line in raw text.
    for lineno, line in enumerate(lines, 1):
        stripped = line.lstrip()
        if stripped.startswith("#"):
            continue
        marker = MARKER_COMMENT_RE.search(line)
        if not marker:
            continue
        match = SYNC_LINE_RE.match(line)
        if not match:
            errors.append(
                f"{name}:{lineno}: malformed # pin marker line: {line.strip()}"
            )
            continue
        kind = match.group("kind")
        key = match.group("key")
        val = match.group("val")
        token_span = (match.start("token"), match.end("token"))
        if kind == "runner":
            marked_runner_keys[lineno] = (key, token_span)
            if key not in hosted:
                errors.append(f"{name}:{lineno}: unknown runner pin key {key!r}")
            elif val != hosted[key]:
                errors.append(
                    f"{name}:{lineno}: runner pin {key!r} is {val!r}, expected {hosted[key]!r} (run make ci-pins-sync)"
                )
        elif kind == "rust":
            marked_rust_keys[lineno] = (key, token_span)
            if key not in rust_pins:
                errors.append(f"{name}:{lineno}: unknown rust pin key {key!r}")
            elif val != rust_pins[key]:
                errors.append(
                    f"{name}:{lineno}: rust pin {key!r} is {val!r}, expected {rust_pins[key]!r} (run make ci-pins-sync)"
                )

    consumed_runner_lines: set[int] = set()
    consumed_rust_lines: set[int] = set()
    visited_runner_nodes: set[int] = set()

    def require_runner_marker(snode: wy.ScalarNode, where: str) -> None:
        visited_runner_nodes.add(id(snode))
        lineno = snode.start_mark.line + 1
        node_span = (snode.start_mark.column, snode.end_mark.column)
        if (
            snode.start_mark.line != snode.end_mark.line
            or snode.style not in (None, "", "'", '"')
            or lineno not in marked_runner_keys
            or marked_runner_keys[lineno][1] != node_span
        ):
            errors.append(
                f"{name}:{lineno}: {where} hosted runner {snode.value!r} lacks # pin:runner=<key> marker"
            )
            return
        if lineno in consumed_runner_lines:
            errors.append(
                f"{name}:{lineno}: {where} hosted runner {snode.value!r} shares # pin:runner marker line with another consumer"
            )
            return
        consumed_runner_lines.add(lineno)
        key, _ = marked_runner_keys[lineno]
        if key in hosted and hosted[key] != snode.value:
            errors.append(
                f"{name}:{lineno}: {where} value {snode.value!r} does not match # pin:runner={key} ({hosted[key]!r})"
            )

    def require_rust_marker(snode: wy.ScalarNode, where: str) -> None:
        lineno = snode.start_mark.line + 1
        node_span = (snode.start_mark.column, snode.end_mark.column)
        if (
            snode.start_mark.line != snode.end_mark.line
            or snode.style not in (None, "", "'", '"')
            or lineno not in marked_rust_keys
            or marked_rust_keys[lineno][1] != node_span
        ):
            errors.append(
                f"{name}:{lineno}: {where} toolchain {snode.value!r} lacks # pin:rust=<key> marker"
            )
            return
        if lineno in consumed_rust_lines:
            errors.append(
                f"{name}:{lineno}: {where} toolchain {snode.value!r} shares # pin:rust marker line with another consumer"
            )
            return
        consumed_rust_lines.add(lineno)
        key, _ = marked_rust_keys[lineno]
        if key in rust_pins and rust_pins[key] != snode.value:
            errors.append(
                f"{name}:{lineno}: {where} value {snode.value!r} does not match # pin:rust={key} ({rust_pins[key]!r})"
            )

    def check_env_mapping(env_node: wy.Node | None, scope: str) -> None:
        if not isinstance(env_node, wy.MappingNode):
            return
        env_fields = _map_entries(env_node)
        if "RUSTUP_TOOLCHAIN" in env_fields:
            tc_node = env_fields["RUSTUP_TOOLCHAIN"]
            if (
                not isinstance(tc_node, wy.ScalarNode)
                or tc_node.value not in rust_values
            ):
                got = tc_node.value if isinstance(tc_node, wy.ScalarNode) else tc_node
                errors.append(
                    f"{name}: {scope} env.RUSTUP_TOOLCHAIN must come from {PINS_REL}, got {got!r}"
                )
            else:
                require_rust_marker(tc_node, f"{scope} env.RUSTUP_TOOLCHAIN")

    root_fields = _map_entries(root_node)
    check_env_mapping(root_fields.get("env"), "workflow")

    def scalar_env(node: wy.Node | None) -> dict[str, str]:
        return {
            key: value.value
            for key, value in _map_entries(node).items()
            if isinstance(value, wy.ScalarNode)
        }

    def run_defaults(fields: dict) -> str | None:
        defaults = _map_entries(_map_entries(fields.get("defaults")).get("run"))
        shell_node = defaults.get("shell")
        return shell_node.value if isinstance(shell_node, wy.ScalarNode) else None

    workflow_shell = run_defaults(root_fields)
    errors.extend(check_shell(workflow_shell, f"{name}: workflow defaults.run"))
    workflow_env = scalar_env(root_fields.get("env"))
    errors.extend(check_env_values(workflow_env, f"{name}: workflow"))

    jobs_node = root_fields.get("jobs")
    if not isinstance(jobs_node, wy.MappingNode):
        return [f"{name}: workflow has no jobs mapping"]

    for job_key_node, job_node in jobs_node.value:
        if not isinstance(job_key_node, wy.ScalarNode):
            continue
        job_id = job_key_node.value
        if not isinstance(job_node, wy.MappingNode):
            errors.append(f"{name}: job {job_id!r} is not a mapping")
            continue
        job_fields = _map_entries(job_node)
        check_env_mapping(job_fields.get("env"), f"job {job_id}")
        job_env = scalar_env(job_fields.get("env"))
        # A job container (and service containers) may carry env only under
        # the same rules, and options only `--user <name>`: anything else can
        # set BASH_ENV, PATH or a preload before a step runs.
        containers = [("container", job_fields.get("container"))]
        services = _map_entries(job_fields.get("services"))
        containers += [(f"services.{key}", node) for key, node in services.items()]
        for label, node in containers:
            fields = _map_entries(node)
            errors.extend(
                check_env_values(
                    scalar_env(fields.get("env")), f"{name}: job {job_id} {label}"
                )
            )
            options = fields.get("options")
            if options is not None and not (
                isinstance(options, wy.ScalarNode)
                and CONTAINER_OPTIONS_RE.fullmatch(options.value.strip())
            ):
                errors.append(
                    f"{name}: job {job_id} {label}.options may only be --user <name>"
                )
        job_shell = run_defaults(job_fields)
        errors.extend(check_shell(job_shell, f"{name}: job {job_id} defaults.run"))
        errors.extend(check_env_values(job_env, f"{name}: job {job_id}"))

        if "uses" not in job_fields:
            if "runs-on" not in job_fields:
                errors.append(f"{name}: job {job_id}: missing runs-on")
            else:
                runs_on_node = job_fields["runs-on"]
                if isinstance(runs_on_node, wy.ScalarNode):
                    candidate_nodes = [runs_on_node]
                elif isinstance(runs_on_node, wy.SequenceNode):
                    candidate_nodes = list(runs_on_node.value)
                else:
                    errors.append(f"{name}: job {job_id}: invalid runs-on structure")
                    candidate_nodes = []

                resolved_nodes: list[wy.ScalarNode] = []
                for item_node in candidate_nodes:
                    if not isinstance(
                        item_node, wy.ScalarNode
                    ) or item_node.tag not in ("tag:yaml.org,2002:str",):
                        errors.append(f"{name}: job {job_id}: non-string runs-on value")
                        continue
                    raw_val = item_node.value
                    matrix_match = MATRIX_EXPR_RE.fullmatch(raw_val.strip())
                    if matrix_match:
                        try:
                            resolved_nodes.extend(
                                _resolve_matrix_var_nodes(
                                    job_id, job_node, matrix_match.group(1)
                                )
                            )
                        except PinError as error:
                            errors.append(f"{name}: {error}")
                    elif "${{" in raw_val:
                        errors.append(
                            f"{name}: job {job_id}: expression-valued runs-on cannot be checked: {raw_val}"
                        )
                    else:
                        resolved_nodes.append(item_node)

                for rnode in resolved_nodes:
                    label = rnode.value
                    if label in hosted_values:
                        require_runner_marker(rnode, f"job {job_id}")
                    elif label not in allowlist:
                        errors.append(
                            f"{name}: job {job_id}: unapproved or moving runner label: {label!r}"
                        )

        # Also inspect all scalars in strategy.matrix so custom matrix axes
        # holding hosted runner labels cannot omit a synchronizable marker.
        strategy_node = job_fields.get("strategy")
        if isinstance(strategy_node, wy.MappingNode):
            matrix_node = _map_entries(strategy_node).get("matrix")
            for snode in _walk_scalars(matrix_node):
                if (
                    snode.tag == "tag:yaml.org,2002:str"
                    and snode.value in hosted_values
                    and id(snode) not in visited_runner_nodes
                ):
                    require_runner_marker(snode, f"job {job_id} matrix")

        steps_node = job_fields.get("steps")
        if steps_node is None or (
            isinstance(steps_node, wy.ScalarNode)
            and steps_node.tag == "tag:yaml.org,2002:null"
        ):
            step_nodes: list[wy.Node] = []
        elif isinstance(steps_node, wy.SequenceNode):
            step_nodes = list(steps_node.value)
        else:
            errors.append(f"{name}: job {job_id}: steps is not a list")
            continue

        for idx, step_node in enumerate(step_nodes):
            if not isinstance(step_node, wy.MappingNode):
                errors.append(f"{name}: job {job_id} step {idx}: step is not a mapping")
                continue
            step_fields = _map_entries(step_node)
            check_env_mapping(step_fields.get("env"), f"job {job_id} step {idx}")
            step_env = scalar_env(step_fields.get("env"))
            errors.extend(
                check_env_values(step_env, f"{name}: job {job_id} step {idx}")
            )

            uses_node = step_fields.get("uses")
            if isinstance(uses_node, wy.ScalarNode):
                ref = uses_node.value
                if not local_action_ref_ok(ref):
                    errors.append(
                        f"{name}: job {job_id} step {idx}: local actions must be ./.github/actions/<name> ({ref})"
                    )
                if ref.startswith("dtolnay/rust-toolchain"):
                    errors.append(
                        f"{name}: job {job_id} step {idx}: third-party Rust toolchain action is forbidden: {ref}"
                    )
                if ref in NODE20_DENYLIST:
                    errors.append(
                        f"{name}: job {job_id} step {idx}: action pin targets deprecated Node 20: {ref}"
                    )
                if ref == "./.github/actions/setup-rust":
                    with_fields = _map_entries(step_fields.get("with"))
                    tc_node = with_fields.get("toolchain")
                    if (
                        not isinstance(tc_node, wy.ScalarNode)
                        or tc_node.value not in rust_values
                    ):
                        got = (
                            tc_node.value
                            if isinstance(tc_node, wy.ScalarNode)
                            else None
                        )
                        errors.append(
                            f"{name}: job {job_id} step {idx}: setup-rust toolchain must come from {PINS_REL}, got {got!r}"
                        )
                    else:
                        require_rust_marker(
                            tc_node, f"job {job_id} step {idx} setup-rust"
                        )

            run_node = step_fields.get("run")
            if (
                isinstance(run_node, wy.ScalarNode)
                and run_node.tag == "tag:yaml.org,2002:str"
            ):
                run_cmd = run_node.value
                shell_node = step_fields.get("shell")
                step_shell = (
                    shell_node.value if isinstance(shell_node, wy.ScalarNode) else None
                )
                errors.extend(
                    check_shell(step_shell, f"{name}: job {job_id} step {idx}")
                )
                errors.extend(
                    check_run_text(
                        run_cmd,
                        f"{name}: job {job_id} step {idx}",
                        pins.get("reviewed", frozenset()),
                        pins.get("used"),
                        step_shell or job_shell or workflow_shell,
                    )
                )
                if FLOATING_RUST_RE.search(run_cmd):
                    errors.append(
                        f"{name}: job {job_id} step {idx}: floating Rust channel in run step"
                    )
                if DIRECT_RUSTUP_RE.search(run_cmd):
                    errors.append(
                        f"{name}: job {job_id} step {idx}: direct rustup invocation in workflow step; use ./.github/actions/setup-rust"
                    )
                if "sh.rustup.rs" in run_cmd:
                    if (
                        '--default-toolchain "$RUST_TOOLCHAIN"' not in run_cmd
                        and "--default-toolchain ${RUST_TOOLCHAIN}" not in run_cmd
                    ):
                        errors.append(
                            f'{name}: job {job_id} step {idx}: sh.rustup.rs bootstrap must pass --default-toolchain "$RUST_TOOLCHAIN" from a # pin:rust=<key> env entry'
                        )
                    step_env_fields = _map_entries(step_fields.get("env"))
                    env_tc_node = step_env_fields.get("RUST_TOOLCHAIN")
                    if (
                        not isinstance(env_tc_node, wy.ScalarNode)
                        or env_tc_node.value not in rust_values
                    ):
                        got = (
                            env_tc_node.value
                            if isinstance(env_tc_node, wy.ScalarNode)
                            else None
                        )
                        errors.append(
                            f"{name}: job {job_id} step {idx}: sh.rustup.rs step env.RUST_TOOLCHAIN must come from {PINS_REL}, got {got!r}"
                        )
                    else:
                        require_rust_marker(
                            env_tc_node, f"job {job_id} step {idx} sh.rustup.rs"
                        )

    for lineno in sorted(set(marked_runner_keys) - consumed_runner_lines):
        errors.append(
            f"{name}:{lineno}: # pin:runner marker is not attached to a runner consumer"
        )
    for lineno in sorted(set(marked_rust_keys) - consumed_rust_lines):
        errors.append(
            f"{name}:{lineno}: # pin:rust marker is not attached to a Rust toolchain consumer"
        )

    return errors


PINNED_REMOTE_REF_RE = re.compile(
    r"^(?:[A-Za-z0-9_.-]+/[A-Za-z0-9_./-]+@[0-9a-f]{40}|docker://[^@\s]+@sha256:[0-9a-f]{64})$"
)
LOCAL_ACTION_REF_RE = re.compile(r"^\./\.github/actions/[A-Za-z0-9_-][A-Za-z0-9._-]*$")


def local_action_ref_ok(ref: str) -> bool:
    """A step `uses` that is not a remote owner/repo@ref or docker:// image is
    a local action, and must name a direct child of ./.github/actions with no
    `..` or nesting, so it is always one of the scanned, digest-pinned actions."""
    if ref.startswith("."):
        return LOCAL_ACTION_REF_RE.fullmatch(ref) is not None and ".." not in ref
    if ref.startswith("docker://") or "@" in ref:
        return True
    return False


def check_local_actions(root: Path, pins: dict) -> list[str]:
    """Every local action under .github/actions is a composite whose run
    steps and env values pass the same review rules as workflow steps, and
    whose own local uses stay under ./.github/actions/."""
    errors: list[str] = []
    base = root / ".github/actions"
    if not base.is_dir():
        return errors
    for nested in sorted(base.rglob("action.y*ml")):
        if len(nested.relative_to(base).parts) != 2:
            errors.append(
                f"{nested.relative_to(root).as_posix()}: local actions must be direct children of .github/actions"
            )
    for entry in sorted(base.iterdir()):
        rel = entry.relative_to(root).as_posix()
        manifests = [entry / "action.yml", entry / "action.yaml"]
        present = [m for m in manifests if m.is_file()]
        if not entry.is_dir() or len(present) != 1:
            errors.append(f"{rel}: each local action needs exactly one action.yml")
            continue
        manifest = present[0].relative_to(root).as_posix()
        try:
            doc = wy.load_text(present[0].read_text(encoding="utf-8"), manifest)
        except wy.WorkflowError as error:
            errors.append(str(error))
            continue
        runs = doc.get("runs")
        if not isinstance(runs, dict) or runs.get("using") != "composite":
            errors.append(f"{manifest}: local actions must be composite")
            continue
        for idx, step in enumerate(runs.get("steps") or []):
            if not isinstance(step, dict):
                errors.append(f"{manifest}: step {idx} is not a mapping")
                continue
            where = f"{manifest}: step {idx}"
            uses = step.get("uses")
            if isinstance(uses, str) and not local_action_ref_ok(uses):
                errors.append(
                    f"{where}: local actions must be ./.github/actions/<name> ({uses})"
                )
            elif (
                isinstance(uses, str)
                and not uses.startswith(".")
                and not PINNED_REMOTE_REF_RE.fullmatch(uses)
            ):
                errors.append(
                    f"{where}: remote uses must be pinned to a full commit SHA or an image digest ({uses})"
                )
            env = step.get("env")
            if isinstance(env, dict):
                errors.extend(
                    check_env_values({k: str(v) for k, v in env.items()}, where)
                )
            run_body = step.get("run")
            if isinstance(run_body, str):
                shell = (
                    step.get("shell") if isinstance(step.get("shell"), str) else None
                )
                errors.extend(check_shell(shell, where))
                errors.extend(
                    check_run_text(
                        run_body,
                        where,
                        pins.get("reviewed", frozenset()),
                        pins.get("used"),
                        shell,
                    )
                )
    return errors


def check_setup_rust_action(root: Path, pins: dict) -> list[str]:
    try:
        action_path = _check_rel_no_symlinks(root, SETUP_RUST_REL)
    except PinError as exc:
        return [str(exc)]
    if not action_path.is_file():
        return [f"missing {SETUP_RUST_REL}"]
    text = action_path.read_text(encoding="utf-8")
    try:
        doc = wy.load_text(text, SETUP_RUST_REL)
    except wy.WorkflowError as error:
        return [str(error)]
    runs = doc.get("runs")
    if not isinstance(runs, dict) or runs.get("using") != "composite":
        return [f"{SETUP_RUST_REL}: runs.using must be 'composite'"]
    steps = runs.get("steps")
    if not isinstance(steps, list) or not steps:
        return [f"{SETUP_RUST_REL}: composite action must declare steps"]
    errors: list[str] = []
    for idx, step in enumerate(steps):
        if not isinstance(step, dict):
            errors.append(f"{SETUP_RUST_REL}: step {idx} is not a mapping")
            continue
        if "uses" in step:
            errors.append(
                f"{SETUP_RUST_REL}: step {idx} must not delegate to another action ({step['uses']})"
            )
        run_body = step.get("run", "")
        if "--no-self-update" not in str(run_body):
            errors.append(
                f"{SETUP_RUST_REL}: step {idx} must pass --no-self-update to rustup"
            )
    return errors


def sync_text(text: str, pins: dict, name: str) -> tuple[str, int]:
    trailing_newline = text.endswith("\n")
    out_lines: list[str] = []
    updated = 0
    for lineno, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith("#") or not MARKER_COMMENT_RE.search(line):
            out_lines.append(line)
            continue
        match = SYNC_LINE_RE.match(line)
        if not match:
            raise PinError(
                f"{name}:{lineno}: malformed # pin marker line: {line.strip()}"
            )
        kind = match.group("kind")
        key = match.group("key")
        table = pins["hosted"] if kind == "runner" else pins["rust"]
        if key not in table:
            raise PinError(f"{name}:{lineno}: unknown {kind} pin key {key!r}")
        new_val = table[key]
        quote = match.group("quote")
        rebuilt = (
            f"{match.group('prefix')}{quote}{new_val}{quote}"
            f"{match.group('ws')}# pin:{kind}={key}"
        )
        if rebuilt != line:
            updated += 1
        out_lines.append(rebuilt)
    result = "\n".join(out_lines) + ("\n" if trailing_newline else "")
    return result, updated


def write_workflow_sha256(root: Path) -> int:
    validate_repo_tree_no_symlinks(root)
    entries: list[tuple[str, str]] = []
    for directory in (".github/actions", ".github/workflows"):
        base = root / directory
        if not base.exists():
            continue
        for path in sorted(base.rglob("*")):
            if path.is_symlink():
                raise PinError(f"symlink under {directory} is not allowed: {path}")
            if path.is_file() and path.suffix in (".yml", ".yaml"):
                rel = path.relative_to(root).as_posix()
                digest = hashlib.sha256(path.read_bytes()).hexdigest()
                entries.append((rel, f"{digest}  {rel}"))
    entries.sort(key=lambda item: item[0])
    content = "\n".join(line for _, line in entries) + "\n"
    _write_text_nofollow(root, WORKFLOW_SHA_REL, content)
    return len(entries)


def run_sync(root: Path) -> None:
    validate_repo_tree_no_symlinks(root)
    pins = load_pins(root)
    errors = check_setup_rust_action(root, pins) + check_local_actions(root, pins)
    workflows_dir = root / ".github/workflows"
    paths = sorted(workflows_dir.glob("*.y*ml"))
    if not paths:
        raise PinError("no workflow files found under .github/workflows")

    pending_writes: list[tuple[str, str]] = []
    changed_files = 0
    changed_lines = 0
    for path in paths:
        rel = path.relative_to(root).as_posix()
        _check_rel_no_symlinks(root, rel)
        original = path.read_text(encoding="utf-8")
        synced, count = sync_text(original, pins, path.name)
        errors.extend(check_workflow_text(path.name, synced, pins))
        if synced != original:
            pending_writes.append((rel, synced))
            changed_files += 1
            changed_lines += count

    if errors:
        raise PinError("\n".join(errors))

    validate_repo_tree_no_symlinks(root)
    for rel, synced in pending_writes:
        _write_text_nofollow(root, rel, synced)
    pinned_count = write_workflow_sha256(root)
    run_check(root)
    print(
        f"[ok] synced {changed_lines} pin line(s) across {changed_files} workflow(s); "
        f"refreshed {pinned_count} entry(ies) in {WORKFLOW_SHA_REL}"
    )


def run_check(root: Path) -> None:
    validate_repo_tree_no_symlinks(root)
    pins = load_pins(root)
    errors = check_setup_rust_action(root, pins) + check_local_actions(root, pins)
    workflows_dir = root / ".github/workflows"
    paths = sorted(workflows_dir.glob("*.y*ml"))
    if not paths:
        raise PinError("no workflow files found under .github/workflows")
    for path in paths:
        errors.extend(
            check_workflow_text(path.name, path.read_text(encoding="utf-8"), pins, root)
        )
    errors.extend(check_reviewed_lines_used(pins["reviewed"], pins["used"]))
    errors.extend(check_reviewed_installers(root, pins["reviewed"]))
    if errors:
        raise PinError("\n".join(errors))


def main(argv: list[str]) -> int:
    root = Path.cwd()
    args = list(argv)
    while len(args) >= 2 and args[0] == "--root":
        raw_root = Path(args[1])
        if raw_root.is_symlink():
            raise PinError(f"symlink root is not allowed: {raw_root}")
        root = raw_root.resolve()
        args = args[2:]
    if args == ["sync"]:
        run_sync(root)
        return 0
    if args == ["check"]:
        run_check(root)
        print(
            "[ok] CI runner labels, Rust toolchain pins and tool fetches match committed pins"
        )
        return 0
    raise PinError("usage: ci-pins.py [--root PATH] <sync|check>")


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except (PinError, wy.ReaderError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)
