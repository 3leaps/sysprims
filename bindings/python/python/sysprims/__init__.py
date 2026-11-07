# SPDX-License-Identifier: MIT OR Apache-2.0
"""Cross-platform process inspection, signal dispatch, containment, timeout, and RFC 5545 recurrence primitives."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from enum import IntEnum
import json
from typing import Any, Final, Union
import weakref

from . import _sysprims as _native

__all__ = [
    "ContainedProcess",
    "RECURRENCE_CONTRACT",
    "RECURRENCE_CORPUS_DIGEST",
    "RECURRENCE_REFUSAL_REASONS",
    "RecurrenceQueryError",
    "RecurrenceRefusal",
    "RecurrenceRule",
    "SIGHUP",
    "SIGINT",
    "SIGKILL",
    "SIGTERM",
    "SIGUSR1",
    "SIGUSR2",
    "SysprimsError",
    "SysprimsErrorCode",
    "abi_version",
    "ancestors",
    "descendants",
    "force_kill",
    "force_kill_many",
    "guard_step",
    "kill_descendants",
    "kill_many",
    "list_fds",
    "listening_ports",
    "platform",
    "proc_get",
    "process_list",
    "run_nohup",
    "run_setsid",
    "run_with_timeout",
    "self_getpgid",
    "self_getsid",
    "signal_send",
    "signal_send_group",
    "spawn_contained",
    "spawn_in_group",
    "terminate",
    "terminate_many",
    "terminate_tree",
    "tzdb_version",
    "wait_pid",
]

__version__: Final[str] = _native.VERSION

RECURRENCE_CONTRACT: Final[str] = _native.RECURRENCE_CONTRACT
RECURRENCE_CORPUS_DIGEST: Final[str] = _native.RECURRENCE_CORPUS_DIGEST
RECURRENCE_REFUSAL_REASONS: Final[tuple[str, ...]] = (
    "invalid_rule",
    "invalid_text",
    "unsupported_part",
    "invalid_tzid",
    "invalid_date",
    "dtstart_not_synchronized",
)

SIGHUP: Final[int] = _native.SIGHUP
SIGINT: Final[int] = _native.SIGINT
SIGKILL: Final[int] = _native.SIGKILL
SIGTERM: Final[int] = _native.SIGTERM
SIGUSR1: Final[int] = _native.SIGUSR1
SIGUSR2: Final[int] = _native.SIGUSR2

_U32_MAX: Final[int] = 0xFFFF_FFFF
_MAX_SAFE_PID: Final[int] = 0x7FFF_FFFF
_MAX_DURATION_MS: Final[int] = 86_400_000

_BATCH_KILL_SCHEMA_ID: Final[str] = (
    "https://schemas.3leaps.dev/sysprims/signal/v1.0.0/batch-kill-result.schema.json"
)
_RUN_SETSID_SCHEMA_ID: Final[str] = (
    "https://schemas.3leaps.dev/sysprims/session/v1.0.0/run-setsid-config.schema.json"
)
_RUN_NOHUP_SCHEMA_ID: Final[str] = (
    "https://schemas.3leaps.dev/sysprims/session/v1.0.0/run-nohup-config.schema.json"
)
_SPAWN_IN_GROUP_SCHEMA_ID: Final[str] = (
    "https://schemas.3leaps.dev/sysprims/process/v1.0.0/spawn-in-group-config.schema.json"
)
_TERMINATE_TREE_SCHEMA_ID: Final[str] = (
    "https://schemas.3leaps.dev/sysprims/process/v1.0.0/terminate-tree-config.schema.json"
)
_CONTAINMENT_SPAWN_SCHEMA_ID: Final[str] = (
    "https://schemas.3leaps.dev/sysprims/timeout/v1.0.0/containment-spawn-config.schema.json"
)

BytesInput = Union[bytes, bytearray, memoryview, str]


class SysprimsErrorCode(IntEnum):
    """Error codes shared across sysprims language bindings."""

    OK = 0
    INVALID_ARGUMENT = 1
    SPAWN_FAILED = 2
    TIMEOUT = 3
    PERMISSION_DENIED = 4
    NOT_FOUND = 5
    NOT_SUPPORTED = 6
    GROUP_CREATION_FAILED = 7
    SYSTEM = 8
    INTERNAL = 99


class SysprimsError(RuntimeError):
    """Structured error raised by sysprims process, signal, session, and timeout APIs."""

    def __init__(self, code: SysprimsErrorCode | int, message: str) -> None:
        try:
            resolved = SysprimsErrorCode(int(code))
        except ValueError:
            resolved = SysprimsErrorCode.INTERNAL
        super().__init__(f"[{resolved.name}] {message}")
        self.code: SysprimsErrorCode = resolved
        self.message: str = message


class RecurrenceRefusal(ValueError):
    """Refusal raised when an RFC 5545 recurrence rule or JSON document is invalid."""

    def __init__(self, reason: str, detail: str) -> None:
        super().__init__(f"{reason}: {detail}")
        self.reason: str = reason
        self.detail: str = detail


class RecurrenceQueryError(RuntimeError):
    """Error raised when a recurrence expansion query fails (`reversed_range` or `search_limit_exceeded`)."""

    def __init__(self, kind: str, message: str) -> None:
        super().__init__(message)
        self.kind: str = kind
        self.message: str = message


def _require_int(value: Any, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise SysprimsError(
            SysprimsErrorCode.INVALID_ARGUMENT,
            f"{name} must be an int, not {type(value).__name__}",
        )
    return value


def _require_pid(value: Any, name: str = "pid") -> int:
    iv = _require_int(value, name)
    if iv < 1 or iv > _MAX_SAFE_PID:
        raise SysprimsError(
            SysprimsErrorCode.INVALID_ARGUMENT,
            f"{name} must be between 1 and {_MAX_SAFE_PID}",
        )
    return iv


def _require_bool(value: Any, name: str) -> bool:
    if not isinstance(value, bool):
        raise SysprimsError(
            SysprimsErrorCode.INVALID_ARGUMENT,
            f"{name} must be a bool, not {type(value).__name__}",
        )
    return value


def _require_argv(argv: Sequence[str]) -> list[str]:
    if isinstance(argv, (str, bytes, bytearray)) or not isinstance(argv, Sequence):
        raise SysprimsError(
            SysprimsErrorCode.INVALID_ARGUMENT,
            "argv must be a non-empty sequence of strings",
        )
    items = list(argv)
    if not items:
        raise SysprimsError(SysprimsErrorCode.INVALID_ARGUMENT, "argv must not be empty")
    for idx, entry in enumerate(items):
        if not isinstance(entry, str) or not entry:
            raise SysprimsError(
                SysprimsErrorCode.INVALID_ARGUMENT,
                f"argv[{idx}] must be a non-empty string",
            )
    return items


def _require_env(env: Mapping[str, str] | None) -> dict[str, str] | None:
    if env is None:
        return None
    if not isinstance(env, Mapping):
        raise SysprimsError(
            SysprimsErrorCode.INVALID_ARGUMENT,
            "env must be a mapping of string keys to string values",
        )
    out: dict[str, str] = {}
    for k, v in env.items():
        if not isinstance(k, str) or not isinstance(v, str):
            raise SysprimsError(
                SysprimsErrorCode.INVALID_ARGUMENT,
                "env keys and values must be strings",
            )
        out[k] = v
    return out


def _bytes_input(data: BytesInput, label: str = "input") -> bytes:
    if isinstance(data, str):
        return data.encode("utf-8")
    try:
        view = memoryview(data)
    except TypeError:
        raise TypeError(f"{label} must be bytes-like or str, not {type(data).__name__}") from None
    with view:
        return view.tobytes()


def _serialize_json_mapping(mapping: Mapping[str, Any] | None, label: str) -> str:
    if mapping is None:
        return ""
    if not isinstance(mapping, Mapping):
        raise SysprimsError(
            SysprimsErrorCode.INVALID_ARGUMENT,
            f"{label} must be a mapping",
        )
    if not mapping:
        return ""
    try:
        return json.dumps(dict(mapping), allow_nan=False)
    except (TypeError, ValueError) as exc:
        raise SysprimsError(
            SysprimsErrorCode.INVALID_ARGUMENT,
            f"invalid {label}: {exc}",
        ) from None


def _process_options_json(include_env: bool, include_threads: bool) -> str:
    _require_bool(include_env, "include_env")
    _require_bool(include_threads, "include_threads")
    if not include_env and not include_threads:
        return ""
    wire: dict[str, bool] = {}
    if include_env:
        wire["include_env"] = True
    if include_threads:
        wire["include_threads"] = True
    return json.dumps(wire)


def _call_json(fn: Any, *args: Any) -> dict[str, Any]:
    try:
        payload = fn(*args)
    except _native.RawSysprimsError as exc:
        raise SysprimsError(*exc.args) from None
    result: dict[str, Any] = json.loads(payload)
    return result


def _call_void(fn: Any, *args: Any) -> None:
    try:
        fn(*args)
    except _native.RawSysprimsError as exc:
        raise SysprimsError(*exc.args) from None


def _call_u32(fn: Any, *args: Any) -> int:
    try:
        value: int = fn(*args)
        return value
    except _native.RawSysprimsError as exc:
        raise SysprimsError(*exc.args) from None


def abi_version() -> int:
    """Return the sysprims native ABI version."""
    return int(_native.abi_version())


def platform() -> str:
    """Return the canonical sysprims platform identifier (`linux`, `macos`, `windows`)."""
    return str(_native.platform())


# =============================================================================
# Process Inspection & Guard
# =============================================================================


def proc_get(
    pid: int,
    *,
    include_env: bool = False,
    include_threads: bool = False,
) -> dict[str, Any]:
    """Inspect a single process by PID."""
    checked_pid = _require_pid(pid, "pid")
    opts_json = _process_options_json(include_env, include_threads)
    return _call_json(_native.proc_get, checked_pid, opts_json)


def process_list(
    filter: Mapping[str, Any] | None = None,
    *,
    include_env: bool = False,
    include_threads: bool = False,
) -> dict[str, Any]:
    """Return a filtered snapshot of running processes."""
    filter_json = _serialize_json_mapping(filter, "filter")
    opts_json = _process_options_json(include_env, include_threads)
    return _call_json(_native.proc_list, filter_json, opts_json)


def listening_ports(filter: Mapping[str, Any] | None = None) -> dict[str, Any]:
    """Return listening TCP/UDP socket port bindings."""
    filter_json = _serialize_json_mapping(filter, "filter")
    return _call_json(_native.proc_listening_ports, filter_json)


def list_fds(pid: int, filter: Mapping[str, Any] | None = None) -> dict[str, Any]:
    """Return open file descriptors for `pid` (Unix/macOS/Linux where supported)."""
    checked_pid = _require_pid(pid, "pid")
    filter_json = _serialize_json_mapping(filter, "filter")
    return _call_json(_native.proc_list_fds, checked_pid, filter_json)


def wait_pid(pid: int, timeout_ms: int) -> dict[str, Any]:
    """Wait up to `timeout_ms` milliseconds for `pid` to exit."""
    checked_pid = _require_pid(pid, "pid")
    checked_timeout = _require_int(timeout_ms, "timeout_ms")
    if checked_timeout < 0 or checked_timeout > _U32_MAX:
        raise SysprimsError(
            SysprimsErrorCode.INVALID_ARGUMENT,
            f"timeout_ms must be between 0 and {_U32_MAX}",
        )
    return _call_json(_native.proc_wait_pid, checked_pid, checked_timeout)


def _descendants_config_json(
    filter: Mapping[str, Any] | None,
    cpu_mode: str | None,
    sample_duration_ms: int | None,
    cascade: bool = False,
) -> str:
    wire: dict[str, Any] = {}
    if filter is not None:
        if not isinstance(filter, Mapping):
            raise SysprimsError(SysprimsErrorCode.INVALID_ARGUMENT, "filter must be a mapping")
        wire.update(filter)
    if cpu_mode is not None:
        if cpu_mode not in ("lifetime", "monitor"):
            raise SysprimsError(
                SysprimsErrorCode.INVALID_ARGUMENT,
                f"invalid cpu_mode: {cpu_mode!r}",
            )
        wire["cpu_mode"] = cpu_mode
    if sample_duration_ms is not None:
        dur = _require_int(sample_duration_ms, "sample_duration_ms")
        if dur < 0 or dur > _MAX_DURATION_MS:
            raise SysprimsError(
                SysprimsErrorCode.INVALID_ARGUMENT,
                f"sample_duration_ms must be between 0 and {_MAX_DURATION_MS}",
            )
        wire["sample_duration_ms"] = dur
    _require_bool(cascade, "cascade")
    if cascade:
        wire["cascade"] = True
    return json.dumps(wire, allow_nan=False) if wire else ""


def descendants(
    root_pid: int,
    *,
    max_levels: int | None = None,
    filter: Mapping[str, Any] | None = None,
    cpu_mode: str | None = None,
    sample_duration_ms: int | None = None,
    include_env: bool = False,
    include_threads: bool = False,
) -> dict[str, Any]:
    """Traverse descendant processes below `root_pid`."""
    checked_root = _require_pid(root_pid, "root_pid")
    levels = _U32_MAX if max_levels is None else _require_int(max_levels, "max_levels")
    if levels < 0 or levels > _U32_MAX:
        raise SysprimsError(
            SysprimsErrorCode.INVALID_ARGUMENT,
            f"max_levels must be between 0 and {_U32_MAX}",
        )
    cfg_json = _descendants_config_json(filter, cpu_mode, sample_duration_ms, False)
    opts_json = _process_options_json(include_env, include_threads)
    return _call_json(_native.proc_descendants, checked_root, levels, cfg_json, opts_json)


def kill_descendants(
    root_pid: int,
    *,
    signal: int = SIGTERM,
    max_levels: int | None = None,
    filter: Mapping[str, Any] | None = None,
    cpu_mode: str | None = None,
    sample_duration_ms: int | None = None,
    cascade: bool = False,
) -> dict[str, Any]:
    """Send `signal` to matched descendants of `root_pid` (excluding self, parent, and PID 1)."""
    checked_root = _require_pid(root_pid, "root_pid")
    checked_sig = _require_int(signal, "signal")
    levels = _U32_MAX if max_levels is None else _require_int(max_levels, "max_levels")
    if levels < 0 or levels > _U32_MAX:
        raise SysprimsError(
            SysprimsErrorCode.INVALID_ARGUMENT,
            f"max_levels must be between 0 and {_U32_MAX}",
        )
    cfg_json = _descendants_config_json(filter, cpu_mode, sample_duration_ms, cascade)
    return _call_json(_native.proc_kill_descendants, checked_root, levels, checked_sig, cfg_json)


def guard_step(config: Mapping[str, Any]) -> dict[str, Any]:
    """Run one single-shot descendant guard evaluation/remediation step."""
    if not isinstance(config, Mapping) or not config:
        raise SysprimsError(SysprimsErrorCode.INVALID_ARGUMENT, "guard config must be a non-empty mapping")
    cfg_json = _serialize_json_mapping(config, "guard config")
    return _call_json(_native.proc_guard_step, cfg_json)


def ancestors(
    pid: int,
    *,
    max_depth: int = 64,
    include_env: bool = False,
    include_threads: bool = False,
) -> dict[str, Any]:
    """Walk the parent chain starting at `pid` up to `max_depth` ancestors."""
    checked_pid = _require_pid(pid, "pid")
    depth = _require_int(max_depth, "max_depth")
    if depth < 0 or depth > _U32_MAX:
        raise SysprimsError(
            SysprimsErrorCode.INVALID_ARGUMENT,
            f"max_depth must be between 0 and {_U32_MAX}",
        )
    opts_json = _process_options_json(include_env, include_threads)
    return _call_json(_native.proc_ancestors, checked_pid, depth, opts_json)


# =============================================================================
# Self Introspection & Signals
# =============================================================================


def self_getpgid() -> int:
    """Return the calling process's process group ID (Unix only)."""
    return _call_u32(_native.self_getpgid)


