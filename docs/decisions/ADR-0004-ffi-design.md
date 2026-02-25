# ADR-0004: FFI Design

> **Status**: Accepted
> **Date**: 2025-12-31
> **Authors**: Architecture Council

## Context

sysprims must support bindings for Go, Python, and TypeScript. Each language has different FFI capabilities:

| Language   | FFI Mechanism   | Memory Model      | String Handling                   |
| ---------- | --------------- | ----------------- | --------------------------------- |
| Go         | CGo             | Manual            | Requires copy or careful lifetime |
| Python     | PyO3 / cffi     | Reference counted | UTF-8 with copy                   |
| TypeScript | NAPI-RS / N-API | GC + ref counting | UTF-8 with copy                   |

We need an FFI design that:

1. Works across all target languages
2. Minimizes memory management complexity
3. Provides stable ABI for versioning
4. Avoids complex C structs across boundary

## Decision

### Pattern: Opaque Pointer + JSON String

Instead of exposing complex C structs, we return JSON strings that each language parses natively.

```c
// Instead of this (complex struct):
typedef struct {
    int pid;
    int ppid;
    char* name;  // Who owns this?
    float cpu_percent;
    // ... many fields
} SysprimsProcessInfo;

// We do this (JSON string):
SysprimsErrorCode sysprims_proc_get(uint32_t pid, char** result_json_out);
// Returns: {"pid": 1234, "name": "nginx", ...}
```

### Memory Ownership Contract

**Rule 1**: All `char*` returned by sysprims functions are owned by the caller.

**Rule 2**: Caller MUST free using `sysprims_free_string()`, NEVER C's `free()`.

**Rule 3**: Strings are allocated by Rust's allocator via `CString`.

```c
char* result = NULL;
SysprimsErrorCode err = sysprims_proc_get(1234, &result);
if (err == SYSPRIMS_OK) {
    // Use result...
    sysprims_free_string(result);  // MUST call this
}
```

### Error Handling

**Error Codes**: Returned from every function.

```c
typedef enum {
    SYSPRIMS_OK = 0,
    SYSPRIMS_ERR_INVALID_ARGUMENT = 1,
    SYSPRIMS_ERR_SPAWN_FAILED = 2,
    SYSPRIMS_ERR_TIMEOUT = 3,
    SYSPRIMS_ERR_PERMISSION_DENIED = 4,
    SYSPRIMS_ERR_NOT_FOUND = 5,
    SYSPRIMS_ERR_NOT_SUPPORTED = 6,
    SYSPRIMS_ERR_INTERNAL = 99,
} SysprimsErrorCode;
```

**Error Details**: Thread-local with explicit contract.

```c
// Returns owned string (must free)
// After successful call: returns ""
// Lifetime: Overwritten on next failing call on same thread
char* sysprims_last_error(void);

// For hot paths (no allocation)
SysprimsErrorCode sysprims_last_error_code(void);
```

### String Encoding

All strings are UTF-8 encoded. Invalid UTF-8 inputs result in `SYSPRIMS_ERR_INVALID_ARGUMENT`.

### Array Parameters

Arrays are NOT null-terminated. Length is explicit.

```c
typedef struct {
    const char* command;           // UTF-8 command
    const char* const* args;       // Array of UTF-8 strings
    size_t args_len;               // Number of elements (not null-terminated)
    // ...
} SysprimsTimeoutConfig;
```

### PID Type

PIDs use `uint32_t` to avoid signed overflow on platforms with large PIDs.

### Header Generation

The C header is generated via cbindgen to ensure Rust and C types stay in sync.

```bash
cbindgen --config cbindgen.toml -o ffi/sysprims.h
```

### Opaque Handles (Future)

For long-running operations (e.g., `sysprims-waitfor` polling), we'll use opaque handles:

```c
typedef struct SysprimsHandle SysprimsHandle;

SysprimsErrorCode sysprims_waitfor_start(const SysprimsWaitforConfig* config, SysprimsHandle** handle_out);
SysprimsErrorCode sysprims_waitfor_poll(SysprimsHandle* handle, bool* ready_out);
void sysprims_destroy_handle(SysprimsHandle* handle);
```

## Consequences

### Positive

- Simple memory model (always caller-owned)
- JSON parsing is native in all target languages
- No complex struct versioning across ABI
- cbindgen ensures sync between Rust and C

### Negative

- JSON parsing overhead (acceptable for sysprims's use cases)
- More verbose than direct struct access
- Thread-local error storage requires care

### Neutral

- Bindings do JSON parsing instead of struct mapping
- Schema validation can happen in binding layer
- ABI version field allows compatibility checks

## Alternatives Considered

### Alternative 1: Complex C Structs

Return native C structs for process info, etc.

**Rejected**:

- Memory ownership unclear (who frees nested pointers?)
- Struct layout versioning is fragile
- Different padding/alignment across platforms

### Alternative 2: Protobuf / FlatBuffers

Use a binary serialization format.

**Rejected**:

- Adds dependency complexity
- JSON is human-readable for debugging
- Target languages already have JSON parsing

### Alternative 3: Direct Rust Bindings (PyO3/NAPI-RS only)

Skip C-ABI, use native Rust bindings for each language.

**Rejected**:

- Go requires CGo (C-ABI)
- Maintaining separate binding strategies increases complexity
- C-ABI provides common baseline

### Alternative 4: Caller-Provided Buffers

Caller allocates buffer, sysprims fills it.

**Rejected**:

- Buffer sizing is error-prone
- JSON length is unpredictable
- Rust-allocated strings are simpler

## References

- [cbindgen documentation](https://github.com/eqrion/cbindgen)
- [CString in Rust](https://doc.rust-lang.org/std/ffi/struct.CString.html)
- [seekable-zstd FFI pattern](https://github.com/3leaps/seekable-zstd)
