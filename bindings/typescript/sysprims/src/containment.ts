import { SysprimsError, SysprimsErrorCode } from "./errors";
import { callJsonReturnAsync, callTokenReturnAsync, callVoidAsync, loadSysprims } from "./ffi";
import type { ContainmentSnapshot, SpawnContainedOptions } from "./types";
import { validateDuration, validateSignal } from "./validation";

const CONTAINMENT_SPAWN_SCHEMA_ID =
  "https://schemas.3leaps.dev/sysprims/timeout/v1.0.0/containment-spawn-config.schema.json";
const MAX_DURATION_MS = 86_400_000;
const MAX_ARGV_ENTRIES = 256;
const MAX_ARGV_ENTRY_BYTES = 4096;

const leakBackstop = new FinalizationRegistry<bigint>((token) => {
  try {
    const native = loadSysprims();
    void Promise.resolve(native.sysprimsContainmentClose(token)).catch(() => {
      // Finalizer is a leak backstop only.
    });
  } catch {
    // Finalizer is a leak backstop only.
  }
});

export interface ContainedProcessWaitOptions {
  timeoutMs?: number;
  signal?: AbortSignal;
}

function validateArgv(argv: string[]): void {
  if (!Array.isArray(argv) || argv.length === 0) {
    throw new SysprimsError(SysprimsErrorCode.InvalidArgument, "argv must not be empty");
  }
  if (argv.length > MAX_ARGV_ENTRIES) {
    throw new SysprimsError(
      SysprimsErrorCode.InvalidArgument,
      `argv exceeds maximum of ${MAX_ARGV_ENTRIES} entries`,
    );
  }
  for (const [index, value] of argv.entries()) {
    if (typeof value !== "string" || value.length === 0) {
      throw new SysprimsError(
        SysprimsErrorCode.InvalidArgument,
        `argv[${index}] must be a non-empty string`,
      );
    }
    if (value.length > MAX_ARGV_ENTRY_BYTES) {
      throw new SysprimsError(
        SysprimsErrorCode.InvalidArgument,
        `argv[${index}] exceeds maximum of ${MAX_ARGV_ENTRY_BYTES} bytes`,
      );
    }
  }
}

/**
 * Opaque TypeScript projection of a native-owned contained process.
 *
 * The 64-bit capability token stays on the native side of this object as a
 * bigint and is never exposed as a JavaScript number.
 */
export class ContainedProcess {
  #token: bigint | null;
  readonly #native: ReturnType<typeof loadSysprims>;
  #state: "open" | "closing" | "closed" = "open";
  #closeWaiters: Array<() => void> = [];

  private constructor(token: bigint, native: ReturnType<typeof loadSysprims>) {
    this.#token = token;
    this.#native = native;
    leakBackstop.register(this, token, this);
  }

  /** @internal */
  static fromNative(token: bigint, native: ReturnType<typeof loadSysprims>): ContainedProcess {
    return new ContainedProcess(token, native);
  }

