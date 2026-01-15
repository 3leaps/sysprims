//go:build linux && arm64 && !musl

package sysprims

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib/local/linux-arm64 -L${SRCDIR}/lib/linux-arm64 -lsysprims_ffi -lm -lpthread -ldl
#include "sysprims.h"
*/
import "C"
