//go:build windows && amd64

package sysprims

// Windows builds use MinGW (x86_64-pc-windows-gnu target) for CGo compatibility.
// The FFI library is libsysprims_ffi.a (not .lib) to work with MinGW linker.
// See: docs/architecture/adr/0012-language-bindings-distribution.md

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib/local/windows-amd64 -L${SRCDIR}/lib/windows-amd64 -lsysprims_ffi -lws2_32 -luserenv -lbcrypt
#include "sysprims.h"
*/
import "C"