def self_getsid() -> int:
    """Return the calling process's session ID (Unix only)."""
    return _call_u32(_native.self_getsid)


def signal_send(pid: int, signal: int) -> None:
    """Send `signal` to `pid`."""
    checked_pid = _require_pid(pid, "pid")
    checked_sig = _require_int(signal, "signal")
    _call_void(_native.signal_send, checked_pid, checked_sig)


def signal_send_group(pgid: int, signal: int) -> None:
    """Send `signal` to process group `pgid` (Unix only)."""
    checked_pgid = _require_pid(pgid, "pgid")
    checked_sig = _require_int(signal, "signal")
    _call_void(_native.signal_send_group, checked_pgid, checked_sig)


def terminate(pid: int) -> None:
    """Terminate `pid` gracefully (`SIGTERM` on Unix, `TerminateProcess` on Windows)."""
    checked_pid = _require_pid(pid, "pid")
    _call_void(_native.terminate, checked_pid)


def force_kill(pid: int) -> None:
    """Forcefully kill `pid` (`SIGKILL` on Unix, `TerminateProcess` on Windows)."""
    checked_pid = _require_pid(pid, "pid")
    _call_void(_native.force_kill, checked_pid)


def _validate_pid_list(pids: Sequence[int]) -> list[int]:
    if isinstance(pids, (str, bytes, bytearray)) or not isinstance(pids, Sequence):
        raise SysprimsError(
            SysprimsErrorCode.INVALID_ARGUMENT,
            "pids must be a non-empty sequence of PIDs",
        )
    items = [_require_pid(p, f"pids[{idx}]") for idx, p in enumerate(pids)]
    if not items:
        raise SysprimsError(SysprimsErrorCode.INVALID_ARGUMENT, "pids must not be empty")
    return items


