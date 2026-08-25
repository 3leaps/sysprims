import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { loadSysprims } from "../src/ffi";
import {
  ancestors,
  descendants,
  forceKill,
  guardStep,
  listeningPorts,
  listFds,
  processList,
  procGet,
  runNohup,
  runSetsid,
  SysprimsError,
  SysprimsErrorCode,
  selfPGID,
  selfSID,
  terminate,
  terminateTree,
  waitPID,
} from "../src/index";

// -----------------------------------------------------------------------------
// Test Helpers
// -----------------------------------------------------------------------------

async function waitForExit(child: ReturnType<typeof spawn>, timeoutMs: number): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  await Promise.race([
    once(child, "exit"),
    new Promise((_, reject) =>
      setTimeout(() => reject(new Error("child did not exit in time")), timeoutMs),
    ),
  ]);
}

function ownedChildPid(child: ReturnType<typeof spawn>): number {
  const pid = child.pid;
  if (pid === undefined || pid <= 1 || pid > 0x7fffffff) {
    throw new Error("spawned child did not have a safe owned PID");
  }
  return pid;
}

async function cleanupOwnedChild(child: ReturnType<typeof spawn>): Promise<void> {
  if (child.exitCode === null && child.signalCode === null) {
    child.kill();
  }
  await waitForExit(child, 5000);
  assert.ok(child.exitCode !== null || child.signalCode !== null, "child cleanup was not proven");
}

function spawnLongRunningChild(env?: NodeJS.ProcessEnv): ReturnType<typeof spawn> {
  // Long-running process we fully control.
  // Using setInterval keeps the process alive until terminated.
  return spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], {
    env,
    stdio: "ignore",
  });
}

async function pollUntil<T>(probe: () => T | undefined, timeoutMs: number): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const result = probe();
    if (result !== undefined) return result;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error("condition was not observed before timeout");
}

// -----------------------------------------------------------------------------
// Process Inspection Tests
// -----------------------------------------------------------------------------

test("procGet(process.pid) returns matching pid", () => {
  const info = procGet(process.pid);
  assert.equal(info.pid, process.pid);
});

test("procGet(process.pid, { includeThreads }) returns matching pid", () => {
  const info = procGet(process.pid, { includeThreads: true });
  assert.equal(info.pid, process.pid);
  if (info.thread_count != null) {
    assert.ok(info.thread_count > 0, "thread_count should be > 0 when present");
  }
});

test("processList() returns processes including current process", () => {
  const snapshot = processList();

  assert.ok(snapshot.schema_id, "snapshot should have schema_id");
  assert.ok(snapshot.timestamp, "snapshot should have timestamp");
  assert.ok(Array.isArray(snapshot.processes), "processes should be an array");
  assert.ok(snapshot.processes.length > 0, "should have at least one process");

  // Current process should be in the list
  const self = snapshot.processes.find((p) => p.pid === process.pid);
  assert.ok(self, "current process should be in the list");
});

test("processList({ pid_in: [...] }) filters correctly", () => {
  const snapshot = processList({ pid_in: [process.pid] });

  assert.equal(snapshot.processes.length, 1, "should return exactly one process");
  assert.equal(snapshot.processes[0].pid, process.pid);
});

test("processList(filter, { includeThreads }) accepts options", () => {
  const snapshot = processList({ pid_in: [process.pid] }, { includeThreads: true });
  assert.equal(snapshot.processes.length, 1, "should return exactly one process");
  assert.equal(snapshot.processes[0].pid, process.pid);
});

test("processList({ name_contains }) filters correctly", () => {
  // Get current process name first
  const current = procGet(process.pid);
  const snapshot = processList({ name_contains: current.name });

  assert.ok(snapshot.processes.length >= 1, "should find at least current process");
  const found = snapshot.processes.find((p) => p.pid === process.pid);
  assert.ok(found, "current process should match its own name filter");
});

