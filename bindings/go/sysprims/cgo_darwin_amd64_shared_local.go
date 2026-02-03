//go:build darwin && amd64 && sysprims_shared && sysprims_shared_local

package sysprims

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib-shared/local/darwin-amd64 -L${SRCDIR}/lib-shared/darwin-amd64
#cgo LDFLAGS: -Wl,-rpath,${SRCDIR}/lib-shared/local/darwin-amd64 -Wl,-rpath,${SRCDIR}/lib-shared/darwin-amd64
#cgo LDFLAGS: -lsysprims_ffi -lm -lpthread
#include "sysprims.h"
*/
import "C"
