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


if __name__ == "__main__":
    unittest.main()
