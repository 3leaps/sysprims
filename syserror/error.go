// SPDX-License-Identifier: MIT OR Apache-2.0

// Package syserror defines the canonical sysprims error codes and structured
// error type shared across pure-Go sysprims packages (CGO_ENABLED=0).
//
// Error codes and messages match crates/sysprims-core::SysprimsError per
// ADR-0008. Error messages never include process argv, executable paths, or
// working directories.
package syserror

import (
	"errors"
	"fmt"
)

// ErrorCode represents canonical sysprims error codes per ADR-0008.
type ErrorCode int32

const (
	// ErrOK indicates no error - operation succeeded.
	ErrOK ErrorCode = 0
	// ErrInvalidArgument indicates an invalid argument was provided.
	ErrInvalidArgument ErrorCode = 1
	// ErrSpawnFailed indicates the child process failed to spawn.
	ErrSpawnFailed ErrorCode = 2
	// ErrTimeout indicates the operation timed out.
	ErrTimeout ErrorCode = 3
	// ErrPermissionDenied indicates permission was denied for the operation.
	ErrPermissionDenied ErrorCode = 4
	// ErrNotFound indicates the process or command was not found.
	ErrNotFound ErrorCode = 5
	// ErrNotSupported indicates the operation is not supported on this platform.
	ErrNotSupported ErrorCode = 6
	// ErrGroupCreationFailed indicates process group or job object creation failed.
	ErrGroupCreationFailed ErrorCode = 7
	// ErrSystem indicates a system-level error (errno or Win32 error code).
	ErrSystem ErrorCode = 8
	// ErrInternal indicates an internal error in sysprims.
	ErrInternal ErrorCode = 99
)

// String returns the canonical snake-free variant name for the error code.
func (c ErrorCode) String() string {
	switch c {
	case ErrOK:
		return "OK"
	case ErrInvalidArgument:
		return "InvalidArgument"
	case ErrSpawnFailed:
		return "SpawnFailed"
	case ErrTimeout:
		return "Timeout"
	case ErrPermissionDenied:
		return "PermissionDenied"
	case ErrNotFound:
		return "NotFound"
	case ErrNotSupported:
		return "NotSupported"
	case ErrGroupCreationFailed:
		return "GroupCreationFailed"
	case ErrSystem:
		return "System"
	case ErrInternal:
		return "Internal"
	default:
		return "Unknown"
	}
}

// Category returns the canonical snake_case category identifier used in
// cross-language conformance vectors.
func (c ErrorCode) Category() string {
	switch c {
	case ErrOK:
		return "ok"
	case ErrInvalidArgument:
		return "invalid_argument"
	case ErrSpawnFailed:
		return "spawn_failed"
	case ErrTimeout:
		return "timeout"
	case ErrPermissionDenied:
		return "permission_denied"
	case ErrNotFound:
		return "not_found"
	case ErrNotSupported:
		return "not_supported"
	case ErrGroupCreationFailed:
		return "group_creation_failed"
	case ErrSystem:
		return "system"
	case ErrInternal:
		return "internal"
	default:
		return "unknown"
	}
}

// Error represents a structured sysprims error with a canonical ErrorCode and
// message.
type Error struct {
	Code      ErrorCode `json:"code"`
	Message   string    `json:"message"`
	PID       uint32    `json:"pid,omitempty"`
	Operation string    `json:"operation,omitempty"`
	Errno     int32     `json:"errno,omitempty"`
}

// Error implements the error interface.
func (e *Error) Error() string {
	if e == nil {
		return ""
	}
	if e.Message != "" {
		return e.Message
	}
	return e.Code.String()
}

// Is supports errors.Is matching against another *Error by ErrorCode.
func (e *Error) Is(target error) bool {
	if e == nil {
		return target == nil
	}
	t, ok := target.(*Error)
	if !ok || t == nil {
		return false
	}
	return e.Code == t.Code
}

// InvalidArgument constructs an ErrInvalidArgument error matching
// SysprimsError::invalid_argument.
func InvalidArgument(message string) *Error {
	return &Error{
		Code:    ErrInvalidArgument,
		Message: "Invalid argument: " + message,
	}
}

// InvalidArgumentf constructs a formatted ErrInvalidArgument error.
func InvalidArgumentf(format string, args ...any) *Error {
	return InvalidArgument(fmt.Sprintf(format, args...))
}

// NotFound constructs an ErrNotFound error for a numeric PID matching
// SysprimsError::not_found.
func NotFound(pid uint32) *Error {
	return &Error{
		Code:    ErrNotFound,
		Message: fmt.Sprintf("Process %d not found", pid),
		PID:     pid,
	}
}

// NotFoundCommand constructs an ErrNotFound error for a command name matching
// SysprimsError::not_found_command.
func NotFoundCommand(command string) *Error {
	return &Error{
		Code:    ErrNotFound,
		Message: fmt.Sprintf("Command '%s' not found", command),
	}
}

// PermissionDenied constructs an ErrPermissionDenied error for a PID and
// operation matching SysprimsError::permission_denied.
func PermissionDenied(pid uint32, operation string) *Error {
	return &Error{
		Code:      ErrPermissionDenied,
		Message:   fmt.Sprintf("Permission denied for '%s' on PID %d", operation, pid),
		PID:       pid,
		Operation: operation,
	}
}

// NotSupported constructs an ErrNotSupported error matching
// SysprimsError::not_supported.
func NotSupported(feature, platform string) *Error {
	return &Error{
		Code:      ErrNotSupported,
		Message:   fmt.Sprintf("Operation '%s' not supported on %s", feature, platform),
		Operation: feature,
	}
}

// Timeout constructs an ErrTimeout error matching SysprimsError::Timeout.
func Timeout() *Error {
	return &Error{
		Code:    ErrTimeout,
		Message: "Operation timed out",
	}
}

// SpawnFailed constructs an ErrSpawnFailed error matching
// SysprimsError::spawn_failed.
func SpawnFailed(reason string) *Error {
	return &Error{
		Code:    ErrSpawnFailed,
		Message: "Failed to spawn process: " + reason,
	}
}

// GroupCreationFailed constructs an ErrGroupCreationFailed error matching
// SysprimsError::group_creation_failed.
func GroupCreationFailed(message string) *Error {
	return &Error{
		Code:    ErrGroupCreationFailed,
		Message: "Failed to create process group: " + message,
	}
}

// System constructs an ErrSystem error matching SysprimsError::system.
func System(message string, errno int32) *Error {
	return &Error{
		Code:    ErrSystem,
		Message: fmt.Sprintf("System error: %s (errno: %d)", message, errno),
		Errno:   errno,
	}
}

// Internal constructs an ErrInternal error matching SysprimsError::internal.
func Internal(message string) *Error {
	return &Error{
		Code:    ErrInternal,
		Message: "Internal error: " + message,
	}
}

// CodeOf extracts the ErrorCode from err if it is or wraps a *Error.
func CodeOf(err error) (ErrorCode, bool) {
	if err == nil {
		return ErrOK, true
	}
	var target *Error
	if errors.As(err, &target) && target != nil {
		return target.Code, true
	}
	return ErrInternal, false
}

// IsCode reports whether err is or wraps a *Error with the given ErrorCode.
func IsCode(err error, code ErrorCode) bool {
	got, ok := CodeOf(err)
	return ok && got == code
}