test("guardStep action-disabled returns structured event", () => {
  const event = guardStep({
    rule: {
      root_pid: process.pid,
      max_levels: 1,
    },
    action_enabled: false,
    max_targets: 8,
  });

  assert.ok(event.schema_id.includes("guard-event"));
  assert.equal(event.targeted, 0);
  assert.equal(event.killed, 0);
  assert.equal(event.failed, 0);
});

test("descendants projects opt-in process detail for an owned child", async (t) => {
  const markerName = "SYSPRIMS_DESCENDANTS_TEST_MARKER";
  const markerValue = `${process.pid}-${Date.now()}`;
  const child = spawnLongRunningChild({ ...process.env, [markerName]: markerValue });
  const pid = ownedChildPid(child);

  try {
    await once(child, "spawn");
    const info = await pollUntil(() => {
      const result = descendants(process.pid, {
        includeEnv: true,
        includeThreads: true,
        maxLevels: 1,
      });
      return result.levels.flatMap((level) => level.processes).find((item) => item.pid === pid);
    }, 5000);

    const direct = procGet(pid, { includeEnv: true, includeThreads: true });
    if (direct.env?.[markerName] === markerValue) {
      assert.equal(
        info.env?.[markerName],
        markerValue,
        "descendant environment marker must round-trip",
      );
    } else if (direct.thread_count != null) {
      assert.ok(
        info.thread_count != null && info.thread_count > 0,
        "thread detail must be enriched",
      );
    } else {
      t.skip("environment and thread details are unavailable under current platform permissions");
      return;
    }
  } finally {
    await cleanupOwnedChild(child);
  }
});

test("direct native boundary rejects lossy numeric inputs without unsafe targets", async () => {
  const lib = loadSysprims();
  const invalidCode = SysprimsErrorCode.InvalidArgument;
  const aliasOfPidOne = 4294967297;
  const child = spawnLongRunningChild(process.env);
  const pid = ownedChildPid(child);
  const aliasOfOwnedPid = pid + 4294967296;

  try {
    await once(child, "spawn");
    assert.equal(lib.sysprimsProcGet(aliasOfPidOne).code, invalidCode);
    assert.equal(lib.sysprimsProcGetEx(aliasOfPidOne, "").code, invalidCode);
    assert.equal(lib.sysprimsProcListFds(aliasOfOwnedPid, "").code, invalidCode);
    assert.equal(lib.sysprimsProcWaitPid(pid, 0.5).code, invalidCode);
    assert.equal(lib.sysprimsProcWaitPid(pid, 4294967297).code, invalidCode);
    assert.equal(lib.sysprimsProcWaitPid(aliasOfOwnedPid, 0).code, invalidCode);
    assert.equal(lib.sysprimsProcDescendants(aliasOfOwnedPid, 0, "").code, invalidCode);
    assert.equal(lib.sysprimsProcDescendants(process.pid, -0.5, "").code, invalidCode);
    assert.equal(lib.sysprimsProcDescendants(process.pid, 4294967297, "").code, invalidCode);
    assert.equal(lib.sysprimsProcKillDescendants(aliasOfOwnedPid, 0, 15, "").code, invalidCode);
    assert.equal(lib.sysprimsProcKillDescendants(pid, 4294967297, 15, "").code, invalidCode);
    assert.equal(lib.sysprimsProcKillDescendants(pid, 0, 0.5, "").code, invalidCode);
    assert.equal(lib.sysprimsProcAncestors(aliasOfOwnedPid, 0, "").code, invalidCode);
    assert.equal(lib.sysprimsProcAncestors(process.pid, 4294967297, "").code, invalidCode);
    assert.equal(lib.sysprimsSignalSend(aliasOfOwnedPid, 0).code, invalidCode);
    assert.equal(lib.sysprimsSignalSend(pid, 0.5).code, invalidCode);
    assert.equal(lib.sysprimsSignalSend(pid, 4294967297).code, invalidCode);
    assert.equal(lib.sysprimsSignalSendGroup(aliasOfOwnedPid, 0).code, invalidCode);
    assert.equal(lib.sysprimsTerminate(aliasOfOwnedPid).code, invalidCode);
    assert.equal(lib.sysprimsForceKill(aliasOfOwnedPid).code, invalidCode);
    assert.equal(lib.sysprimsTerminateTree(aliasOfOwnedPid, "").code, invalidCode);
  } finally {
    await cleanupOwnedChild(child);
  }
});

