//go:build windows && arm64 && sysprims_shared && sysprims_shared_local

package sysprims

// Windows arm64 shared builds use llvm-mingw (aarch64-pc-windows-gnullvm).
// Links against the GNU import library (libsysprims_ffi.dll.a) and requires
// sysprims_ffi.dll to be present on PATH or alongside the executable.

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib-shared/local/windows-arm64 -L${SRCDIR}/lib-shared/windows-arm64 -lsysprims_ffi -lws2_32 -luserenv -lbcrypt -lkernel32 -lntdll -ladvapi32 -liphlpapi -lpsapi
#include "sysprims.h"
*/
import "C"
