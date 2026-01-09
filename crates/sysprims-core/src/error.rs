//! Error types for sysprims operations.
//!
//! This module defines the error taxonomy per ADR-0008:
//! - [`SysprimsError`] - Canonical error type for all sysprims operations
//!
//! ## Design Principles
//!
//! - **Structured**: Errors carry typed context (pid, operation) not just messages
//! - **FFI-friendly**: Maps cleanly to error codes for C-ABI
//! - **ABI-aligned**: Uses `u32` for PIDs (unsigned for cross-platform consistency)
//! - **Secure**: No sensitive information (paths, credentials) in error messages
//!
//! See ADR-0008 for the full error handling strategy.

use std::io;
use thiserror::Error;

// ============================================================================
// Canonical Error Type (per ADR-0008)
// ============================================================================

/// Canonical error type for all sysprims operations.
///
/// This is the single error type used across the sysprims ecosystem. It maps
/// cleanly to FFI error codes and provides structured context for programmatic
/// handling.
///
/// ## FFI Error Code Mapping
///
/// | Variant | FFI Code |
/// |---------|----------|
/// | `InvalidArgument` | `SYSPRIMS_ERR_INVALID_ARGUMENT` (1) |
/// | `SpawnFailed` | `SYSPRIMS_ERR_SPAWN_FAILED` (2) |
/// | `Timeout` | `SYSPRIMS_ERR_TIMEOUT` (3) |
/// | `PermissionDenied` | `SYSPRIMS_ERR_PERMISSION_DENIED` (4) |
/// | `NotFound` | `SYSPRIMS_ERR_NOT_FOUND` (5) |
/// | `NotSupported` | `SYSPRIMS_ERR_NOT_SUPPORTED` (6) |
/// | `GroupCreationFailed` | `SYSPRIMS_ERR_GROUP_CREATION_FAILED` (7) |
/// | `System` | `SYSPRIMS_ERR_SYSTEM` (8) |
/// | `Internal` | `SYSPRIMS_ERR_INTERNAL` (99) |
#[derive(Debug, Error)]
pub enum SysprimsError {
    /// Invalid argument provided.
    ///
    /// Returned when input validation fails (e.g., pid = 0, empty command).
    #[error("Invalid argument: {message}")]
    InvalidArgument {
        /// Description of what was invalid.
        message: String,
    },

    /// Failed to spawn a child process.
    ///
    /// Wraps the underlying IO error from process creation.
    #[error("Failed to spawn process: {source}")]
    SpawnFailed {
        /// The underlying IO error.
        #[source]
        source: io::Error,
    },

    /// Operation timed out.
    ///
    /// The child process did not complete within the deadline.
    #[error("Operation timed out")]
    Timeout,

    /// Permission denied for the operation.
    ///
    /// Typically returned when signaling a process owned by another user.
    #[error("Permission denied for '{operation}' on PID {pid}")]
    PermissionDenied {
        /// The process ID we attempted to operate on.
        pid: u32,
        /// The operation that was denied (e.g., "terminate", "signal").
        operation: String,
    },

    /// Target process not found.
    ///
    /// The specified PID does not exist or has already exited.
    #[error("Process {pid} not found")]
    NotFound {
        /// The process ID that was not found.
        pid: u32,
    },

    /// Command not found.
    ///
    /// The specified command could not be found in PATH.
    #[error("Command '{command}' not found")]
    NotFoundCommand {
        /// The command that was not found.
        command: String,
    },

    /// Permission denied for command execution.
    ///
    /// The specified command exists but cannot be executed (e.g., not executable).
    #[error("Permission denied: cannot execute '{command}'")]
    PermissionDeniedCommand {
        /// The command that could not be executed.
        command: String,
    },

    /// Operation not supported on the current platform.
    ///
    /// Some operations are platform-specific (e.g., `killpg` on Windows).
    #[error("Operation '{feature}' not supported on {platform}")]
    NotSupported {
        /// The feature that is not supported.
        feature: String,
        /// The platform where it's not supported.
        platform: String,
    },

    /// Failed to create process group or job object.
    ///
    /// On Unix, this means `setpgid()` failed.
    /// On Windows, this means Job Object creation failed.
    #[error("Failed to create process group: {message}")]
    GroupCreationFailed {
        /// Description of what failed.
        message: String,
    },

    /// System-level error with errno/GetLastError context.
    ///
    /// Used when a syscall fails with an unexpected error code.
    #[error("System error: {message} (errno: {errno})")]
    System {
        /// Description of the error.
        message: String,
        /// The errno value (Unix) or GetLastError (Windows).
        errno: i32,
    },

    /// Internal error (should not happen in normal operation).
    ///
    /// Indicates a bug in sysprims or unexpected system state.
    #[error("Internal error: {message}")]
    Internal {
        /// Description of the internal error.
        message: String,
    },
}

impl SysprimsError {
    /// Get the FFI error code for this error.
    ///
    /// Maps to `SysprimsErrorCode` enum in C-ABI.
    pub fn error_code(&self) -> i32 {
        match self {
            SysprimsError::InvalidArgument { .. } => 1,
            SysprimsError::SpawnFailed { .. } => 2,
            SysprimsError::Timeout => 3,
            SysprimsError::PermissionDenied { .. } => 4,
            SysprimsError::PermissionDeniedCommand { .. } => 4,
            SysprimsError::NotFound { .. } => 5,
            SysprimsError::NotFoundCommand { .. } => 5,
            SysprimsError::NotSupported { .. } => 6,
            SysprimsError::GroupCreationFailed { .. } => 7,
            SysprimsError::System { .. } => 8,
            SysprimsError::Internal { .. } => 99,
        }
    }
}

// ============================================================================
// Convenience Constructors
// ============================================================================