test("direct native descendants remains compatible with three arguments", () => {
  const result = loadSysprims().sysprimsProcDescendants(process.pid, 0, "");
  assert.equal(result.code, SysprimsErrorCode.Ok);
});

test("direct native guard rejects explicit zero max_targets", () => {
  const result = loadSysprims().sysprimsProcGuardStep(
    JSON.stringify({
      action_enabled: false,
      max_targets: 0,
      rule: { root_pid: process.pid },
    }),
  );
  assert.equal(result.code, SysprimsErrorCode.InvalidArgument);
});

test("ancestors(process.pid) returns chain starting with self", () => {
  const result = ancestors(process.pid);
  assert.ok(result.schema_id.includes("ancestors-result"));
  assert.equal(result.pid, process.pid);
  assert.ok(result.chain.length >= 1, "chain should include at least the starting process");
  assert.equal(result.chain[0].pid, process.pid, "first chain element should be starting PID");
});

test("listFds(process.pid) returns a snapshot", () => {
  if (process.platform === "win32") {
    assert.throws(
      () => listFds(process.pid),
      (e: unknown) => e instanceof SysprimsError && e.code === SysprimsErrorCode.NotSupported,
    );
    return;
  }

  const snap = listFds(process.pid);
  assert.ok(snap.schema_id);
  assert.ok(snap.timestamp);
  assert.ok(snap.platform);
  assert.equal(snap.pid, process.pid);
  assert.ok(Array.isArray(snap.fds));
  assert.ok(Array.isArray(snap.warnings));
});

test("listeningPorts() returns a snapshot with required fields", () => {
  const snapshot = listeningPorts();

  assert.ok(snapshot.schema_id, "snapshot should have schema_id");
  assert.ok(snapshot.timestamp, "snapshot should have timestamp");
  assert.ok(snapshot.platform, "snapshot should have platform");
  assert.ok(Array.isArray(snapshot.bindings), "bindings should be an array");
  assert.ok(Array.isArray(snapshot.warnings), "warnings should be an array");
});

test("listeningPorts({ protocol: 'tcp' }) filters correctly", () => {
  const snapshot = listeningPorts({ protocol: "tcp" });

  for (const binding of snapshot.bindings) {
    assert.equal(binding.protocol, "tcp", "all bindings should be TCP");
  }
});

// -----------------------------------------------------------------------------
// Self Introspection Tests
// -----------------------------------------------------------------------------

test("selfPGID/selfSID are > 0 on Unix or NotSupported on Windows", () => {
  if (process.platform === "win32") {
    assert.throws(
      () => selfPGID(),
      (e: unknown) => e instanceof SysprimsError && e.code === SysprimsErrorCode.NotSupported,
    );
    assert.throws(
      () => selfSID(),
      (e: unknown) => e instanceof SysprimsError && e.code === SysprimsErrorCode.NotSupported,
    );
    return;
  }

  assert.ok(selfPGID() > 0);
  assert.ok(selfSID() > 0);
});

// -----------------------------------------------------------------------------
// Session Spawn Tests
// -----------------------------------------------------------------------------

