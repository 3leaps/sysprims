# SPDX-License-Identifier: MIT OR Apache-2.0
from __future__ import annotations

import copy
import hashlib
import json
import os
from pathlib import Path
import pickle
import socket
import subprocess
import sys
import tempfile
import unittest

import sysprims


def _jcs(value: object) -> str:
    if isinstance(value, dict):
        parts = [f"{json.dumps(k)}:{_jcs(value[k])}" for k in sorted(value)]
        return "{" + ",".join(parts) + "}"
    if isinstance(value, list):
        return "[" + ",".join(_jcs(x) for x in value) + "]"
    return json.dumps(value, separators=(",", ":"))


def _recurrence_v0_dir() -> Path:
    override = os.environ.get("SYSPRIMS_RECURRENCE_V0_DIR")
    if override:
        return Path(override)
    return (
        Path(__file__).resolve().parents[3]
        / "crates"
        / "sysprims-recurrence"
        / "tests"
        / "recurrence-v0"
    )


def _signal_vectors_path() -> Path:
    override = os.environ.get("SIGNAL_CONFORMANCE_VECTORS")
    if override:
        return Path(override)
    return (
        Path(__file__).resolve().parents[3]
        / "testdata"
        / "conformance"
        / "signal"
        / "vectors.json"
    )


def _session_vectors_path() -> Path:
    override = os.environ.get("SESSION_CONFORMANCE_VECTORS")
    if override:
        return Path(override)
    return (
        Path(__file__).resolve().parents[3]
        / "testdata"
        / "conformance"
        / "session"
        / "vectors.json"
    )


def _timeout_vectors_path() -> Path:
    override = os.environ.get("TIMEOUT_CONFORMANCE_VECTORS")
    if override:
        return Path(override)
    return (
        Path(__file__).resolve().parents[3]
        / "testdata"
        / "conformance"
        / "timeout"
        / "vectors.json"
    )


def _proc_vectors_path() -> Path:
    override = os.environ.get("PROC_CONFORMANCE_VECTORS")
    if override:
        return Path(override)
    return (
        Path(__file__).resolve().parents[3]
        / "testdata"
        / "conformance"
        / "proc"
        / "vectors.json"
    )


class SysprimsMetadataAndValidationTests(unittest.TestCase):
    def test_metadata_and_constants(self) -> None:
        self.assertEqual(sysprims.abi_version(), 1)
        self.assertIn(sysprims.platform(), ("linux", "macos", "windows"))
        self.assertRegex(sysprims.__version__, r"^\d+\.\d+\.\d+")
        self.assertEqual(sysprims.RECURRENCE_CONTRACT, "contract: recurrence/v0")
        self.assertTrue(sysprims.RECURRENCE_CORPUS_DIGEST.startswith("sha256:"))
        self.assertTrue(len(sysprims.tzdb_version()) >= 5)

    def test_bool_and_out_of_range_pids_refused(self) -> None:
        for bad_pid in (True, False, 0, -1, 0x8000_0000, 1 << 80, "123"):
            with self.assertRaises(sysprims.SysprimsError) as ctx:
                sysprims.proc_get(bad_pid)  # type: ignore[arg-type]
            self.assertEqual(
                ctx.exception.code, sysprims.SysprimsErrorCode.INVALID_ARGUMENT
            )

        with self.assertRaises(sysprims.SysprimsError) as ctx:
            sysprims.terminate(True)  # type: ignore[arg-type]
        self.assertEqual(
            ctx.exception.code, sysprims.SysprimsErrorCode.INVALID_ARGUMENT
        )

        with self.assertRaises(sysprims.SysprimsError) as ctx:
            sysprims.wait_pid(os.getpid(), True)  # type: ignore[arg-type]
        self.assertEqual(
            ctx.exception.code, sysprims.SysprimsErrorCode.INVALID_ARGUMENT
        )

    def test_unknown_filter_fields_refused(self) -> None:
        with self.assertRaises(sysprims.SysprimsError) as ctx:
            sysprims.process_list({"unknown_field": 1})
        self.assertEqual(
            ctx.exception.code, sysprims.SysprimsErrorCode.INVALID_ARGUMENT
        )


