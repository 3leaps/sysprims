//go:build windows && amd64 && sysprims_shared

package sysprims

// Windows builds use MinGW (x86_64-pc-windows-gnu target) for CGo compatibility.
// Shared mode links against the import library (libsysprims_ffi.dll.a) and
// requires sysprims_ffi.dll to be present on PATH or alongside the executable.

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib-shared/local/windows-amd64 -L${SRCDIR}/lib-shared/windows-amd64 -lsysprims_ffi -lws2_32 -luserenv -lbcrypt -lkernel32 -lntdll -ladvapi32 -liphlpapi -lpsapi
#include "sysprims.h"
*/
import "C"