def kill_many(pids: Sequence[int], signal: int) -> dict[str, Any]:
    """Send `signal` to multiple PIDs, collecting per-PID successes and failures."""
    checked_pids = _validate_pid_list(pids)
    checked_sig = _require_int(signal, "signal")
    return _call_json(_native.signal_kill_many, checked_pids, checked_sig)


def terminate_many(pids: Sequence[int]) -> dict[str, Any]:
    """Terminate multiple PIDs gracefully across Unix and Windows."""
    checked_pids = _validate_pid_list(pids)
    succeeded: list[int] = []
    failed: list[dict[str, Any]] = []
    for pid in checked_pids:
        try:
            terminate(pid)
            succeeded.append(pid)
        except SysprimsError as exc:
            failed.append({"pid": pid, "error": exc.message})
    return {
        "schema_id": _BATCH_KILL_SCHEMA_ID,
        "succeeded": succeeded,
        "failed": failed,
    }


def force_kill_many(pids: Sequence[int]) -> dict[str, Any]:
    """Forcefully kill multiple PIDs across Unix and Windows."""
    checked_pids = _validate_pid_list(pids)
    succeeded: list[int] = []
    failed: list[dict[str, Any]] = []
    for pid in checked_pids:
        try:
            force_kill(pid)
            succeeded.append(pid)
        except SysprimsError as exc:
            failed.append({"pid": pid, "error": exc.message})
    return {
        "schema_id": _BATCH_KILL_SCHEMA_ID,
        "succeeded": succeeded,
        "failed": failed,
    }