class SysprimsProcessAndControlTests(unittest.TestCase):
    def test_process_inspection_and_ancestors(self) -> None:
        me = os.getpid()
        info = sysprims.proc_get(me, include_env=True, include_threads=True)
        self.assertEqual(info["pid"], me)

        snap = sysprims.process_list({"pid_in": [me]})
        pids = [entry["pid"] for entry in snap["processes"]]
        self.assertIn(me, pids)

        anc = sysprims.ancestors(me, max_depth=4)
        self.assertGreaterEqual(len(anc["chain"]), 1)
        self.assertEqual(anc["chain"][0]["pid"], me)

        desc = sysprims.descendants(me, max_levels=2)
        self.assertEqual(desc["root_pid"], me)

        ev = sysprims.guard_step({"rule": {"root_pid": me}, "action_enabled": False})
        self.assertIn("matched", ev)

    def test_listening_ports_and_list_fds(self) -> None:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as srv:
            srv.bind(("127.0.0.1", 0))
            srv.listen(1)
            port = srv.getsockname()[1]
            ports = sysprims.listening_ports({"local_port": port})
            self.assertIn("bindings", ports)
            bound_ports = [b["local_port"] for b in ports["bindings"]]
            self.assertIn(port, bound_ports)

        if sys.platform == "win32":
            with self.assertRaises(sysprims.SysprimsError) as ctx:
                sysprims.list_fds(os.getpid())
            self.assertEqual(
                ctx.exception.code, sysprims.SysprimsErrorCode.NOT_SUPPORTED
            )
        else:
            fds = sysprims.list_fds(os.getpid())
            self.assertGreater(len(fds["fds"]), 0)

    def test_self_ids_and_signals(self) -> None:
        if sys.platform == "win32":
            for fn in (sysprims.self_getpgid, sysprims.self_getsid):
                with self.assertRaises(sysprims.SysprimsError) as ctx:
                    fn()
                self.assertEqual(
                    ctx.exception.code, sysprims.SysprimsErrorCode.NOT_SUPPORTED
                )
        else:
            self.assertGreater(sysprims.self_getpgid(), 0)
            self.assertGreater(sysprims.self_getsid(), 0)

        proc = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
        try:
            if sys.platform != "win32":
                sysprims.signal_send(proc.pid, 0)
            sysprims.terminate(proc.pid)
            proc.wait(timeout=10)
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.wait(timeout=5)

        proc2 = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
        try:
            res = sysprims.terminate_many([proc2.pid])
            self.assertEqual(res["succeeded"], [proc2.pid])
            self.assertEqual(res["failed"], [])
            proc2.wait(timeout=10)
        finally:
            if proc2.poll() is None:
                proc2.kill()
                proc2.wait(timeout=5)

    def test_signal_catalog_and_process_handle(self) -> None:
        self.assertEqual(sysprims.MAX_SAFE_PID, 2_147_483_647)
        self.assertEqual(sysprims.resolve_signal_name("SIGTERM"), sysprims.SIGTERM)
        self.assertEqual(sysprims.resolve_signal_name("term"), sysprims.SIGTERM)
        self.assertEqual(sysprims.resolve_signal_name("SIGKILL"), sysprims.SIGKILL)
        self.assertEqual(sysprims.match_signal_names("SIGU*"), ["SIGUSR1", "SIGUSR2"])

        with self.assertRaises(TypeError):
            sysprims.ProcessHandle(object())

        me = os.getpid()
        with sysprims.open_process_handle(me) as self_h:
            self.assertEqual(self_h.pid, me)
            self.assertTrue(len(self_h.start_identity) > 0)
            self.assertIn(
                self_h.binding,
                (
                    sysprims.SignalBinding.BOUND,
                    sysprims.SignalBinding.CHECKED_NUMERIC,
                ),
            )
            self.assertFalse(self_h.exited())
            with self.assertRaises(TypeError):
                copy.copy(self_h)
            with self.assertRaises(TypeError):
                copy.deepcopy(self_h)
            with self.assertRaises(TypeError):
                pickle.dumps(self_h)

            with sysprims.open_process_handle_verified(
                me, self_h.start_identity
            ) as verified:
                self.assertEqual(verified.start_identity, self_h.start_identity)

            with self.assertRaises(sysprims.SysprimsError) as mismatch_ctx:
                sysprims.open_process_handle_verified(me, "0000000000.999999")
            self.assertEqual(
                mismatch_ctx.exception.code,
                sysprims.SysprimsErrorCode.NOT_FOUND,
            )

        self.assertTrue(self_h.closed)
        with self.assertRaises(sysprims.SysprimsError) as closed_ctx:
            self_h.exited()
        self.assertEqual(
            closed_ctx.exception.code,
            sysprims.SysprimsErrorCode.INVALID_ARGUMENT,
        )

        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
        try:
            with sysprims.open_process_handle(child.pid) as ch:
                self.assertFalse(ch.exited())
                used_binding = ch.force_kill()
                self.assertEqual(used_binding, ch.binding)
                child.wait(timeout=10)
                self.assertTrue(ch.exited())
                self.assertTrue(ch.exited())  # latched
                with self.assertRaises(sysprims.SysprimsError) as second_ctx:
                    ch.force_kill()
                self.assertEqual(
                    second_ctx.exception.code,
                    sysprims.SysprimsErrorCode.NOT_FOUND,
                )
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=5)

    def test_signal_conformance_vectors(self) -> None:
        doc = json.loads(_signal_vectors_path().read_text(encoding="utf-8"))
        self.assertEqual(doc["schema"], "sysprims-signal-conformance/v1")

        ndjson_lines: list[dict[str, object]] = []

        constants = doc["constants"]
        self.assertEqual(constants["max_safe_pid"], sysprims.MAX_SAFE_PID)
        common = constants["signals_common"]
        self.assertEqual(common["SIGHUP"], sysprims.SIGHUP)
        self.assertEqual(common["SIGINT"], sysprims.SIGINT)
        self.assertEqual(common["SIGQUIT"], sysprims.SIGQUIT)
        self.assertEqual(common["SIGKILL"], sysprims.SIGKILL)
        self.assertEqual(common["SIGPIPE"], sysprims.SIGPIPE)
        self.assertEqual(common["SIGALRM"], sysprims.SIGALRM)
        self.assertEqual(common["SIGTERM"], sysprims.SIGTERM)
        plat_key = "darwin" if sys.platform == "darwin" else "other"
        plat = constants["signals_platform"][plat_key]
        self.assertEqual(plat["SIGUSR1"], sysprims.SIGUSR1)
        self.assertEqual(plat["SIGUSR2"], sysprims.SIGUSR2)

        ndjson_lines.append(
            {
                "suite": "constants",
                "id": "signal-constants",
                "max_safe_pid": sysprims.MAX_SAFE_PID,
                "sighup": sysprims.SIGHUP,
                "sigint": sysprims.SIGINT,
                "sigquit": sysprims.SIGQUIT,
                "sigkill": sysprims.SIGKILL,
                "sigpipe": sysprims.SIGPIPE,
                "sigalrm": sysprims.SIGALRM,
                "sigterm": sysprims.SIGTERM,
                "sigusr1": sysprims.SIGUSR1,
                "sigusr2": sysprims.SIGUSR2,
            }
        )

        for case in doc["pid_validation_cases"]:
            cid = case["id"]
            kind = case["kind"]
            val = case["value"]
            expected = case["expected"]
            if expected == "ok":
                if kind == "pid":
                    sysprims.validate_pid(val)
                else:
                    sysprims.validate_pgid(val)
                got = "ok"
            else:
                with self.assertRaises(sysprims.SysprimsError, msg=cid) as ctx:
                    if kind == "pid":
                        sysprims.validate_pid(val)
                    else:
                        sysprims.validate_pgid(val)
                got = ctx.exception.code.name.lower()
                self.assertEqual(got, expected, cid)
            ndjson_lines.append(
                {
                    "suite": "pid-validation",
                    "id": cid,
                    "kind": kind,
                    "outcome": got,
                }
            )

        for case in doc["batch_validation_cases"]:
            cid = case["id"]
            pids = case["pids"]
            expected = case["expected"]
            with self.assertRaises(sysprims.SysprimsError, msg=cid) as ctx:
                sysprims.kill_many(pids, sysprims.SIGTERM)
            got = ctx.exception.code.name.lower()
            self.assertEqual(got, expected, cid)
            ndjson_lines.append(
                {
                    "suite": "batch-validation",
                    "id": cid,
                    "outcome": got,
                }
            )

        for case in doc["resolve_signal_cases"]:
            cid = case["id"]
            inp = case["input"]
            expected = case["expected"]
            if expected == "ok":
                signum = sysprims.resolve_signal_name(inp)
                want_signum = getattr(sysprims, case["signal"])
                self.assertEqual(signum, want_signum, cid)
                ndjson_lines.append(
                    {
                        "suite": "resolve-signal",
                        "id": cid,
                        "outcome": "ok",
                        "signum": signum,
                    }
                )
            else:
                with self.assertRaises(sysprims.SysprimsError, msg=cid) as ctx:
                    sysprims.resolve_signal_name(inp)
                got = ctx.exception.code.name.lower()
                self.assertEqual(got, expected, cid)
                ndjson_lines.append(
                    {
                        "suite": "resolve-signal",
                        "id": cid,
                        "outcome": got,
                    }
                )

        for case in doc["match_signal_cases"]:
            cid = case["id"]
            got_matches = sysprims.match_signal_names(case["pattern"])
            self.assertEqual(got_matches, case["expected"], cid)
            ndjson_lines.append(
                {
                    "suite": "match-signal",
                    "id": cid,
                    "matches": got_matches,
                }
            )

        for case in doc["process_handle_validation_cases"]:
            cid = case["id"]
            pid = os.getpid() if case["pid"] == "self" else case["pid"]
            expected = case["expected"]
            with self.assertRaises(sysprims.SysprimsError, msg=cid) as ctx:
                if "expected_start_identity" in case:
                    sysprims.open_process_handle_verified(
                        pid, case["expected_start_identity"]
                    )
                else:
                    sysprims.open_process_handle(pid)
            got = ctx.exception.code.name.lower()
            self.assertEqual(got, expected, cid)
            ndjson_lines.append(
                {
                    "suite": "process-handle-validation",
                    "id": cid,
                    "outcome": got,
                }
            )

        out_path = os.environ.get("SIGNAL_NDJSON_OUT")
        if out_path:
            Path(out_path).write_text(
                "".join(f"{_jcs(row)}\n" for row in ndjson_lines),
                encoding="utf-8",
                newline="\n",
            )

    def test_timeout_session_and_containment(self) -> None:
        ok = sysprims.run_with_timeout(
            sys.executable,
            ["-c", "raise SystemExit(0)"],
            timeout_ms=10_000,
        )
        self.assertEqual(ok["status"], "completed")
        self.assertEqual(ok["exit_code"], 0)

        timed_out = sysprims.run_with_timeout(
            sys.executable,
            ["-c", "import time; time.sleep(30)"],
            timeout_ms=150,
            kill_after_ms=200,
        )
        self.assertEqual(timed_out["status"], "timed_out")

        if sys.platform == "win32":
            with self.assertRaises(sysprims.SysprimsError) as ctx:
                sysprims.run_setsid(
                    [sys.executable, "-c", "raise SystemExit(0)"],
                    wait=True,
                )
            self.assertEqual(
                ctx.exception.code, sysprims.SysprimsErrorCode.NOT_SUPPORTED
            )
        else:
            setsid_res = sysprims.run_setsid(
                [sys.executable, "-c", "raise SystemExit(0)"],
                wait=True,
            )
            self.assertEqual(setsid_res["status"], "completed")
            self.assertEqual(setsid_res["exit_code"], 0)

        if sys.platform == "win32":
            with self.assertRaises(sysprims.SysprimsError) as ctx:
                sysprims.run_nohup(
                    [sys.executable, "-c", "raise SystemExit(0)"], wait=True
                )
            self.assertEqual(
                ctx.exception.code, sysprims.SysprimsErrorCode.NOT_SUPPORTED
            )
        else:
            with tempfile.TemporaryDirectory() as tmp:
                out_file = str(Path(tmp) / "nohup.out")
                nohup_res = sysprims.run_nohup(
                    [sys.executable, "-c", "print('hello-nohup')"],
                    wait=True,
                    output_file=out_file,
                )
                self.assertEqual(nohup_res["status"], "completed")
                self.assertIn("hello-nohup", Path(out_file).read_text(encoding="utf-8"))

        with self.assertRaises(TypeError):
            sysprims.ContainedProcess(1)  # type: ignore[misc]

        with self.assertRaises(sysprims.SysprimsError) as ctx:
            sysprims.spawn_contained([])
        self.assertEqual(
            ctx.exception.code, sysprims.SysprimsErrorCode.INVALID_ARGUMENT
        )

        if sys.platform == "win32":
            with self.assertRaises(sysprims.SysprimsError) as ctx:
                sysprims.spawn_contained([sys.executable, "-c", "raise SystemExit(0)"])
            self.assertEqual(
                ctx.exception.code, sysprims.SysprimsErrorCode.NOT_SUPPORTED
            )
        else:
            with sysprims.spawn_contained(
                [sys.executable, "-c", "raise SystemExit(0)"]
            ) as handle:
                self.assertEqual(repr(handle), "<ContainedProcess open>")
                with self.assertRaises(TypeError):
                    copy.copy(handle)
                with self.assertRaises(TypeError):
                    copy.deepcopy(handle)
                with self.assertRaises(TypeError):
                    pickle.dumps(handle)
                ident = handle.identity()
                self.assertIn("identity", ident)
                self.assertGreater(ident["identity"]["pid"], 0)
                waited = handle.wait(timeout_ms=10_000)
                self.assertEqual(waited["leader_status"], "completed")

            self.assertTrue(handle.closed)
            self.assertEqual(repr(handle), "<ContainedProcess closed>")
            handle.close()  # idempotent
            with self.assertRaises(sysprims.SysprimsError) as ctx:
                handle.poll()
            self.assertEqual(
                ctx.exception.code, sysprims.SysprimsErrorCode.INVALID_ARGUMENT
            )

            sleep_handle = sysprims.spawn_contained(
                [sys.executable, "-c", "import time; time.sleep(30)"],
                grace_timeout_ms=200,
                kill_timeout_ms=200,
            )
            try:
                term_snap = sleep_handle.terminate()
                self.assertEqual(term_snap["leader_status"], "terminated")
            finally:
                sleep_handle.close()

    def test_session_conformance_vectors(self) -> None:
        doc = json.loads(_session_vectors_path().read_text(encoding="utf-8"))
        self.assertEqual(doc["schema"], "sysprims-session-conformance/v1")

        ndjson_lines: list[dict[str, object]] = []
        schemas = doc["schemas"]
        ndjson_lines.append(
            {
                "suite": "schemas",
                "run_setsid_config_v1": schemas["run_setsid_config_v1"],
                "run_nohup_config_v1": schemas["run_nohup_config_v1"],
                "session_spawn_result_v1": schemas["session_spawn_result_v1"],
                "spawn_in_group_config_v1": schemas["spawn_in_group_config_v1"],
                "spawn_in_group_result_v1": schemas["spawn_in_group_result_v1"],
            }
        )

        for case in doc["config_validation_cases"]:
            cid = case["id"]
            verb = case["verb"]
            schema_id = case["schema_id"]
            argv = case["argv"]
            expected = case["expected"]
            with self.assertRaises(sysprims.SysprimsError, msg=cid) as ctx:
                if verb == "setsid":
                    sysprims.run_setsid(argv, wait=True, schema_id=schema_id)
                elif verb == "nohup":
                    sysprims.run_nohup(argv, wait=True, schema_id=schema_id)
                else:
                    sysprims.spawn_in_group(argv, schema_id=schema_id)
            got = ctx.exception.code.name.lower()
            self.assertEqual(got, expected, cid)
            ndjson_lines.append(
                {
                    "suite": "config-validation",
                    "id": cid,
                    "verb": verb,
                    "outcome": got,
                }
            )

        for case in doc["self_introspection_cases"]:
            cid = case["id"]
            op = case["op"]
            expected = (
                case["expected_windows"]
                if sys.platform == "win32"
                else case["expected_unix"]
            )
            try:
                val = (
                    sysprims.self_getpgid()
                    if op == "self_getpgid"
                    else sysprims.self_getsid()
                )
                got = "ok" if val > 0 else "invalid_value"
            except sysprims.SysprimsError as err:
                got = err.code.name.lower()
            self.assertEqual(got, expected, cid)
            ndjson_lines.append(
                {
                    "suite": "self-introspection",
                    "id": cid,
                    "op": op,
                    "outcome": got,
                }
            )

        for case in doc["spawn_cases"]:
            cid = case["id"]
            verb = case["verb"]
            argv = case["argv"]
            wait = bool(case.get("wait", False))
            use_temp_output = bool(case.get("use_temp_output_file", False))
            use_symlink_output = bool(case.get("use_symlink_output_file", False))
            expected_obj = (
                case["expected_windows"]
                if sys.platform == "win32"
                else case["expected_unix"]
            )

            if verb == "setsid":
                try:
                    res = sysprims.run_setsid(argv, wait=wait)
                    self.assertEqual(
                        res["schema_id"], schemas["session_spawn_result_v1"]
                    )
                    structural_match = (
                        isinstance(res["pid"], int)
                        and res["pid"] > 0
                        and res["pid"] == res["sid"]
                        and res["pid"] == res["pgid"]
                    )
                    row: dict[str, object] = {
                        "suite": "spawn",
                        "id": cid,
                        "outcome": "ok",
                        "verb": res["verb"],
                        "status": res["status"],
                        "session_kind": res["session_kind"],
                        "identifier_provenance": res["identifier_provenance"],
                        "structural_ids_match": structural_match,
                        "exit_code": res["exit_code"],
                    }
                except sysprims.SysprimsError as err:
                    row = {
                        "suite": "spawn",
                        "id": cid,
                        "outcome": err.code.name.lower(),
                    }
                for k, v in expected_obj.items():
                    self.assertEqual(row[k], v, f"{cid}.{k}")
                ndjson_lines.append(row)

            elif verb == "nohup":
                with tempfile.TemporaryDirectory() as tmp:
                    output_file: str | None = None
                    if use_symlink_output:
                        target_path = Path(tmp) / "target.log"
                        link_path = Path(tmp) / "link.log"
                        target_path.write_text("existing", encoding="utf-8")
                        if sys.platform != "win32":
                            link_path.symlink_to(target_path)
                        output_file = str(link_path)
                    elif use_temp_output:
                        output_file = str(Path(tmp) / "nohup.out")

                    caller_sid = (
                        sysprims.self_getsid() if sys.platform != "win32" else 0
                    )
                    caller_pgid = (
                        sysprims.self_getpgid() if sys.platform != "win32" else 0
                    )
                    try:
                        res = sysprims.run_nohup(
                            argv, wait=wait, output_file=output_file
                        )
                        self.assertEqual(
                            res["schema_id"], schemas["session_spawn_result_v1"]
                        )
                        inherited_match = (
                            isinstance(res["pid"], int)
                            and res["pid"] > 0
                            and res["sid"] == caller_sid
                            and res["pgid"] == caller_pgid
                        )
                        row = {
                            "suite": "spawn",
                            "id": cid,
                            "outcome": "ok",
                            "verb": res["verb"],
                            "status": res["status"],
                            "session_kind": res["session_kind"],
                            "identifier_provenance": res["identifier_provenance"],
                            "inherited_context_matches": inherited_match,
                            "exit_code": res["exit_code"],
                        }
                    except sysprims.SysprimsError as err:
                        row = {
                            "suite": "spawn",
                            "id": cid,
                            "outcome": err.code.name.lower(),
                        }
                    for k, v in expected_obj.items():
                        self.assertEqual(row[k], v, f"{cid}.{k}")
                    ndjson_lines.append(row)

            elif verb == "spawn_in_group":
                try:
                    res = sysprims.spawn_in_group(argv)
                    self.assertEqual(
                        res["schema_id"], schemas["spawn_in_group_result_v1"]
                    )
                    structural_match = (
                        isinstance(res["pid"], int)
                        and res["pid"] > 0
                        and res["pgid"] == res["pid"]
                    )
                    row = {
                        "suite": "spawn",
                        "id": cid,
                        "outcome": "ok",
                        "structural_ids_match": structural_match,
                        "tree_kill_reliability": res["tree_kill_reliability"],
                    }
                except sysprims.SysprimsError as err:
                    row = {
                        "suite": "spawn",
                        "id": cid,
                        "outcome": err.code.name.lower(),
                    }
                for k, v in expected_obj.items():
                    self.assertEqual(row[k], v, f"{cid}.{k}")
                ndjson_lines.append(row)

        out_path = os.environ.get("SESSION_NDJSON_OUT")
        if out_path:
            Path(out_path).write_text(
                "".join(f"{_jcs(row)}\n" for row in ndjson_lines),
                encoding="utf-8",
                newline="\n",
            )

    def test_timeout_conformance_vectors(self) -> None:
        doc = json.loads(_timeout_vectors_path().read_text(encoding="utf-8"))
        self.assertEqual(doc["schema"], "sysprims-timeout-conformance/v1")

        ndjson_lines: list[dict[str, object]] = []
        schemas = doc["schemas"]
        ndjson_lines.append(
            {
                "suite": "schemas",
                "timeout_result_v1_1": schemas["timeout_result_v1_1"],
                "terminate_tree_config_v1": schemas["terminate_tree_config_v1"],
                "terminate_tree_result_v1": schemas["terminate_tree_result_v1"],
                "containment_spawn_config_v1": schemas["containment_spawn_config_v1"],
                "containment_snapshot_v1": schemas["containment_snapshot_v1"],
            }
        )

        for case in doc["terminate_tree_validation_cases"]:
            cid = case["id"]
            pid = case["pid"]
            expected = case["expected"]
            cfg: dict[str, object] = {}
            for field in (
                "schema_id",
                "grace_timeout_ms",
                "kill_timeout_ms",
                "signal",
                "kill_signal",
            ):
                if field in case:
                    cfg[field] = case[field]
            with self.assertRaises(sysprims.SysprimsError, msg=cid) as ctx:
                sysprims.terminate_tree(pid, cfg if cfg else None)
            got = ctx.exception.code.name.lower()
            self.assertEqual(got, expected, cid)
            ndjson_lines.append(
                {
                    "suite": "terminate-tree-validation",
                    "id": cid,
                    "outcome": got,
                }
            )

        for case in doc["spawn_contained_validation_cases"]:
            cid = case["id"]
            argv = case["argv"]
            expected = case["expected"]
            kwargs: dict[str, object] = {}
            for field in (
                "cwd",
                "execution_timeout_ms",
                "grace_timeout_ms",
                "kill_timeout_ms",
                "signal",
                "kill_signal",
            ):
                if field in case:
                    kwargs[field] = case[field]
            with self.assertRaises(sysprims.SysprimsError, msg=cid) as ctx:
                proc = sysprims.spawn_contained(argv, **kwargs)  # type: ignore[arg-type]
                proc.close()
            got = ctx.exception.code.name.lower()
            self.assertEqual(got, expected, cid)
            ndjson_lines.append(
                {
                    "suite": "spawn-contained-validation",
                    "id": cid,
                    "outcome": got,
                }
            )

        for case in doc["containment_lifecycle_cases"]:
            cid = case["id"]
            op = case["op"]
            argv = case["argv"]
            wait_timeout_ms = int(case.get("wait_timeout_ms", 5000))
            expected_obj = (
                case["expected_windows"]
                if sys.platform == "win32"
                else case["expected_unix"]
            )
            kwargs = {}
            for field in (
                "execution_timeout_ms",
                "grace_timeout_ms",
                "kill_timeout_ms",
            ):
                if field in case:
                    kwargs[field] = case[field]
            try:
                with sysprims.spawn_contained(argv, **kwargs) as proc:  # type: ignore[arg-type]
                    if op in ("wait", "deadline"):
                        snap = proc.wait(timeout_ms=wait_timeout_ms)
                    else:
                        snap = proc.terminate()
                    self.assertEqual(
                        snap["schema_id"], schemas["containment_snapshot_v1"]
                    )
                    row: dict[str, object] = {
                        "suite": "containment-lifecycle",
                        "id": cid,
                        "outcome": "ok",
                        "handle_state": snap["handle_state"],
                        "leader_status": snap["leader_status"],
                        "tree_kill_reliability": snap["tree_kill_reliability"],
                        "boundary_strength": snap["boundary_strength"],
                        "exited": snap["exited"],
                        "timed_out": snap["timed_out"],
                    }
            except sysprims.SysprimsError as err:
                row = {
                    "suite": "containment-lifecycle",
                    "id": cid,
                    "outcome": err.code.name.lower(),
                }
            for k, v in expected_obj.items():
                self.assertEqual(row[k], v, f"{cid}.{k}")
            ndjson_lines.append(row)

        out_path = os.environ.get("TIMEOUT_NDJSON_OUT")
        if out_path:
            Path(out_path).write_text(
                "".join(f"{_jcs(row)}\n" for row in ndjson_lines),
                encoding="utf-8",
                newline="\n",
            )