  #requireToken(): bigint {
    if (this.#token == null) {
      throw new SysprimsError(
        SysprimsErrorCode.InvalidArgument,
        "containment handle is stale or closed",
      );
    }
    return this.#token;
  }

  async identity(): Promise<ContainmentSnapshot> {
    const token = this.#requireToken();
    return (await callJsonReturnAsync(() =>
      this.#native.sysprimsContainmentIdentity(token),
    )) as ContainmentSnapshot;
  }

  async poll(): Promise<ContainmentSnapshot> {
    const token = this.#requireToken();
    return (await callJsonReturnAsync(() =>
      this.#native.sysprimsContainmentPoll(token),
    )) as ContainmentSnapshot;
  }

  async wait(options?: ContainedProcessWaitOptions): Promise<ContainmentSnapshot> {
    const token = this.#requireToken();
    if (options?.signal?.aborted) {
      throw new SysprimsError(SysprimsErrorCode.Timeout, "containment wait was cancelled");
    }
    const timeoutMs =
      options?.timeoutMs == null ? 0 : validateDuration(options.timeoutMs, "timeoutMs", MAX_DURATION_MS);
    const waitPromise = callJsonReturnAsync(() =>
      this.#native.sysprimsContainmentWait(token, timeoutMs),
    ) as Promise<ContainmentSnapshot>;
    const abortSignal = options?.signal;
    if (!abortSignal) {
      return waitPromise;
    }
    return await new Promise<ContainmentSnapshot>((resolve, reject) => {
      const onAbort = () => {
        abortSignal.removeEventListener("abort", onAbort);
        reject(new SysprimsError(SysprimsErrorCode.Timeout, "containment wait was cancelled"));
      };
      abortSignal.addEventListener("abort", onAbort, { once: true });
      waitPromise.then(
        (value) => {
          abortSignal.removeEventListener("abort", onAbort);
          resolve(value);
        },
        (error) => {
          abortSignal.removeEventListener("abort", onAbort);
          reject(error);
        },
      );
    });
  }

  async terminate(): Promise<ContainmentSnapshot> {
    const token = this.#requireToken();
    return (await callJsonReturnAsync(() =>
      this.#native.sysprimsContainmentTerminate(token),
    )) as ContainmentSnapshot;
  }

  async close(): Promise<void> {
    for (;;) {
      if (this.#state === "closed" || this.#token == null) {
        return;
      }
      if (this.#state === "closing") {
        await new Promise<void>((resolve) => {
          this.#closeWaiters.push(resolve);
        });
        continue;
      }
      this.#state = "closing";
      const token = this.#token;
      try {
        await callVoidAsync(() => this.#native.sysprimsContainmentClose(token));
        this.#token = null;
        leakBackstop.unregister(this);
        this.#state = "closed";
        return;
      } catch (error) {
        this.#state = "open";
        throw error;
      } finally {
        const waiters = this.#closeWaiters.splice(0);
        for (const waiter of waiters) {
          waiter();
        }
      }
    }
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }
}

export async function spawnContained(
  argv: string[],
  options?: SpawnContainedOptions,
): Promise<ContainedProcess> {
  validateArgv(argv);
  if (options?.cwd != null && options.cwd.length === 0) {
    throw new SysprimsError(SysprimsErrorCode.InvalidArgument, "cwd must not be empty");
  }
  if (options?.executionTimeoutMs != null) {
    validateDuration(options.executionTimeoutMs, "executionTimeoutMs", MAX_DURATION_MS);
    if (options.executionTimeoutMs === 0) {
      throw new SysprimsError(
        SysprimsErrorCode.InvalidArgument,
        "executionTimeoutMs must be between 1 and 86400000",
      );
    }
  }
  if (options?.graceTimeoutMs != null) {
    validateDuration(options.graceTimeoutMs, "graceTimeoutMs", MAX_DURATION_MS);
  }
  if (options?.killTimeoutMs != null) {
    validateDuration(options.killTimeoutMs, "killTimeoutMs", MAX_DURATION_MS);
  }
  if (options?.signal != null) {
    validateSignal(options.signal, "signal");
    if (options.signal === 0) {
      throw new SysprimsError(
        SysprimsErrorCode.InvalidArgument,
        "signal 0 is not a valid termination policy",
      );
    }
  }
  if (options?.killSignal != null) {
    validateSignal(options.killSignal, "killSignal");
    if (options.killSignal === 0) {
      throw new SysprimsError(
        SysprimsErrorCode.InvalidArgument,
        "kill_signal 0 is not a valid termination policy",
      );
    }
  }

  const wire: Record<string, unknown> = {
    schema_id: CONTAINMENT_SPAWN_SCHEMA_ID,
    argv,
  };
  if (options?.cwd) wire.cwd = options.cwd;
  if (options?.env) wire.env = options.env;
  if (options?.executionTimeoutMs != null) wire.execution_timeout_ms = options.executionTimeoutMs;
  if (options?.graceTimeoutMs != null) wire.grace_timeout_ms = options.graceTimeoutMs;
  if (options?.killTimeoutMs != null) wire.kill_timeout_ms = options.killTimeoutMs;
  if (options?.signal != null) wire.signal = options.signal;
  if (options?.killSignal != null) wire.kill_signal = options.killSignal;

  const native = loadSysprims();
  const token = await callTokenReturnAsync(() =>
    native.sysprimsContainmentSpawn(JSON.stringify(wire)),
  );
  return ContainedProcess.fromNative(token, native);
}
