//go:build linux && arm64 && musl && sysprims_shared && sysprims_shared_local

package sysprims

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib-shared/local/linux-arm64-musl -L${SRCDIR}/lib-shared/linux-arm64-musl
#cgo LDFLAGS: -Wl,-rpath,${SRCDIR}/lib-shared/local/linux-arm64-musl -Wl,-rpath,${SRCDIR}/lib-shared/linux-arm64-musl
#cgo LDFLAGS: -lsysprims_ffi -lm -lpthread -ldl
#include "sysprims.h"
*/
import "C"
