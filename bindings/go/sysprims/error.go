package sysprims

/*
#include "sysprims.h"
*/
import "C"
import "runtime"

// ErrorCode represents sysprims FFI error codes.
//
// These map directly to the underlying C error codes.
type ErrorCode int32

// Error codes returned by sysprims functions.
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
	// ErrSystem indicates a system-level error (errno/GetLastError).
	ErrSystem ErrorCode = 8
	// ErrInternal indicates an internal error (bug in sysprims).
	ErrInternal ErrorCode = 99
)

// String returns a human-readable name for the error code.
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

// Error represents a sysprims error with code and message.
//
// Use type assertion to access detailed error information:
//
//	err := sysprims.Kill(pid, sysprims.SIGTERM)
//	if err != nil {
//	    if sErr, ok := err.(*sysprims.Error); ok {
//	        fmt.Printf("Error code: %d (%s)\n", sErr.Code, sErr.Code)
//	    }
//	}
type Error struct {
	// Code is the error code returned by the FFI function.
	Code ErrorCode
	// Message is a detailed error message from the library.
	Message string
}

// Error implements the error interface.
func (e *Error) Error() string {
	if e.Message != "" {
		return e.Message
	}
	return e.Code.String()
}

// callAndCheck executes an FFI call and converts the returned code to a Go error.
//
// Important: sysprims stores error details in thread-local storage (TLS). Go
// goroutines can move between OS threads between cgo calls, so we lock the OS
// thread to ensure `sysprims_last_error()` reads the error for the same thread
// that performed the failing call.
func callAndCheck(call func() C.SysprimsErrorCode) error {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()

	code := call()
	if code == C.SYSPRIMS_ERROR_CODE_OK {
		return nil
	}

	msgPtr := C.sysprims_last_error()
	defer C.sysprims_free_string(msgPtr)

	return &Error{
		Code:    ErrorCode(code),
		Message: C.GoString(msgPtr),
	}
}
