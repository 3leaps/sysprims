//go:build linux && amd64 && !musl && sysprims_shared

package sysprims

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib-shared/local/linux-amd64 -L${SRCDIR}/lib-shared/linux-amd64
#cgo LDFLAGS: -Wl,-rpath,${SRCDIR}/lib-shared/local/linux-amd64 -Wl,-rpath,${SRCDIR}/lib-shared/linux-amd64
#cgo LDFLAGS: -lsysprims_ffi -lm -lpthread -ldl
#include "sysprims.h"
*/
import "C"
