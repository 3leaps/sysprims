//go:build linux && amd64 && musl && !sysprims_shared

package sysprims

/*
#cgo CFLAGS: -I${SRCDIR}/include
#cgo LDFLAGS: -L${SRCDIR}/lib/local/linux-amd64-musl -L${SRCDIR}/lib/linux-amd64-musl -lsysprims_ffi -lm -lpthread
#include "sysprims.h"
*/
import "C"
