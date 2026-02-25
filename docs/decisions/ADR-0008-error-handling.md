# ADR-0008: Error Handling Strategy

> **Status**: Accepted
> **Date**: 2025-12-31
> **Authors**: Architecture Council

## Context

sysprims needs consistent error handling across:

1. Rust library API
2. CLI tools
3. FFI boundary (C-ABI)
4. Language bindings (Go, Python, TypeScript)

Errors must be:

- Informative for debugging
- Structured for programmatic handling
- Consistent across all interfaces
- Secure (no sensitive information leakage)

## Decision

### Error Type Hierarchy

Core error type using `thiserror`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum SysprimsError {
    #[error("Invalid argument: {message}")]
    InvalidArgument { message: String },

    #[error("Failed to spawn process: {source}")]
    SpawnFailed {
        #[source]
        source: std::io::Error
    },

    #[error("Operation timed out")]
    Timeout,

    #[error("Permission denied for {operation} on PID {pid}")]
    PermissionDenied { pid: u32, operation: String },

    #[error("Process {pid} not found")]
    NotFound { pid: u32 },

    #[error("Operation '{feature}' not supported on {platform}")]
    NotSupported { feature: String, platform: String },

    #[error("Internal error: {message}")]
    Internal { message: String },
}
```

### Error Codes (FFI)

Flat error codes for C-ABI:

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

### Error Details (FFI)

Thread-local error message:

```c
// After a failing call:
SysprimsErrorCode code = osu_last_error_code();  // No allocation
char* message = osu_last_error();           // Allocates; must free
```

Contract:

- After successful call: `osu_last_error()` returns `""`
- After failing call: `osu_last_error()` returns descriptive message
- Thread-local: Each thread has independent error state
- Lifetime: Overwritten on next osu call on same thread

### CLI Exit Codes

CLI tools map errors to exit codes:

| Error             | Exit Code | Notes                  |
| ----------------- | --------- | ---------------------- |
| Success           | 0         |                        |
| Invalid argument  | 1         |                        |
| Spawn failed      | 125       | GNU timeout compatible |
| Not executable    | 126       | GNU compatible         |
| Command not found | 127       | GNU compatible         |
| Timeout           | 124       | GNU timeout compatible |
| Signal N          | 128+N     | GNU compatible         |
| Permission denied | 1         |                        |
| Internal error    | 1         |                        |

### Binding Error Mapping

Each binding maps to idiomatic error handling:

**Go**:

```go
type SysprimsError struct {
    Code    SysprimsErrorCode
    Message string
}

func (e *SysprimsError) Error() string {
    return e.Message
}

// Type assertions for specific errors
type PermissionError struct {
    SysprimsError
    PID uint32
}
```

**Python**:

```python
class SysprimsError(Exception):
    def __init__(self, code: int, message: str):
        self.code = code
        self.message = message
        super().__init__(message)

class PermissionDeniedError(SysprimsError):
    def __init__(self, pid: int, message: str):
        self.pid = pid
        super().__init__(SYSPRIMS_ERR_PERMISSION_DENIED, message)
```

**TypeScript**:

```typescript
export class SysprimsError extends Error {
  constructor(
    public code: SysprimsErrorCode,
    message: string,
  ) {
    super(message);
    this.name = "SysprimsError";
  }
}

export class PermissionDeniedError extends SysprimsError {
  constructor(
    public pid: number,
    message: string,
  ) {
    super(SysprimsErrorCode.PermissionDenied, message);
    this.name = "PermissionDeniedError";
  }
}
```

### Error Message Guidelines

1. **Be specific**: Include relevant context (PID, operation, platform)
2. **Be secure**: Never include file paths, environment variables, or credentials
3. **Be actionable**: Suggest what the user can do
4. **Be consistent**: Use same phrasing across crates

Good:

```
"Permission denied for 'terminate' on PID 1234"
"Signal HUP not supported on Windows; use TERM instead"
```

Bad:

```
"Error"
"Permission denied at /home/user/.config/secret.txt"
```

## Consequences

### Positive

- Consistent error handling across all interfaces
- GNU-compatible exit codes for CLI
- Idiomatic errors in each language
- Secure by default (no sensitive info)

### Negative

- Error mapping overhead in bindings
- Thread-local state has complexity
- Must maintain consistency across updates

### Neutral

- Error documentation required
- Test coverage for error paths
- Exit code table in docs

## Alternatives Considered

### Alternative 1: String-Only Errors

Return error strings only, no structured types.

**Rejected**: Can't programmatically distinguish error types; poor for automation.

### Alternative 2: Exception Numbers Only

FFI returns only error code, no message.

**Rejected**: Poor debugging experience; users can't understand what went wrong.

### Alternative 3: Result<T, String>

Simple string errors in Rust.

**Rejected**: Loses structure; can't match on error types; poor for libraries.

## References

- [thiserror crate](https://docs.rs/thiserror)
- [GNU coreutils exit codes](https://www.gnu.org/software/coreutils/manual/html_node/Exit-status.html)
- [Rust Error Handling](https://doc.rust-lang.org/book/ch09-00-error-handling.html)
