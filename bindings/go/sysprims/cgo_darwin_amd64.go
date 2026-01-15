//go:build darwin && amd64

package sysprims

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib/local/darwin-amd64 -L${SRCDIR}/lib/darwin-amd64 -lsysprims_ffi -lm -lpthread
#include "sysprims.h"
*/
import "C"
