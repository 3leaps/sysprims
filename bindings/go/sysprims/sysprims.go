// Package sysprims provides Go bindings for the sysprims process utilities library.
//
// sysprims offers GPL-free, cross-platform process control primitives with
// group-by-default behavior - when you timeout a process, the entire tree dies.
//
// # Replacing Shell-outs (v0.1.14)
//
// Prefer typed APIs over parsing process tool output:
//   - `ps eww -p <pid>` -> [ProcessGetWithOptions] with [ProcessOptions.IncludeEnv]
//   - `ps -M -p <pid>` -> [ProcessGetWithOptions] with [ProcessOptions.IncludeThreads]
//   - `lsof -p <pid>` -> [ListFds]
//   - `kill -9 <pid>` -> [Kill] with [SIGKILL]
//   - `kill` loops for process trees -> [KillDescendantsWithOptions] with filter + [CpuModeMonitor]
//
// This keeps behavior cross-platform and avoids fragile text parsing.
//
// # Memory Management
//
// All memory management is handled automatically by the Go bindings.
// You do not need to free strings or other resources returned by this package.
//
// # Error Handling
//
// Functions return Go errors that wrap sysprims error codes. Use type assertion
// to access the underlying [Error] type for detailed error information.
//
// # Thread Safety
//
// The underlying sysprims library uses thread-local storage for error state
// (per OS thread). The Go bindings fetch the error message immediately after
// each failing FFI call, so callers do not need to manage thread affinity.
//
// # Platform Notes
//
// Some operations have platform-specific behavior:
//   - [KillGroup] returns [ErrNotSupported] on Windows
//   - Signal mapping differs between Unix and Windows (see [Kill] documentation)
package sysprims

/*
#include "sysprims.h"
#include <stdlib.h>
*/
import "C"

// Version returns the sysprims library version string.
//
// The returned string is in semver format (e.g., "0.1.2").
func Version() string {
	cVer := C.sysprims_version()
	// Static string from library, do not free
	return C.GoString(cVer)
}

// ABIVersion returns the FFI ABI version number.
//
// Use this to verify compatibility between the Go bindings and the
// underlying library. If the ABI version changes, the bindings may
// not work correctly.
//
// The current bindings expect ABI version 1.
func ABIVersion() uint32 {
	return uint32(C.sysprims_abi_version())
}

// Platform returns the current platform name.
//
// Returns one of: "linux", "macos", "windows", "freebsd", etc.
func Platform() string {
	cPlatform := C.sysprims_get_platform()
	defer C.sysprims_free_string(cPlatform)
	return C.GoString(cPlatform)
}

// ClearError clears the thread-local error state.
//
// This is rarely needed as each operation clears the error state
// before executing. Provided for completeness.
func ClearError() {
	C.sysprims_clear_error()
}