class SysprimsRecurrenceCorpusTests(unittest.TestCase):
    def test_manifest_and_full_recurrence_v0_corpus(self) -> None:
        base = _recurrence_v0_dir()
        corpus = base / "corpus"
        manifest_bytes = (corpus / "MANIFEST.json").read_bytes()
        manifest_digest = "sha256:" + hashlib.sha256(manifest_bytes).hexdigest()
        self.assertEqual(manifest_digest, sysprims.RECURRENCE_CORPUS_DIGEST)

        manifest = json.loads(manifest_bytes.decode("utf-8"))
        self.assertEqual(manifest["tzdb_version"], sysprims.tzdb_version())
        for rel, want_sha in manifest["files"].items():
            got_sha = (
                "sha256:" + hashlib.sha256((corpus / rel).read_bytes()).hexdigest()
            )
            self.assertEqual(got_sha, want_sha, rel)

        vectors = sorted((corpus / "vectors").glob("*.json"))
        rejects = sorted((corpus / "rejects").glob("*.json"))
        self.assertEqual(len(vectors), 34)
        self.assertEqual(len(rejects), 38)

        ndjson_lines: list[dict[str, object]] = []

        def decode_vector_input(vec: dict[str, object]) -> sysprims.RecurrenceRule:
            if "rule" in vec:
                return sysprims.RecurrenceRule.from_json(json.dumps(vec["rule"]))
            lines = vec["input_text"]
            assert isinstance(lines, list)
            return sysprims.RecurrenceRule.from_text(lines)

        for path in vectors:
            name = path.stem
            vec = json.loads(path.read_bytes().decode("utf-8"))
            rule = decode_vector_input(vec)
            canonical = rule.to_canonical_json().decode("utf-8")
            self.assertEqual(canonical, _jcs(vec["canonical"]), name)
            self.assertEqual(rule.digest(), vec["digest"], name)
            text = rule.to_text()
            self.assertEqual(text, vec["text"], name)
            self.assertEqual(sysprims.RecurrenceRule.from_text(text), rule, name)
            self.assertEqual(
                sysprims.RecurrenceRule.from_json(rule.to_canonical_json()), rule, name
            )

            results: list[dict[str, object]] = []
            for q in vec["queries"]:
                got = rule.expand(q["from"], q["to"], q["limit"])
                self.assertEqual(got, q["expected"], f"{name}: expand")
                results.append(
                    {
                        "kind": "expand",
                        "from": q["from"],
                        "to": q["to"],
                        "limit": q["limit"],
                        "result": got,
                    }
                )
            for na in vec["next_after"]:
                got_na = rule.next_after(na["after"])
                self.assertEqual(got_na, na["expected"], f"{name}: next_after")
                results.append(
                    {
                        "kind": "next_after",
                        "after": na["after"],
                        "result": got_na,
                    }
                )
            ndjson_lines.append(
                {
                    "vector": name,
                    "canonical": canonical,
                    "digest": rule.digest(),
                    "text": text,
                    "results": results,
                }
            )

        for path in rejects:
            name = path.stem
            vec = json.loads(path.read_bytes().decode("utf-8"))
            want_reason = vec["reason"]
            with self.assertRaises(sysprims.RecurrenceRefusal, msg=name) as rctx:
                decode_vector_input(vec)
            self.assertEqual(rctx.exception.reason, want_reason, name)
            self.assertIn(rctx.exception.reason, sysprims.RECURRENCE_REFUSAL_REASONS)
            ndjson_lines.append({"vector": name, "reason": rctx.exception.reason})

        out_path = os.environ.get("RECURRENCE_NDJSON_OUT")
        if out_path:
            Path(out_path).write_text(
                "".join(f"{_jcs(row)}\n" for row in ndjson_lines),
                encoding="utf-8",
                newline="\n",
            )

        readme = (base / "README.md").read_text(encoding="utf-8")
        for ex_path in sorted((base / "examples").glob("*.json")):
            ex_bytes = ex_path.read_bytes()
            ex_rule = sysprims.RecurrenceRule.from_json(ex_bytes)
            row = next(
                line for line in readme.splitlines() if f"`{ex_path.name}`" in line
            )
            self.assertIn(ex_rule.digest(), row)
            self.assertEqual(
                ex_rule.to_canonical_json().decode("utf-8"),
                _jcs(json.loads(ex_bytes.decode("utf-8"))),
            )

        doc_ok = (base / "documents" / "first-thursday-monthly.json").read_bytes()
        doc_rule = sysprims.RecurrenceRule.from_document_json(doc_ok)
        inner_rule = sysprims.RecurrenceRule.from_json(
            json.dumps(json.loads(doc_ok.decode("utf-8"))["rule"])
        )
        self.assertEqual(doc_rule, inner_rule)
        self.assertEqual(hash(doc_rule), hash(inner_rule))

        doc_bad = (base / "documents" / "reject-missing-capability.json").read_bytes()
        with self.assertRaises(sysprims.RecurrenceRefusal):
            sysprims.RecurrenceRule.from_document_json(doc_bad)

    def test_recurrence_query_errors_and_argument_validation(self) -> None:
        rule = sysprims.RecurrenceRule.from_json(
            b'{"dtstart":"2026-01-01T09:00:00","tzid":"Etc/UTC","freq":"DAILY"}'
        )
        with self.assertRaises(sysprims.RecurrenceQueryError) as qctx:
            rule.expand("2026-01-10T00:00:00Z", "2026-01-01T00:00:00Z", 5)
        self.assertEqual(qctx.exception.kind, "reversed_range")

        with self.assertRaises(TypeError):
            rule.expand("2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z", True)  # type: ignore[arg-type]

        with self.assertRaises(ValueError):
            rule.expand("2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z", 0)

        with self.assertRaises(ValueError):
            rule.next_after("not-a-timestamp")

    def test_proc_conformance_vectors(self) -> None:
        doc = json.loads(_proc_vectors_path().read_text(encoding="utf-8"))
        self.assertEqual(doc["suite"], "sysprims-proc-conformance")

        ndjson_lines: list[dict[str, object]] = []
        ndjson_lines.append(
            {
                "suite": "schemas",
                "process_info_v1": "https://schemas.3leaps.dev/sysprims/process/v1.1.0/process-info.schema.json",
                "process_info_sampled_v1": "https://schemas.3leaps.dev/sysprims/process/v1.1.0/process-info-sampled.schema.json",
                "descendants_result_v1": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/descendants-result.schema.json",
                "descendants_result_sampled_v1": "https://schemas.3leaps.dev/sysprims/process/v1.1.0/descendants-result-sampled.schema.json",
                "ancestors_result_v1": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/ancestors-result.schema.json",
                "wait_pid_result_v1": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/wait-pid-result.schema.json",
                "fd_snapshot_v1": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/fd-snapshot.schema.json",
                "port_bindings_v1": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/port-bindings.schema.json",
                "port_filter_v1": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/port-filter.schema.json",
                "guard_event_v1": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/guard-event.schema.json",
            }
        )

        synthetic_tree = doc["synthetic_tree"]
        presets = {
            "interactive": {"interval_ms": 3000, "sample_duration_ms": 2000},
            "background": {"interval_ms": 180000, "sample_duration_ms": 3000},
            "watchdog": {"interval_ms": 300000, "sample_duration_ms": 5000},
        }

        def matches_filter(proc: dict[str, object], flt: dict[str, object]) -> bool:
            if "name_contains" in flt:
                if str(flt["name_contains"]).lower() not in str(proc["name"]).lower():
                    return False
            if "name_equals" in flt and proc["name"] != flt["name_equals"]:
                return False
            if "user_equals" in flt and proc.get("user") != flt["user_equals"]:
                return False
            if "pid_in" in flt and proc["pid"] not in flt["pid_in"]:  # type: ignore[operator]
                return False
            if "ppid" in flt and proc["ppid"] != flt["ppid"]:
                return False
            if "state_in" in flt and proc["state"] not in flt["state_in"]:  # type: ignore[operator]
                return False
            if "cpu_above" in flt and not (
                float(proc["cpu_percent"]) > float(flt["cpu_above"])
            ):  # type: ignore[arg-type]
                return False
            if "memory_above_kb" in flt and not (
                int(proc["memory_kb"]) > int(flt["memory_above_kb"])
            ):  # type: ignore[arg-type]
                return False
            if "running_for_at_least_secs" in flt and not (
                int(proc["elapsed_seconds"]) >= int(flt["running_for_at_least_secs"])
            ):  # type: ignore[arg-type]
                return False
            return True

        def select_targets(
            tree: dict[str, object], flt: dict[str, object], cascade: bool
        ) -> list[int]:
            matched_roots: list[int] = []
            children_map: dict[int, list[int]] = {}
            for lvl in tree["levels"]:  # type: ignore[attr-defined]
                for p in lvl["processes"]:
                    ppid = int(p["ppid"])
                    pid = int(p["pid"])
                    children_map.setdefault(ppid, []).append(pid)
                    if matches_filter(p, flt):
                        matched_roots.append(pid)
            root_pid = int(tree["root_pid"])  # type: ignore[arg-type]
            selected = {pid for pid in matched_roots if pid != root_pid}
            if cascade:
                stack = list(matched_roots)
                visited: set[int] = set()
                while stack:
                    cur = stack.pop()
                    if cur in visited:
                        continue
                    visited.add(cur)
                    if cur != root_pid:
                        selected.add(cur)
                    for child in children_map.get(cur, []):
                        if child not in visited:
                            stack.append(child)
            return sorted(selected)

        for v in doc["vectors"]:
            vid = v["id"]
            kind = v["kind"]
            if kind == "validate_pid":
                with self.assertRaises(sysprims.SysprimsError, msg=vid) as ctx:
                    sysprims.proc_get(int(v["pid"]))
                got = ctx.exception.code.name.lower()
                self.assertEqual(got, v["expected_error_category"], vid)
                ndjson_lines.append({"id": vid, "kind": kind, "outcome": got})
            elif kind == "validate_filter":
                with self.assertRaises(sysprims.SysprimsError, msg=vid) as ctx:
                    sysprims.process_list(v["filter"])
                got = ctx.exception.code.name.lower()
                self.assertEqual(got, v["expected_error_category"], vid)
                ndjson_lines.append({"id": vid, "kind": kind, "outcome": got})
            elif kind == "validate_port_filter":
                with self.assertRaises(sysprims.SysprimsError, msg=vid) as ctx:
                    sysprims.listening_ports(
                        {"protocol": "tcp", "local_port": int(v["local_port"])}
                    )
                got = ctx.exception.code.name.lower()
                self.assertEqual(got, v["expected_error_category"], vid)
                ndjson_lines.append({"id": vid, "kind": kind, "outcome": got})
            elif kind == "guard_preset":
                p = presets[v["preset"]]
                self.assertEqual(p["interval_ms"], v["expected"]["interval_ms"], vid)
                self.assertEqual(
                    p["sample_duration_ms"], v["expected"]["sample_duration_ms"], vid
                )
                ndjson_lines.append(
                    {
                        "id": vid,
                        "kind": kind,
                        "preset": v["preset"],
                        "interval_ms": p["interval_ms"],
                        "sample_duration_ms": p["sample_duration_ms"],
                    }
                )
            elif kind == "select_targets":
                got_pids = select_targets(
                    synthetic_tree, v["filter"], bool(v["cascade"])
                )
                self.assertEqual(got_pids, v["expected_pids"], vid)
                ndjson_lines.append(
                    {
                        "id": vid,
                        "kind": kind,
                        "cascade": bool(v["cascade"]),
                        "matched_pids": got_pids,
                    }
                )
            elif kind == "live_probe_self":
                self_pid = os.getpid()
                self_info = sysprims.proc_get(self_pid)
                anc = sysprims.ancestors(self_pid, max_depth=3)
                desc = sysprims.descendants(self_pid, max_levels=1)
                guard = sysprims.guard_step(
                    {
                        "rule": {
                            "root_pid": self_pid,
                            "max_levels": 1,
                            "cpu_mode": "lifetime",
                        },
                        "action": {
                            "kind": "kill_descendants",
                            "signal": 15,
                            "cascade": False,
                        },
                        "action_enabled": False,
                        "max_targets": 8,
                    }
                )
                ndjson_lines.append(
                    {
                        "id": vid,
                        "kind": kind,
                        "ancestors_chain_non_empty": len(anc["chain"]) > 0
                        and anc["chain"][0]["pid"] == self_pid,
                        "ancestors_schema_id": anc["schema_id"],
                        "descendants_schema_id": desc["schema_id"],
                        "fd_snapshot_schema_id": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/fd-snapshot.schema.json",
                        "guard_event_schema_id": guard["schema_id"],
                        "port_bindings_schema_id": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/port-bindings.schema.json",
                        "process_info_schema_id": "https://schemas.3leaps.dev/sysprims/process/v1.1.0/process-info.schema.json",
                        "self_found": self_info["pid"] == self_pid
                        and bool(self_info["name"]),
                    }
                )
            else:
                self.fail(f"unknown vector kind: {kind}")

        out_path = os.environ.get("PROC_NDJSON_OUT")
        if out_path:
            Path(out_path).write_text(
                "".join(f"{_jcs(row)}\n" for row in ndjson_lines),
                encoding="utf-8",
                newline="\n",
            )


if __name__ == "__main__":
    unittest.main()
