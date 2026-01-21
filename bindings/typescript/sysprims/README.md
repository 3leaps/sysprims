# sysprims (TypeScript bindings)

TypeScript/Node.js bindings for sysprims using the stable C-ABI shared library via `koffi`.

## Platform support

Supported (aligned with ADR-0012):
- macOS: `darwin-arm64`, `darwin-amd64`
- Linux (glibc only): `linux-arm64`, `linux-amd64`
- Windows (x64): `windows-amd64` (MSVC `sysprims_ffi.dll`)

Not supported:
- Linux musl (Alpine): this package refuses to load on musl. Use a glibc-based image.

## Library loading

At runtime, the binding loads the shared library from:

`_lib/<platform>/<filename>`

- Linux: `_lib/<platform>/libsysprims_ffi.so`
- macOS: `_lib/<platform>/libsysprims_ffi.dylib`
- Windows: `_lib/<platform>/sysprims_ffi.dll`

The loader verifies `sysprims_abi_version()` matches the expected ABI and fails fast if it does not.

### Local development (populate `_lib/`)

This repository does not commit shared libraries. For local dev:

1. Build a local shared library:
   - `make build-local-ffi-shared`
2. Vendor it into the TS package:
   - `npm run vendor:local` (from this directory)

This copies from `dist/local/release/sysprims-ffi/lib/<platform>/shared/` into `_lib/<platform>/`.

## API (minimal)

- `procGet(pid)` → returns parsed JSON from `sysprims_proc_get`
- `selfPGID()` → calls `sysprims_self_getpgid`
- `selfSID()` → calls `sysprims_self_getsid`

## Safety

These bindings call into a process-control library. Do not use dangerous PIDs.
