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
         known Node 20 action pins
"""

from __future__ import annotations

import hashlib
import re
import sys
import tomllib
from pathlib import Path

# Keep the script directory off sys.path before importing workflow_yaml (which
# imports PyYAML) so a stray scripts/yaml.py cannot shadow the installed module.
SCRIPT_DIR = Path(__file__).resolve().parent
sys.path = [entry for entry in sys.path if Path(entry or ".").resolve() != SCRIPT_DIR]
try:
    import yaml as _yaml  # noqa: F401,E402
except ModuleNotFoundError:
    pass
sys.path.append(str(SCRIPT_DIR))
import workflow_yaml as wy  # noqa: E402

PINS_REL = "config/ci/pins.toml"
WORKFLOW_SHA_REL = "config/release/release-workflow.sha256"
SETUP_RUST_REL = ".github/actions/setup-rust/action.yml"

SEMVER_RE = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
MARKER_COMMENT_RE = re.compile(r"#\s*pin:(runner|rust)=([A-Za-z0-9_-]+)\s*$")
SYNC_LINE_RE = re.compile(
    r"^(?P<prefix>.*?(?:[:=]\s*|-\s+))"
    r"(?P<quote>['\"]?)(?P<val>[A-Za-z0-9._-]+)(?P=quote)"
    r"(?P<ws>\s+)#\s*pin:(?P<kind>runner|rust)=(?P<key>[A-Za-z0-9_-]+)\s*$"
)
MATRIX_EXPR_RE = re.compile(r"^\$\{\{\s*matrix\.([A-Za-z0-9_-]+)\s*\}\}$")

# Known deprecated Node 20 action commit pins.
NODE20_DENYLIST = {
    "actions/checkout@11d5960a326750d5838078e36cf38b85af677262",
    "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02",
}

FLOATING_RUST_RE = re.compile(
    r"(?:\b--default-toolchain\s+(?:stable|beta|nightly)\b|"
    r"\brustup\s+(?:toolchain\s+install|default|install|run)\s+(?:stable|beta|nightly)\b|"
    r"\bcargo\s+\+(?:stable|beta|nightly)\b)"
)
DIRECT_RUSTUP_RE = re.compile(
    r"(?:^|[;&|\s])rustup\s+(?:toolchain|target|component|default|show|install|update)\b"
)


class PinError(Exception):
    pass


def load_pins(root: Path) -> dict:
    pins_path = root / PINS_REL
    if not pins_path.is_file() or pins_path.is_symlink():
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

    cargo_toml = root / "Cargo.toml"
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
    }


def _resolve_matrix_var(job_id: str, job: dict, var: str) -> list[str]:
    strategy = job.get("strategy")
    if not isinstance(strategy, dict):
        raise PinError(f"job {job_id}: uses matrix.{var} without strategy.matrix")
    matrix = strategy.get("matrix")
    if not isinstance(matrix, dict):
        raise PinError(f"job {job_id}: strategy.matrix is not a mapping")
    values: list[str] = []
    axis = matrix.get(var)
    if axis is not None:
        if not isinstance(axis, list) or not axis:
            raise PinError(
                f"job {job_id}: strategy.matrix.{var} must be a non-empty list"
            )
        for item in axis:
            if not isinstance(item, str):
                raise PinError(
                    f"job {job_id}: strategy.matrix.{var} entry is not a string: {item!r}"
                )
            values.append(item)
    include = matrix.get("include")
    if include is not None:
        if not isinstance(include, list):
            raise PinError(f"job {job_id}: strategy.matrix.include must be a list")
        for entry in include:
            if not isinstance(entry, dict):
                raise PinError(
                    f"job {job_id}: strategy.matrix.include entry is not a mapping"
                )
            if var in entry:
                val = entry[var]
                if not isinstance(val, str):
                    raise PinError(
                        f"job {job_id}: strategy.matrix.include.{var} is not a string: {val!r}"
                    )
                values.append(val)
    if not values:
        raise PinError(f"job {job_id}: matrix.{var} resolves to zero runner labels")
    return values


def check_workflow_text(name: str, text: str, pins: dict) -> list[str]:
    errors: list[str] = []
    try:
        doc = wy.load_text(text, name)
        job_map = wy.jobs(doc)
    except wy.WorkflowError as error:
        return [str(error)]

    hosted = pins["hosted"]
    hosted_values = set(hosted.values())
    allowlist = pins["allowlist"]
    rust_pins = pins["rust"]
    rust_values = set(rust_pins.values())

    lines = text.splitlines()
    marked_runner_lines: set[int] = set()
    marked_rust_lines: set[int] = set()

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
        if kind == "runner":
            marked_runner_lines.add(lineno)
            if key not in hosted:
                errors.append(f"{name}:{lineno}: unknown runner pin key {key!r}")
            elif val != hosted[key]:
                errors.append(
                    f"{name}:{lineno}: runner pin {key!r} is {val!r}, expected {hosted[key]!r} (run make ci-pins-sync)"
                )
        elif kind == "rust":
            marked_rust_lines.add(lineno)
            if key not in rust_pins:
                errors.append(f"{name}:{lineno}: unknown rust pin key {key!r}")
            elif val != rust_pins[key]:
                errors.append(
                    f"{name}:{lineno}: rust pin {key!r} is {val!r}, expected {rust_pins[key]!r} (run make ci-pins-sync)"
                )

    # Ensure no unmarked hosted runner value appears on a runner-setting line.
    for lineno, line in enumerate(lines, 1):
        if line.lstrip().startswith("#"):
            continue
        for label in hosted_values:
            if re.search(
                rf"^\s*(?:-\s*)?(?:(?:runs-on|os|runner):\s*)?['\"]?{re.escape(label)}['\"]?(?:\s*(?:#.*)?)$",
                line,
            ):
                if lineno not in marked_runner_lines:
                    errors.append(
                        f"{name}:{lineno}: hosted runner {label!r} lacks # pin:runner=<key> marker"
                    )

    # 2. Validate every job's runs-on, uses, and run steps from parsed YAML.
    rust_pin_consumers = 0
    for job_id, job in job_map.items():
        if not isinstance(job, dict):
            errors.append(f"{name}: job {job_id!r} is not a mapping")
            continue
        if "uses" not in job:
            if "runs-on" not in job:
                errors.append(f"{name}: job {job_id}: missing runs-on")
            else:
                raw_runs_on = job["runs-on"]
                candidates = (
                    raw_runs_on if isinstance(raw_runs_on, list) else [raw_runs_on]
                )
                resolved_labels: list[str] = []
                for item in candidates:
                    if not isinstance(item, str):
                        errors.append(
                            f"{name}: job {job_id}: non-string runs-on value: {item!r}"
                        )
                        continue
                    matrix_match = MATRIX_EXPR_RE.fullmatch(item.strip())
                    if matrix_match:
                        try:
                            resolved_labels.extend(
                                _resolve_matrix_var(job_id, job, matrix_match.group(1))
                            )
                        except PinError as error:
                            errors.append(f"{name}: {error}")
                    elif "${{" in item:
                        errors.append(
                            f"{name}: job {job_id}: expression-valued runs-on cannot be checked: {item}"
                        )
                    else:
                        resolved_labels.append(item)

                for label in resolved_labels:
                    if label not in hosted_values and label not in allowlist:
                        errors.append(
                            f"{name}: job {job_id}: unapproved or moving runner label: {label!r}"
                        )

        try:
            step_list = wy.steps(job)
        except wy.WorkflowError as error:
            errors.append(f"{name}: job {job_id}: {error}")
            continue

        for idx, step in enumerate(step_list):
            if not isinstance(step, dict):
                errors.append(f"{name}: job {job_id} step {idx}: step is not a mapping")
                continue
            uses = step.get("uses")
            if uses is not None:
                ref = str(uses)
                if ref.startswith("dtolnay/rust-toolchain"):
                    errors.append(
                        f"{name}: job {job_id} step {idx}: third-party Rust toolchain action is forbidden: {ref}"
                    )
                if ref in NODE20_DENYLIST:
                    errors.append(
                        f"{name}: job {job_id} step {idx}: action pin targets deprecated Node 20: {ref}"
                    )
                if ref == "./.github/actions/setup-rust":
                    rust_pin_consumers += 1
                    with_map = step.get("with")
                    toolchain = (
                        with_map.get("toolchain")
                        if isinstance(with_map, dict)
                        else None
                    )
                    if toolchain is None or str(toolchain) not in rust_values:
                        errors.append(
                            f"{name}: job {job_id} step {idx}: setup-rust toolchain must come from {PINS_REL}, got {toolchain!r}"
                        )
                    else:
                        tc_str = str(toolchain)
                        if not any(
                            lineno in marked_rust_lines and tc_str in lines[lineno - 1]
                            for lineno in range(1, len(lines) + 1)
                        ):
                            errors.append(
                                f"{name}: job {job_id} step {idx}: setup-rust toolchain {tc_str!r} lacks # pin:rust=<key> marker"
                            )

            run_cmd = step.get("run")
            if isinstance(run_cmd, str):
                if FLOATING_RUST_RE.search(run_cmd):
                    errors.append(
                        f"{name}: job {job_id} step {idx}: floating Rust channel in run step"
                    )
                if DIRECT_RUSTUP_RE.search(run_cmd):
                    errors.append(
                        f"{name}: job {job_id} step {idx}: direct rustup invocation in workflow step; use ./.github/actions/setup-rust"
                    )
                if "sh.rustup.rs" in run_cmd:
                    rust_pin_consumers += 1
                    if (
                        '--default-toolchain "$RUST_TOOLCHAIN"' not in run_cmd
                        and "--default-toolchain ${RUST_TOOLCHAIN}" not in run_cmd
                    ):
                        errors.append(
                            f'{name}: job {job_id} step {idx}: sh.rustup.rs bootstrap must pass --default-toolchain "$RUST_TOOLCHAIN" from a # pin:rust=<key> env entry'
                        )
                    step_env = step.get("env")
                    env_tc = (
                        str(step_env.get("RUST_TOOLCHAIN"))
                        if isinstance(step_env, dict) and "RUST_TOOLCHAIN" in step_env
                        else None
                    )
                    if env_tc not in rust_values:
                        errors.append(
                            f"{name}: job {job_id} step {idx}: sh.rustup.rs step env.RUST_TOOLCHAIN must come from {PINS_REL}, got {env_tc!r}"
                        )

    # Also verify every setup-rust toolchain line in raw text carries # pin:rust=<key>.
    in_setup_rust = False
    for lineno, line in enumerate(lines, 1):
        stripped = line.strip()
        if stripped.startswith("- "):
            in_setup_rust = "uses: ./.github/actions/setup-rust" in stripped
        elif "uses: ./.github/actions/setup-rust" in stripped:
            in_setup_rust = True
        elif in_setup_rust and re.match(r"^toolchain:\s*", stripped):
            if lineno not in marked_rust_lines:
                errors.append(
                    f"{name}:{lineno}: setup-rust toolchain line lacks # pin:rust=<key> marker"
                )
            in_setup_rust = False

    if len(marked_rust_lines) < rust_pin_consumers:
        errors.append(
            f"{name}: found {rust_pin_consumers} Rust toolchain consumer step(s) "
            f"but only {len(marked_rust_lines)} # pin:rust=<key> marker line(s)"
        )

    return errors


def check_setup_rust_action(root: Path) -> list[str]:
    action_path = root / SETUP_RUST_REL
    if not action_path.is_file() or action_path.is_symlink():
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
    entries: list[str] = []
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
    pin_file = root / WORKFLOW_SHA_REL
    pin_file.parent.mkdir(parents=True, exist_ok=True)
    pin_file.write_text("\n".join(line for _, line in entries) + "\n", encoding="utf-8")
    return len(entries)


def run_sync(root: Path) -> None:
    pins = load_pins(root)
    workflows_dir = root / ".github/workflows"
    paths = sorted(workflows_dir.glob("*.y*ml"))
    if not paths:
        raise PinError("no workflow files found under .github/workflows")
    changed_files = 0
    changed_lines = 0
    for path in paths:
        original = path.read_text(encoding="utf-8")
        synced, count = sync_text(original, pins, path.name)
        if synced != original:
            path.write_text(synced, encoding="utf-8")
            changed_files += 1
            changed_lines += count
    pinned_count = write_workflow_sha256(root)
    run_check(root)
    print(
        f"[ok] synced {changed_lines} pin line(s) across {changed_files} workflow(s); "
        f"refreshed {pinned_count} entry(ies) in {WORKFLOW_SHA_REL}"
    )


def run_check(root: Path) -> None:
    pins = load_pins(root)
    errors = check_setup_rust_action(root)
    workflows_dir = root / ".github/workflows"
    paths = sorted(workflows_dir.glob("*.y*ml"))
    if not paths:
        raise PinError("no workflow files found under .github/workflows")
    for path in paths:
        errors.extend(
            check_workflow_text(path.name, path.read_text(encoding="utf-8"), pins)
        )
    if errors:
        raise PinError("\n".join(errors))


def main(argv: list[str]) -> int:
    root = Path.cwd()
    args = list(argv)
    while len(args) >= 2 and args[0] == "--root":
        root = Path(args[1]).resolve()
        args = args[2:]
    if args == ["sync"]:
        run_sync(root)
        return 0
    if args == ["check"]:
        run_check(root)
        print("[ok] CI runner labels and Rust toolchain pins match config/ci/pins.toml")
        return 0
    raise PinError("usage: ci-pins.py [--root PATH] <sync|check>")


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except PinError as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)