# =============================================================================
# Terminate Tree & Timeout Execution
# =============================================================================


def terminate_tree(pid: int, config: Mapping[str, Any] | None = None) -> dict[str, Any]:
    """Terminate a process with escalation (`signal` -> wait -> `kill_signal`)."""
    checked_pid = _require_pid(pid, "pid")
    if config is None:
        return _call_json(_native.terminate_tree, checked_pid, "")
    if not isinstance(config, Mapping):
        raise SysprimsError(SysprimsErrorCode.INVALID_ARGUMENT, "config must be a mapping")
    wire = {"schema_id": config.get("schema_id", _TERMINATE_TREE_SCHEMA_ID), **dict(config)}
    return _call_json(_native.terminate_tree, checked_pid, _serialize_json_mapping(wire, "config"))


def run_with_timeout(
    command: str,
    args: Sequence[str] = (),
    *,
    timeout_ms: int,
    kill_after_ms: int | None = None,
    signal: int | None = None,
    grouping: str = "group_by_default",
    preserve_status: bool = False,
) -> dict[str, Any]:
    """Execute `command` with `args` under `timeout_ms`, escalating if needed."""
    if not isinstance(command, str) or not command:
        raise SysprimsError(SysprimsErrorCode.INVALID_ARGUMENT, "command must be a non-empty string")
    if isinstance(args, (str, bytes, bytearray)) or not isinstance(args, Sequence):
        raise SysprimsError(SysprimsErrorCode.INVALID_ARGUMENT, "args must be a sequence of strings")
    arg_list: list[str] = []
    for idx, item in enumerate(args):
        if not isinstance(item, str):
            raise SysprimsError(SysprimsErrorCode.INVALID_ARGUMENT, f"args[{idx}] must be a string")
        arg_list.append(item)
    checked_timeout = _require_int(timeout_ms, "timeout_ms")
    if checked_timeout < 0 or checked_timeout > _MAX_DURATION_MS:
        raise SysprimsError(
            SysprimsErrorCode.INVALID_ARGUMENT,
            f"timeout_ms must be between 0 and {_MAX_DURATION_MS}",
        )
    if grouping not in ("group_by_default", "foreground"):
        raise SysprimsError(
            SysprimsErrorCode.INVALID_ARGUMENT,
            f"grouping must be 'group_by_default' or 'foreground', got {grouping!r}",
        )
    _require_bool(preserve_status, "preserve_status")
    wire: dict[str, Any] = {
        "command": command,
        "args": arg_list,
        "timeout_ms": checked_timeout,
        "grouping": grouping,
        "preserve_status": preserve_status,
    }
    if kill_after_ms is not None:
        ka = _require_int(kill_after_ms, "kill_after_ms")
        if ka < 0 or ka > _MAX_DURATION_MS:
            raise SysprimsError(
                SysprimsErrorCode.INVALID_ARGUMENT,
                f"kill_after_ms must be between 0 and {_MAX_DURATION_MS}",
            )
        wire["kill_after_ms"] = ka
    if signal is not None:
        wire["signal"] = _require_int(signal, "signal")
    return _call_json(_native.run_with_timeout, json.dumps(wire, allow_nan=False))


