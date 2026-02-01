//go:build darwin && arm64 && !sysprims_shared

package sysprims

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib/local/darwin-arm64 -L${SRCDIR}/lib/darwin-arm64 -lsysprims_ffi -lm -lpthread
#include "sysprims.h"
*/
import "C"
