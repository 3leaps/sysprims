// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build unix

package signal

import (
	"errors"
	"fmt"
	"syscall"

	"github.com/3leaps/sysprims/syserror"
	"golang.org/x/sys/unix"
)

func killImpl(pid uint32, sig int) error {
	err := unix.Kill(int(pid), syscall.Signal(sig))
	if err == nil {
		return nil
	}
	return mapUnixSignalError(pid, "signal", sig, err)
}

func killGroupImpl(pgid uint32, sig int) error {
	err := unix.Kill(-int(pgid), syscall.Signal(sig))
	if err == nil {
		return nil
	}
	return mapUnixSignalError(pgid, "signal_group", sig, err)
}

func mapUnixSignalError(id uint32, op string, sig int, err error) error {
	var errno syscall.Errno
	if errors.As(err, &errno) {
		switch errno {
		case unix.EPERM, unix.EACCES:
			return syserror.PermissionDenied(id, op)
		case unix.ESRCH:
			return syserror.NotFound(id)
		case unix.EINVAL:
			return syserror.InvalidArgument(fmt.Sprintf("invalid signal: %d", sig))
		default:
			return syserror.System(errno.Error(), int32(errno))
		}
	}
	return syserror.System(err.Error(), int32(unix.EIO))
}