# =============================================================================
# Session Spawn, Group Spawn & Containment
# =============================================================================


def run_setsid(
    argv: Sequence[str],
    *,
    cwd: str | None = None,
    env: Mapping[str, str] | None = None,
    wait: bool = False,
) -> dict[str, Any]:
    """Spawn `argv` in a detached session (`setsid` on Unix, detached console/group on Windows)."""
    checked_argv = _require_argv(argv)
    checked_env = _require_env(env)
    _require_bool(wait, "wait")
    if cwd is not None and (not isinstance(cwd, str) or not cwd):
        raise SysprimsError(SysprimsErrorCode.INVALID_ARGUMENT, "cwd must be a non-empty string")
    wire: dict[str, Any] = {
        "schema_id": _RUN_SETSID_SCHEMA_ID,
        "argv": checked_argv,
        "wait": wait,
    }
    if cwd is not None:
        wire["cwd"] = cwd
    if checked_env is not None:
        wire["env"] = checked_env
    return _call_json(_native.run_setsid, json.dumps(wire, allow_nan=False))


def run_nohup(
    argv: Sequence[str],
    *,
    cwd: str | None = None,
    env: Mapping[str, str] | None = None,
    wait: bool = False,
    output_file: str | None = None,
) -> dict[str, Any]:
    """Spawn `argv` with `SIGHUP` ignored in the inherited session (Unix only)."""
    checked_argv = _require_argv(argv)
    checked_env = _require_env(env)
    _require_bool(wait, "wait")
    if cwd is not None and (not isinstance(cwd, str) or not cwd):
        raise SysprimsError(SysprimsErrorCode.INVALID_ARGUMENT, "cwd must be a non-empty string")
    if output_file is not None and (not isinstance(output_file, str) or not output_file):
        raise SysprimsError(
            SysprimsErrorCode.INVALID_ARGUMENT,
            "output_file must be a non-empty string",
        )
    wire: dict[str, Any] = {
        "schema_id": _RUN_NOHUP_SCHEMA_ID,
        "argv": checked_argv,
        "wait": wait,
    }
    if cwd is not None:
        wire["cwd"] = cwd
    if checked_env is not None:
        wire["env"] = checked_env
    if output_file is not None:
        wire["output_file"] = output_file
    return _call_json(_native.run_nohup, json.dumps(wire, allow_nan=False))


