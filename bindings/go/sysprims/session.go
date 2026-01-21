package sysprims

/*
#include "sysprims.h"
*/
import "C"

// SelfPGID returns the current process group ID (PGID).
//
// Platform notes:
// - Unix: calls getpgid(0)
// - Windows: returns [ErrNotSupported]
func SelfPGID() (uint32, error) {
	var pgid C.uint32_t
	if err := callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_self_getpgid(&pgid)
	}); err != nil {
		return 0, err
	}
	return uint32(pgid), nil
}

// SelfSID returns the current session ID (SID).
//
// Platform notes:
// - Unix: calls getsid(0)
// - Windows: returns [ErrNotSupported]
func SelfSID() (uint32, error) {
	var sid C.uint32_t
	if err := callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_self_getsid(&sid)
	}); err != nil {
		return 0, err
	}
	return uint32(sid), nil
}
