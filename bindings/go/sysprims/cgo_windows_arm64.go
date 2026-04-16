//go:build windows && arm64 && !sysprims_shared

package sysprims

// Windows arm64 builds use llvm-mingw (aarch64-pc-windows-gnullvm target) for
// CGo compatibility. The FFI library is libsysprims_ffi.a (GNU ABI) to link
// against Go's cgo-compatible GCC driver (aarch64-w64-mingw32-gcc from llvm-mingw).
// See: docs/decisions/ADR-0012-language-bindings-distribution.md

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib/local/windows-arm64 -L${SRCDIR}/lib/windows-arm64 -lsysprims_ffi -lws2_32 -luserenv -lbcrypt -lkernel32 -lntdll -ladvapi32 -liphlpapi -lpsapi
#include "sysprims.h"
*/
import "C"