def spawn_in_group(
    argv: Sequence[str],
    *,
    cwd: str | None = None,
    env: Mapping[str, str] | None = None,
) -> dict[str, Any]:
    """Spawn `argv` in a new process group or Windows Job Object."""
    checked_argv = _require_argv(argv)
    checked_env = _require_env(env)
    if cwd is not None and (not isinstance(cwd, str) or not cwd):
        raise SysprimsError(SysprimsErrorCode.INVALID_ARGUMENT, "cwd must be a non-empty string")
    wire: dict[str, Any] = {
        "schema_id": _SPAWN_IN_GROUP_SCHEMA_ID,
        "argv": checked_argv,
    }
    if cwd is not None:
        wire["cwd"] = cwd
    if checked_env is not None:
        wire["env"] = checked_env
    return _call_json(_native.spawn_in_group, json.dumps(wire, allow_nan=False))


def _close_token_quietly(token: int) -> None:
    try:
        _native.containment_close(token)
    except Exception:
        pass


class ContainedProcess:
    """Opaque handle to a native-owned contained process (`spawn_contained`).

    Supports `with spawn_contained(...) as proc:` context management; exiting the
    `with` block closes the handle (terminating and reaping the contained group
    or Job Object if still active).
    """

    __slots__ = ("_finalizer", "_token", "__weakref__")

    def __init__(self, _token: int, *, _internal: bool = False) -> None:
        if not _internal:
            raise TypeError("ContainedProcess must be created via spawn_contained()")
        self._token: int | None = _token
        self._finalizer = weakref.finalize(self, _close_token_quietly, _token)

    def _require_token(self) -> int:
        if self._token is None:
            raise SysprimsError(
                SysprimsErrorCode.INVALID_ARGUMENT,
                "containment handle is stale or closed",
            )
        return self._token

    @property
    def closed(self) -> bool:
        """Whether `close()` has completed on this handle."""
        return self._token is None

    def identity(self) -> dict[str, Any]:
        """Return the active or inert containment snapshot without polling child exit."""
        return _call_json(_native.containment_identity, self._require_token())

    def poll(self) -> dict[str, Any]:
        """Poll the contained leader process without blocking."""
        return _call_json(_native.containment_poll, self._require_token())

    def wait(self, timeout_ms: int | None = None) -> dict[str, Any]:
        """Wait for the contained leader process to finish (`None` waits without timeout)."""
        token = self._require_token()
        if timeout_ms is None:
            return _call_json(_native.containment_wait, token, None)
        checked = _require_int(timeout_ms, "timeout_ms")
        if checked < 0 or checked > _MAX_DURATION_MS:
            raise SysprimsError(
                SysprimsErrorCode.INVALID_ARGUMENT,
                f"timeout_ms must be between 0 and {_MAX_DURATION_MS}",
            )
        return _call_json(_native.containment_wait, token, checked)

    def terminate(self) -> dict[str, Any]:
        """Terminate the contained group or Job Object with configured escalation."""
        return _call_json(_native.containment_terminate, self._require_token())

    def close(self) -> None:
        """Release the native containment slot (idempotent; terminates if still running)."""
        token = self._token
        if token is None:
            return
        _call_void(_native.containment_close, token)
        self._token = None
        self._finalizer.detach()

    def __enter__(self) -> ContainedProcess:
        self._require_token()
        return self

    def __exit__(self, exc_type: Any, exc: Any, tb: Any) -> None:
        self.close()

    def __repr__(self) -> str:
        state = "closed" if self._token is None else "open"
        return f"<ContainedProcess {state}>"

    def __copy__(self) -> ContainedProcess:
        raise TypeError("ContainedProcess cannot be copied")

    def __deepcopy__(self, memo: Any) -> ContainedProcess:
        raise TypeError("ContainedProcess cannot be deep-copied")

    def __reduce__(self) -> Any:
        raise TypeError("ContainedProcess cannot be pickled")


