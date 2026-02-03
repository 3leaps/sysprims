//go:build linux && arm64 && !musl && sysprims_shared && sysprims_shared_local

package sysprims

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib-shared/local/linux-arm64 -L${SRCDIR}/lib-shared/linux-arm64
#cgo LDFLAGS: -Wl,-rpath,${SRCDIR}/lib-shared/local/linux-arm64 -Wl,-rpath,${SRCDIR}/lib-shared/linux-arm64
#cgo LDFLAGS: -lsysprims_ffi -lm -lpthread -ldl
#include "sysprims.h"
*/
import "C"
