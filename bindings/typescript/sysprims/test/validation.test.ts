import assert from "node:assert/strict";
import test from "node:test";

import { SysprimsError, SysprimsErrorCode } from "../src/errors";
import {
  ancestors,
  descendants,
  guardStep,
  listeningPorts,
  processList,
  procGet,
  signalSend,
  signalSendGroup,
  terminateTree,
  waitPID,
} from "../src/index";
import {
  MAX_SAFE_PID,
  U32_MAX,
  validateDuration,
  validateMaxLevels,
  validatePid,
  validatePort,
  validateSignal,
} from "../src/validation";

function rejectsInvalidArgument(fn: () => unknown): void {
  assert.throws(
    fn,
    (error: unknown) =>
      error instanceof SysprimsError && error.code === SysprimsErrorCode.InvalidArgument,
  );
}

test("PID validation rejects lossy and dangerous JavaScript values", () => {
  assert.equal(validatePid(1), 1);
  assert.equal(validatePid(MAX_SAFE_PID), MAX_SAFE_PID);

  for (const value of [0, -1, 1.5, NaN, Infinity, -Infinity, MAX_SAFE_PID + 1, 4294967297]) {
    rejectsInvalidArgument(() => validatePid(value));
  }
});

test("signal validation enforces the signed i32 range", () => {
  assert.equal(validateSignal(-0x80000000), -0x80000000);
  assert.equal(validateSignal(0x7fffffff), 0x7fffffff);

  for (const value of [-0x80000001, 0x80000000, 1.5, NaN, Infinity, -Infinity]) {
    rejectsInvalidArgument(() => validateSignal(value));
  }
});

test("maxLevels preserves only the documented Infinity sentinel", () => {
  assert.equal(validateMaxLevels(undefined), U32_MAX);
  assert.equal(validateMaxLevels(Infinity), U32_MAX);
  assert.equal(validateMaxLevels(0), 0);
  assert.equal(validateMaxLevels(U32_MAX), U32_MAX);

  for (const value of [-1, 1.5, NaN, -Infinity, U32_MAX + 1]) {
    rejectsInvalidArgument(() => validateMaxLevels(value));
  }
});

test("duration and port validation reject non-integral or out-of-range values", () => {
  assert.equal(validateDuration(0, "duration"), 0);
  assert.equal(validateDuration(Number.MAX_SAFE_INTEGER, "duration"), Number.MAX_SAFE_INTEGER);
  assert.equal(validatePort(1), 1);
  assert.equal(validatePort(65535), 65535);

  for (const value of [-1, 0.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) {
    rejectsInvalidArgument(() => validateDuration(value, "duration"));
  }
  for (const value of [0, -1, 1.5, 65536, NaN, Infinity]) {
    rejectsInvalidArgument(() => validatePort(value));
  }
});

test("public APIs reject numeric inputs before serialization or native loading", () => {
  const aliasOfPidOne = 4294967297;

  rejectsInvalidArgument(() => procGet(aliasOfPidOne));
  rejectsInvalidArgument(() => processList({ pid_in: [aliasOfPidOne] }));
  rejectsInvalidArgument(() => listeningPorts({ local_port: 0 }));
  rejectsInvalidArgument(() => descendants(process.pid, { maxLevels: NaN }));
  rejectsInvalidArgument(() => ancestors(process.pid, { maxDepth: Infinity }));
  rejectsInvalidArgument(() => waitPID(process.pid, 0.5));
  rejectsInvalidArgument(() => signalSend(process.pid, 0x80000000));
  rejectsInvalidArgument(() => signalSendGroup(aliasOfPidOne, 0));
  rejectsInvalidArgument(() => terminateTree(process.pid, { grace_timeout_ms: 0.5 }));
  rejectsInvalidArgument(() =>
    guardStep({ rule: { root_pid: aliasOfPidOne }, action_enabled: false }),
  );
  rejectsInvalidArgument(() =>
    guardStep({ rule: { root_pid: process.pid }, action_enabled: false, max_targets: 0 }),
  );
});