def spawn_contained(
    argv: Sequence[str],
    *,
    cwd: str | None = None,
    env: Mapping[str, str] | None = None,
    execution_timeout_ms: int | None = None,
    grace_timeout_ms: int | None = None,
    kill_timeout_ms: int | None = None,
    signal: int | None = None,
    kill_signal: int | None = None,
) -> ContainedProcess:
    """Spawn `argv` inside an owned containment boundary and return a `ContainedProcess`."""
    checked_argv = _require_argv(argv)
    checked_env = _require_env(env)
    if cwd is not None and (not isinstance(cwd, str) or not cwd):
        raise SysprimsError(SysprimsErrorCode.INVALID_ARGUMENT, "cwd must be a non-empty string")
    wire: dict[str, Any] = {
        "schema_id": _CONTAINMENT_SPAWN_SCHEMA_ID,
        "argv": checked_argv,
    }
    if cwd is not None:
        wire["cwd"] = cwd
    if checked_env is not None:
        wire["env"] = checked_env
    if execution_timeout_ms is not None:
        et = _require_int(execution_timeout_ms, "execution_timeout_ms")
        if et < 1 or et > _MAX_DURATION_MS:
            raise SysprimsError(
                SysprimsErrorCode.INVALID_ARGUMENT,
                f"execution_timeout_ms must be between 1 and {_MAX_DURATION_MS}",
            )
        wire["execution_timeout_ms"] = et
    if grace_timeout_ms is not None:
        gt = _require_int(grace_timeout_ms, "grace_timeout_ms")
        if gt < 0 or gt > _MAX_DURATION_MS:
            raise SysprimsError(
                SysprimsErrorCode.INVALID_ARGUMENT,
                f"grace_timeout_ms must be between 0 and {_MAX_DURATION_MS}",
            )
        wire["grace_timeout_ms"] = gt
    if kill_timeout_ms is not None:
        kt = _require_int(kill_timeout_ms, "kill_timeout_ms")
        if kt < 0 or kt > _MAX_DURATION_MS:
            raise SysprimsError(
                SysprimsErrorCode.INVALID_ARGUMENT,
                f"kill_timeout_ms must be between 0 and {_MAX_DURATION_MS}",
            )
        wire["kill_timeout_ms"] = kt
    if signal is not None:
        sig = _require_int(signal, "signal")
        if sig == 0:
            raise SysprimsError(
                SysprimsErrorCode.INVALID_ARGUMENT,
                "signal 0 is not a valid termination policy",
            )
        wire["signal"] = sig
    if kill_signal is not None:
        ksig = _require_int(kill_signal, "kill_signal")
        if ksig == 0:
            raise SysprimsError(
                SysprimsErrorCode.INVALID_ARGUMENT,
                "kill_signal 0 is not a valid termination policy",
            )
        wire["kill_signal"] = ksig
    try:
        token: int = _native.containment_spawn(json.dumps(wire, allow_nan=False))
    except _native.RawSysprimsError as exc:
        raise SysprimsError(*exc.args) from None
    return ContainedProcess(token, _internal=True)