test("runSetsid wait result returns structural identifiers", () => {
  if (process.platform === "win32") {
    assert.throws(
      () => runSetsid({ argv: ["cmd", "/C", "exit 0"], wait: true }),
      (e: unknown) => e instanceof SysprimsError && e.code === SysprimsErrorCode.NotSupported,
    );
    return;
  }

  const result = runSetsid({ argv: ["sh", "-c", "exit 0"], wait: true });
  assert.equal(result.verb, "setsid");
  assert.equal(result.status, "completed");
  assert.equal(result.session_kind, "new_session");
  assert.equal(result.identifier_provenance, "setsid_structural_child_pid");
  assert.ok(result.pid != null && result.pid > 0);
  assert.equal(result.sid, result.pid);
  assert.equal(result.pgid, result.pid);
  assert.equal(result.exit_code, 0);
});

test("runNohup completed result returns inherited caller context", () => {
  if (process.platform === "win32") {
    assert.throws(
      () => runNohup({ argv: ["cmd", "/C", "exit 0"], wait: true }),
      (e: unknown) => e instanceof SysprimsError && e.code === SysprimsErrorCode.NotSupported,
    );
    return;
  }

  const callerSID = selfSID();
  const callerPGID = selfPGID();
  const result = runNohup({
    argv: ["sh", "-c", "exit 0"],
    output_file: "/dev/null",
    wait: true,
  });
  assert.equal(result.verb, "nohup");
  assert.equal(result.status, "completed");
  assert.equal(result.session_kind, "inherited_session");
  assert.equal(result.identifier_provenance, "caller_context_before_spawn");
  assert.ok(result.pid != null && result.pid > 0);
  assert.equal(result.sid, callerSID);
  assert.equal(result.pgid, callerPGID);
  assert.equal(result.exit_code, 0);
});

test("runNohup rejects symlink output_file", () => {
  if (process.platform === "win32") {
    return;
  }

  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sysprims-nohup-"));
  try {
    const target = path.join(dir, "target.log");
    const link = path.join(dir, "link.log");
    fs.writeFileSync(target, "existing");
    fs.symlinkSync(target, link);

    assert.throws(
      () => runNohup({ argv: ["sh", "-c", "exit 0"], wait: true, output_file: link }),
      (e: unknown) => e instanceof SysprimsError && e.code === SysprimsErrorCode.PermissionDenied,
    );
  } finally {
    fs.rmSync(dir, { force: true, recursive: true });
  }
});

// -----------------------------------------------------------------------------
// Signal Tests (error-path only, using spawn-and-exit for safe PIDs)
// -----------------------------------------------------------------------------

test("terminate rejects pid 0", () => {
  assert.throws(
    () => terminate(0),
    (e: unknown) => e instanceof SysprimsError && e.code === SysprimsErrorCode.InvalidArgument,
  );
});

test("forceKill rejects pid 0", () => {
  assert.throws(
    () => forceKill(0),
    (e: unknown) => e instanceof SysprimsError && e.code === SysprimsErrorCode.InvalidArgument,
  );
});

test("waitPID(process.pid, 1ms) returns timed_out", () => {
  const pid = process.pid;
  const r = waitPID(pid, 1);
  assert.equal(r.pid, pid);
  assert.equal(r.timed_out, true);
});

test("terminate kills a spawned child process", async () => {
  const child = spawnLongRunningChild();
  const pid = ownedChildPid(child);

  try {
    await once(child, "spawn");
    terminate(pid);
    await waitForExit(child, 5000);
  } finally {
    await cleanupOwnedChild(child);
  }
});

test("forceKill kills a spawned child process", async () => {
  const child = spawnLongRunningChild();
  const pid = ownedChildPid(child);

  try {
    await once(child, "spawn");
    forceKill(pid);
    await waitForExit(child, 5000);
  } finally {
    await cleanupOwnedChild(child);
  }
});

test("terminateTree kills a spawned child process", async () => {
  const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "ignore" });
  const pid = ownedChildPid(child);

  try {
    await once(child, "spawn");
    const r = terminateTree(pid, { grace_timeout_ms: 100, kill_timeout_ms: 5000 });
    assert.equal(r.pid, pid);
    await waitForExit(child, 5000);
  } finally {
    await cleanupOwnedChild(child);
  }
});
