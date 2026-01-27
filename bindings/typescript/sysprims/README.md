# sysprims (TypeScript bindings)

TypeScript/Node.js bindings for sysprims using a Node-API (N-API) native addon (napi-rs).

## Platform support

Supported:
- macOS: arm64
- Linux: glibc and musl (Alpine)
- Windows: x64, arm64 (msvc)

## Local development

The native addon is built from Rust:

```bash
npm install
npm run build
npm run build:native
npm run test:ci
```

## API (minimal)

- `procGet(pid)` → returns parsed JSON from `sysprims_proc_get`
- `selfPGID()` → calls `sysprims_self_getpgid`
- `selfSID()` → calls `sysprims_self_getsid`

## Safety

These bindings call into a process-control library. Do not use dangerous PIDs.
