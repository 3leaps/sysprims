export type JsonObject = Record<string, unknown>;

export type ProcessInfo = JsonObject & {
  pid: number;
};

