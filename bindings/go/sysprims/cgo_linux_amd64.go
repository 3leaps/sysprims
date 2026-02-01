//go:build linux && amd64 && !musl && !sysprims_shared

package sysprims

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib/local/linux-amd64 -L${SRCDIR}/lib/linux-amd64 -lsysprims_ffi -lm -lpthread -ldl
#include "sysprims.h"
*/
import "C"