# =============================================================================
# RFC 5545 Recurrence (`contract: recurrence/v0`)
# =============================================================================


class RecurrenceRule:
    """Validated, normalized RFC 5545 recurrence rule (`contract: recurrence/v0`)."""

    __slots__ = ("_raw",)

    def __init__(self, _raw: Any, *, _internal: bool = False) -> None:
        if not _internal:
            raise TypeError(
                "RecurrenceRule must be constructed via from_json(), from_document_json(), or from_text()"
            )
        self._raw = _raw

    @classmethod
    def from_json(cls, data: BytesInput) -> RecurrenceRule:
        """Decode and validate a rule from a JSON rule object (`bytes` or `str`)."""
        raw_bytes = _bytes_input(data, "data")
        try:
            return cls(_native.RawRecurrenceRule.from_json(raw_bytes), _internal=True)
        except _native.RawRecurrenceRefusal as exc:
            raise RecurrenceRefusal(*exc.args) from None

    @classmethod
    def from_document_json(cls, data: BytesInput) -> RecurrenceRule:
        """Decode and validate a rule from a capability-enveloped JSON document."""
        raw_bytes = _bytes_input(data, "data")
        try:
            return cls(_native.RawRecurrenceRule.from_document_json(raw_bytes), _internal=True)
        except _native.RawRecurrenceRefusal as exc:
            raise RecurrenceRefusal(*exc.args) from None

    @classmethod
    def from_text(cls, lines: Sequence[str]) -> RecurrenceRule:
        """Decode and validate a rule from RFC 5545 content lines."""
        if isinstance(lines, (str, bytes, bytearray)) or not isinstance(lines, Sequence):
            raise TypeError("lines must be a sequence of strings, not a single str/bytes")
        line_list = list(lines)
        for idx, line in enumerate(line_list):
            if not isinstance(line, str):
                raise TypeError(f"lines[{idx}] must be a str, not {type(line).__name__}")
        try:
            return cls(_native.RawRecurrenceRule.from_text(line_list), _internal=True)
        except _native.RawRecurrenceRefusal as exc:
            raise RecurrenceRefusal(*exc.args) from None

    @property
    def dtstart(self) -> str:
        """Local civil start timestamp (`YYYY-MM-DDTHH:MM:SS`)."""
        return str(self._raw.dtstart())

    @property
    def tzid(self) -> str:
        """IANA time-zone identifier from the compiled tzdb bundle."""
        return str(self._raw.tzid())

    @property
    def freq(self) -> str:
        """Normalized frequency (`MINUTELY`, `HOURLY`, `DAILY`, `WEEKLY`, `MONTHLY`, `YEARLY`)."""
        return str(self._raw.freq())

    def to_canonical_json(self) -> bytes:
        """Return RFC 8785 (JCS) canonical UTF-8 bytes of the normalized rule."""
        out: bytes = self._raw.to_canonical_json()
        return out

    def digest(self) -> str:
        """Return `sha256:<hex>` of `to_canonical_json()`."""
        return str(self._raw.digest())

    def to_text(self) -> list[str]:
        """Return canonical RFC 5545 content lines."""
        return list(self._raw.to_text())

    def expand(self, from_utc: str, to_utc: str, limit: int) -> list[str]:
        """Expand occurrences in the half-open UTC range `[from_utc, to_utc)` up to `limit`."""
        if isinstance(limit, bool) or not isinstance(limit, int):
            raise TypeError(f"limit must be an int, not {type(limit).__name__}")
        try:
            return list(self._raw.expand(from_utc, to_utc, limit))
        except _native.RawRecurrenceQueryError as exc:
            raise RecurrenceQueryError(*exc.args) from None

    def next_after(self, after_utc: str) -> str | None:
        """Return the earliest occurrence strictly after `after_utc`, or `None` if exhausted."""
        try:
            res: str | None = self._raw.next_after(after_utc)
            return res
        except _native.RawRecurrenceQueryError as exc:
            raise RecurrenceQueryError(*exc.args) from None

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, RecurrenceRule):
            return NotImplemented
        return self.to_canonical_json() == other.to_canonical_json()

    def __hash__(self) -> int:
        return hash(self.to_canonical_json())

    def __repr__(self) -> str:
        return f"RecurrenceRule(dtstart={self.dtstart!r}, tzid={self.tzid!r}, freq={self.freq!r})"


def tzdb_version() -> str:
    """Return the IANA tzdb version compiled into `sysprims-recurrence`."""
    return str(_native.tzdb_version())
