//go:build darwin && arm64 && sysprims_shared

package sysprims

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib-shared/local/darwin-arm64 -L${SRCDIR}/lib-shared/darwin-arm64
#cgo LDFLAGS: -Wl,-rpath,${SRCDIR}/lib-shared/local/darwin-arm64 -Wl,-rpath,${SRCDIR}/lib-shared/darwin-arm64
#cgo LDFLAGS: -lsysprims_ffi -lm -lpthread
#include "sysprims.h"
*/
import "C"