impl SysprimsError {
    /// Create an `InvalidArgument` error.
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        SysprimsError::InvalidArgument {
            message: message.into(),
        }
    }

    /// Create a `SpawnFailed` error from an IO error.
    pub fn spawn_failed_io(source: io::Error) -> Self {
        SysprimsError::SpawnFailed { source }
    }

    /// Create a `PermissionDenied` error.
    pub fn permission_denied(pid: u32, operation: impl Into<String>) -> Self {
        SysprimsError::PermissionDenied {
            pid,
            operation: operation.into(),
        }
    }

    /// Create a `NotFound` error.
    pub fn not_found(pid: u32) -> Self {
        SysprimsError::NotFound { pid }
    }

    /// Create a `NotFoundCommand` error.
    pub fn not_found_command(command: impl Into<String>) -> Self {
        SysprimsError::NotFoundCommand {
            command: command.into(),
        }
    }

    /// Create a `PermissionDeniedCommand` error.
    pub fn permission_denied_command(command: impl Into<String>) -> Self {
        SysprimsError::PermissionDeniedCommand {
            command: command.into(),
        }
    }

    /// Create a `SpawnFailed` error with a command and reason.
    pub fn spawn_failed(command: impl Into<String>, reason: impl Into<String>) -> Self {
        let msg = format!("{}: {}", command.into(), reason.into());
        SysprimsError::SpawnFailed {
            source: io::Error::other(msg),
        }
    }

    /// Create a `NotSupported` error.
    pub fn not_supported(feature: impl Into<String>, platform: impl Into<String>) -> Self {
        SysprimsError::NotSupported {
            feature: feature.into(),
            platform: platform.into(),
        }
    }

    /// Create a `GroupCreationFailed` error.
    pub fn group_creation_failed(message: impl Into<String>) -> Self {
        SysprimsError::GroupCreationFailed {
            message: message.into(),
        }
    }

    /// Create a `System` error.
    pub fn system(message: impl Into<String>, errno: i32) -> Self {
        SysprimsError::System {
            message: message.into(),
            errno,
        }
    }

    /// Create an `Internal` error.
    pub fn internal(message: impl Into<String>) -> Self {
        SysprimsError::Internal {
            message: message.into(),
        }
    }
}

// ============================================================================
// Conversions
// ============================================================================

impl From<io::Error> for SysprimsError {
    fn from(source: io::Error) -> Self {
        // Map common IO errors to structured variants
        match source.kind() {
            io::ErrorKind::NotFound => SysprimsError::Internal {
                message: format!("IO not found: {}", source),
            },
            io::ErrorKind::PermissionDenied => SysprimsError::Internal {
                message: format!("IO permission denied: {}", source),
            },
            _ => SysprimsError::SpawnFailed { source },
        }
    }
}

// ============================================================================
// Result Type Alias
// ============================================================================

/// Result type alias for sysprims operations.
pub type SysprimsResult<T> = Result<T, SysprimsError>;

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = SysprimsError::invalid_argument("pid must be > 0");
        assert_eq!(err.to_string(), "Invalid argument: pid must be > 0");

        let err = SysprimsError::permission_denied(1234, "terminate");
        assert_eq!(
            err.to_string(),
            "Permission denied for 'terminate' on PID 1234"
        );

        let err = SysprimsError::not_found(5678);
        assert_eq!(err.to_string(), "Process 5678 not found");

        let err = SysprimsError::not_supported("killpg", "windows");
        assert_eq!(
            err.to_string(),
            "Operation 'killpg' not supported on windows"
        );

        let err = SysprimsError::Timeout;
        assert_eq!(err.to_string(), "Operation timed out");
    }

    #[test]
    fn test_error_codes() {
        assert_eq!(SysprimsError::invalid_argument("").error_code(), 1);
        assert_eq!(
            SysprimsError::spawn_failed_io(io::Error::other("test")).error_code(),
            2
        );
        assert_eq!(SysprimsError::Timeout.error_code(), 3);
        assert_eq!(SysprimsError::permission_denied(0, "").error_code(), 4);
        assert_eq!(SysprimsError::not_found(0).error_code(), 5);
        assert_eq!(SysprimsError::not_supported("", "").error_code(), 6);
        assert_eq!(SysprimsError::group_creation_failed("").error_code(), 7);
        assert_eq!(SysprimsError::system("", 0).error_code(), 8);
        assert_eq!(SysprimsError::internal("").error_code(), 99);
    }

    #[test]
    fn test_spawn_failed_source() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "command not found");
        let err = SysprimsError::spawn_failed_io(io_err);

        // Verify source is accessible
        match err {
            SysprimsError::SpawnFailed { ref source } => {
                assert_eq!(source.kind(), io::ErrorKind::NotFound);
            }
            _ => panic!("Expected SpawnFailed"),
        }
    }

    #[test]
    fn test_pid_is_u32() {
        // Verify PIDs are unsigned (ABI alignment per ADR-0008)
        let err = SysprimsError::permission_denied(u32::MAX, "signal");
        match err {
            SysprimsError::PermissionDenied { pid, .. } => {
                assert_eq!(pid, u32::MAX);
            }
            _ => panic!("Expected PermissionDenied"),
        }

        let err = SysprimsError::not_found(u32::MAX);
        match err {
            SysprimsError::NotFound { pid } => {
                assert_eq!(pid, u32::MAX);
            }
            _ => panic!("Expected NotFound"),
        }
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = io::Error::other("test error");
        let sysprims_err: SysprimsError = io_err.into();

        match sysprims_err {
            SysprimsError::SpawnFailed { source } => {
                assert_eq!(source.kind(), io::ErrorKind::Other);
            }
            _ => panic!("Expected SpawnFailed from IO error"),
        }
    }
}
